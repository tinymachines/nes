# N4 report: the glue, authored

Run stamp: 2026-09-05, rustc 1.97.1, nes-bus v0.1.1. `cargo test
--workspace`: 16 tests green, five test files, one per part. Nothing
here goes through halfphi and nothing is measured off a die; the console
sketch (section 3, N4) asks for exactly that, each part a few lines held
to its datasheet, labelled authored, with its own small test. This crate
is `crates/nes-glue`.

## The parts

| part | as wired on the NES-001 | held to |
|---|---|---|
| U3, 74LS139 | half A: 1G = A15, 1A = A13, 1B = A14, so 1Y0 is /RAM CS ($0000..$1FFF) and 1Y1 /PPU CS ($2000..$3FFF); half B: 2G grounded, 2A = M2, 2B = A15, so 2Y3 is /ROMSEL = not (A15 and M2) | the SN74LS139A function table row for row, the two windows over every 256-byte page, and the M2 term: A15 high with M2 low must not select the cartridge |
| U8, 74LS373 | LE = PPU ALE, D = AD0..AD7, /OE grounded; Q is PPU A0..A7 at the cartridge edge | transparent while high, held from the fall; and the one case where the 2C02 harness's rising-edge sample would differ (AD moving inside the pulse) is shown, the datasheet keeping the value at the fall |
| U1 and U4, TMM2115 | CPU WRAM behind /RAM CS, mirrored four times by the lines the part lacks; PPU CIRAM behind the cartridge's /CIRAM CE and CIRAM A10 | eleven address lines, /CS and /WE as the datasheet says, a power-on fill that is visibly not data; the -12 grade's 120 ns access time recorded and unused |
| U9 and U10, 74LS368 | enabled by /OE1 and /OE2; D0 from each port's data line inverted, D3 and D4 (port 1) and D1..D4 (port 2) from the expansion port | inverting; every undriven bit is the bus's own value, taken in and merged around, never invented |
| the controller, 4021 | on the other end of the port | loads while OUT0 is high, latches at its fall (the buttons as they stand then), clocks A B Select Start Up Down Left Right out one read at a time, and shows a low line (D0 = 1) past the eighth read, as documented |
| the 74HC04 | PPU A13 inverted onto cartridge pin 58 | inversion |
| the reset chain | RST_PB and the CIC's reset output onto /RESET | asserted while the button is down, held for `HOLD_MASTER_CYCLES` after power good and after the button's release |

## What the tests taught

Two of the six parts were authored wrong the first time and the tests
said so, which is the point of writing them against the datasheet
rather than against the code:

- **The controller latches at the strobe's fall.** The first draft
  loaded the register at OUT0's rise; a button pressed while the strobe
  was high would then have been missed. The 4021 loads continuously
  while its parallel/serial input is high and keeps what it holds as
  that input falls. And past the eighth read the port line sits low, so
  the CPU reads a 1 on D0, which the first draft had as a released
  button; nesdev's page is the claim, and the module comment now says
  which level the line shows and why.
- **A raw A12 watcher sees eight rises a line.** Over a synthetic line
  of the PPU's measured fetch schedule (the P3 plan's positions:
  background from pattern table 0, sprites from table 1 on dots
  257..320, two garbage nametable fetches before each sprite's pattern
  pair), A12 rises at every sprite, because the garbage fetches take it
  low for two latch falls between them. The MMC3 counts one per line
  because it filters rises whose low was short (stated as three or more
  M2 falls, about nine dots); `A12Watcher` carries that filter in latch
  falls, and the test holds both counts, 8 x 240 raw and 240 filtered,
  so the reason for the filter is a number and not a sentence.

## Authored and labelled, awaiting the bench

- `reset::HOLD_MASTER_CYCLES` is a placeholder: about 50 ms of the
  master clock, a round figure inside the range the lockout handshake
  takes. The sketch's section 5 names the capture that replaces it
  (CPU_RST, RST_PB, CIC_RST and PWR_LED at power-on, 1 MSa/s, a long
  record), and the test pins the number to its label so a change comes
  with a new label or the capture that retires both.
- `sram::ACCESS_TIME_NS_GRADE_12` is a datasheet transcription used by
  nothing; contention, when someone models it, is where it goes.
- The controller's ninth-read level and the expansion lines' idle
  levels are nesdev's claims, not measurements.

## Carried

- Bus arbitration, open bus as a value, the master clock and the two
  chips' alignment are N5's: this crate takes levels in and gives levels
  out, and knows no chip.
- A cartridge's own decode of $6000..$7FFF and any mapper beyond NROM
  are out of scope, as the sketch says.
