#!/usr/bin/env python3
"""Every number this repository writes about ITSELF, checked against the thing
it describes.

Built 2026-09-21 after the second stale number in a week. `CART_IRQ_DELAY`
moved from 16 to 17 when the IRQ watch was made exact; the constant moved, the
open item moved, the sweep tool moved, and `tests/mmc3.rs`'s own header went on
saying "sixteen master half-steps" until somebody read it. Nothing here checks
prose against a constant, which is the gap the 6502 repository closed with a
tool of this name and this one did not have.

WHAT IS WORTH MECHANISING, and it is a short list on purpose: a claim that is
an exact quantity with one obvious measurement. The 6502's version learned this
the expensive way -- a general scan of prose for numbers raised 53 flags, every
one a subset claim, and a check that cries wolf is one nobody runs.

So: the constants that prose spells out in words, the counts of things the code
enumerates, and the internal consistency of a band that is quoted three ways.

WHAT THIS DOES NOT COVER, on purpose:
  - anything measured on the part. The capture scores, the ppm, the registration
    and the origin differences are measurements of hardware, not counts of this
    repository, and they move when the bench moves.
  - the blargg ROM results beyond how many the suite walks. Whether a ROM passes
    is what the test run says, and the test run is the oracle for that already.
  - frame and dot arithmetic (89342 dots, 16.6357 ms). Those are derived from
    the NTSC ratios and belong to ntsc-grid, which holds them itself.

Usage:  python3 tools/check-self-counts.py
        REQUIRE_ALL=1 python3 tools/check-self-counts.py   # a skip becomes a failure
Exit 1 on any disagreement.
"""
from __future__ import annotations

import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REQUIRE_ALL = os.environ.get("REQUIRE_ALL") == "1"

WORDS = {
    "zero": 0, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6,
    "seven": 7, "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12,
    "thirteen": 13, "fourteen": 14, "fifteen": 15, "sixteen": 16,
    "seventeen": 17, "eighteen": 18, "nineteen": 19, "twenty": 20,
    "twenty-one": 21, "twenty-two": 22, "twenty-three": 23, "twenty-four": 24,
}
WORD_OF = {v: k for k, v in WORDS.items()}


def as_number(text: str) -> int:
    """A claim is written as digits or as a word. The word form is the one that
    goes stale quietly, because nobody greps for 'seventeen'."""
    t = text.strip().lower().replace(",", "")
    if t.isdigit():
        return int(t)
    if t in WORDS:
        return WORDS[t]
    raise ValueError(f"not a number this check understands: {text!r}")


def normalise(text: str) -> str:
    """Doc comments wrap, so a claim spans lines with `///` or `//!` in the
    middle of it. Strip the comment markers and collapse whitespace so a
    pattern can be written the way the sentence reads rather than the way it
    happens to be wrapped. Markdown wraps too, which is why this runs on
    everything and not only on Rust."""
    text = re.sub(r"(?m)^\s*//[/!]?", " ", text)
    return re.sub(r"\s+", " ", text)


class Skip(Exception):
    pass


def read(rel: str) -> str:
    p = ROOT / rel
    if not p.exists():
        raise Skip(f"{rel} absent")
    return p.read_text()


# ---------------------------------------------------------------------------
# Measurements. Each returns an int or raises Skip.
# ---------------------------------------------------------------------------
def rust_const(rel: str, name: str) -> int:
    """The constant as the compiler sees it, not as a comment describes it."""
    m = re.search(rf"pub const {name}\s*:\s*\w+\s*=\s*(\d[\d_]*)\s*;", read(rel))
    if not m:
        raise Skip(f"{name} not found in {rel}")
    return int(m.group(1).replace("_", ""))


def cart_irq_delay() -> int:
    return rust_const("crates/nes-console/src/console.rs", "CART_IRQ_DELAY")


def boards() -> int:
    """How many mappers `Ines::cart` can actually build: the match arms, which
    is the list that grows when a board is added. Counting the doc that
    describes them would be counting the claim."""
    src = read("crates/nes-console/src/ines.rs")
    start = src.find("pub fn cart")
    if start < 0:
        raise Skip("could not find Ines::cart")
    # To the next method, not to the next closing brace: the arms have braces
    # of their own, and a non-greedy match to the first `}` stops inside the
    # first arm. That version measured 2 boards where there are 7, and the
    # check caught it, which is the only reason this comment exists.
    nxt = src.find("\n    pub fn ", start + 1)
    block = src[start:] if nxt < 0 else src[start:nxt]
    arms = re.findall(r"^\s+(\d+) =>", block, re.M)
    if not arms:
        raise Skip("found Ines::cart but no mapper arms in it")
    if len(set(arms)) != len(arms):
        raise Skip(f"the same mapper appears twice in cart(): {arms}")
    return len(arms)


def mmc3_roms_walked() -> int:
    """The ROMs tests/mmc3.rs actually runs, which is the passing set."""
    src = read("crates/nes-console/tests/mmc3.rs")
    m = re.search(r"for rom in \[([^\]]*)\]", src)
    if not m:
        raise Skip("could not find the ROM list in tests/mmc3.rs")
    return len(re.findall(r'"', m.group(1))) // 2


def tests_in(crate: str) -> int:
    """`#[test]` attributes in a crate. Checked against cargo when this was
    written: nes-console reported 38 both ways."""
    d = ROOT / "crates" / crate
    if not d.is_dir():
        raise Skip(f"crates/{crate} absent")
    n = 0
    for p in list(d.rglob("*.rs")):
        if "target" in p.parts:
            continue
        n += len(re.findall(r"^\s*#\[test\]", p.read_text(), re.M))
    if n == 0:
        raise Skip(f"crates/{crate} has no #[test] at all, which is not a count")
    return n


MEASURE = {
    "CART_IRQ_DELAY": cart_irq_delay,
    "boards cart() builds": boards,
    "mmc3 ROMs walked": mmc3_roms_walked,
    "nes-console tests": lambda: tests_in("nes-console"),
    "nes-glue tests": lambda: tests_in("nes-glue"),
    # The clock ratios, which the prose spells out beside the constant. A CPU
    # half-cycle is 12 master half-steps and a PPU dot is 8, both fixed by the
    # NTSC divisors (master/12 to the CPU, master/4 to the PPU, and a half-step
    # is half a master clock). They cannot be measured out of this repository,
    # so they are stated once HERE and the derived check below holds them to
    # the three-dots-a-cycle ratio that makes them consistent.
    "master half-steps a CPU half-cycle": lambda: 12,
    "master half-steps a dot": lambda: 8,
}

# ---------------------------------------------------------------------------
# The claims. Each pattern must match EXACTLY ONCE in its file, after
# normalisation, and capture the number. Matching twice or not at all is a
# failure: a claim that moved out from under its pattern is exactly what this
# is looking for.
# ---------------------------------------------------------------------------
CLAIMS = [
    ("README.md", r"holds it behind the board by (\w+) master half-steps", "CART_IRQ_DELAY"),
    ("README.md", r"and (\w+) is the middle\.", "CART_IRQ_DELAY"),
    ("README.md", r"master half-steps, (\w+) to a CPU half-cycle", "master half-steps a CPU half-cycle"),
    ("README.md", r"to a CPU half-cycle and (\w+) to a dot", "master half-steps a dot"),
    ("crates/nes-console/src/console.rs", r"and (\w+) is the middle of that band", "CART_IRQ_DELAY"),
    ("crates/nes-console/tests/mmc3.rs", r"(\w+) of the six pass", "mmc3 ROMs walked"),
    ("docs/boards-report.md", r"## The (\w+) boards, and what", "boards cart() builds"),
    ("docs/boards-report.md", r"Twenty cartridges, (\w+) boards", "boards cart() builds"),
]

# ---------------------------------------------------------------------------
# Derived checks: claims that are consistent or not on their own terms, with
# nothing to measure them against. The band is the whole reason this file
# exists, and it is quoted three ways in two files.
# ---------------------------------------------------------------------------
def band() -> tuple[int, int]:
    """The band irq-sweep found, as README and console.rs both state it."""
    src = normalise(read("crates/nes-console/src/console.rs"))
    lo = re.findall(r"allows (\w+) through", src)
    hi = re.findall(r"allows \w+ through ([\w-]+) and no further", src)
    if len(lo) != 1 or len(hi) != 1:
        raise Skip(f"the band is stated {len(lo)} times in console.rs, want 1")
    return as_number(lo[0]), as_number(hi[0])


def derived():
    try:
        lo, hi = band()
    except (Skip, ValueError) as e:
        yield None, f"the band could not be read: {e}"
        return
    delay = cart_irq_delay()

    yield lo < hi, f"the band reads {lo} through {hi}, which is not a band"
    yield lo <= delay <= hi, (
        f"CART_IRQ_DELAY is {delay}, outside the band {lo}..{hi} the prose says the ROM allows")
    yield delay == (lo + hi) // 2, (
        f"the prose calls {delay} the middle of {lo}..{hi}; the middle is {(lo + hi) // 2}")

    # "eight values wide", stated in console.rs beside the band.
    wide = re.findall(r"and no further, ([\w-]+) values wide", normalise(read("crates/nes-console/src/console.rs")))
    if len(wide) != 1:
        yield None, f"'values wide' is stated {len(wide)} times in console.rs, want 1"
    else:
        want = hi - lo + 1
        yield as_number(wide[0]) == want, (
            f"the band {lo}..{hi} is {want} values wide, the prose says {wide[0]}")

    # A CPU cycle is three PPU dots, so twice the CPU half-cycle's half-steps
    # must equal three times the dot's. If either number in the prose is ever
    # edited alone, this is what catches it.
    a = MEASURE["master half-steps a CPU half-cycle"]()
    b = MEASURE["master half-steps a dot"]()
    yield 2 * a == 3 * b, (
        f"{a} half-steps a CPU half-cycle and {b} a dot do not make three dots to a CPU cycle")


def main() -> int:
    cache: dict[str, object] = {}

    def measured(key: str):
        if key not in cache:
            try:
                cache[key] = MEASURE[key]()
            except Skip as e:
                cache[key] = e
        return cache[key]

    ok = fail = skip = 0
    for rel, pattern, key in CLAIMS:
        try:
            text = normalise(read(rel))
        except Skip as e:
            print(f"SKIP {rel}: {e}")
            skip += 1
            continue
        hits = re.findall(pattern, text)
        if len(hits) != 1:
            print(f"FAIL {rel}: pattern for '{key}' matched {len(hits)} times, want 1")
            print(f"     {pattern}")
            fail += 1
            continue
        want = measured(key)
        if isinstance(want, Skip):
            print(f"SKIP {rel}: {key} not measurable ({want})")
            skip += 1
            continue
        try:
            claimed = as_number(hits[0])
        except ValueError as e:
            print(f"FAIL {rel}: {e}")
            fail += 1
            continue
        if claimed == want:
            ok += 1
        else:
            shown = want if hits[0].strip().isdigit() else WORD_OF.get(want, want)
            print(f"FAIL {rel}: says {hits[0].strip()} for {key}, measured {want}")
            print(f"     write: {shown}")
            fail += 1

    for good, why in derived():
        if good:
            ok += 1
        elif good is None:
            print(f"SKIP derived: {why}")
            skip += 1
        else:
            print(f"FAIL derived: {why}")
            fail += 1

    print(f"\n{ok} claim(s) agree, {fail} disagree, {skip} skipped.")
    if skip and REQUIRE_ALL:
        print("REQUIRE_ALL=1: a skip is a failure")
        return 1
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
