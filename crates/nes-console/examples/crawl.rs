//! The crawl: the game played automatically until nothing new is
//! reached. From a saved moment, each pad byte in a small set is held
//! for a stretch of frames; the trace says which PRG offsets the CPU
//! fetched opcodes from; a step that reached code no step had reached
//! before keeps its end as a new moment, and the moments that found the
//! most are tried first. Behind them come the steps that wrote a value
//! into RAM the game had never written (a position, a state); a level
//! has to be walked through before its next routine runs, so deeper
//! chains go first among equals; a step that did neither, or ended in a
//! console already seen, is dropped. WEIGHT=1 makes each new value worth
//! less the more values its address has shown, so a counter or a
//! temporary says nothing by ticking; tried on the multicart over 1600
//! steps it reached 5306 opcode sites against 5462 for the plain count,
//! so the plain count stays the default and the rule stays as a switch.
//! Bounded by a step budget and a frontier cap, so it ends. BATCH
//! moments (4) are taken at a time and all their actions run together
//! on THREADS threads (all the cores); BATCH=1 is one moment at a time.
//! PICTURE=1 also keeps a step that ended on a picture not seen before,
//! so the frontier does not run dry; tried on two games that had run dry
//! before 6400 steps, it reached 14002 opcode sites against 13839 and
//! 4511 against 4327, a hundred or two for the rest of the budget, so it
//! too stays a switch.
//!
//!   cargo run --release -p nes-console --example crawl -- ROM.nes OUTDIR [STEPS] [START]
//!
//! START is a script of `AT <frame> <hh>` lines played first (the way
//! into a game: past its menu and title); the crawl begins where it
//! ends; its `# frames N` line, if any, says where. HOLD=n frames a step
//! holds its pad (20), FRONTIER=n moments kept (1000), WEIGHT=1 the
//! weighted novelty, NOVELTY=1 a report of which RAM addresses took the
//! most values. OUTDIR gets:
//!
//!   crawl.json      the coverage, in the flow report's shape: prg_len,
//!                   frames, instructions, sites (key, addr, count) and
//!                   no routines, so `listing from ROM crawl.json` reads it
//!   scripts/N.txt   the path from power-on to each moment that found new
//!                   code, as `AT` lines, with `# frames N` for script-trace
//!   best.txt        the longest of those
//!
//! A commercial cartridge's coverage names addresses and counts and no
//! bytes; its scripts are pad bytes by frame. Both may leave the ROM
//! store; a trace may not, and none is kept.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::io::Write;

use nes_console::record::{F_READ, F_SYNC, KIND_CYCLE, RECORD_BYTES};
use nes_console::{ines, Alignment, Console};
use nes_glue::controller::Buttons;

/// The pad bytes a step holds (A, B, Select, Start, Up, Down, Left,
/// Right from bit 0), each for HOLD frames, and one long wait with
/// nothing held, for the stretches a game ignores its pad.
const ACTIONS: [u8; 15] = [0x00, 0x01, 0x02, 0x08, 0x04, 0x10, 0x20, 0x40, 0x80, 0x81, 0x41, 0x82, 0x83, 0x21, 0x11];
const WAIT_FRAMES: usize = 120;

/// What one step found, computed on its own thread.
struct Step {
    pad: u8,
    hold: usize,
    sites: Vec<(u32, u16, u32)>,
    /// Every (address, value) the CPU wrote into its 2 KiB of RAM, once.
    writes: Vec<u32>,
    instructions: u64,
    state: Vec<u8>,
    digest: Option<u32>,
}

/// The opcode fetches from the ROM in a stretch of trace, folded to
/// (offset, address, count), sorted by offset.
fn sites_of(trace: &[u8]) -> (Vec<(u32, u16, u32)>, Vec<u32>, u64) {
    let mut m: std::collections::HashMap<u32, (u16, u32)> = std::collections::HashMap::new();
    let mut writes: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut instructions = 0u64;
    for r in trace.chunks_exact(RECORD_BYTES) {
        let flags = r[3];
        if flags & 0xc0 != KIND_CYCLE {
            continue;
        }
        if flags & F_READ == 0 {
            let a = u16::from_le_bytes([r[0], r[1]]);
            if a < 0x2000 {
                writes.insert(((a & 0x7ff) as u32) << 8 | r[2] as u32);
            }
            continue;
        }
        if flags & F_SYNC == 0 {
            continue;
        }
        instructions += 1;
        let p = u32::from_le_bytes([r[4], r[5], r[6], r[7]]);
        if p == 0 {
            continue;
        }
        let e = m.entry(p - 1).or_insert((u16::from_le_bytes([r[0], r[1]]), 0));
        e.1 += 1;
    }
    let mut v: Vec<(u32, u16, u32)> = m.into_iter().map(|(o, (a, c))| (o, a, c)).collect();
    v.sort_unstable();
    (v, writes.into_iter().collect(), instructions)
}

fn run_step(rom: &[u8], state: &[u8], pad: u8, hold: usize) -> Step {
    let mut c = at(rom, state);
    c.set_pad(0, Buttons::from_byte(pad));
    c.run_frames(hold);
    // A state is taken where a CPU cycle ends, and a frame can end
    // in the middle of one.
    c.run_to_cycle_end();
    let tr = take(&mut c);
    let (sites, writes, instructions) = sites_of(&tr);
    // The pictures so far are not the console: a state that carried
    // every frame since power-on grew by the path's length (tens of
    // megabytes a moment, and a 1600-step crawl took 52 GB before the
    // kernel stopped it), and two paths to the same console hashed
    // apart. `last_frame_digest` is what a step needs of them.
    c.frames.clear();
    Step { pad, hold, sites, writes, instructions, state: c.save_state().expect("save"), digest: c.last_frame_digest }
}

struct Moment {
    state: Vec<u8>,
    script: Vec<(u32, u8)>,
    frames: u32,
    digest: Option<u32>,
    gain: usize,
    /// The RAM novelty score, in thousandths.
    novelty: usize,
    depth: u32,
    id: u32,
}

impl PartialEq for Moment {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}
impl Eq for Moment {}
impl PartialOrd for Moment {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Moment {
    /// More new code first, then more new RAM values, then the deeper
    /// (a chain that keeps moving the game on keeps its turn), then the
    /// older.
    fn cmp(&self, o: &Self) -> Ordering {
        self.gain.cmp(&o.gain).then(self.novelty.cmp(&o.novelty)).then(self.depth.cmp(&o.depth)).then(o.id.cmp(&self.id))
    }
}

struct Coverage {
    count: Vec<u32>,
    addr: Vec<u16>,
    instructions: u64,
    frames: u64,
    covered: usize,
    /// (RAM address, value) pairs ever written: 2048 x 256 bits.
    ram: Vec<u64>,
    /// Distinct values seen per RAM address: where the novelty came from.
    distinct: Vec<u32>,
    /// WEIGHT=1: a new value is worth one over its address's distinct count.
    weighted: bool,
}

impl Coverage {
    /// A step's RAM writes folded in, as a novelty score in thousandths.
    /// Progress through a game shows in RAM before it shows as code: a
    /// position, a state the game had not been in. But a byte that
    /// takes a new value every step (a frame counter, a random byte, a
    /// temporary, a sprite's coordinate) says nothing by doing so, so a
    /// new value at an address is worth one over the number of distinct
    /// values that address has shown: the first is worth 1, the
    /// hundredth 0.01. Measured on the multicart before this rule: four
    /// addresses ran through all 256 values and fifty-one through 64 or
    /// more, and those fifty-five carried half of all the new pairs. Yet
    /// the plain count found more code in the same 1600 steps (5462
    /// sites against 5306), so the weighting is WEIGHT=1, off by default.
    fn fold_writes(&mut self, writes: &[u32]) -> usize {
        let mut score = 0.0f64;
        for &w in writes {
            let (i, b) = ((w >> 6) as usize, w & 63);
            if self.ram[i] & (1 << b) == 0 {
                self.ram[i] |= 1 << b;
                let a = (w >> 8) as usize;
                self.distinct[a] += 1;
                score += if self.weighted { 1.0 / self.distinct[a] as f64 } else { 1.0 };
            }
        }
        (score * 1000.0).round() as usize
    }

    /// A step's sites folded in. Returns how many offsets were fetched
    /// for the first time.
    fn fold(&mut self, sites: &[(u32, u16, u32)], instructions: u64) -> usize {
        let mut new = 0;
        self.instructions += instructions;
        for &(o, a, n) in sites {
            let o = o as usize;
            if o >= self.count.len() {
                continue;
            }
            if self.count[o] == 0 {
                new += 1;
                self.covered += 1;
                self.addr[o] = a;
            }
            self.count[o] += n;
        }
        new
    }
}

fn power_on(rom: &[u8]) -> Console {
    let r = ines::parse(rom).unwrap_or_else(|e| panic!("{e:?}"));
    let chr_ram = r.chr_ram.then(|| vec![0u8; 0x2000]);
    let cart = r.cart().unwrap_or_else(|e| panic!("{e:?}"));
    let mut c = Console::with_prg_ram(cart, chr_ram, Alignment::default(), true);
    c.trace = Some(Default::default());
    c
}

fn at(rom: &[u8], state: &[u8]) -> Console {
    let mut c = power_on(rom);
    c.load_state(state).unwrap_or_else(|e| panic!("{e}"));
    c.trace = Some(Default::default());
    c
}

fn take(c: &mut Console) -> Vec<u8> {
    c.trace.as_mut().map(|t| std::mem::take(&mut t.bytes)).unwrap_or_default()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: crawl ROM.nes OUTDIR [STEPS] [START]");
        std::process::exit(2);
    }
    let rom = std::fs::read(&a[1]).expect("the ROM");
    let out = std::path::Path::new(&a[2]);
    std::fs::create_dir_all(out.join("scripts")).expect("the output directory");
    let steps: usize = a.get(3).map(|s| s.parse().expect("STEPS")).unwrap_or(2000);
    let hold: usize = std::env::var("HOLD").ok().map(|s| s.parse().expect("HOLD")).unwrap_or(20);
    let cap: usize = std::env::var("FRONTIER").ok().map(|s| s.parse().expect("FRONTIER")).unwrap_or(1000);
    let prg_len = ines::parse(&rom).unwrap().prg.len();
    let mut cov = Coverage { count: vec![0; prg_len], addr: vec![0; prg_len], instructions: 0, frames: 0, covered: 0, ram: vec![0; 2048 * 256 / 64], distinct: vec![0; 2048], weighted: std::env::var("WEIGHT").is_ok() };

    // The way in.
    let mut c = power_on(&rom);
    let mut script: Vec<(u32, u8)> = Vec::new();
    let mut frames = 0u32;
    if let Some(p) = a.get(4) {
        let mut at_lines: Vec<(u32, u8)> = std::fs::read_to_string(p)
            .expect("START")
            .lines()
            .filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                (w.len() == 3 && w[0] == "AT").then(|| (w[1].parse().expect("a frame"), u8::from_str_radix(w[2], 16).expect("a byte")))
            })
            .collect();
        at_lines.sort();
        let named_end: Option<u32> = std::fs::read_to_string(p).unwrap().lines().find_map(|l| l.strip_prefix("# frames ").and_then(|n| n.trim().parse().ok()));
        let end = named_end.unwrap_or_else(|| at_lines.last().map(|x| x.0 + 1).unwrap_or(0));
        let mut next = 0;
        let mut pad = 0u8;
        for f in 0..end {
            while next < at_lines.len() && at_lines[next].0 <= f {
                pad = at_lines[next].1;
                script.push((f, pad));
                next += 1;
            }
            c.set_pad(0, Buttons::from_byte(pad));
            c.run_frames(1);
        }
        frames = end;
        c.run_to_cycle_end();
        let t = take(&mut c);
        cov.frames += end as u64;
        let (sites, writes, instructions) = sites_of(&t);
        cov.fold(&sites, instructions);
        cov.fold_writes(&writes);
        eprintln!("the way in: {end} frames, {} opcode sites of PRG reached", cov.covered);
    }
    c.run_to_cycle_end();
    c.frames.clear();
    let root = Moment { state: c.save_state().expect("save"), script, frames, digest: c.last_frame_digest, gain: 1, novelty: 1, depth: 0, id: 0 };
    let mut frontier = BinaryHeap::new();
    frontier.push(root);
    let mut seen: HashSet<u64> = HashSet::new();
    let pictures = std::env::var_os("PICTURE").is_some();
    let mut shown_pictures: HashSet<u32> = HashSet::new();
    let mut next_id = 1u32;
    let mut kept = 0usize;
    let mut best: Option<(u32, Vec<(u32, u8)>)> = None;
    let t0 = std::time::Instant::now();
    let mut step = 0usize;
    let mut shown = 0usize;
    let threads: usize = std::env::var("THREADS").ok().map(|s| s.parse().expect("THREADS")).unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2));
    // BATCH moments are taken off the frontier at a time and all their
    // actions run across the threads together: one moment's actions are
    // sixteen, the longest six times the others, so a moment at a time
    // leaves most of the cores waiting on its one long wait. The batch
    // is a number of its own, not the thread count, so a crawl is the
    // same crawl on any machine.
    let batch: usize = std::env::var("BATCH").ok().map(|s| s.parse().expect("BATCH")).unwrap_or(4).max(1);
    while step < steps {
        let moments: Vec<Moment> = (0..batch).map_while(|_| frontier.pop()).collect();
        if moments.is_empty() {
            break;
        }
        let mut actions: Vec<(u8, usize)> = ACTIONS.iter().map(|&p| (p, hold)).collect();
        actions.push((0x00, WAIT_FRAMES));
        // Every action from every moment of the batch, the long ones
        // dealt out first so no thread gets two while another has none.
        let mut tasks: Vec<(usize, usize)> = (0..moments.len()).flat_map(|mi| (0..actions.len()).map(move |ai| (mi, ai))).collect();
        tasks.sort_by_key(|&(mi, ai)| (std::cmp::Reverse(actions[ai].1), mi, ai));
        let mut done: Vec<(usize, usize, Step)> = std::thread::scope(|sc| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let mine: Vec<(usize, usize)> = tasks.iter().copied().skip(t).step_by(threads).collect();
                    let (rom, moments, actions) = (&rom, &moments, &actions);
                    sc.spawn(move || mine.into_iter().map(|(mi, ai)| (mi, ai, run_step(rom, &moments[mi].state, actions[ai].0, actions[ai].1))).collect::<Vec<_>>())
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("a step thread")).collect()
        });
        // Folded in the order one moment at a time would have run them.
        done.sort_by_key(|d| (d.0, d.1));
        for (mi, _, r) in done {
            let m = &moments[mi];
            step += 1;
            cov.frames += r.hold as u64;
            let new = cov.fold(&r.sites, r.instructions);
            let novelty = cov.fold_writes(&r.writes);
            let h = {
                use std::hash::{Hash, Hasher};
                let mut s = std::collections::hash_map::DefaultHasher::new();
                r.state.hash(&mut s);
                s.finish()
            };
            // PICTURE=1: a step that ended on a picture no step had ended
            // on is kept too, behind everything that found code or wrote
            // a new value (its gain and novelty are nothing), so the
            // frontier does not run dry while the game can still be
            // walked somewhere it has not been seen.
            let fresh = pictures && r.digest.is_some_and(|d| shown_pictures.insert(d));
            if !seen.insert(h) || (new == 0 && novelty == 0 && !fresh) {
                continue; // the same console again, or nothing the game had not done
            }
            let mut script = m.script.clone();
            script.push((m.frames, r.pad));
            let frames = m.frames + r.hold as u32;
            let child = Moment { state: r.state, script, frames, digest: r.digest, gain: new, novelty, depth: m.depth + 1, id: next_id };
            next_id += 1;
            if new > 0 {
                kept += 1;
                let mut f = std::fs::File::create(out.join("scripts").join(format!("{:04}.txt", kept))).expect("a script");
                writeln!(f, "# frames {frames}").unwrap();
                writeln!(f, "# reached {new} new opcode sites of PRG, {} in all, at step {step}", cov.covered).unwrap();
                for (fr, p) in &child.script {
                    writeln!(f, "AT {fr} {p:02X}").unwrap();
                }
                if best.as_ref().map_or(true, |b| frames > b.0) {
                    best = Some((frames, child.script.clone()));
                }
            }
            frontier.push(child);
        }
        if frontier.len() > cap {
            let mut v = frontier.into_sorted_vec(); // ascending
            v.drain(..v.len() - cap);
            frontier = v.into_iter().collect();
        }
        if step / 640 != shown {
            shown = step / 640;
            eprintln!("step {step}: {} of {prg_len} opcode sites of PRG reached ({:.1}%), frontier {}, {} moments found new code, {:.0} s", cov.covered, cov.covered as f64 * 100.0 / prg_len as f64, frontier.len(), kept, t0.elapsed().as_secs_f64());
        }
    }
    let sites: Vec<String> = (0..prg_len).filter(|&o| cov.count[o] > 0).map(|o| format!("{{\"key\":{o},\"addr\":{},\"count\":{}}}", cov.addr[o], cov.count[o])).collect();
    let json = format!("{{\"version\":0,\"prg_len\":{prg_len},\"frames\":{},\"instructions\":{},\"steps\":{step},\"covered\":{},\"sites\":[{}],\"routines\":[]}}\n", cov.frames, cov.instructions, cov.covered, sites.join(","));
    std::fs::write(out.join("crawl.json"), json).expect("crawl.json");
    if let Some((frames, script)) = best {
        let mut f = std::fs::File::create(out.join("best.txt")).expect("best.txt");
        writeln!(f, "# frames {frames}").unwrap();
        for (fr, p) in script {
            writeln!(f, "AT {fr} {p:02X}").unwrap();
        }
    }
    if std::env::var("NOVELTY").is_ok() {
        let mut top: Vec<(usize, u32)> = cov.distinct.iter().copied().enumerate().filter(|x| x.1 > 0).collect();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        let total: u32 = top.iter().map(|x| x.1).sum();
        eprintln!("novelty: {} distinct (address, value) pairs over {} addresses; the top 32 addresses:", total, top.len());
        for (a, n) in top.iter().take(32) {
            eprintln!("  ${a:04X} {n:>4} values");
        }
        let hist = [(256, "256 (a counter or a random byte)"), (64, "64 to 255"), (16, "16 to 63"), (4, "4 to 15"), (1, "1 to 3")];
        for (min, label) in hist {
            let max = if min == 256 { 256 } else { hist.iter().map(|x| x.0).filter(|&m| m > min).min().unwrap_or(257) - 1 };
            let addrs = top.iter().filter(|x| x.1 as usize >= min && x.1 as usize <= max).count();
            let pairs: u32 = top.iter().filter(|x| x.1 as usize >= min && x.1 as usize <= max).map(|x| x.1).sum();
            eprintln!("  {label}: {addrs} addresses, {pairs} pairs");
        }
    }
    eprintln!("done: {step} steps, {} of {prg_len} opcode sites of PRG reached ({:.1}%), {kept} moments found new code, {:.0} s", cov.covered, cov.covered as f64 * 100.0 / prg_len as f64, t0.elapsed().as_secs_f64());
}
