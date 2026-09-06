# N6 report: the picture, and the capture path's machine half

Run stamp: 2026-09-06, rustc 1.97.1. Pins: ntsc-crt v0.2.4 by tag
(ntsc-grid, ntsc-source-nes, ntsc-decode, ntsc-crt; ntsc-source-cap as
a dev-dependency), 2c02 `0b25e83` (v2c02-fast by path), 2a03
`44277e1`, 6502 `8b9e0b5`, nes-bus v0.1.1. `cargo test -p
nes-console`: 8 tests (N5's six, the two here). Plan:
`docs/n6-plan.md`, written first.

Step 1 is closed and gated. Step 2's machine half runs end to end on
this box and is recorded against the tolerances the plan stated: luma
holds on every region, hue and saturation do not on 69 of 436, and the
cause is located in the capture model, not the console. The real
capture of the bars cartridge is the bench item it was.

## What the bars cartridge needed first

`full_palette.nes` paints every palette entry as bars with rendering
off: v parked in palette RAM through a $2006 pair, $2007 writes
stepping it mid-line, $2001's emphasis bits cycling per band. Through
the console it came out as blue stripes, because the fast PPU painted
the backdrop with rendering off and said so (a labelled authoring, the
P3 worlds never having disabled rendering). Measured on the
switch-level 2C02 (`2c02: v2c02-dots/examples/blank-probe.rs`, rows
60..67 of a frame with mask $00): with v in $3F00..$3FFF the picture
is the entry v addresses through the register file's own mirror rule,
the backdrop otherwise; a $2006 pair shows five dots after its second
write's access starts, a $2007 step eight, the emphasis bit three. The
stepper now paints that per dot (`blank_colour`, the $2007 step held
`BLANK_2007_HOLD` = 3 dots past the write, 2 and 4 red) and carries
$2001's emphasis bits beside every dot, held to the capture in `2c02:
v2c02-fast/tests/blank.rs` (`MUTATE=1` backdrop only, red on 1,625
dots). The emphasis lead of two dots is recorded in the 2c02 report
and not modelled: the harness puts the byte on the bus at the start of
the access, which a 6502's write does not.

## Step 1: the picture

`nes_console::picture::Picture`: each frame encoded by ntsc-crt's NES
source at the subcarrier phase the previous frame left, decoded on
Rung C (the three-line comb with the NES weights), and through the CRT
stages at ntsc-crt's authored parameters, scale 3 (768 x 720). The
module adds two things to ntsc-crt's chain, the phase carried across
frames and the order; nothing in it is fitted. `run-rom` writes it
with `CRT=out.ppm` and the bare decoded grid with `DECODED=out.ppm`.
`full_palette.nes` through it: the bars, the eight emphasis bands, the
comb's picture. 30 frames through Rung C and the stages take 2.2 s on
one core (13.7 frames a second): the picture is not real time on the
CPU, which N8's shell knows from the roof's wasm figures.

Gate (`tests/picture.rs`, no die data, no goldens):

- A console frame through the picture is the standalone fast rung's
  frame from the same world through the same picture on every decoded
  sample: 1,474,560 components equal (2048 x 240 x 3), the two frames
  at the same parity. The CRT stages run on it and put something on
  the 768 x 720 screen.
- The phase the picture carries across twelve console frames (four of
  them short) is what ntsc-grid's arithmetic gives that parity
  sequence: 4. `MUTATE=1` pushes every frame as Even (a full frame's
  residue is 4, a short frame's 8) and reads 0: red.

## Step 2: the capture path, machine half

`examples/capture-score.rs`: a ROM through the console, its frames
encoded in order, and either a real record (the `.u8` at the scope's
rate the M4 tools read) or the console's last three frames through
ntsc-crt's capture-card model at the same rate (125 MHz, 5 ppm off,
20 mV of DC, 2 mV of noise), recovered the way M4 recovers a real one
(`auto_level_nes`, then `recover_nes`), and scored region by region
against the console's own synthesis through the identical decoder
(Rung C): mean luma, saturation and hue over the region, one dot in
from each edge. Regions are found in the console's frame: the largest
flat rectangle of every distinct (colour, emphasis) at least 12 dots
wide and 6 rows tall, with a bar's stepped edges merged (full_palette's
$2007 timing steps them). On `full_palette.nes`, 436 regions, 18 of
them grey.

Tolerances, as stated in the plan before measuring: luma within 0.01,
hue within 1.0 degree where the synthesis has a hue (saturation above
0.02), saturation within 5 percent of the synthesis or 0.005 absolute,
whichever is larger; a miss on any region fails the synthetic
roundtrip. The real capture is recorded, not held.

What the first run found belonged to the instrument. The luma missed
on 130 regions with a bias that grew with the level, a 3.7 percent
gain: ntsc-crt's re-referencing placed sync tip and blanking at the
centres of 4 mV histogram bins, 1.4 percent of the sync depth at each
end. Writing the test that holds it finer exposed a second weakness in
the same finder, a picture level below blanking (the NES's row-1
colours dip under it) taken for blanking once it fills a percent of the
record, which a solid dark frame does and a bars frame can. Both are
fixed in ntsc-crt 0.2.4 (the levels are read as medians inside every
sync pulse and on the front porch before it, the histogram only placing
the threshold; the M4 report's fourth addendum), the console re-pinned.

With that, the synthetic roundtrip (`+0.7 ppm recovered, worst burst
residual 0.010`):

| | regions | within | worst |
|---|---|---|---|
| luma (0.01) | 436 | 436 | 0.0096 |
| hue (1.0 degree, 418 with a hue) | 418 | 360 | 12.1 degrees, on a region of saturation 0.02 |
| saturation (5 percent or 0.005) | 436 | 431 | 0.0078 |
| all three | 436 | 367 | |

69 regions miss, so the synthetic roundtrip does not close at the
stated tolerances, and the example exits 1 saying so. The miss is one
residual: a chroma vector error of at most 0.0092, deterministic (the
same with the noise at zero), the same at the grid's own rate with the
rate error at zero, so it is not the resampling or the lock's rate; on
the saturated colours it reads as a rotation of +0.4 degrees (the burst
lock's residual, 0.010 grid samples, is that angle) and a gain scatter
of 1 percent, and on the near-greys the degree tolerance turns 0.005
of chroma into 12 degrees. What is left when the rate and the noise are
out is the card model's anti-alias lowpass, 6.5 MHz over 41 taps, on
the encoder's square-wave chroma: the synthesis side is decoded
unfiltered, and at twelve samples a cycle the square wave's eleventh
and thirteenth harmonics alias onto the fundamental the decoder reads,
so the two sides hand the identical decoder a fundamental that differs
by a percent depending on the colour's waveform. That is ntsc-crt's
comparison procedure, not the console's picture, and the plan's rule
was not to fit the tolerance to the data: the figures stand as
measured, and the procedure question (filter the synthesis with the
same front end, or state the tolerance from the aliased harmonics) is
named for the next plan.

The real half: no bars record exists (the five records on hand are
Super Mario Bros. and Duck Hunt, whose ROMs are not on this box, so no
region of theirs can be scored against a console frame). The example
takes the record and runs the same procedure; the recording of
`full_palette.nes` on the real console is the bench item, and M4's
terminated-capture finding (the probe run flattering chroma by about
40 percent) says the record must be taken terminated.

## What stays for the bench

- The bars record: `full_palette.nes` on the real console, terminated,
  at the scope's rate, then `capture-score rom frames record.u8
  125000000`.
- The emphasis lead (2c02 report): whether the real $2001 write's
  emphasis lands two dots ahead of a colour change, or whether the
  harness's data-at-start access shape made it look so.
- N5's carried items, unchanged: the NMI arrival on the scope, gate
  3's ROM, 8x16 sprites, the reset hold, the DMC fetch inside sprite
  DMA.
