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
cargo run --release -p nes-console --example run-rom -- rom.nes [frames] [out.ppm]
                                  # a NROM ROM through the console: the
                                  # last frame as PPM, the rate, and
                                  # blargg's $6000 report if there is one;
                                  # ALIGN=cpu,ppu picks another power-on
                                  # alignment; CRT=out.ppm the picture
                                  # through Rung C and the CRT stages,
                                  # DECODED=out.ppm the decoded grid;
                                  # WAV=out.wav the sound at 48 kHz
cargo run --release -p nes-console --example export-testrom -- out.nes [bars|pad|pad-dmc]
                                  # the test cartridge, the bars
                                  # cartridge, or the bench's polling
                                  # cartridge (eight reads a frame, with
                                  # or without a DMC loop), as an iNES
                                  # file: nobody's game, the console's own
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
cargo run --release -p nes-console --example capture-score -- rom.nes [frames] [record.u8 rate]
                                  # the capture path: the ROM's frames
                                  # through the card model (or a real
                                  # record) and back, every flat region
                                  # scored against the console's own
                                  # synthesis; the synthetic roundtrip
                                  # is held to the plan's tolerances
                                  # (exit 1 on a miss), a real record
                                  # is recorded
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
