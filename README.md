# nes

The console: where the family's chips meet. The plan is the end-to-end
sketch, ratified 2026-09-02 and now at home here
(`docs/nes-end-to-end-v0_2.md`; the copy in
[nes-bus](https://github.com/tinymachines/nes-bus) stays as the pointer
it was). The parts it plugs together live in their own repositories:
[nes-bus](https://github.com/tinymachines/nes-bus) (the contracts),
[6502](https://github.com/tinymachines/6502),
[2a03](https://github.com/tinymachines/2a03),
[2c02](https://github.com/tinymachines/2c02) and
[ntsc-crt](https://github.com/tinymachines/ntsc-crt). The rule that
makes this work: a chip crate never knows what is on the other side of
its pins; this repository is the only thing that plugs anything into
anything.

## Status

The boards the console has are NROM (mapper 0), MMC1 (1), UxROM (2),
CNROM (3), MMC3 (4) and GxROM (66). Between them they take nineteen of
the twenty cartridges dumped from this desk with the OSCR reader: two
NROM, nine MMC1, three UxROM, two CNROM, two MMC3 and one GxROM. The
twentieth is Mike Tyson's Punch-Out, which is MMC2 (mapper 9) and is
refused by name.

MMC1, UxROM and CNROM arrived 2026-09-21 (nes-bus v0.1.4).
`tests/mappers.rs` runs a program on the die through each of them and
holds what it reads back; the boards' own logic is held a register at a
time in nes-bus's contract suite. The one that needed a console to
test is MMC1's serial port: a write to its window carries ONE bit, and
two writes on CONSECUTIVE CPU cycles are one write, which is what an
RMW instruction on the window is. The dot that decides it is the
console's, so the cartridge trait gained `cpu_write_at` and `Board`
hands the dot over with every write.

**Eighteen of the nineteen play.** The one that does not is Super Mario
Bros., whose only dump is one the reader's own CRC32 could not match:
that cartridge wants reading again before its blank screen means
anything. Nothing here has been played past its title screen with a
controller, so "plays" is "draws its own picture from its own program",
which is what the console can say on its own.

MMC3 arrived 2026-09-20 for the games that bank, Super Mario
Bros. 2 and 3 among them: PRG and CHR in banks either way up, mirroring
under software, and the scanline counter that watches PPU A12 and pulls
/IRQ low, which is how those games split the screen. Four of blargg's
six `mmc3_test_2` ROMs pass and `tests/mmc3.rs` names what the other two
say: `6-MMC3_alt` is the other chip revision (his Crystalis; this board
is the one his Super Mario Bros. 3 is on, and `5-MMC3` holds it), and
`4-scanline_timing` fails on where inside a CPU cycle the console hands
a cartridge's /IRQ to the core, not on the board's count, which
`2-details` passes and a purpose-written cartridge confirms from
outside at 241 clocks a frame. Getting there needed the 2C02's stepper
to fetch the sprite slots it will not draw, since a line with no sprites
still moves A12 on the part (2c02 @ 10b9089).

Five of those games were stuck for an afternoon on what looked like a
vertical-blank regression, and were not. blargg's `ppu_vbl_nmi` 01, 02
and 03 had started failing where `docs/n5-report.md` records them
passing, `01-vbl_basics` saying "VBL period is way off"; Paperboy,
Goonies II and Blaster Master never turned rendering on, and the Legend
of Zelda and Battle Chess drew one flat colour. The flag itself was
exact the whole time: `examples/vbl-probe` puts its rise at (241, 2),
its fall at (261, 2) and its period at 29780 or 29781 CPU cycles, with
rendering on or off. What was wrong was one branch. `01-vbl_basics`
reads $2002 and does `jpl test_failed`, which is `bmi` over a `jmp`, and
rung 3 had stopped taking a BMI that is taken, forward, on its page:
the commonest branch on the chip, dropped by the selector mask search
hours earlier in tinymachines/6502 while fixing a different branch bug.
The walk from the flat screen to the instruction was
`examples/where-it-sits` and `examples/vbl-probe`, and the fix is 6502 @
3805107. All five draw now, and `ppu_vbl_nmi` is back to the 5 of 10
the n5 report records, which was never stale.

N8 (the shell) is built and gated as far as a box without a screen can
gate it (`docs/n8-report.md`): `nes-shell rom.nes` puts the console in
a window with the three-line comb decode and the five CRT stages as
compute passes on the GPU, held to ntsc-crt's CPU chain on every
component of every pixel (worst 4.8e-7 with the authored parameters,
2.8e-5 with the mask and geometry on, against 1e-3 stated; about a
millisecond a frame where the CPU took 83); the console on its own
thread paced by the wall clock through ntsc-wasm's drift policy,
duplicates and drops counted; the sound through cpal; the keyboard and
a gamepad (gilrs) as controller 1. It ran under a virtual display here. `nes-wasm` is the
browser target: the console with its sound behind wasm-bindgen, 91
frames a second under node. The desk items are the real display, a
speaker and a hand.

N7 (the sound) has its machine half closed (`docs/n7-report.md`):
`Sound` takes the 2A03's five output codes after every CPU half-cycle
through the two DACs (the nesdev table, now `v2a03-dac`) and the
NES-001's audio stage read off the schematic (the 100 ohm pulldowns,
the 20K and 12K summing resistors the table's two constants already
carry, C23 into the 74HC04 inverter with R6 and C21 around it:
a high-pass at 21 Hz, a gain of 2.35 inverted, a low-pass at 15 kHz),
resampled to 48 kHz. Held to the schematic's arithmetic and to
blargg's four mixer ROMs cancelling through the whole console within
5 percent of the beep (the linear approximation is red at 34), with
his real-hardware recordings measured the same way beside: triangle
and noise agree with the console to a fraction of a percent, square
and dmc carry twice the console's residual, which is the real DAC
curves' question for the scope. The AUDIO_OUT record is the bench
item.

N6 (the picture) has step 1 closed and step 2's machine half recorded
(`docs/n6-report.md`): `Picture` takes the console's frames in order
through ntsc-crt's NES source, Rung C and the CRT stages, the
subcarrier phase carried by each frame's parity; a console frame
through it is the standalone PPU rung's own through it on every decoded
sample, and the phase after twelve frames is the grid's arithmetic
(forcing every frame Even is red). `full_palette.nes`, the bars
cartridge, paints through the console now that the 2C02's picture with
rendering off is measured (the palette entry v addresses, the $2006 and
$2007 timings, the emphasis bit). `capture-score` runs a bars cartridge
through ntsc-crt's capture-card model and recovers it the way the real
capture is recovered, the synthesis through the same front end, every
region scored a decoder-derived settling distance in from its edges:
on this repository's own bars cartridge (`export-testrom bars`,
thirty-two-dot cells of the twelve hues at each luma row) every region
holds the plan's tolerances at all four rows (worst luma 0.0001, hue
0.3 degrees, saturation 0.0012). The first runs found the instrument
three times (a level re-referencing a histogram bin coarse, a dark
picture taken for blanking, a chroma trough taken for a sync edge),
each fixed in ntsc-crt; blargg's full_palette bars are too narrow for
a hue verdict at the decoder's resolution and say so. The real bars
record is the bench item, and the cartridge for it exists now.

N5 (the console) has gates 1 and 2 recorded and gate 3 open
(`docs/n5-report.md`): `nes-console` runs the 2A03 core on rung 3 and
the 2C02 on the fast rung on one master half-step counter through the
glue, at the alignment measured off the two switch-level chips' own
dividers, 2.1x to 2.3x real time. Gate 1 holds three ways: the
plumbing; the PPU's real NMI landing around a BRK at eight offsets,
the console's CPU against the switch-level 6502 half-cycle for
half-cycle; and the $2002 race, the console's reads under all
twenty-four alignments against the table measured on the switch-level
2C02 with the console's own access shape, set side and clear side.
Gate 2 is recorded in full: cpu_timing_test, all sixteen instr_test
ROMs, all eleven sprite_hit tests (the double-height one since the
fast PPU's 8x16 rule was measured on the switch-level chip) and five of
ten ppu_vbl_nmi tests
pass; the five that do not are one question, named in the report: the
documented console's NMI reaches the CPU about two dots later than the
two chips, held to their own measurements, allow, and a scope on the
real board is what settles it. All eight apu_test ROMs pass, the six
that had failed each measured on the switch-level 2A03 and authored
(the $4017 write's parity jitter and immediate clock, the status read
latched a half-step after the bus is asked, the IRQ flag's three-cycle
set, the DMC's byte counted off where its read lands). Running real
programs found eight misses in rung 3 (a seam bit, a shift carry,
three bus-fight opcodes, the interrupt sample point twice over, and a
read latched later than the bus is asked) and four in the fast PPU,
each now held by a fixture in its own repository.

N4 (the glue, authored) is closed (`docs/n4-report.md`): `nes-glue`
is the NES-001 mainboard's handful of parts, each a few lines held to
its datasheet with its own test and labelled authored, nothing through
a netlist: the 74LS139 decoder (both halves as wired; the M2 term in
/ROMSEL is the test), the 74LS373 PPU address latch (transparent high,
held from the fall, with the case where a rising-edge sample would
differ shown, and an A12 watcher over a synthetic line of the PPU's
measured fetch schedule seeing one rise per line), the two TMM2115
SRAMs (ideal, eleven address lines, a visible power-on fill, the access
time recorded and unused), the 74LS368 controller port buffers with the
4021 controller behind them (inverting, open bus on every undriven bit),
the 74HC04 behind PPU /A13, and the reset chain, whose hold is a
labelled placeholder until the scope capture the sketch names replaces
it. 16 tests.

## Commands

```bash
cargo test --workspace            # every part against its datasheet,
                                  # and the console's gates: the
                                  # plumbing, the NMI replay against the
                                  # switch-level 6502 (a dev-dependency,
                                  # git-pinned), the race replay under
                                  # every alignment, the picture (a
                                  # frame through ntsc-crt equal to the
                                  # rung's own, the phase across the
                                  # parity sequence; MUTATE=1 red), the
                                  # sound (the stage's arithmetic,
                                  # blargg's mixer ROMs cancelling, his
                                  # recordings beside; MUTATE_SOUND=1
                                  # red, its own variable because the
                                  # 2A03 rung reads MUTATE itself)
cargo run --release -p nes-console --example where-it-sits -- rom.nes [frames]
                                  # a game that never draws, located:
                                  # the opcode fetch addresses counted
                                  # (TOP=n rows, COUNT=addr one by
                                  # name), WRITES=1 or WRITES=xxxx the
                                  # register writes, TRAP=xxxx the
                                  # hundred fetches before an address is
                                  # first reached, BUS=a-b every CPU
                                  # cycle in a half-cycle range. Super
                                  # Mario Bros. 2's crash was walked
                                  # back to one branch this way
cargo run --release -p nes-console --example vbl-probe -- [rom.nes] [frames]
                                  # every edge of the PPU's vblank flag:
                                  # where it fell in the frame and how
                                  # many CPU cycles since the last rise.
                                  # With no ROM it runs its own (turn
                                  # rendering off and spin; RENDER=1 for
                                  # on), so the flag is the only thing
                                  # moving. READS=1 the CPU's own $2002
                                  # reads beside the edges, NEAR=1 only
                                  # the ones landing on the set dot,
                                  # REPORT=1 the last forty reads dumped
                                  # when a blargg ROM stops running.
                                  # The flag being right and the read
                                  # being wrong are different faults and
                                  # this is what tells them apart
cargo run --release -p nes-console --example run-rom -- rom.nes [frames] [out.ppm]
                                  # a ROM on any of the six boards through the
                                  # console: the
                                  # last frame as PPM, the rate, and
                                  # blargg's $6000 report if there is one;
                                  # ALIGN=cpu,ppu picks another power-on
                                  # alignment; CRT=out.ppm the picture
                                  # through Rung C and the CRT stages,
                                  # DECODED=out.ppm the decoded grid;
                                  # WAV=out.wav the sound at 48 kHz
cargo run --release -p nes-console --example export-testrom -- out.nes [bars|pad|pad-dmc|pad-paint|cal]
                                  # the test cartridge, the bars
                                  # cartridge, or the bench's polling
                                  # cartridge (eight reads a frame, with
                                  # or without a DMC loop; pad-paint
                                  # colours its band with the byte it
                                  # read, for B3's bisection), as an
                                  # iNES file: nobody's game, the
                                  # console's own. `cal` is the bench's
                                  # calibration cartridge (src/cal.rs:
                                  # eight measured screens under a strip
                                  # that names every frame, its regions
                                  # written beside it as out.json from
                                  # the same generator); tests/cal.rs
                                  # reads the strip off the model's own
                                  # frames, MUTATE=1 one tile off is red
cargo run --release -p nes-console --example cal-screens -- out_dir
                                  # the calibration cartridge's eight
                                  # screens as decoded PPMs, stepped by
                                  # Select as a hand would; VARIANT=n
                                  # parks the palette and bars screens
# Every runner below takes KNOBS=runs/<stamp>/knobs.toml, the bench's
# knobs file (nes-console/src/knobs.rs): the model's alignment and its
# work RAM's power-on pattern (a fill byte or a seeded pattern: the model's
# blank RAM is a knob, not a fact), and the part's warmth ([warmth]
# seconds on, measured off the head's logs, on [warmth_curve], the
# picture gain fitted to the bench's warm-up series: capture-score
# scales the model's encoded picture about blanking by it), each with
# where it came from (measured, authored or fitted, and by what), read
# at the start and printed, so a run's report carries its sources; a
# key or table the reader does not know is refused by name, a fitted
# knob without its residual too. tests/knobs.rs: the shape parses, the
# refusals fire, and the alignment knob moves the scheduler (MUTATE=1
# feeds both runs one alignment and must go red), and the warmth reaches
# the picture and not the sync or the burst (MUTATE_WARMTH=1 must go red).
cargo run -p nes-console --example knobs -- runs/<stamp>/knobs.toml
                                  # the file read back and described,
                                  # or refused (exit 1); what nes-bench's
                                  # tools/knobs.py check runs
PAD=a5 cargo run --release -p nes-console --example pad-log -- rom.nes [frames] [script]
                                  # the controller port's poll log, one
                                  # line per latch (L index byte clocks),
                                  # the line the bench's bridge streams
                                  # from the part; a script of AT frame hh
                                  # lines sets the pad. On pad-dmc some
                                  # polls take nine reads: the DMC's double
                                  # clock, measured on the 2A03's die and
                                  # held on its rung; tests/pad_log.rs
                                  # records the count, MUTATE_HELD=1 red
cargo run --release -p nes-console --example mmc3-probe -- rom.nes [frames]
                                  # where an MMC3 board's counter is
                                  # clocked, in the PPU's own frame: the
                                  # position of every filtered A12 rise
                                  # with the counter and latch it left.
                                  # A game's split rides on this, and
                                  # blargg's 4-scanline_timing measures
                                  # the same thing from inside
cargo run --release -p nes-console --example bench-script -- rom.nes script.txt [tail]
                                  # a bench script played with its time:
                                  # WAIT s S runs s seconds of master
                                  # clock, RESET holds the CPU's /RESET
                                  # half a second (the head's pulse) and
                                  # releases it (Console::reset_button,
                                  # the CPU's warm reset; the PPU has no
                                  # reset here), latches counted from each
                                  # RESET as the bridge counts them; prints
                                  # each latch's poll line after the last
                                  # one (menu 119, game 250).
                                  # tests/warm_reset.rs, MUTATE_RESET=1 red
cargo run --release -p nes-console --example capture-score -- rom.nes [frames] [record.u8 rate]
                                  # the capture path: the ROM's frames
                                  # through the card model (or a real
                                  # record) and back, every flat region
                                  # scored against the console's own
                                  # synthesis; the synthetic roundtrip
                                  # is held to the plan's tolerances
                                  # (exit 1 on a miss), a real record
                                  # is recorded. The bench's B1: SCRIPT=
                                  # (the bench script's SET and AT lines),
                                  # LATCH=n (the model's frame is the
                                  # picture the part's recovery hands back
                                  # for a trigger at latch n, placed by
                                  # where the latch fell against the
                                  # vertical sync's onset, row 244 dot 280:
                                  # Console::run_to_picture_after_latch,
                                  # tests/latch_frame.rs, MUTATE=1 red),
                                  # TRIGGER_SAMPLE=i (the record sliced
                                  # from the trigger on, so the recovery's
                                  # first full frame is that frame on the
                                  # part);
                                  # SYNTH_TRIGGER=1 is the tool's own
                                  # green run on the synthesis, and
                                  # MUTATE_TRIGGER=1 (one frame late) is
                                  # red across the bars cartridge's
                                  # luma-row step, frames 122;
                                  # SYNTH_OUT=path writes the synthesis
                                  # as a u8 record with its .toml, which
                                  # the bench's fake scope serves;
                                  # PROFILE=$cc reports one colour's luma
                                  # row by row on both sides, over the
                                  # dots the model draws clear of anything
                                  # else: a tilt inside one colour is the
                                  # picture's, a step between colours at
                                  # the same rows is the colour's (which
                                  # is what the part's low luma turned out
                                  # to be, nes-bench open-items)
cargo run --release -p nes-console --example split-score -- rom.nes [frames] [record.u8 rate]
                                  # the split: every picture row's
                                  # horizontal shift between two frames,
                                  # on the model and on a triggered
                                  # record, the still rows the status
                                  # bar and the moving rows the level;
                                  # then which of the model's frames the
                                  # record's triggered frame is (F+0 on
                                  # the first scrolling record once the
                                  # frame was placed from the latch's
                                  # position; F+1 under the earlier rule,
                                  # which is how the rule was found). SCRIPT,
                                  # LATCH, TRIGGER_SAMPLE as above, GAP=n
                                  # frames apart; the synthetic roundtrip
                                  # is held (MUTATE_FRAME=1 and
                                  # MUTATE_STILL=1 red), a real record
                                  # recorded; nes-bench/tools/split-score.py
                                  # drives it from a run.
                                  # It also scores the candidates F-2..F+2
                                  # two ways where they differ: on the dots
                                  # they draw differently (what moved), and
                                  # on the decoded picture (the colour
                                  # phase, which alternates with the PPU's
                                  # frame parity and so names a parity, not
                                  # a frame). A still screen can only
                                  # answer the second, which is how Duck
                                  # Hunt's field read F-1 and F+1 alike
cargo run --release -p nes-console --example frame-motion -- rom.nes [frames]
                                  # how much of the picture moves, frame
                                  # by frame, under SCRIPT's own AT lines:
                                  # the probe that picks the latch for a
                                  # capture that has to tell one frame from
                                  # its neighbour. LATCH=n also names the
                                  # picture that latch lands on, SHOW=dir
                                  # with MARK=a-b writes those frames
                                  # decoded
cargo build --release -p nes-shell && target/release/nes-shell rom.nes
                                  # the console in a window (a display
                                  # session, a GPU): arrows, Z and X for
                                  # B and A, Enter and right Shift for
                                  # Start and Select, Escape to quit;
                                  # counters on exit. NES_SHELL_TICKS=n
                                  # exits after n redraws (the smoke run
                                  # under Xvfb)
cargo test --release -p nes-shell # the GPU picture against the CPU chain
                                  # (SKIPs without an adapter,
                                  # REQUIRE_GPU=1 insists; MUTATE=1 drops
                                  # persistence, red), the paced loop and
                                  # the ring on a synthetic clock
cargo run --release -p nes-console --example picture-bench -- rom.nes
                                  # where a frame's time goes on one core
cargo check --target wasm32-unknown-unknown -p nes-console
wasm-pack build crates/nes-wasm --target nodejs --out-dir /tmp/nes-wasm --release
node tools/wasm-bench.mjs /tmp/nes-wasm rom.nes [frames]
                                  # the browser target, measured under node
cargo run --release -p nes-console --example trace-cpu -- rom.nes
cargo run --release -p nes-console --example flat-cpu -- rom.nes <half-cycles>
                                  # the instruments: the CPU's bus through
                                  # the console, and rung 3 alone on a flat
                                  # image of the same ROM
```

ROMs are never committed; blargg's tests are read from a checkout of
the nes-test-roms collection.

## Licensing

MIT. Nothing here embeds die data; the chip crates this repository will
depend on carry their own NonCommercial and ShareAlike obligations, and
a console binary built with them inherits those.
