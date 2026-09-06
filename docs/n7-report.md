# N7 report: the sound, the machine half closed

Run stamp: 2026-09-06, rustc 1.97.1. Pins: 2a03 `78a1f6e` (v2a03-micro
and the new v2a03-dac by path), 2c02 `0b25e83`, 6502 `8b9e0b5`,
ntsc-crt v0.2.4, nes-bus v0.1.1. `cargo test -p nes-console`: 11
tests (N5's six, N6's two, the three here). Plan: `docs/n7-plan.md`,
written first.

The console has sound: the 2A03's codes through the two DACs, the
NES-001's audio stage as the schematic gives it, and a resampler to
48 kHz, held to blargg's four mixer ROMs cancelling and measured
beside his recordings of the same ROMs on real hardware. The scope
record of AUDIO_OUT is the bench item it was.

## The stage, read off the schematic

RetroTechCollection's NES-001.pdf (read 2026-09-06), the audio corner
by U6: AD1 (pin 1) and AD2 (pin 2) each pulled down by 100 ohms, R4
and R3. AD1 through R7 20K, AD2 through R8 12K and the cartridge's
AUX_AUDIO_IN through R9 20K meet at one node. C23 1 uF couples that
node into pin 11 of U9E, a 74HC04 inverter, with R6 47K from its
output (pin 10) back to its input and C21 220 pF across R6; C20 220 pF
from the output to ground. The output is AUX_AUDIO_OUT and, through
FC1 39 uH with C4 0.01 uF to ground, AUDIO_OUT at the jack.

Two things fall out of reading the wiki's mixer table against it. The
table's "+100" in both groups is R3/R4: each DAC is a current source
into the board's pulldown. And the two groups' numerators, 95.88 and
159.79, are in the ratio 12/20, the board's summing resistors, so the
table's `pulse_out + tnd_out` is the summing node's current in the
table's units, not the two pins. The DAC table therefore stays what
it was (the nesdev page, authored, now `v2a03-dac` so the console
reaches it without the switch-level crates, `v2a03-sim` re-exporting
it as `mixer` for A3), and the stage here is what the board does to
that node's current: a first-order high-pass at 1/(2 pi (R7 || R8)
C23), 21.2 Hz (the cartridge input open on a cartridge without
audio), a gain of R6/R7 = 2.35 with the inverter's sign, and a
first-order low-pass at 1/(2 pi R6 C21), 15.4 kHz. The LC at the jack
is at 255 kHz and left out.

Not modelled, recorded: the inverter's finite open-loop gain (the
closed loop is taken as ideal), its rails, C20 against the output
resistance, and the table's absolute volts, which is one scale factor
the bench record supplies. The wiki's 90 Hz and 440 Hz high-pass
corners are not between the pins and the jack on this schematic;
whatever makes them is past the jack, and the scope goes to the jack.

## The path

`nes_console::sound::Sound`, fed the five codes after every CPU
half-cycle (the APU steps once per half-cycle; one sample at
39,375,000/11 Hz, the subcarrier exactly, and the count is held equal
to the CPU's half-cycles). The stage is two first-order sections in
f64 at that rate; the resampler a Blackman-windowed sinc cut off at
20 kHz, four cycles each side, tabulated at 1/64 of a sample and
evaluated at the exact rational output times. `run-rom` writes it
with `WAV=out.wav`. With sound on the console runs at about 80 frames
a second on one core (1.3x real time), against 125 to 140 without.

## Gate

`tests/sound.rs`, three tests, no die data or goldens read; the ROMs
and recordings from the nes-test-roms checkout (SKIP by name without
it, and the recordings without ffmpeg).

**The stage does the schematic's arithmetic.** A step comes out
inverted, its size the gain to a percent, and decays by e^-1 per
(R7 || R8) C23 (7.50 ms) to 2 percent; a 10 kHz tone against a 200 Hz
one is attenuated by the first-order product of the two corners:
0.8401 measured, 0.8433 from the values (the difference is the
resampler's own roll-off at 10 kHz). This holds the code to the values
written above, not to anything real.

**blargg's mixer ROMs cancel.** Each of the four through the console
for 1,200 frames; the beeps found by their shape (300 ms of the
998.8 Hz tone with silence either side); the test section between
them measured in 100 ms windows against the beep. Tolerance, from the
plan: at most 5 percent of the beep's RMS in every window (the DMC's
step is about 2 percent), the noise ROM held on its tone alone.

| ROM | console: worst window, RMS of beep | tone | recording (real hardware): RMS | tone |
|---|---|---|---|---|
| square | 2.4% | 1.5% | 6.1% | 6.0% |
| triangle | 2.7% | 1.3% | 3.0% | 1.7% |
| noise | 21.1% (fades by design) | 3.7% | 19.0% | 3.3% |
| dmc | 3.1% | 1.4% | 5.6% | 5.4% |

All four hold. `MUTATE_SOUND=1` mixes through the wiki's own linear
approximation and reads 34.4 percent on square and 33.6 on dmc: red
by the residual. (Its own variable: the 2A03 rung reads `MUTATE` to
swap a fitted APU phase, and a console-level `MUTATE=1` mutates the
chip, whose shell then never plays its beeps; the first red here was
the beep finder's, not the mixer's, which is exactly the kind of red
that proves nothing.)

**The recordings beside.** blargg's mp3 of each ROM on real hardware,
decoded to 48 kHz and measured by the same code (the beeps sit at the
files' full scale, 0.359, so the files are normalised; the ratios are
not). The triangle and noise figures agree with the console's to a
fraction of a percent, which says the table's triangle and noise
weights against the DMC are what the real chip does within what the
DMC's step can resolve. The square and dmc recordings carry twice the
console's residual, 6 percent against 3, and their residual is tonal
(the tone figure equals the RMS figure): the real pulse DAC and the
real DMC DAC depart from the table's curves by more than the table
departs from blargg's inverse, which he calibrated on his own
hardware. Recorded, not held: the recording carries the real stage,
the real room and the codec, and what the real pulse and DMC curves
are is the scope's question below.

## What stays for the bench

- AUDIO_OUT of the real console, terminated as the video was, under
  the four mixer ROMs and blargg's apu_test, at 1 MSa/s or better.
  Compared as waveforms after the same resampling with one scale
  factor fitted; the tolerance for that comparison is stated when the
  record exists and before it is scored: the beep's waveform within
  5 percent RMS after the scale, the cancellation residuals within
  2 percent of the beep of the recording's own.
- The pulse and DMC DAC curves, if the record shows the 6 percent to
  be theirs: the real pin levels per code, which would replace the
  wiki's table with a measurement, the way the 2C02's DAC levels were
  transcribed.
- N6's and N5's bench items, unchanged.
