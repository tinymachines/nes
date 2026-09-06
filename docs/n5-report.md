# N5 report: the console, gate 2 recorded, gates 1 and 3 open

Run stamp: 2026-09-05, rustc 1.97.1. Pins: 6502 `9c177cd` (v6502-micro,
v6502-pins), 2a03 `ac098d0` (v2a03-micro by path), 2c02 `3a48687`
(v2c02-fast by path), nes-bus v0.1.1, nes-glue from N4. Alignment
stamp: `Alignment::MEASURED`, cpu_phase 4, ppu_phase 3. Throughput:
125 to 140 frames a second on one core, 2.1x to 2.3x real time, the
CPU on rung 3 and the PPU on the fast rung. `cargo test -p nes-console`:
3 tests. This crate is `crates/nes-console`.

N5 is not closed. Gate 2 is recorded in full below; gate 1 has its
plumbing held and its two named replays not yet run; gate 3 has no ROM.

## The scheduler

One master half-step counter. The PPU steps a dot when the counter is
`ppu_phase` modulo 8, the CPU a half-cycle when it is `cpu_phase`
modulo 12, the PPU first (its /INT and the APU's IRQ are what the CPU
samples as its half-cycle begins). The two phases are MEASURED off the
switch-level chips' own dividers, each stepped on the master clock from
the last pulse of its reset: the 2A03's clk0 changes on master
half-steps 4, 16, 28 (`2a03: v2a03-sim/examples/clk-phase.rs`) and the
2C02's pclk0 rises on 3, 11, 19 (`2c02: v2c02-sim/examples/clk-phase.rs`).
That is one of the four alignments the dividers can power up in (the
CPU's half-cycle start visits two of a dot's eight half-steps, so the
classes are cpu_phase 4, 5, 6 and 7 against ppu_phase 3); a console
records the one it ran in this stamp, and `run-rom` takes `ALIGN=cpu,ppu`
to run another. Wall-clock pacing is not built: the console runs as
fast as it can and reports the rate.

The board (`board.rs`) routes every CPU access through nes-glue's
decode: the two TMM2115s, the cartridge, the PPU's registers (a
$2002 read carries the half-step within the current dot at which the
CPU's half-cycle began, for the fast PPU's race model), $4016/$4017
through the 74LS368s and the controllers, and open bus as the last
value the bus carried. 8 KiB of cartridge RAM at $6000 is offered to
the test ROMs (`Console::with_prg_ram`), labelled as theirs. The PPU's
CHR and CIRAM are the cartridge's through `PpuBus`, NROM mirroring by
the cartridge's own CIRAM A10.

## Gate 1: the alignment

Held: `tests/plumbing.rs` runs six frames of the test cartridge
(`testrom.rs`: a program that waits two vblanks, paints four rows, and
counts NMIs in RAM) and asserts that the master counter is eight per
dot from the PPU phase, that each odd frame with rendering on is one
dot short and says so in its parity, that the picture equals the fast
PPU's standalone frame on the same world, and that the NMI count is
one per frame. So the two chips are on one clock at the measured
phases, and the seam does not lose or duplicate a dot.

Not yet run, and named as such: the NMI-during-BRK halfscore and the
P2 VBL race trace replayed through the console at their standalone
half-cycle positions. What the console does instead is stamp each
$2002 read with the half-step within the dot at which the CPU's phi1
began, and the fast PPU decides the race from that against P2's fitted
window (`race::CONSUME_FROM = -9`, `SUPPRESS_FROM = -21` half-steps).
Two things blargg's timing tests then showed, both alignment-shaped
and both still open:

- **The set-side race depends on the alignment, as the readme says it
  does on a real NES** ("after some resets this is - -"). 02-vbl_set_time
  fails on cpu_phase 4, 5 and 6 with row 03 reading `- -` where the
  documented alignment reads `- V`, and passes on cpu_phase 7. On 7,
  06-suppression's suppressed row moves to 04 as documented, and
  08-nmi_off_timing's first N moves from 04 to 05 (documented: 07).
- **The clear side does not move with the alignment.** 03-vbl_clear_time
  reads V one dot past the documented row on all four phases, and
  07-nmi_on_timing stops firing one dot before the documented row on
  all four. The fast PPU's timed read models the race at the set only;
  a read or a write landing in the dot before the clear at (261, 1) is
  served the stale flag. And after the set, the window in which the
  flag reads back set but no NMI follows is one dot here and two on
  the documented hardware (06 rows 05 and 06).

The question underneath is the meaning of "read start". P2's harness
applied the reference's 24-edge protocol: the address at edge 24, chip
select low eight half-steps later, the byte sampled at edge 1; its
offsets are from the address. On the NES-001 the PPU's chip select is
the 74LS139's decode of the address alone, /RD and /WR are M2 gated,
and the CPU samples at M2's fall. Which of those edges the 2C02's race
keys off is a measurement on the switch-level 2C02 with the console's
access shape, not a reading; until it is made the console stamps the
phi1 start and says so here.

## Gate 2: blargg, end to end

Each ROM through the whole console: the 2A03 core on rung 3 with the
APU tables, the 2C02 on the fast rung, the glue, 8 KiB at $6000 for the
report. `run-rom` prints the report window and the last frame; the
sprite-hit and cpu_timing ROMs report on screen only and were read
there. Alignment 4,3 unless stated.

| suite | result |
|---|---|
| cpu_timing_test6 | **PASSED** (on screen, official instructions) |
| instr_test-v5 01..16 | **16 of 16 pass** |
| ppu_vbl_nmi 01 vbl_basics | pass |
| ppu_vbl_nmi 02 vbl_set_time | fails on 4,3 (row 03 `- -`); **passes on 7,3** |
| ppu_vbl_nmi 03 vbl_clear_time | fails: V through row 06, documented through 05; every alignment |
| ppu_vbl_nmi 04 nmi_control | pass (#11 needed the NMI edge rule below) |
| ppu_vbl_nmi 05 nmi_timing | pass |
| ppu_vbl_nmi 06 suppression | fails: on 4,3 rows 03 and 04 suppressed (documented: 04 alone); on 7,3 row 04 alone but the no-NMI window is row 05 alone (documented: 05 and 06) |
| ppu_vbl_nmi 07 nmi_on_timing | fails: N through row 03, documented through 04; every alignment |
| ppu_vbl_nmi 08 nmi_off_timing | fails: N from row 04 (7,3: 05), documented from 07 |
| ppu_vbl_nmi 09 even_odd_frames | pass (00 01 01 02) |
| ppu_vbl_nmi 10 even_odd_timing | pass (08 08 09 07) |
| sprite_hit_tests 01..07, 09..11 | **pass** (on screen) |
| sprite_hit_tests 08 double_height | refused by name: the fast PPU does not model 8x16 sprites |
| apu_test 3 irq_flag, 8 dmc_rates | pass |
| apu_test 1 len_ctr | fails #4: a $4017 write with bit 7 set must clock the length counters at once; `Apu::apply` only reseats the frame position |
| apu_test 2 len_table | fails (channel 0): follows from 1 |
| apu_test 4 jitter | fails #5: the $4017 write's effect is not delayed by the extra cycle on an odd CPU cycle |
| apu_test 5 len_timing | fails #3: the first length clock after a mode-0 write comes too late (`fit::FRAME_WRITE_LAG` and the phase table were fitted to the die's own free-running sequence, not to a write) |
| apu_test 6 irq_flag_timing | fails #3: the frame IRQ flag first sets too late after the write, the same fit |
| apu_test 7 dmc_basics | fails #19: no one-byte sample buffer filled at once when empty |

The APU rows are N3's tables meeting a CPU-side oracle for the first
time; every one is the frame sequencer's position after a $4017 write,
or the DMC's buffer, and none is the sequence itself (3 and 8 pass).
They carry to the 2a03 repository by name.

## What the ROMs taught the chips

The console is the first thing to run real programs on rung 3 and the
fast PPU for millions of cycles, and every miss below was located, not
reasoned about, then measured on the switch-level chip before being
authored, then held by a fixture that goes red without the fix. The
detail is in each repository's note; the list is here because the
console found them.

In the 6502 (rung 3, `docs/notes/engine.md`):

- ROR A with the carry set puts the carry into bit 7 by leaving
  ADD/SB7 off into the next instruction's first half-cycle; the seam
  word now carries that as a bit the sequencer clears. Found by
  02-implied's CRC routine; every instruction had failed.
- The left memory shifts' carry is the operand's bit 7, not the
  mid-span ALU capture, which the write cycle overwrites.
- ASR, ARR and ATX authored against blargg's checksums where rung 0
  is not the oracle (bus fights); ATX's constant is $FF, and $EE fails
  by name.
- An NMI edge in an instruction's final cycle waits one instruction
  (04-nmi_control #11).
- PLP and RTI take P from the third read, not the dummy at S
  (01-basics #4); the selector peeks operands through the console's
  bus rather than a flat image; a read cycle asks the bus once.
- Three recorder contexts (an index crossing with C clear, one with C
  set, a taken backward branch across a page) for keys the ROMs hit.

In the 2c02 (the fast rung): no sprite evaluation on the pre-render
line (an all-$ff OAM set the overflow flag before any program ran),
the address increments and copies gated on rendering, the odd frame's
short pre-render line, and $2001's left clip.

In the 2a03: the core's bus through the console with $4015 answered by
the APU and DMA reads and writes delivered on the put's phi2.

## Gate 3: play

The Micro Mages demo is not on this machine and the publisher's site
offers only the commercial builds; the gate was not run, on that ROM
or the fallback. `Console::set_pad` and the $4016/$4017 path are held
by the plumbing test (a button through OUT0, the eight reads, open bus
on the undriven bits), which is the plumbing and not the play.

## Carried

- Gate 1's two replays (the NMI-during-BRK halfscore, the P2 race trace)
  through the console; the read-start measurement above; the clear-side
  race and the two-dot no-NMI window in the fast PPU.
- Gate 3, when a ROM is at hand.
- 8x16 sprites in the fast PPU (sprite_hit 08).
- The APU's $4017 write behaviour and the DMC sample buffer (2a03).
- The RES hold on the 2A03 core and $4015's reads, from N3.
- Rung 0 differs from the part on ANC #imm and ASR #imm with A=$ff
  (recorded in the 6502 note); whether that is the switch model's bus
  fight or the die data is a question for that repository.
- Wall-clock pacing and the drift policy (the sketch's separate layer).
