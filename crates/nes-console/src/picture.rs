//! The picture (N6): the console's frames, in order, through ntsc-crt.
//! Each `DotFrame` is encoded by the NES source at the subcarrier phase
//! the previous frame left (the odd frame's short line moves it, so the
//! console passes its parity, not just its dots), decoded on Rung C (the
//! three-line comb with the NES weights), and, unless asked not to, run
//! through the CRT stages (beam, scanlines, persistence, mask, geometry)
//! with ntsc-crt's authored parameters. Nothing here fits anything: the
//! chain is ntsc-crt's, at the tag Cargo.toml pins, and the two things
//! this module adds are the phase carried across frames and the order.
//!
//! Held in tests/picture.rs: a console frame through this is what the
//! standalone PPU rung's frame from the same world is through it, and
//! the phase carried across the console's real parity sequence is what
//! ntsc-grid's arithmetic gives that sequence.

use nes_bus::DotFrame;
use ntsc_crt::{CrtParams, CrtPipeline, DisplayFrame};
use ntsc_decode::{Decoder, LinearRgbFrame};
use ntsc_grid::{CompositeFrame, Phase, Profile};
use ntsc_source_nes::{burst_axis_offset, encode_frame, levels, Levels};

/// The decoded picture's grid: active samples per line by visible rows,
/// the three-line comb's first row being 1 (it wants both neighbours).
pub const DECODED_WIDTH: usize = 2048;
pub const DECODED_HEIGHT: usize = 240;
const COMB_ROW0: usize = 1;

/// One frame shown: the decoded picture on the sample grid, and the
/// CRT's output when the stages are on.
pub struct Shown {
    pub decoded: LinearRgbFrame,
    pub displayed: Option<DisplayFrame>,
}

pub struct Picture {
    levels: Levels,
    decoder: Decoder,
    origin: Phase,
    crt: Option<CrtPipeline>,
    /// Frames pushed so far.
    pub frames: usize,
}

impl Default for Picture {
    fn default() -> Self {
        Picture::new()
    }
}

impl Picture {
    /// Rung C and the CRT stages at ntsc-crt's authored parameters,
    /// scale 3 (the shell's integer-scale rule; 768 x 720 out).
    pub fn new() -> Picture {
        Picture::with_crt(Some(CrtPipeline::new(CrtParams::authored(3))))
    }

    /// Rung C alone: the decoded grid, no CRT.
    pub fn decode_only() -> Picture {
        Picture::with_crt(None)
    }

    fn with_crt(crt: Option<CrtPipeline>) -> Picture {
        let (theta0, black, white) = (burst_axis_offset(), levels::LOW[1], levels::HIGH[2]);
        Picture {
            levels: Levels::transcribed(),
            decoder: Decoder::comb_three_line(Profile::Nes, theta0, black, white),
            origin: Phase::new(0),
            crt,
            frames: 0,
        }
    }

    /// The subcarrier phase the next frame will be encoded at.
    pub fn origin(&self) -> Phase {
        self.origin
    }

    /// Encode one frame at the carried phase and carry the phase it
    /// leaves: the composite signal itself, which is what a capture of
    /// the console sees (the `capture-score` example).
    pub fn encode(&mut self, dots: &DotFrame) -> CompositeFrame {
        let frame = encode_frame(&self.levels, dots, self.origin);
        self.origin = frame.next_origin();
        self.frames += 1;
        frame
    }

    /// The decoder the picture decodes with (Rung C), for scoring a
    /// capture through the identical decoder.
    pub fn decoder(&self) -> &Decoder {
        &self.decoder
    }

    /// Encode, decode and (with the stages on) display one frame. The
    /// phase the frame leaves is carried to the next push.
    pub fn push(&mut self, dots: &DotFrame) -> Shown {
        let frame = self.encode(dots);
        let decoded = self.decoder.decode(&frame, COMB_ROW0, DECODED_HEIGHT, DECODED_WIDTH);
        let displayed = self.crt.as_mut().map(|crt| crt.process(&decoded));
        Shown { decoded, displayed }
    }
}

/// A displayed frame as a binary PPM (display gamma applied, ntsc-crt's
/// own conversion).
pub fn display_ppm(frame: &DisplayFrame) -> Vec<u8> {
    let rgba = frame.to_rgba8();
    let mut ppm = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    for px in rgba.chunks_exact(4) {
        ppm.extend_from_slice(&px[..3]);
    }
    ppm
}

/// A decoded frame as a binary PPM on its sample grid (signal RGB, the
/// decoder's own gamma).
pub fn decoded_ppm(frame: &LinearRgbFrame) -> Vec<u8> {
    let mut ppm = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    for i in 0..frame.width * frame.height {
        let rgb = frame.signal_rgb(i);
        for c in rgb {
            ppm.push((c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }
    ppm
}
