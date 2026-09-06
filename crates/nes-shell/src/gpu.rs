//! The picture on the GPU (N8 step 1): ntsc-crt's three-line comb
//! decode and its five CRT stages as eight compute passes over storage
//! buffers (`picture.wgsl`), every constant uploaded from the `Decoder`
//! and `CrtParams` instances. Held to the CPU chain pixel for pixel in
//! tests/gpu.rs; the window blits `final_out` (`blit.wgsl`).

use ntsc_crt::{CrtParams, DisplayFrame};
use ntsc_decode::Decoder;
use ntsc_grid::CompositeFrame;

const PASSES: [&str; 8] = ["decimate", "uv_filter", "rgb", "beam_pass", "scanlines", "persist", "mask", "geometry"];

/// The uniform block, laid out as `picture.wgsl` declares it (scalars
/// at four bytes, the two vec4s at sixteen).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Params {
    n: u32,
    nd: u32,
    d: u32,
    rows: u32,
    row0: u32,
    width: u32,
    taps: u32,
    uv_half: u32,
    out_w: u32,
    out_h: u32,
    scale: u32,
    flags: u32,
    black: f32,
    scale_y: f32,
    amp_k: f32,
    gamma: f32,
    r_from_v: f32,
    g_from_u: f32,
    g_from_v: f32,
    b_from_u: f32,
    comb: [f32; 4],
    beam_sigma: f32,
    beam_ratio: f32,
    beam_reach: i32,
    scan_base: f32,
    scan_bloom: f32,
    scan_reach: i32,
    mask_pitch: u32,
    mask_off: f32,
    decay: [f32; 4],
    barrel_k: f32,
    corner_r: f32,
    pad0: f32,
    pad1: f32,
}

impl Params {
    fn bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(160);
        let u = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        let f = |b: &mut Vec<u8>, v: f32| b.extend_from_slice(&v.to_le_bytes());
        let i = |b: &mut Vec<u8>, v: i32| b.extend_from_slice(&v.to_le_bytes());
        for v in [self.n, self.nd, self.d, self.rows, self.row0, self.width, self.taps, self.uv_half, self.out_w, self.out_h, self.scale, self.flags] {
            u(&mut b, v);
        }
        for v in [self.black, self.scale_y, self.amp_k, self.gamma, self.r_from_v, self.g_from_u, self.g_from_v, self.b_from_u] {
            f(&mut b, v);
        }
        for v in self.comb {
            f(&mut b, v);
        }
        f(&mut b, self.beam_sigma);
        f(&mut b, self.beam_ratio);
        i(&mut b, self.beam_reach);
        f(&mut b, self.scan_base);
        f(&mut b, self.scan_bloom);
        i(&mut b, self.scan_reach);
        u(&mut b, self.mask_pitch);
        f(&mut b, self.mask_off);
        for v in self.decay {
            f(&mut b, v);
        }
        for v in [self.barrel_k, self.corner_r, self.pad0, self.pad1] {
            f(&mut b, v);
        }
        assert_eq!(b.len(), 160);
        b
    }
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub struct GpuPicture {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
    pipelines: Vec<wgpu::ComputePipeline>,
    bind0: wgpu::BindGroup,
    bind1: wgpu::BindGroup,
    params_buf: wgpu::Buffer,
    samples_buf: wgpu::Buffer,
    lines_buf: wgpu::Buffer,
    /// The last stage's output, linear RGB as vec4 per pixel.
    pub final_buf: wgpu::Buffer,
    staging: wgpu::Buffer,
    params: Params,
    has_state: bool,
    n: usize,
    lines: usize,
    row0: usize,
    /// (256 x scale) by (240 x scale).
    pub out_w: usize,
    pub out_h: usize,
}

impl GpuPicture {
    /// The decoder's and the CRT's instances supply every constant; the
    /// decoded rows are `row0..row0+rows` of `width` samples (the
    /// picture's 1..241 of 2048). None without an adapter.
    pub fn new(decoder: &Decoder, crt: &CrtParams, row0: usize, rows: usize, width: usize, n: usize, lines: usize) -> Option<GpuPicture> {
        let instance = wgpu::Instance::default();
        let adapter = match std::env::var("GPU_INDEX").ok().and_then(|s| s.parse::<usize>().ok()) {
            Some(i) => instance.enumerate_adapters(wgpu::Backends::all()).into_iter().nth(i)?,
            None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))?,
        };
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("nes-shell picture"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;
        Self::on_device(device, queue, adapter_name, decoder, crt, row0, rows, width, n, lines)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn on_device(device: wgpu::Device, queue: wgpu::Queue, adapter_name: String, decoder: &Decoder, crt: &CrtParams, row0: usize, rows: usize, width: usize, n: usize, lines: usize) -> Option<GpuPicture> {
        assert_eq!(decoder.separation, ntsc_decode::Separation::CombThreeLine, "the GPU picture is Rung C");
        let d = decoder.uv_decimation;
        let nd = n.div_ceil(d);
        let out_w = 256 * crt.scale;
        let out_h = 240 * crt.scale;
        let scale_y = 1.0 / (decoder.white - decoder.black);
        let beam_reach = (3.0 * crt.beam_sigma_samples).ceil() as i32;
        let max_sigma = crt.scanline_sigma_rows * (1.0 + crt.bloom);
        let scan_reach = (3.0 * max_sigma).ceil() as i32 + crt.scale as i32;
        let decay: [f32; 3] = std::array::from_fn(|c| (-crt.frame_period / crt.persistence_tau[c]).exp());
        let params = Params {
            n: n as u32,
            nd: nd as u32,
            d: d as u32,
            rows: rows as u32,
            row0: row0 as u32,
            width: width as u32,
            taps: decoder.uv_taps.len() as u32,
            uv_half: (decoder.uv_taps.len() / 2) as u32,
            out_w: out_w as u32,
            out_h: out_h as u32,
            scale: crt.scale as u32,
            flags: (crt.mask.is_some() as u32) | ((crt.geometry.is_some() as u32) << 1),
            black: decoder.black,
            scale_y,
            amp_k: scale_y * ntsc_decode::tables::CHROMA_SAT_CORRECTION / decoder.chroma_gain,
            gamma: decoder.display_gamma,
            r_from_v: decoder.r_from_v,
            g_from_u: decoder.g_from_u,
            g_from_v: decoder.g_from_v,
            b_from_u: decoder.b_from_u,
            comb: [decoder.comb_weights[0], decoder.comb_weights[1], decoder.comb_weights[2], 0.0],
            beam_sigma: crt.beam_sigma_samples,
            beam_ratio: width as f32 / out_w as f32,
            beam_reach,
            scan_base: crt.scanline_sigma_rows,
            scan_bloom: crt.bloom,
            scan_reach,
            mask_pitch: crt.mask.as_ref().map(|m| m.pitch as u32).unwrap_or(1),
            mask_off: crt.mask.as_ref().map(|m| m.off_gain).unwrap_or(1.0),
            decay: [decay[0], decay[1], decay[2], 0.0],
            barrel_k: crt.geometry.as_ref().map(|g| g.barrel_k).unwrap_or(0.0),
            corner_r: crt.geometry.as_ref().map(|g| g.corner_radius).unwrap_or(0.0),
            pad0: 0.0,
            pad1: 0.0,
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("picture"),
            source: wgpu::ShaderSource::Wgsl(include_str!("picture.wgsl").into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture 0"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                storage(1, true),
                storage(2, true),
                storage(3, true),
                storage(4, true),
                storage(5, false),
                storage(6, false),
                storage(7, false),
            ],
        });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture 1"),
            entries: &[storage(0, false), storage(1, false), storage(2, false), storage(3, false), storage(4, false), storage(5, false)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("picture"), bind_group_layouts: &[&bgl0, &bgl1], push_constant_ranges: &[] });
        let pipelines = PASSES
            .iter()
            .map(|entry| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: entry,
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
            .collect();
        let buf = |label: &str, size: u64, usage: wgpu::BufferUsages| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let st = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let params_buf = buf("params", 160, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let samples_buf = buf("samples", (lines * n * 4) as u64, st);
        let lines_buf = buf("lines", (rows * 16) as u64, st);
        let sincos_buf = buf("sincos", 12 * 16, st);
        let taps_buf = buf("taps", (decoder.uv_taps.len() * 4) as u64, st);
        let dec_buf = buf("dec", (rows * nd * 8) as u64, st);
        let lp_buf = buf("lp", (rows * nd * 8) as u64, st);
        let grid_buf = buf("grid", (rows * width * 16) as u64, st);
        let beam_buf = buf("beam", (rows * out_w * 16) as u64, st);
        let px = (out_w * out_h * 16) as u64;
        let scan_buf = buf("scan", px, st);
        let state_buf = buf("state", px, st);
        let pers_buf = buf("pers", px, st);
        let masked_buf = buf("masked", px, st);
        let final_buf = buf("final", px, st);
        let staging = buf("staging", px, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        let sincos: Vec<f32> = (0..12)
            .flat_map(|p| {
                let theta = std::f64::consts::TAU * p as f64 / 12.0 + decoder.demod_offset;
                [theta.sin() as f32, theta.cos() as f32, 0.0, 0.0]
            })
            .collect();
        queue.write_buffer(&sincos_buf, 0, &f32_bytes(&sincos));
        queue.write_buffer(&taps_buf, 0, &f32_bytes(&decoder.uv_taps));
        queue.write_buffer(&params_buf, 0, &params.bytes());
        fn entry(b: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding: b, resource: buf.as_entire_binding() }
        }
        let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture 0"),
            layout: &bgl0,
            entries: &[entry(0, &params_buf), entry(1, &samples_buf), entry(2, &lines_buf), entry(3, &sincos_buf), entry(4, &taps_buf), entry(5, &dec_buf), entry(6, &lp_buf), entry(7, &grid_buf)],
        });
        let bind1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture 1"),
            layout: &bgl1,
            entries: &[entry(0, &beam_buf), entry(1, &scan_buf), entry(2, &state_buf), entry(3, &pers_buf), entry(4, &masked_buf), entry(5, &final_buf)],
        });
        Some(GpuPicture { device, queue, adapter_name, pipelines, bind0, bind1, params_buf, samples_buf, lines_buf, final_buf, staging, params, has_state: false, n, lines, row0, out_w, out_h })
    }

    /// Forget the persistence state (the CPU pipeline's `reset`).
    pub fn reset(&mut self) {
        self.has_state = false;
    }

    /// Upload the frame and run the eight passes; `final_buf` holds the
    /// picture afterwards. Returns nothing: `read_back` fetches it.
    pub fn run(&mut self, frame: &CompositeFrame) {
        assert!(frame.lines.len() <= self.lines, "{} lines for a buffer of {}", frame.lines.len(), self.lines);
        let mut samples = Vec::with_capacity(self.lines * self.n);
        for l in &frame.lines {
            samples.extend_from_slice(&l.samples);
            // A short line (the odd frame's last) is padded with its last
            // sample; nothing decoded reads it.
            samples.extend(std::iter::repeat_n(*l.samples.last().unwrap(), self.n - l.samples.len()));
        }
        samples.resize(self.lines * self.n, 0.0);
        self.queue.write_buffer(&self.samples_buf, 0, &f32_bytes(&samples));
        let rows = self.params.rows as usize;
        let mut lines = Vec::with_capacity(rows * 4);
        for r in 0..rows {
            let line = self.row0 + r;
            lines.extend_from_slice(&(line as u32).to_le_bytes());
            lines.extend_from_slice(&(frame.lines[line].active_start as u32).to_le_bytes());
            lines.extend_from_slice(&(frame.phase_at(line, 0).get() as u32).to_le_bytes());
            lines.extend_from_slice(&0u32.to_le_bytes());
        }
        self.queue.write_buffer(&self.lines_buf, 0, &lines);
        let mut p = self.params;
        p.flags = (p.flags & 3) | ((self.has_state as u32) << 2);
        if std::env::var("MUTATE").is_ok_and(|v| v == "1") {
            // The mutation tests/gpu.rs proves the gate with: no
            // persistence, ever.
            p.flags &= 3;
        }
        self.queue.write_buffer(&self.params_buf, 0, &p.bytes());
        let nd = self.params.nd as usize;
        let width = self.params.width as usize;
        let counts = [rows * nd, rows * nd, rows * width, rows * self.out_w, self.out_h * self.out_w, self.out_h * self.out_w, self.out_h * self.out_w, self.out_h * self.out_w];
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("picture") });
        for (pipeline, count) in self.pipelines.iter().zip(counts) {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.bind0, &[]);
            pass.set_bind_group(1, &self.bind1, &[]);
            pass.dispatch_workgroups(count.div_ceil(64) as u32, 1, 1);
        }
        self.queue.submit(Some(enc.finish()));
        self.has_state = true;
    }

    /// The picture as the CPU chain's `DisplayFrame`.
    pub fn read_back(&self) -> DisplayFrame {
        let px = (self.out_w * self.out_h * 16) as u64;
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("read back") });
        enc.copy_buffer_to_buffer(&self.final_buf, 0, &self.staging, 0, px);
        self.queue.submit(Some(enc.finish()));
        let slice = self.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let mut data = Vec::with_capacity(self.out_w * self.out_h * 3);
        {
            let view = slice.get_mapped_range();
            for px in view.chunks_exact(16) {
                for c in 0..3 {
                    data.push(f32::from_le_bytes([px[c * 4], px[c * 4 + 1], px[c * 4 + 2], px[c * 4 + 3]]));
                }
            }
        }
        self.staging.unmap();
        DisplayFrame { width: self.out_w, height: self.out_h, data, samples_per_pixel: self.params.beam_ratio }
    }

    /// Wait for the queue: what a frame-time measurement brackets.
    pub fn wait(&self) {
        self.device.poll(wgpu::Maintain::Wait);
    }
}
