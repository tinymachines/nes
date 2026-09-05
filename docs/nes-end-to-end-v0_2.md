# The console: end-to-end NES with CRT output, v0.2

Drafted 2026-09-02 as v0.1; amended and ratified by the director the
same day. This version is operative. Every number here is either
quoted from a stamped report in the family or marked **authored**;
nothing authored gets quoted anywhere until a milestone replaces it
with a measurement.

Changed from v0.1:

- The frame rate in section 0 is corrected: v0.1 paced at "60.0988 Hz",
  the exact conflation ntsc-crt's M0 report caught and the ratified
  v0.3 handoff pins apart. All three NES rates are now named.
- The PPU real-time figure in section 1 is corrected: v0.1's
  "10.74 M h/s" was the dot clock's edge rate, a unit no 2C02 report
  uses. In the unit every stamped 2C02 number is measured in (the
  master half-clock), real time is 42.95 M half-steps/s, 4x more. The
  P3 gate is restated in frame time so the unit cannot be confused
  again.
- The four open questions are resolved (section 6 is now the decision
  record): `nes-bus` is its own repository; P3 measures a per-dot
  table-driven number first with the bit-slice designed now and built
  on a recorded shortfall; the N5 gate-3 ROM is the Micro Mages demo;
  the 2A03 die pages move after the ladder and the probes they would
  have hosted become N3 deliverables.

## 0. Definition of "functional"

The console is functional when all of the following hold at once, each
with a number attached:

1. An NROM (mapper 0) program boots from power-on through the reset
   vector with no intervention.
2. The picture on screen is *demodulated* by ntsc-crt from a composite
   waveform the 2C02 ladder produced, never looked up from a palette.
3. Both controllers work through the 74LS368 path, not a shortcut.
4. Sound comes out of the 2A03 ladder's AD1/AD2 taps through the
   authored mixer, resampled to the host rate.
5. Frames are paced at the source's own rate, per frame type: full
   frames 29,531,250/491,381 = 60.09848 Hz, short frames 60.09915 Hz,
   two-frame rendering-on average 39,375,000/655,171 = 60.09881 Hz
   (the figure the literature rounds to "60.0988"). The three are the
   rates ntsc-crt's `field_rates_exact` already pins separately;
   duplicates and drops are counted (ntsc-wasm already has this
   policy).
6. Every chip in the box is held to its rung 0 by a golden, and the
   whole box is held to a real capture from a real NES.

"Functional" does not mean every mapper, every test ROM, or every
sprite-overflow corner. It means one honest path from the crystal to
the phosphor.

## 1. Where the family stands (quoted)

| Part of the box | Repo | Status |
|---|---|---|
| 6502 core | `6502` + `halfphi` | rung 0 bit-exact on 1,725 nodes; rung 3 (`v6502-micro`) 39.0 M h/s, all 274 traces replay exactly (2026-08-31) |
| 2C02 | `2c02` | P0 and P1 closed 2026-09-02; 32,900 h/s rendering-on; DAC zero mismatches over 7,680 samples |
| Composite to phosphor | `ntsc-crt` | M0 through M5 closed; M4's real-recording half waits on one capture file |
| 2A03 wrapper (÷12, APU, DMA, strobes) | none | visual2A03 "responds 200; not fetched or measured" |
| Mainboard glue and cartridge | none | |

Real-time needs, from the clocks on the schematic (21.477272 MHz
master, ÷12 CPU): CPU 3.58 M half-cycles/s, against rung 3's measured
39.0 M, a 10.9x margin. The PPU's unit is the master half-clock (one
half-step is one NTSC grid sample, 12 x f_sc, per the P1 report), so
its real time is **42.95 M half-steps/s**: one full frame is 714,736
half-steps in 16.639 ms. Stating the PPU requirement in any other
unit is how v0.1 got it 4x wrong; milestone gates below use frame
time, which no unit change can bend.

## 2. The shape

Three new repositories.

- **`nes-bus`**: its own repository, like `halfphi`, published and
  version-pinned. The frame types every chip crate speaks: CPU pins,
  PPU pins, the cartridge edge, `DotFrame`, and the audio sample
  stream. A crate inside `nes` would make the chip repos depend on the
  console repo, inverting the rule below; `v6502-pins` (one contract,
  no dependencies) and `halfphi` (shared across repos, gate-checked)
  are the precedents. Resolves the PPU handoff's open question 2
  (where `DotFrame` lives) by moving *all* the inter-chip contracts
  into one place at once.
- **`2a03`**: the fifth chip through halfphi. Rung 0 plus a ladder,
  exactly the 2c02 pattern.
- **`nes`**: the console. Owns the authored glue, the scheduler, the
  shell, and the end-to-end goldens. Depends on `nes-bus`, `6502`,
  `2a03`, `2c02`, `ntsc-crt`.

The rule that makes this work: a chip crate never knows what is on the
other side of its pins. The console is the only thing that plugs
anything into anything.

This document's committed home is `nes-bus/docs` until the `nes`
repository exists, at which point it moves there; the drafting copy in
the 6502 repository stays uncommitted, the same posture as the
ntsc-crt handoff copy.

## 3. Milestones

Each milestone has a gate that is a number, a `MUTATE=1` that must go
red, and a `check-self-counts.py` row. Order is dependency order;
N1 and N2 can run in parallel.

### N0: the contract

- `nes-bus` with `CpuPins`, `PpuPins`, `CartEdge` (the 72 pins as a
  typed struct, the cartridge's outputs CIRAM_A10 and CIRAM_CE
  included), `DotFrame` (moved from ntsc-crt with a compatibility
  re-export), `AudioSamples`.
- The cartridge is a trait; NROM is its first implementor and the
  only one in scope.
- Gate: `2c02` and `ntsc-crt` compile against `nes-bus` with their
  existing goldens unchanged. `MUTATE=1` flips one pin's polarity in
  the contract and every dependent golden must go red.

### N1: 2A03 rung 0 (the `2a03` repo, milestones A0 to A3)

- **A0**: netlist fetched by pinned hash; counts asserted; power-on
  converges; JSSim node golden bit-exact. First measurement to record:
  whether the 2C02's 38 permanently-conducting supply-gated transistors
  recur. If yes, promote the `power_on` fix-up into halfphi.
- **A3**: first sound: tap the AD1/AD2 driver nodes every half-step,
  hold the levels to the wiki's two-output mixer table sample for
  sample, stream to WebAudio.
- The new kind of gate: the core region held to `v6502-pins` through
  the rung 3 6502 in pin lockstep. Chip versus chip through the
  contract. Decimal instructions are the expected divergence and are
  listed by name.
- The die pages v0.1 numbered A1 and A2 (the renderer with the 2A03
  layout, the atlas and watch panel) move to after N3. The family has
  never measured with a page: decimal-probe, reset-probe and
  `_block-probe.html` did the measuring and the pages presented it.
  The instrumentation those pages would have carried is built as
  headless probes in N3, where the table measurement needs it; the
  pages come later, off the critical path, the same work as the 6502
  page.

### N2: PPU P2 and P3 (the `2c02` repo, as already written)

- P2: sprite-0, VBL race, OAM corruption, by crafted micro-trace.
- P3: the ladder. The gate is stated in frame time: **one frame
  rendered in at most 16.639 ms**, the full-frame period, measured
  over a stated number of frames with the margin recorded. The order
  inside P3:
  1. Measure first: a per-dot table-driven stepper (a ladder rung is
     not obliged to step at the half-step; `v6502-micro` has no
     nodes, and a per-dot step does eight half-steps of work per
     step, which changes the arithmetic entirely). Record its frame
     time before any slicing work starts.
  2. The bit-sliced datapath (the way `halfphi::slice` already
     works) is designed and budgeted **now**, in P3's plan, because
     the gap is large: **authored expectation**, if a naive
     half-step ladder reaches a tenth of the 6502's 1,465x it lands
     near 4.8 M half-steps/s against the 42.95 M real time needs,
     roughly 9x short. It is *built* only on a recorded shortfall
     from step 1, not on this authored number.

### N3: 2A03 ladder

- `v2a03-micro`: `v6502-micro` for the core (decimal disabled by
  flag, verified against N1's divergence list), plus the APU as tables
  measured out of rung 0 the way the 6502's decode was: frame
  sequencer, sweep, envelope, length table, noise LFSR, DMC. The $4014
  and DMC stalls authored as RDY spans, replayed against rung 0
  goldens that show which half-cycle the steal lands in.
- The probes are deliverables, not scaffolding: headless dumps of the
  frame sequencer, channel timers and length counters, OUT0-2, OE1-2,
  and RDY during $4014, run against rung 0 before the tables are
  authored. These are the instruments the deferred die pages will
  later present.
- Gate: pin golden against rung 0 on a register program that exercises
  every channel; the AD1/AD2 sample stream bit-exact against rung 0's
  tap; throughput measured.

### N4: the glue, authored

Nothing here goes through halfphi. Each part is a few lines held to
its datasheet, labelled authored, with its own small test.

- 74LS139 half A: CPU_RAM_CS, PPU_CS from A13/A14.
- 74LS139 half B: ROMSEL = not (A15 and M2). The M2 term is the test.
- 74LS373: transparent on ALE high, latched on the falling edge.
  The test is a mapper-style A12 watcher seeing the right edge count.
- 2x TMM2115: ideal 2 KB SRAM. Access time recorded as a constant
  from the datasheet, unused until someone needs contention.
- 74LS368 x2 plus the diode arrays: the controller read path,
  including open bus on unused bits.
- 74HC04: PPU_!A13.
- The power-on reset chain: RST_PB, the CIC's reset output as a fixed
  delay (**authored**, replaced by the scope, see section 5).

### N5: the console

- The scheduler: one master half-step counter at 21.477272 MHz ÷ 2,
  the CPU advancing every 12 master ticks and the PPU every 4, with
  the power-on alignment chosen from the set rung 0 produces and
  recorded in the run stamp. Wall-clock pacing is a separate layer,
  labelled authored, paced by the per-frame-type periods of section 0
  item 5 with the ntsc-wasm drift policy.
- Gate 1: the CPU/PPU alignment. The NMI-during-BRK halfscore and the
  VBL race from P2 replay through the console with the same
  half-cycle positions as the standalone traces.
- Gate 2: blargg's `cpu_timing_test`, `ppu_vbl_nmi`, `sprite_hit_tests`,
  `apu_test` end to end through the ladder rungs, each result recorded
  as pass or as a named, understood failure.
- Gate 3: the **Micro Mages demo** (Morphcat Games) boots and plays
  with both controllers: NROM 32K, co-op two-player, so "both
  controllers" is exercised by actual play rather than a menu. It is
  freely distributed but not open-licensed, so it drives the gate
  locally and is never committed, the same posture as the manual in
  `reference/`. Fallback if its licensing bothers anyone: Shiru's
  NROM titles (Lan Master), accepting that a second ROM is then
  needed for the second controller.

### N6: the picture

- `DotFrame` from the PPU ladder into ntsc-crt's NES source, Rung C
  decode by default, the CRT stages on.
- Gate: the M4 real-capture comparison against the scope recording
  from section 5, run on a colour-bars cartridge in both the real box
  and the console, with the same alignment procedure ntsc-oracle
  already uses. Tolerances stated before measuring.

### N7: the sound

- AD1/AD2 sample streams into the authored mixer (R7, R8, R9, R6 and
  the RC constants off the schematic), then a resampler to 48 kHz.
- Gate: the scope's AUDIO_OUT recording of a known register program
  versus the console's, compared as waveforms after the same
  resampling. Tolerances stated before measuring.

### N8: the shell and packaging

- A Linux native binary: the console, a window, the CRT stages in a
  shader (the one place the GPUs earn their keep), controller input,
  audio out. The die explorers stay in the browser; the console does
  not need them.
- The WASM build kept alive as a second target, measured separately.

## 4. What the two GPUs are for

Not the console. The measured GPU rung 2 runs 128,000 machines at
39 h/s each. They are for:

1. The CRT shader in N8.
2. Regression sweeps: every opcode from every seed on rung 0 kernels,
   nightly, the way `v6502-gpu` already runs.
3. Golden generation for N3 and N2 in parallel rather than serially.

## 5. The scope session

Everything below feeds a named gate. Capture in this order; the first
item alone closes ntsc-crt M4.

| Probe point (schematic) | Rate needed | Feeds |
|---|---|---|
| VIDEO_OUT (AUX port, or the RF_MOD1 input, after Q1) | at least 4x subcarrier; 12x subcarrier (42.95 MSa/s) matches the grid exactly | ntsc-crt M4 real capture; N6 |
| X1 / CPU_CLK / PPU_CLK (the 21.477272 MHz master) together with M2 (U6 pin 31) and PPU_ALE (U5 pin 39) on one timebase | at least 100 MSa/s, 3 channels | the ÷12 versus ÷4 power-on alignment; N5 gate 1 |
| AUDIO_OUT (AUX port) and, if reachable, U6 pins 1 and 2 (AD1, AD2) | 1 MSa/s is plenty | N7; the mixer constants |
| ROMSEL at the cartridge edge against M2 | 100 MSa/s | N4's 139 half B test |
| CPU_RST, RST_PB, CIC_RST, PWR_LED at power-on | 1 MSa/s, long record | replaces the authored reset chain in N4 |
| CIRAM_CE and PPU_A13 against PPU_ALE | 100 MSa/s | the cartridge contract's mirroring path |

Notes for the session:

- Save raw samples with the sample rate and probe attenuation in the
  filename or a sidecar. `ntsc-source-cap` reads WAV; the rest can be
  CSV. Nothing gets a name until it has a rate stamped.
- Capture the master clock and M2 on the *same* record so the
  alignment is a measurement, not an inference from two records.
- For VIDEO_OUT, a colour-bars screen (any NROM test suite that draws
  bars) plus a plain black frame plus a plain white frame. Bars close
  M4; black and white pin the sync and blanking levels the DAC gate
  already holds.
- Do the power-on capture more than once. The alignment is expected to
  vary between power-ons; the set of values you see is the set N5's
  scheduler must draw from.
- Probe the composite after the video buffer, not at the PPU pin;
  the console models the buffer, and the comparison should be
  buffer-to-buffer.

## 6. Decision record

v0.1 put four questions to the director; all four are decided above
and recorded here so the reasoning survives.

1. **`nes-bus` is its own repository** (section 2). A crate inside
   `nes` inverts the dependency the whole shape exists to prevent.
2. **P3 measures first, in frame time, with the slice designed now
   and built on a recorded shortfall** (N2). The 4x unit correction
   in section 1 is what forced the gate into frame time; the authored
   gap is roughly 9x, large enough to design for, not large enough to
   build for unmeasured.
3. **Micro Mages demo for N5 gate 3** (N5). v0.1 cited "the original
   roster", which nothing in the family defines; the pick is made on
   criteria instead (mapper 0, both controllers in real play,
   distributable enough to test with, never committed).
4. **The 2A03 die pages come after the ladder** (N1, N3). The probes
   they would have hosted are N3 deliverables; the pages are
   presentation and off the critical path.

Nothing is open for the director in this version.
