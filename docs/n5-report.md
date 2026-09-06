# N5 report: the console, gates 1 and 2 recorded, gate 3 open

Run stamp: 2026-09-06, rustc 1.97.1. Pins: 6502 `8b9e0b5` (v6502-micro,
v6502-pins, v6502-sim as the gate's oracle), 2a03 `44277e1` (v2a03-micro
by path), 2c02 `474b7e7` (v2c02-fast by path), nes-bus v0.1.1, nes-glue
from N4. Alignment stamp: `Alignment::MEASURED`, cpu_phase 4, ppu_phase
3. Throughput: 125 to 140 frames a second on one core, 2.1x to 2.3x
real time, the CPU on rung 3 and the PPU on the fast rung. `cargo test
-p nes-console`: 6 tests (the plumbing, the NMI replay, the race replay
both sides). This crate is `crates/nes-console`.

Gate 1 is closed: both replays run through the console and hold. Gate 2
is recorded in full below, and the misses it leaves are one question,
named at the end of gate 1. Gate 3 has no ROM.

## The scheduler

One master half-step counter. The PPU steps a dot when the counter is
`ppu_phase` modulo 8, the CPU a half-cycle when it is `cpu_phase`
modulo 12, the PPU first (its /INT and the APU's IRQ are what the CPU
samples as its half-cycle begins). The two phases are MEASURED off the
switch-level chips' own dividers, each stepped on the master clock from
the last pulse of its reset: the 2A03's clk0 changes on master
half-steps 4, 16, 28 (`2a03: v2a03-sim/examples/clk-phase.rs`) and the
2C02's pclk0 rises on 3, 11, 19 (`2c02: v2c02-sim/examples/clk-phase.rs`).
That is one of the alignments the dividers can power up in: a CPU read
begins on a phi1, which recurs every twenty-four master half-steps, so
against the dot's eight there are twenty-four classes, `cpu_phase` 0 to
23 with `ppu_phase` 3 (a first draft counted four, having taken the
half-cycle grid for the phi1 grid). A console records the one it ran in
this stamp, and `run-rom` takes `ALIGN=cpu,ppu` to run another. Wall-clock pacing is not built: the console runs as
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

Held three ways, each a test in `crates/nes-console/tests`.

**The plumbing** (`plumbing.rs`): six frames of the test cartridge
(`testrom.rs`: a program that waits two vblanks, paints four rows, and
counts NMIs in RAM); the master counter is eight per dot from the PPU
phase, each odd frame with rendering on is one dot short and says so in
its parity, the picture equals the fast PPU's standalone frame on the
same world, and the NMI count in RAM is one a frame (less one where a
polling wait lost a vblank to the race, which is the race doing what
the chip does).

**The NMI-during-BRK replay** (`gate1_nmi.rs`): the PPU's first vblank
asserts /NMI at a master half-step the program cannot move; a sled of
known length puts a BRK's fetch at a chosen half-cycle from that edge,
eight offsets a cycle apart from thirteen half-cycles before the edge to
one after, so the edge falls in every cycle of the BRK and the ones
before it. The console logs every CPU half-cycle's inputs and pin
frame; rung 0 (the switch-level 6502, `v6502-sim`, a dev-dependency at
the same pin) runs the same bytes on the console's own power-on RAM
image with /NMI driven low from the half-cycle the console saw it, and
the two pin traces agree at every half-cycle from thirty before the
BRK's fetch to ninety after: the vector taken, the pushes, the timing.
The stack page is compared with the two cores' power-on stack pointers'
difference removed, derived from the first push each makes; the data
byte is skipped in a write's phi1 (the 2A03 lockstep's named class). 960
half-cycles, eight offsets, exact.

It did not pass the first time, and that was the point of running it.
With the edge inside the BRK's vector reads, rung 3 hijacked the
handler's first fetch where rung 0 let the handler's first instruction
run. `brk-nmi-probe` (6502) then measured both rungs at every half-cycle
around a NOP and a BRK, edge and level, with pulses down to one
half-cycle, and rung 3 was authored to what rung 0 does: an input
present as a cycle's phi1 begins is taken at the coming fetch, the
final cycle's included, and one arriving in its phi2 waits; the NMI
edge is two phi1 samples compared, so a low confined to a phi2 is not
an edge; a BRK whose edge is sampled by its fifth cycle's phi1 takes the
NMI's vector, and every BRK ends without a poll. Seven scripted traces
joined the 6502's golden on both sides of each seam. The 2A03's own
pad was then timed at the master clock (`nmi-latency-probe`, 2a03): a
low arriving one master pulse before the final phi1 begins is taken,
one arriving on that pulse waits, and nothing else is in the path.

**The race replay** (`gate1_race.rs`): the table measured on the
switch-level 2C02 with the console's access shape is the oracle
(`race-shape-probe`, 2c02: the register address, R/W and /CS applied
together at the CPU's phi1, as the 74LS139 decodes them, the byte taken
at the eleventh half-step; every half-step from forty-eight before each
event to twenty-four after, from one saved chip state). Against the
first half-step of the set's dot (vpos 241, hpos 1; the flag and /INT
rise on the dot boundary), a read starting eight or more half-steps
before misses, one starting one to seven before suppresses the set, one
starting on the dot or later consumes it; against the clear's dot (vpos
261, hpos 1), a read on it or later reads clear, one before reads set.
The fast PPU holds that table in its own test; the console test holds
the console to it: a polling program whose loop alternates cycle parity
walks its reads across the set, and a program that reads once a frame a
computed delay after the NMI, placed within half a cycle of the clear,
walks the clear; both run under all twenty-four alignments, and every
read that starts within five dots of an event must read what the chip
reads there and leave the NMI as the chip leaves it. 145 reads around
the set, 185 around the clear, every half-step of both windows covered,
every outcome the chip's.

**What the two chips together do not explain.** With the race and the
NMI both held to the switch-level chips, blargg's 02-vbl_set_time and
03-vbl_clear_time pass on the measured alignment, and four of his
timing tests still sit one or two dots from the documented console:
05-nmi_timing and 10-even_odd_timing by one dot on a sync the ROM takes
from the race, 07-nmi_on_timing and 08-nmi_off_timing by two, and
06-suppression's two rows where the documented console reads the flag
set and takes no NMI. Every one of them says the same thing: the
documented console's NMI reaches the CPU about two dots later than a
PPU whose /INT falls with the flag, into a CPU that samples it at phi1
with one master pulse of setup, allows. The probe found what would
make such a window in the chip: a read whose address leads its select
by L half-steps reads the flag set, clears it, and /INT never falls,
for selects up to L after the set (P2's reference protocol led by eight
and showed eight; an M2-qualified select would lead by six). Two dots
is sixteen, and the board wires no lead at all. So the question is
what the PPU's /INT and the CPU's NMI pin do against M2 on a real
NES-001, which is a scope on four lines, a row for the sketch's section
5; until it is taken the console holds the chips, and the five tests
are named here as the measurement's stake.

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
| ppu_vbl_nmi 02 vbl_set_time | **pass** (with the race measured under the console's shape) |
| ppu_vbl_nmi 03 vbl_clear_time | **pass** |
| ppu_vbl_nmi 04 nmi_control | pass (#11 needed the NMI edge rule below) |
| ppu_vbl_nmi 05 nmi_timing | fails by one dot: row 01 reads 3 where the documented console reads 4 (the NMI question, gate 1) |
| ppu_vbl_nmi 06 suppression | fails: rows 05 and 06 read `V N` where the documented console reads `V -` (the NMI question) |
| ppu_vbl_nmi 07 nmi_on_timing | fails by two dots: N through row 02, documented through 04 (the NMI question) |
| ppu_vbl_nmi 08 nmi_off_timing | fails by two dots: N from row 05, documented from 07 (the NMI question) |
| ppu_vbl_nmi 09 even_odd_frames | pass (00 01 01 02) |
| ppu_vbl_nmi 10 even_odd_timing | fails #2 by one dot on the sync it takes from the race ("skipped too soon"); it passed under the race's earlier, fitted window |
| sprite_hit_tests 01..07, 09..11 | **pass** (on screen) |
| sprite_hit_tests 08 double_height | **pass** (2026-09-06; refused by name until the fast PPU's tall-sprite rule was measured on rung 0 and held, 2c02 `1dc887a`) |
| apu_test 1..8 | **8 of 8 pass** (2026-09-06; six had failed on the first run) |

The APU rows were N3's tables meeting a CPU-side oracle for the first
time, and the six that failed were every one the sequencer's position
after a $4017 write, the status register's timing, or the DMC's byte
count, none the sequence itself. Each was measured on the switch-level
2A03 before being authored (the 2a03 repository, 2026-09-06): the
write's jitter (its reset lands one half-step after the strobe on one
APU parity and three on the other, every later event two apart, the
recorder now holding both parities); a mode-1 write's immediate
quarter and half clocks; the status latched at the end of the read's
phi2, a half-step after the core asks its bus (the 6502's
`MicroBus::read_late` carries that, the pins keeping the bus's byte);
the frame IRQ flag set on three consecutive cycles, so a read clearing
it inside them finds it set again; the triangle's and the noise's
length bits on the die's one half-step; and the DMC's byte counted off
six half-steps after its bus read, which lands on the DMA's grid three
after a request six (or eight) after the enable, so the enable's clear
of the DMC IRQ comes before the flag the fetch raises. Held by
`tests/reads.rs` there (the Rung on a bus against rung 0 at the pins,
the latched byte included) and by ten worlds of the code gate.

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

- The NMI question at the end of gate 1: /INT and the CPU's NMI pin
  against M2 on a real NES-001, the one measurement that would move
  05, 06, 07, 08 and 10.
- Gate 3, when a ROM is at hand.
- The RES hold on the 2A03 core and $4015's reads, from N3.
- Rung 0 differs from the part on ANC #imm and ASR #imm with A=$ff
  (recorded in the 6502 note); whether that is the switch model's bus
  fight or the die data is a question for that repository.
- Wall-clock pacing and the drift policy (the sketch's separate layer).
