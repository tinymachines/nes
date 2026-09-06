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
    pub fn new(console: Console) -> Loop {
        Loop::with_ring(console, Arc::new(Mutex::new(AudioRing { scale: 0.25, ..Default::default() })))
    }

    pub fn with_ring(mut console: Console, ring: Arc<Mutex<AudioRing>>) -> Loop {
        console.sound = Some(Sound::default());
        Loop { console, pacing: Pacing::nes_rendering_enabled(), ring, frames_run: 0 }
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

/// The console on its own thread, paced by the wall clock: each frame
/// period it runs what the pacing says is due, publishes the newest
/// frame and banks the sound. The display presents the latest published
/// frame whenever it redraws and counts nothing itself.
pub struct Handle {
    pub latest: Arc<Mutex<Option<(u64, nes_bus::DotFrame)>>>,
    pub buttons: Arc<Mutex<nes_glue::controller::Buttons>>,
    pub ring: Arc<Mutex<AudioRing>>,
    quit: Arc<std::sync::atomic::AtomicBool>,
    stats: Arc<Mutex<(PacingStats, u64)>>,
    /// Per console frame, wall time: (sum ns, max ns, frames), and a
    /// histogram of frames over 16, 20, 25 and 33 ms.
    pub frame_time: Arc<Mutex<(u64, u64, u64)>>,
    pub slow: Arc<Mutex<[u64; 4]>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    /// The console is built on its own thread (its chips share state
    /// through `Rc`, which does not cross threads), from the closure.
    pub fn spawn(make: Box<dyn FnOnce() -> Console + Send>) -> Handle {
        let latest = Arc::new(Mutex::new(None));
        let buttons = Arc::new(Mutex::new(nes_glue::controller::Buttons::default()));
        let ring = Arc::new(Mutex::new(AudioRing { scale: 0.25, ..Default::default() }));
        let quit = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stats = Arc::new(Mutex::new((PacingStats::default(), 0u64)));
        let frame_time = Arc::new(Mutex::new((0u64, 0u64, 0u64)));
        let slow = Arc::new(Mutex::new([0u64; 4]));
        let (lt, bt, qt, st, rg, ft, sl) = (latest.clone(), buttons.clone(), quit.clone(), stats.clone(), ring.clone(), frame_time.clone(), slow.clone());
        let join = std::thread::Builder::new()
            .name("console".into())
            .spawn(move || {
                let mut l = Loop::with_ring(make(), rg);
                let period = std::time::Duration::from_nanos(l.pacing_period_ns());
                let mut last = std::time::Instant::now();
                let mut seq = 0u64;
                while !qt.load(std::sync::atomic::Ordering::Relaxed) {
                    let now = std::time::Instant::now();
                    let dt = now.duration_since(last).as_nanos() as u64;
                    last = now;
                    l.console.set_pad(0, *bt.lock().unwrap());
                    let before = l.frames_run;
                    if let Some(f) = l.tick(dt) {
                        seq += 1;
                        *lt.lock().unwrap() = Some((seq, f));
                    }
                    let ran = l.frames_run - before;
                    if ran > 0 {
                        let ns = now.elapsed().as_nanos() as u64 / ran;
                        let mut t = ft.lock().unwrap();
                        t.0 += ns * ran;
                        t.1 = t.1.max(ns);
                        t.2 += ran;
                        let mut h = sl.lock().unwrap();
                        for (k, lim) in [16_000_000u64, 20_000_000, 25_000_000, 33_000_000].iter().enumerate() {
                            if ns > *lim {
                                h[k] += 1;
                            }
                        }
                    }
                    *st.lock().unwrap() = (l.stats(), l.frames_run);
                    // Sleep out the rest of the period; a frame that
                    // overran gets no sleep and the pacing counts the
                    // drop next time round.
                    let spent = now.elapsed();
                    if spent < period {
                        std::thread::sleep(period - spent);
                    }
                }
            })
            .expect("the console thread");
        Handle { latest, buttons, ring, quit, stats, frame_time, slow, join: Some(join) }
    }

    /// (pacing, frames run).
    pub fn stats(&self) -> (PacingStats, u64) {
        *self.stats.lock().unwrap()
    }

    pub fn stop(&mut self) {
        self.quit.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Loop {
    pub fn pacing_period_ns(&self) -> u64 {
        period_ns()
    }
}

/// The source period in nanoseconds: the NES rendering-enabled pair
/// rate ntsc-wasm's pacing states, two frames per 714,736 + 714,728
/// grid samples.
pub fn period_ns() -> u64 {
    ((714_736.0 + 714_728.0) / 2.0 / (472_500_000.0 / 11.0) * 1e9) as u64
}
