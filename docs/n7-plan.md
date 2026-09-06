# N7 plan: the sound

Written before the code (2026-09-06), the way N6's was. The sketch
(`docs/nes-end-to-end-v0_2.md`, section 3, N7) asks for the 2A03's
AD1 and AD2 streams into the authored mixer, the resistor and RC
constants off the schematic, then a resampler to 48 kHz; the gate is
the scope's AUDIO_OUT recording of a known register program against
the console's, compared as waveforms after the same resampling, with
tolerances stated before measuring.

## What exists

- The 2A03 rung exposes the five output codes every CPU half-step
  (`Apu::codes`: two squares, triangle, noise at four bits, the DMC
  level at seven), held to rung 0's own `sq0_out`..`pcm_out` nodes in
  the APU gate. The console steps it once per CPU half-cycle.
- The DAC table (`2a03: v2a03-sim/src/mixer.rs`) is the nesdev wiki's
  APU Mixer page, AUTHORED and labelled so: pulse and tnd groups, each
  a current-source DAC into a resistor, the constants the page's own.
  A3's first sound was mixed through it.
- The NES-001 schematic (RetroTechCollection's NES-001.pdf, read
  2026-09-06), the audio stage, read off the drawing: AD1 and AD2 each
  pulled down by 100 ohms (R4, R3); AD1 through R7 20K, AD2 through R8
  12K and the cartridge's AUX_AUDIO_IN through R9 20K into one node;
  C23 1 uF from that node into U9E, a 74HC04 inverter held linear by
  R6 47K from its output to its input with C21 220 pF across it; C20
  220 pF from the output to ground; the output is AUX_AUDIO_OUT and,
  through FC1 39 uH and C4 0.01 uF, AUDIO_OUT. The wiki's two group
  constants are already in the ratio 20/12 (159.79/95.88), so the
  table's "pulse_out + tnd_out" is the two pins weighted by the
  board's summing resistors, and the "+100" in both is R3/R4.
- blargg's apu_mixer ROMs (square, triangle, noise, dmc: each plays a
  channel and the DMC's inverse, so a right mixer cancels to near
  silence between two reference beeps) and his recordings of the same
  four ROMs on real hardware (mp3), in the nes-test-roms checkout. The
  recordings are a real-hardware measurement of the mixer that this
  repository did not make; their absolute gain is unknown but the
  ratio of the residual to the beep in each is not.
- No scope record of AUDIO_OUT. The sketch's capture list carries it
  (1 MSa/s is plenty), with the bars and the alignment records.

## Steps

1. **The DAC table where the console can reach it.** The wiki table
   moves into a dependency-free crate in the 2A03 repository
   (`v2a03-dac`), re-exported by `v2a03-sim` under its old name so A3
   and its example stand; nothing typed twice.

2. **The sound through the console.** `nes-console` gains `sound`: a
   `Sound` that takes the APU codes after every CPU half-cycle (one
   sample per half-cycle at the exact rate, 39,375,000/11 Hz, the
   subcarrier), mixes them through the DAC table, runs them through
   the NES-001 stage as the schematic gives it, and resamples to
   48 kHz. The stage, AUTHORED from the schematic's values and
   labelled: the summing node's Thevenin resistance (R7 parallel R8,
   the cartridge input open on a cartridge without audio) with C23 is
   a first-order high-pass; R6 with C21 a first-order low-pass; the
   inverter's closed-loop gain R6/R7 on the table's units, sign
   inverted; the LC at the jack is above audio and left out. What is
   not modelled and is recorded: the inverter's finite open-loop gain,
   its rails, and the table's absolute volts (one scale factor the
   bench record supplies). The resampler is a windowed sinc at the
   exact rational output times, cut off below the output Nyquist.
   `run-rom` gains `WAV=out.wav` (48 kHz, 16-bit mono).

   Gate (`tests/sound.rs`):
   - **The stage holds the schematic's arithmetic**: a step through it
     decays with R_thevenin times C23 and a tone above R6 times C21's
     corner is attenuated by the first-order amount, to a percent.
     Not a measurement of anything real; it holds the code to the
     values written above so a change of mind shows.
   - **blargg's mixer ROMs cancel.** Each of the four ROMs through the
     console for its whole run; the 48 kHz output split into the first
     beep, the test, the second beep by the shell's own timing (300 ms
     of silence, 300 ms of tone, 300 ms of silence around each beep).
     Tolerance, stated now: over the test section, the RMS of the
     output in every 100 ms window is at most 5 percent of the beep's
     RMS (the DMC's level step is about 2 percent of the beep at the
     table's values, so the cancellation can never be better than
     that; 5 percent leaves the nonlinearity a little room and no
     more), and the beep itself is above 20 percent of full scale so
     the ratio is not two small numbers. The noise ROM fades noise in
     and out by design and is held only on the beeps and on there
     being no tone (its spectrum's peak below 5 percent of its RMS in
     every window). `MUTATE=1` mixes through the wiki's linear
     approximation instead and the square or dmc ROM must go red.
   - **The recordings beside.** The same ratio measured from blargg's
     mp3 of each ROM (decoded with ffmpeg; SKIPs by name without it),
     printed beside the console's, recorded and not held: the
     recording carries the real stage, the real room and the codec.

3. **The report** (`docs/n7-report.md`): the stage as read off the
   schematic, the gate's figures per ROM, the recordings' ratios
   beside, and the bench item: AUDIO_OUT of the real console under
   the same four ROMs and blargg's apu_test, at 1 MSa/s or better,
   compared as waveforms after the same resampling with one scale
   factor fitted and the shape held to a tolerance the report states
   before that record is taken.

## Not in N7

Real-time audio out and pacing (N8, with the shell). The wiki's 90 Hz
and 440 Hz high-pass corners are not on the schematic between the
pins and AUDIO_OUT; whatever produces them is past the jack (the RF
modulator or the set), and the stage here stops at the jack, which is
where the scope goes.
