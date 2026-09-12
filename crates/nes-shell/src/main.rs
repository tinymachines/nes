//! The console in a window (N8 step 2): `nes-shell rom.nes`.
//!
//! The console with the sound on advances by whole frames per display
//! tick as the pacing decides from the wall clock (`run::Loop`); each
//! new frame is encoded on the CPU (ntsc-crt's NES source) and decoded
//! and drawn on the GPU (`gpu::GpuPicture`, eight compute passes held
//! to the CPU chain), then blitted to a wgpu surface on a winit window
//! at integer scale 3. Audio through cpal at 48 kHz from the ring the
//! loop fills. The keyboard is controller 1: arrows, Z and X for B and
//! A, Enter for Start, right Shift for Select, Escape to quit; a gamepad
//! through gilrs is ORed with it (`pad`). On exit
//! the counters print: frames presented, duplicated, dropped, audio
//! underruns. NES_SHELL_TICKS=n exits after n display ticks (the
//! smoke run under a virtual display).

use nes_console::{ines, Alignment, Console, Picture};
use nes_glue::controller::Buttons;
use nes_shell::gpu::GpuPicture;
use nes_shell::pad::{merge, Pad};
use nes_shell::run::Handle;
use ntsc_crt::CrtParams;
use ntsc_decode::Decoder;
use ntsc_grid::Profile;
use ntsc_source_nes::{burst_axis_offset, levels};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const SCALE: usize = 3;

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: GpuPicture,
    blit: wgpu::RenderPipeline,
    blit_bind: wgpu::BindGroup,
    blit_params: wgpu::Buffer,
    encoder: Picture,
    run: Handle,
    buttons: Buttons,
    pad: Pad,
    presented: u64,
    shown_seq: u64,
    shown: u64,
    /// The display's own clock: one redraw per source period, so a
    /// surface without vsync (a virtual display) does not spin.
    next_redraw: std::time::Instant,
    period: std::time::Duration,
    _stream: Option<cpal::Stream>,
}

struct App {
    rom: Vec<u8>,
    state: Option<State>,
}

fn console(rom: &[u8]) -> Console {
    let r = ines::parse(rom).expect("an iNES image");
    let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = r.cart().expect("a cartridge this console has (NROM or GxROM)");
    Console::with_prg_ram(cart, chr_ram, Alignment::default(), true)
}

fn audio(ring: &Arc<std::sync::Mutex<nes_shell::run::AudioRing>>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = cpal::StreamConfig { channels: 1, sample_rate: cpal::SampleRate(48_000), buffer_size: cpal::BufferSize::Default };
    let ring = ring.clone();
    let stream = device
        .build_output_stream(
            &config,
            move |out: &mut [f32], _| {
                ring.lock().unwrap().pull(out);
            },
            |e| eprintln!("audio: {e}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let (w, h) = ((256 * SCALE) as u32, (240 * SCALE) as u32);
        let window = Arc::new(
            el.create_window(Window::default_attributes().with_title("nes-shell").with_inner_size(winit::dpi::PhysicalSize::new(w, h)).with_resizable(false))
                .expect("a window"),
        );
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).expect("a surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("a GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor { label: Some("nes-shell"), required_features: wgpu::Features::empty(), required_limits: adapter.limits(), memory_hints: wgpu::MemoryHints::Performance },
            None,
        ))
        .expect("a device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            // Vsync on a real display; the clock paces the redraws anyway,
            // and a virtual display's fake vsync would halve the rate.
            present_mode: if std::env::var_os("NES_SHELL_TICKS").is_some() { wgpu::PresentMode::AutoNoVsync } else { wgpu::PresentMode::AutoVsync },
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let decoder = Decoder::comb_three_line(Profile::Nes, burst_axis_offset(), levels::LOW[1], levels::HIGH[2]);
        let crt = CrtParams::authored(SCALE);
        let lines = nes_bus::LINES;
        let n = nes_bus::DOTS_PER_LINE * 8;
        // The picture owns the device and queue; the blit borrows them.
        let gpu = GpuPicture::on_device(device, queue, adapter.get_info().name, &decoder, &crt, 1, 240, 2048, n, lines).expect("the GPU picture");
        let (device, queue) = (&gpu.device, &gpu.queue);

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("blit"), source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("blit"), bind_group_layouts: &[&bgl], push_constant_ranges: &[] });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: "vs", buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: "fs", targets: &[Some(format.into())], compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let blit_params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("blit params"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut pb = Vec::new();
        for v in [gpu.out_w as u32, gpu.out_h as u32, config.width, config.height] {
            pb.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&blit_params, 0, &pb);
        let blit_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: blit_params.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: gpu.final_buf.as_entire_binding() }],
        });

        let rom = self.rom.clone();
        let run = Handle::spawn(Box::new(move || console(&rom)));
        let stream = audio(&run.ring);
        if stream.is_none() {
            eprintln!("nes-shell: no audio output device; running silent");
        }
        eprintln!("nes-shell: running on {}", gpu.adapter_name);
        let period = std::time::Duration::from_nanos(nes_shell::run::period_ns());
        self.state = Some(State { window, surface, config, gpu, blit, blit_bind, blit_params, encoder: Picture::decode_only(), run, buttons: Buttons::default(), pad: Pad::open(), presented: 0, shown_seq: 0, shown: 0, next_redraw: std::time::Instant::now(), period, _stream: stream });
        el.set_control_flow(ControlFlow::Poll);
        self.state.as_ref().unwrap().window.request_redraw();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::KeyboardInput { event: KeyEvent { physical_key: PhysicalKey::Code(code), state, .. }, .. } => {
                let down = state == ElementState::Pressed;
                match code {
                    KeyCode::Escape => el.exit(),
                    KeyCode::ArrowUp => s.buttons.up = down,
                    KeyCode::ArrowDown => s.buttons.down = down,
                    KeyCode::ArrowLeft => s.buttons.left = down,
                    KeyCode::ArrowRight => s.buttons.right = down,
                    KeyCode::KeyZ => s.buttons.b = down,
                    KeyCode::KeyX => s.buttons.a = down,
                    KeyCode::Enter => s.buttons.start = down,
                    KeyCode::ShiftRight => s.buttons.select = down,
                    _ => {}
                }
                *s.run.buttons.lock().unwrap() = merge(s.buttons, s.pad.buttons);
            }
            WindowEvent::Resized(size) => {
                s.config.width = size.width.max(1);
                s.config.height = size.height.max(1);
                s.surface.configure(&s.gpu.device, &s.config);
                let mut pb = Vec::new();
                for v in [s.gpu.out_w as u32, s.gpu.out_h as u32, s.config.width, s.config.height] {
                    pb.extend_from_slice(&v.to_le_bytes());
                }
                s.gpu.queue.write_buffer(&s.blit_params, 0, &pb);
            }
            WindowEvent::RedrawRequested => {
                if let Some(limit) = std::env::var("NES_SHELL_TICKS").ok().and_then(|v| v.parse::<u64>().ok()) {
                    if s.presented >= limit {
                        el.exit();
                        return;
                    }
                }
                s.presented += 1;
                let pad = s.pad.poll();
                *s.run.buttons.lock().unwrap() = merge(s.buttons, pad);
                // The newest frame the console published, if it is new.
                let fresh = {
                    let latest = s.run.latest.lock().unwrap();
                    match latest.as_ref() {
                        Some((seq, f)) if *seq != s.shown_seq => Some((*seq, f.clone())),
                        _ => None,
                    }
                };
                if let Some((seq, frame)) = fresh {
                    s.shown_seq = seq;
                    s.shown += 1;
                    let composite = s.encoder.encode(&frame);
                    s.gpu.run(&composite);
                }
                match s.surface.get_current_texture() {
                    Ok(tex) => {
                        let view = tex.texture.create_view(&Default::default());
                        let mut enc = s.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("blit") });
                        {
                            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("blit"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });
                            pass.set_pipeline(&s.blit);
                            pass.set_bind_group(0, &s.blit_bind, &[]);
                            pass.draw(0..3, 0..1);
                        }
                        s.gpu.queue.submit(Some(enc.finish()));
                        tex.present();
                    }
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => s.surface.configure(&s.gpu.device, &s.config),
                    Err(e) => eprintln!("surface: {e}"),
                }
                // The next redraw one period on from this one's due time
                // (not from now: the display keeps the source's rate).
                s.next_redraw += s.period;
                let now = std::time::Instant::now();
                if s.next_redraw < now {
                    s.next_redraw = now;
                }
                el.set_control_flow(ControlFlow::WaitUntil(s.next_redraw));
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(s) = &self.state {
            if std::time::Instant::now() >= s.next_redraw {
                s.window.request_redraw();
            }
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if let Some(s) = self.state.as_mut() {
            s.run.stop();
            let (st, frames_run) = s.run.stats();
            let under = s.run.ring.lock().unwrap().underrun;
            let (sum, max, n) = *s.run.frame_time.lock().unwrap();
            let slow = *s.run.slow.lock().unwrap();
            eprintln!("nes-shell: console frame {:.2} ms mean, {:.2} ms worst, over {n}; over 16/20/25/33 ms: {}/{}/{}/{}", sum as f64 / n.max(1) as f64 / 1e6, max as f64 / 1e6, slow[0], slow[1], slow[2], slow[3]);
            eprintln!(
                "nes-shell: console {} periods, {} idle, {} dropped, {} frames run; display {} redraws, {} new frames shown; {} audio underrun samples",
                st.presented, st.duplicated, st.dropped, frames_run, s.presented, s.shown, under
            );
            eprintln!("nes-shell: {} gamepad(s) sent events", s.pad.seen.len());
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: nes-shell rom.nes");
        std::process::exit(2);
    });
    let rom = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2);
    });
    let el = EventLoop::new().expect("an event loop (a display session)");
    let mut app = App { rom, state: None };
    el.run_app(&mut app).expect("the event loop");
}
