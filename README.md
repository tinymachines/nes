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
ROMs, ten of eleven sprite_hit tests and five of ten ppu_vbl_nmi tests
pass; the five that do not are one question, named in the report: the
documented console's NMI reaches the CPU about two dots later than the
two chips, held to their own measurements, allow, and a scope on the
real board is what settles it. Six of eight apu_test ROMs fail on the
frame sequencer's position after a $4017 write, carried to the 2a03 by
name. Running real programs found seven misses in rung 3 (a seam bit,
a shift carry, three bus-fight opcodes, and the interrupt sample point
twice over) and four in the fast PPU, each now held by a fixture in
its own repository.

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
                                  # and the console's three gates: the
                                  # plumbing, the NMI replay against the
                                  # switch-level 6502 (a dev-dependency,
                                  # git-pinned), the race replay under
                                  # every alignment
cargo run --release -p nes-console --example run-rom -- rom.nes [frames] [out.ppm]
                                  # a NROM ROM through the console: the
                                  # last frame as PPM, the rate, and
                                  # blargg's $6000 report if there is one;
                                  # ALIGN=cpu,ppu picks another power-on
                                  # alignment
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
