//! The console loop the window drives: the console with the sound on,
//! advanced by whole frames per display tick as ntsc-wasm's `Pacing`
//! decides from the wall clock, the sound handed to a ring the audio
//! callback drains. Held in tests/pacing.rs on a synthetic clock.

use nes_console::{Console, Sound};
use ntsc_wasm::{Pacing, PacingStats};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// The 48 kHz samples not yet played, and how many were asked for
/// while it was empty.
#[derive(Default)]
pub struct AudioRing {
    pub samples: VecDeque<f32>,
    pub underrun: u64,
    /// The listening level: table units times the stage gain to full
    /// scale (the WAV writer's 0.25).
    pub scale: f32,
}

impl AudioRing {
    pub fn pull(&mut self, out: &mut [f32]) {
        for v in out.iter_mut() {
            *v = match self.samples.pop_front() {
                Some(s) => (s * self.scale).clamp(-1.0, 1.0),
                None => {
                    self.underrun += 1;
                    0.0
                }
            };
        }
    }
}

pub struct Loop {
    pub console: Console,
    pub pacing: Pacing,
    pub ring: Arc<Mutex<AudioRing>>,
    /// Frames the console produced that the display never presented
    /// (a tick that advanced two or more).
    pub frames_run: u64,
}

impl Loop {
    pub fn new(mut console: Console) -> Loop {
        console.sound = Some(Sound::default());
        Loop { console, pacing: Pacing::nes_rendering_enabled(), ring: Arc::new(Mutex::new(AudioRing { scale: 0.25, ..Default::default() })), frames_run: 0 }
    }

    /// One display tick `dt_ns` after the last: advance the source by
    /// what the pacing says and return the newest frame if any was
    /// produced (None means present the previous one again).
    pub fn tick(&mut self, dt_ns: u64) -> Option<nes_bus::DotFrame> {
        let advance = self.pacing.tick(dt_ns);
        if advance == 0 {
            return None;
        }
        self.console.frames.clear();
        self.console.run_frames(advance as usize);
        self.frames_run += advance as u64;
        let sound = self.console.sound.as_mut().unwrap();
        let mut ring = self.ring.lock().unwrap();
        ring.samples.extend(sound.out.drain(..));
        // Never let the ring grow past a quarter second: a display that
        // stalls must not bank sound to play late.
        let cap = 48_000 / 4;
        while ring.samples.len() > cap {
            ring.samples.pop_front();
        }
        self.console.frames.pop()
    }

    pub fn stats(&self) -> PacingStats {
        self.pacing.stats
    }
}
