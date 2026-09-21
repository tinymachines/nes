# The boards, plugged in: every cartridge on this desk has one

A Nintendo cartridge is not a box with a program in it. It is a small
circuit, and on most of them one chip does a job the console cannot do
for itself. The console can see 32 kilobytes of program and 8 kilobytes
of pictures at a time, and that is all it can ever see; a game bigger
than that keeps the rest behind that chip and asks for a slice when it
needs one. The chip is called a mapper, after the job. The board it
sits on is a shape our model has to be able to take, or the game does
not run at all.

Until the twelfth of September the model knew one board: NROM, the
plain case, no chip and nothing to switch, the whole game in view at
once. Our own Super Mario Bros. and Duck Hunt cartridge is not that
one, so GxROM came in beside it while we were getting the same bytes to
run on both sides for a picture comparison
(`5ecc62d`, and [the cartridge report](../nes-bench/docs/cartridge.md)
is the story of that dump). The other eighteen cartridges on this desk
wanted five more boards. Those arrived over about seven hours on the
nineteenth and twentieth of September.

## The seven boards, and what the desk's cartridges want

Counted off the dumps taken here with the OSCR reader:

| board | mapper | how it switches | cartridges here |
|---|---|---|---|
| NROM | 0 | it does not | 2 |
| MMC1 | 1 | one bit at a time down a serial port | 9 |
| UxROM | 2 | a whole write picks the program bank | 3 |
| CNROM | 3 | two bits of a write pick the picture bank | 2 |
| MMC3 | 4 | eight banks either way up, and a counter that watches the screen | 2 |
| MMC2 | 9 | the picture chip's own reads flip it | 1 |
| GxROM | 66 | one register picks both | 1 |

Twenty cartridges, seven boards, and no cartridge on the desk without
one.

## A header naming any other board is refused, by name

The model does not load a cartridge it half understands. A mapper
number it has no board for is turned away saying so, and so is a
cartridge whose header disagrees with the board it names:

    mapper 5 is out of scope; this console has NROM (0), MMC1 (1),
    UxROM (2), CNROM (3), MMC3 (4), MMC2 (9) and GxROM (66)

    mapper 2 with 32 KiB of CHR ROM is not a UxROM board;
    UxROM carries CHR RAM

That second refusal matters more than it looks. A file can say
anything, and a board that quietly accepted a cartridge built the other
way round would run it wrong rather than not at all, which is the
harder failure to notice.

## Two of the boards could only be tested with a console around them

The rest of each board's behaviour is held in nes-bus's own tests, a
register at a time, from outside. Two of them cannot be tested that way
at all.

**MMC1 listens one bit at a time.** A write to its window carries a
single bit, and a bank number takes five of them. The catch is that two
writes on consecutive processor cycles count as one, and an
increment-in-place instruction is exactly such a pair: the processor
writes the old value back before it writes the new one. Whether the
second write counts is decided by the clock cycle it lands on, and the
only part that knows the cycle is the console. So a cartridge is now
handed the cycle with every write, and the test runs a program that
does the increment and reads the bank back. Run against a board that is
not told the cycle, the same program shifts twice and the count comes
back 2, which is how we know the test is looking at the thing it says
it is.

**MMC2 is not switched by the program at all.** Each half of its
picture memory has two bank registers and a latch that says which one
answers, and it is the picture chip's own reads that flip the latch:
the tile that draws the top of Little Mac's head is also the switch
that changes the tiles. The test sets the four registers from outside,
runs a program that fills a screen with one tile and turns the picture
on, and then writes nothing more. What it holds is that the latch
moved and no register did. It checks its own program first, because a
screen we never filled would draw tile zero and move nothing, and a
test that passes on nothing is not a test.

## MMC3 splits the screen, and that needed a line rather than a level

MMC3 is the board Super Mario Bros. 2 and 3 are on, and the reason
those games can hold a status bar still while the rest of the screen
scrolls. It counts scan lines by watching one address line from the
picture chip, and when its count runs out it pulls the interrupt line
low and the game's own code takes the screen back.

Two things had to change before that worked.

The picture chip had to fetch the sprite slots it is not going to draw.
On the real part a line with no sprites on it still moves that address
line, because the chip goes looking anyway; our stepper had been
skipping the work it knew was pointless, and skipped the evidence with
it (2c02 @ `10b9089`).

The interrupt had to become a line instead of a level. It had been read
whenever the processor next happened to look, which is not what a wire
does. It is now held behind the board by 17 master half-steps, twelve
of which make a processor half-cycle and eight a dot, which is the only
grain fine enough to hold a one-clock bracket. That number is a fit and
is labelled as one: `examples/irq-sweep` runs the test ROM at every
delay and prints what each reports, the ROM allows fourteen through
twenty-one and nothing outside, and seventeen is the middle of that
band. With it, blargg's `4-scanline_timing` passes.

The filter on that address line moved too. Three falling edges of the
processor clock is the rule the real board has; nine dots was the wrong
rounding of it, and at nine the first pattern fetch of line zero was
counted when the background sits at $1000, which made a frame 242
clocks on alternate frames where the part makes 241. It counts ten dots
now (nes-bus v0.1.6).

## Nineteen of the twenty draw their own picture

One does not, and it is not the board's fault. Super Mario Bros.'s only
dump is one the reader's own checksum could not match against the
database, so that cartridge wants reading again before its blank screen
means anything.

"Draws its own picture" is the whole claim. Nothing here has been
played past its title screen with a controller in somebody's hands, so
what the console can say on its own is that the game's own program put
the game's own picture on the screen, and that is what it says.

## Five games were stuck on a branch, not on a board

Worth recording because the first guess was wrong for an afternoon.
Paperboy, Goonies II and Blaster Master never turned the picture on,
and the Legend of Zelda and Battle Chess drew one flat colour. Three of
blargg's vertical blank tests had started failing where the N5 report
records them passing, one of them saying the vertical blank period was
way off.

It was not. `examples/vbl-probe` puts the flag's rise at dot 2 of line
241, its fall at dot 2 of line 261, and the period at 29780 or 29781
processor cycles, with the picture on or off, which is exact. What was
wrong was a single branch instruction in the processor: the test reads
the status register and branches on its top bit, and the core had
stopped taking that branch when it should, an hour earlier, in a search
that was fixing a different branch (6502 @ `3805107`). All five games
draw now and the vertical blank tests are back where the N5 report
records them.

A board that does not work and a processor that does not branch look
identical from the far end of a blank screen. The walk from one to the
other was `examples/where-it-sits`, which takes a stopped cartridge
back to the instruction it stopped on.

## What is not settled

- **The address-line filter counts dots where the real board counts
  clock edges.** Ten dots agrees with the part everywhere blargg's ROMs
  look, and no ROM here can tell the difference, so this is open on the
  argument rather than on the evidence. It is in nes-bench's open
  items with the case that would decide it.
- **The interrupt delay is a fit.** Seventeen half-steps is more than
  half a processor cycle, which is far too long for a wire from pin 15,
  so most of what it stands in for is probably where inside its cycle
  the processor samples the interrupt rather than anything the
  cartridge does. A scope on pin 15 against the processor's clock would
  turn the middle of a band into a number.
- **Super Mario Bros. wants dumping again**, and until it is, one of
  the twenty is unaccounted for.
- **No game has been played**, in the ordinary sense of the word.

## Where these figures come from

- The boards and the refusals: `crates/nes-console/src/ines.rs`.
- The console-side tests: `crates/nes-console/tests/mappers.rs`, six of
  them, and `crates/nes-console/tests/mmc3.rs`, two.
- Each board's own logic, a register at a time: nes-bus's contract
  suite, v0.1.4 for MMC1, UxROM and CNROM, v0.1.5 for MMC2, v0.1.6 for
  the filter.
- The interrupt delay: `CART_IRQ_DELAY` in
  `crates/nes-console/src/console.rs`, swept by `examples/irq-sweep`.
- The filter: `A12_FILTER_DOTS` in nes-bus, checked by
  `examples/mmc3-probe`, which runs its own cartridge either way so the
  frame count reads straight off.
- The cartridge counts: the dumps on this desk, read with the OSCR.
- The dates: `72200f5` (MMC3), `6d79167` (MMC1, UxROM, CNROM),
  `fda1884` (MMC2).
