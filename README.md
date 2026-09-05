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
cargo test --workspace            # every part against its datasheet
```

## Licensing

MIT. Nothing here embeds die data; the chip crates this repository will
depend on carry their own NonCommercial and ShareAlike obligations, and
a console binary built with them inherits those.
