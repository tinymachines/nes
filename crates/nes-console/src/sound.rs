//! The sound (N7): the 2A03's five output codes after every CPU
//! half-cycle, through the two DACs (the nesdev table, `v2a03-dac`,
//! authored and labelled there), through the NES-001's audio stage as
//! the schematic gives it, resampled to 48 kHz.
//!
//! The stage, AUTHORED from the NES-001 schematic (RetroTechCollection's
//! NES-001.pdf, read 2026-09-06) and labelled so: AD1 and AD2 are each
//! pulled down by 100 ohms (R4, R3; the table's "+100"); AD1 through R7
//! 20K, AD2 through R8 12K and the cartridge's AUX_AUDIO_IN through R9
//! 20K meet at one node (the table's two numerators are already in the
//! 20/12 ratio, so `ad1 + ad2` is that node's current in the table's
//! units); C23 1 uF couples the node into U9E, a 74HC04 inverter held
//! linear by R6 47K from output to input with C21 220 pF across it;
//! C20 220 pF loads the output; FC1 39 uH and C4 0.01 uF sit between
//! the output and AUDIO_OUT. So, to the jack: a first-order high-pass
//! at 1/(2 pi (R7 || R8) C23) (the cartridge input open on a cartridge
//! without audio), a gain of -R6/R7 on the table's units, a first-order
//! low-pass at 1/(2 pi R6 C21); the LC is above audio and left out.
//!
//! Not modelled, recorded: the inverter's finite open-loop gain and its
//! rails (the closed-loop gain is taken as ideal), C20 against the
//! inverter's output resistance (well above audio), and the table's
//! absolute volts (the bench record supplies one scale factor). Held in
//! tests/sound.rs: the stage to the schematic's arithmetic, and the
//! whole path to blargg's mixer ROMs cancelling.

/// Samples per second of the code stream: one per CPU half-cycle, the
/// master's 472,500,000/11 half-steps a second over twelve. The
/// subcarrier, exactly.
pub const RATE_NUM: u64 = 39_375_000;
pub const RATE_DEN: u64 = 11;
/// The output rate.
pub const OUT_RATE: u64 = 48_000;

/// The schematic's values, ohms and farads.
pub const R7: f64 = 20_000.0;
pub const R8: f64 = 12_000.0;
pub const R6: f64 = 47_000.0;
pub const C23: f64 = 1.0e-6;
pub const C21: f64 = 220.0e-12;

/// The high-pass time constant: the summing node's Thevenin resistance
/// (R7 parallel R8) with C23.
pub fn hp_tau() -> f64 {
    (R7 * R8 / (R7 + R8)) * C23
}

/// The low-pass time constant: R6 with C21.
pub fn lp_tau() -> f64 {
    R6 * C21
}

/// The inverter's closed-loop gain on the table's units, sign included.
pub fn gain() -> f64 {
    -R6 / R7
}

/// The resampler's cutoff and half-width: a windowed sinc cut off at
/// 20 kHz, four cycles of the cutoff each side, tabulated at 1/64 of
/// an input sample.
const CUTOFF_HZ: f64 = 20_000.0;
const HALF_CYCLES: f64 = 4.0;
const TABLE_STEP: usize = 64;

/// The mixer the console runs: the table, or (for the mutation in
/// tests/sound.rs) the page's linear approximation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mixer {
    Table,
    Linear,
}

pub struct Sound {
    mixer: Mixer,
    /// The two DACs tabulated: ad1 by the squares' sum (0..=30), ad2 by
    /// (tri, noise, pcm) at 16 x 16 x 128. The table's formulas, once.
    ad1_lut: [f32; 31],
    ad2_lut: Vec<f32>,
    hp_a: f64,
    lp_b: f64,
    hp_x1: f64,
    hp_y1: f64,
    lp_y1: f64,
    /// Stage output awaiting resampling, from input index `pending0`;
    /// compacted when the consumed prefix has grown past the window.
    pending: Vec<f32>,
    pending0: u64,
    /// Input samples pushed.
    pub samples: u64,
    /// Output samples emitted (48 kHz), and the input index the next
    /// one's window ends at (so a push does no arithmetic until then).
    next_out: u64,
    next_needed: u64,
    gain: f64,
    kernel: Vec<f32>,
    half_width: usize,
    /// The 48 kHz output, in the table's units times the stage gain
    /// (volts once the bench supplies the scale).
    pub out: Vec<f32>,
}

impl Default for Sound {
    fn default() -> Self {
        Sound::new(Mixer::Table)
    }
}

impl Sound {
    pub fn new(mixer: Mixer) -> Sound {
        let dt = RATE_DEN as f64 / RATE_NUM as f64;
        let hp_a = hp_tau() / (hp_tau() + dt);
        let lp_b = dt / (lp_tau() + dt);
        // The kernel: h(t) = 2 fc sinc(2 fc t) * blackman(t), t in
        // input samples, tabulated every 1/TABLE_STEP sample out to
        // the half-width.
        let half_width = (HALF_CYCLES / CUTOFF_HZ / dt).ceil() as usize;
        let n = half_width * TABLE_STEP + 1;
        let mut kernel = Vec::with_capacity(n);
        let fc = CUTOFF_HZ * dt; // cycles per input sample
        for i in 0..n {
            let t = i as f64 / TABLE_STEP as f64;
            let x = 2.0 * std::f64::consts::PI * fc * t;
            let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
            let u = t / half_width as f64;
            let w = 0.42 + 0.5 * (std::f64::consts::PI * u).cos() + 0.08 * (2.0 * std::f64::consts::PI * u).cos();
            kernel.push((2.0 * fc * sinc * w) as f32);
        }
        let (ad1_lut, ad2_lut) = match mixer {
            Mixer::Table => (std::array::from_fn(|n| v2a03_dac::ad1(n.min(15) as u8, n.saturating_sub(15) as u8)), (0..16 * 16 * 128).map(|i| v2a03_dac::ad2((i >> 11) as u8, ((i >> 7) & 15) as u8, (i & 127) as u8)).collect()),
            Mixer::Linear => (std::array::from_fn(|n| v2a03_dac::ad1_linear(n.min(15) as u8, n.saturating_sub(15) as u8)), (0..16 * 16 * 128).map(|i| v2a03_dac::ad2_linear((i >> 11) as u8, ((i >> 7) & 15) as u8, (i & 127) as u8)).collect()),
        };
        Sound {
            mixer,
            ad1_lut,
            ad2_lut,
            hp_a,
            lp_b,
            hp_x1: 0.0,
            hp_y1: 0.0,
            lp_y1: 0.0,
            pending: Vec::new(),
            pending0: 0,
            samples: 0,
            next_out: 0,
            next_needed: half_width as u64,
            gain: gain(),
            kernel,
            half_width,
            out: Vec::new(),
        }
    }

    /// The two pins' levels for a set of codes, in the table's units.
    pub fn pins(&self, codes: [u8; 5]) -> (f32, f32) {
        let [sq0, sq1, tri, noi, pcm] = codes;
        match self.mixer {
            Mixer::Table => (v2a03_dac::ad1(sq0, sq1), v2a03_dac::ad2(tri, noi, pcm)),
            Mixer::Linear => (v2a03_dac::ad1_linear(sq0, sq1), v2a03_dac::ad2_linear(tri, noi, pcm)),
        }
    }

    /// One CPU half-cycle's codes.
    #[inline]
    pub fn push(&mut self, codes: [u8; 5]) {
        let [sq0, sq1, tri, noi, pcm] = codes;
        let ad1 = self.ad1_lut[(sq0 + sq1) as usize];
        let ad2 = self.ad2_lut[((tri as usize) << 11) | ((noi as usize) << 7) | pcm as usize];
        self.push_level((ad1 + ad2) as f64);
    }

    /// One CPU half-cycle's summing-node level in the table's units,
    /// through the stage and the resampler (what `push` does after the
    /// DACs; the stage test drives this directly).
    pub fn push_level(&mut self, x: f64) {
        // High-pass (C23 into the summing node's resistance), then the
        // gain, then the low-pass (C21 across R6).
        let y = self.hp_a * (self.hp_y1 + x - self.hp_x1);
        self.hp_x1 = x;
        self.hp_y1 = y;
        let z = self.lp_y1 + self.lp_b * (self.gain * y - self.lp_y1);
        self.lp_y1 = z;
        self.pending.push(z as f32);
        self.samples += 1;
        if self.samples > self.next_needed {
            self.drain();
        }
    }

    /// Emit every output sample whose kernel window the input covers.
    fn drain(&mut self) {
        loop {
            // The output time in input samples: k / OUT_RATE * RATE.
            let centre = self.next_out as f64 * RATE_NUM as f64 / (OUT_RATE as f64 * RATE_DEN as f64);
            let last_needed = (centre + self.half_width as f64).floor() as u64;
            if last_needed >= self.samples {
                self.next_needed = last_needed;
                return;
            }
            let first = (centre - self.half_width as f64).ceil().max(0.0) as u64;
            let base = (first - self.pending0) as usize;
            let src = &self.pending[base..base + (last_needed - first) as usize + 1];
            let mut acc = 0.0f64;
            let step = TABLE_STEP as f64;
            let mut d = (first as f64 - centre) * step; // signed table position
            let kmax = self.kernel.len() - 1;
            for &v in src {
                let a = d.abs();
                let i = a as usize;
                if i < kmax {
                    let frac = (a - i as f64) as f32;
                    let h = self.kernel[i] + (self.kernel[i + 1] - self.kernel[i]) * frac;
                    acc += (h * v) as f64;
                }
                d += step;
            }
            self.out.push(acc as f32);
            self.next_out += 1;
            // Compact what no later window needs, once a good chunk has
            // accumulated.
            let next_centre = self.next_out as f64 * RATE_NUM as f64 / (OUT_RATE as f64 * RATE_DEN as f64);
            let keep_from = (next_centre - self.half_width as f64).floor().max(0.0) as u64;
            let drop = (keep_from.saturating_sub(self.pending0)) as usize;
            if drop > 4 * self.half_width {
                self.pending.drain(..drop);
                self.pending0 += drop as u64;
            }
        }
    }

    /// The output as a 16-bit mono WAV at 48 kHz, one table unit times
    /// the stage gain at `scale` of full scale (a fixed listening
    /// level, not a measurement).
    pub fn wav(&self, scale: f32) -> Vec<u8> {
        let n = self.out.len() as u32;
        let mut w = Vec::with_capacity(44 + 2 * n as usize);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + 2 * n).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&(OUT_RATE as u32).to_le_bytes());
        w.extend_from_slice(&(OUT_RATE as u32 * 2).to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(2 * n).to_le_bytes());
        for &s in &self.out {
            let v = (s * scale).clamp(-1.0, 1.0);
            w.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
        }
        w
    }
}
