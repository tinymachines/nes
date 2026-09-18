//! The knobs file: the few numbers the model carries that it could not
//! measure on a die, each with where it came from, one file per bench
//! run (`runs/<stamp>/knobs.toml` in tinymachines/nes-bench, the
//! exercise notebook's Programme 1).
//!
//! A runner reads it from `KNOBS=path`. Every table names its `source`
//! (`measured`, `authored` or `fitted`) and `by` (the run stamp, the
//! probe or the document that set it), so a report can say which of its
//! figures rest on a fitted number; a `fitted` table must carry its
//! `residual`. A table or key this reader does not know is refused by
//! name, so a typo cannot run the defaults in silence, and a table the
//! model does not act on (the bench's `[capture]`) is known and kept,
//! not acted on. The file is the flat subset of TOML the bench writes:
//! `[table]`, `key = value`, strings in double quotes, integers,
//! floats, `true`/`false`, `#` comments; nothing else parses.
//!
//! Tables today:
//!
//! - `[alignment]` `cpu_phase`, `ppu_phase`: the console's power-on
//!   alignment (`Alignment`); measured off the two dies' clock
//!   recipes, and the number E4's histogram over a hundred power-ons
//!   will set from the part.
//! - `[capture]` `scale_v_per_div`, `offset_v`, `channel`: the bench's
//!   scope window for the video, from the run's `ARM` line; the scorer
//!   picks the channel from it.
//! - `[ram]` `fill`, or `seed`: the byte every work-RAM address holds at
//!   power-on, or a pattern from a 32-bit xorshift seed (a stand-in for
//!   a part's random RAM until its own pattern is measured).
//!   The model's RAM starts blank; the part's does not. Built for a
//!   finding that turned out to be the bench's own (2026-09-18: the
//!   multicart's menu seemed to ignore Start after a cold boot; the
//!   head's WAIT returned early and the scope caught the menu before
//!   the press, and with that fixed the part takes Start cold, as the
//!   model does). A fill is an authored stand-in until a cartridge of
//!   our own shows the part's pattern; `Knobs::apply` writes it into
//!   the console after construction, and every runner calls it.
//!
//! A knob that reaches nothing is not a knob: `tests/knobs.rs` moves
//! the alignment and the scheduler must move with it.

use crate::console::Alignment;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceKind {
    Measured,
    Authored,
    Fitted,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Source {
    pub kind: SourceKind,
    pub by: String,
    /// Required with `Fitted`, refused otherwise.
    pub residual: Option<f64>,
}

impl Source {
    pub fn describe(&self) -> String {
        let kind = match self.kind {
            SourceKind::Measured => "measured",
            SourceKind::Authored => "authored",
            SourceKind::Fitted => "fitted",
        };
        match self.residual {
            Some(r) => format!("{kind} by {} (residual {r})", self.by),
            None => format!("{kind} by {}", self.by),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Capture {
    pub scale_v_per_div: f64,
    pub offset_v: f64,
    pub channel: u8,
    pub source: Source,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Knobs {
    pub path: String,
    pub alignment: Option<(Alignment, Source)>,
    pub capture: Option<Capture>,
    pub ram_fill: Option<(u8, Source)>,
    pub ram_seed: Option<(u32, Source)>,
}

#[derive(Clone, PartialEq, Debug)]
enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Str(_) => "a string",
            Value::Int(_) => "an integer",
            Value::Float(_) => "a float",
            Value::Bool(_) => "a boolean",
        }
    }
}

fn parse_value(raw: &str, line: usize) -> Result<Value, String> {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('"') {
        let end = inner.find('"').ok_or_else(|| format!("line {line}: unterminated string"))?;
        if !inner[end + 1..].trim().is_empty() {
            return Err(format!("line {line}: text after the string"));
        }
        return Ok(Value::Str(inner[..end].to_string()));
    }
    match raw {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    if let Ok(i) = raw.parse::<i64>() {
        return Ok(Value::Int(i));
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Ok(Value::Float(f));
    }
    Err(format!("line {line}: {raw:?} is not a string, an integer, a float or a boolean"))
}

/// The file as tables of keys, in order.
type Tables = Vec<(String, Vec<(String, Value)>)>;

/// A duplicate table or key is refused.
fn parse_tables(text: &str) -> Result<Tables, String> {
    let mut tables: Tables = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let content = match raw.find('#') {
            // A '#' inside a string is part of it.
            Some(h) if raw[..h].matches('"').count() % 2 == 0 => &raw[..h],
            _ => raw,
        }
        .trim();
        if content.is_empty() {
            continue;
        }
        if let Some(name) = content.strip_prefix('[') {
            let name = name.strip_suffix(']').ok_or_else(|| format!("line {line}: a table header without its ']'"))?.trim();
            if tables.iter().any(|(n, _)| n == name) {
                return Err(format!("line {line}: table [{name}] twice"));
            }
            tables.push((name.to_string(), Vec::new()));
            continue;
        }
        let (k, v) = content.split_once('=').ok_or_else(|| format!("line {line}: not `key = value`"))?;
        let k = k.trim();
        let table = tables.last_mut().ok_or_else(|| format!("line {line}: `{k}` before any [table]"))?;
        if table.1.iter().any(|(n, _)| n == k) {
            return Err(format!("line {line}: `{k}` twice in [{}]", table.0));
        }
        table.1.push((k.to_string(), parse_value(v, line)?));
    }
    Ok(tables)
}

struct Table<'a> {
    name: &'a str,
    keys: &'a [(String, Value)],
}

impl Table<'_> {
    fn get(&self, key: &str) -> Result<&Value, String> {
        self.keys.iter().find(|(k, _)| k == key).map(|(_, v)| v).ok_or_else(|| format!("[{}] has no `{key}`", self.name))
    }
    fn int(&self, key: &str) -> Result<i64, String> {
        match self.get(key)? {
            Value::Int(i) => Ok(*i),
            v => Err(format!("[{}] `{key}` is {}, not an integer", self.name, v.kind())),
        }
    }
    fn float(&self, key: &str) -> Result<f64, String> {
        match self.get(key)? {
            Value::Float(f) => Ok(*f),
            Value::Int(i) => Ok(*i as f64),
            v => Err(format!("[{}] `{key}` is {}, not a number", self.name, v.kind())),
        }
    }
    fn str(&self, key: &str) -> Result<&str, String> {
        match self.get(key)? {
            Value::Str(s) => Ok(s),
            v => Err(format!("[{}] `{key}` is {}, not a string", self.name, v.kind())),
        }
    }
    /// Every key must be one of `known` or one of the source keys.
    fn only(&self, known: &[&str]) -> Result<(), String> {
        for (k, _) in self.keys {
            if !known.contains(&k.as_str()) && !["source", "by", "residual"].contains(&k.as_str()) {
                return Err(format!("[{}] has a key this reader does not know: `{k}` (it knows {})", self.name, known.join(", ")));
            }
        }
        Ok(())
    }
    fn source(&self) -> Result<Source, String> {
        let kind = match self.str("source")? {
            "measured" => SourceKind::Measured,
            "authored" => SourceKind::Authored,
            "fitted" => SourceKind::Fitted,
            other => return Err(format!("[{}] `source` is {other:?}; it is measured, authored or fitted", self.name)),
        };
        let by = self.str("by")?.to_string();
        if by.trim().is_empty() {
            return Err(format!("[{}] `by` is empty: name the run, the probe or the document", self.name));
        }
        let residual = match (kind, self.get("residual")) {
            (SourceKind::Fitted, Ok(_)) => Some(self.float("residual")?),
            (SourceKind::Fitted, Err(_)) => return Err(format!("[{}] is fitted and carries no `residual`", self.name)),
            (_, Ok(_)) => return Err(format!("[{}] carries a `residual` but is not fitted", self.name)),
            (_, Err(_)) => None,
        };
        Ok(Source { kind, by, residual })
    }
}

impl Knobs {
    pub fn parse(text: &str, path: &str) -> Result<Knobs, String> {
        let tables = parse_tables(text).map_err(|e| format!("{path}: {e}"))?;
        let mut knobs = Knobs { path: path.to_string(), ..Knobs::default() };
        for (name, keys) in &tables {
            let t = Table { name, keys };
            let r: Result<(), String> = (|| {
                match name.as_str() {
                    "alignment" => {
                        t.only(&["cpu_phase", "ppu_phase"])?;
                        let (c, p) = (t.int("cpu_phase")?, t.int("ppu_phase")?);
                        if !(0..24).contains(&c) {
                            return Err(format!("[alignment] `cpu_phase` {c} is not in 0..24"));
                        }
                        if !(0..8).contains(&p) {
                            return Err(format!("[alignment] `ppu_phase` {p} is not in 0..8"));
                        }
                        knobs.alignment = Some((Alignment { cpu_phase: c as u8, ppu_phase: p as u8 }, t.source()?));
                    }
                    "capture" => {
                        t.only(&["scale_v_per_div", "offset_v", "channel"])?;
                        let ch = t.int("channel")?;
                        if !(1..=4).contains(&ch) {
                            return Err(format!("[capture] `channel` {ch} is not 1..4"));
                        }
                        knobs.capture = Some(Capture { scale_v_per_div: t.float("scale_v_per_div")?, offset_v: t.float("offset_v")?, channel: ch as u8, source: t.source()? });
                    }
                    "ram" => {
                        t.only(&["fill", "seed"])?;
                        match (t.get("fill").is_ok(), t.get("seed").is_ok()) {
                            (true, true) => return Err("[ram] carries both `fill` and `seed`: one pattern".to_string()),
                            (false, false) => return Err("[ram] has neither `fill` nor `seed`".to_string()),
                            (true, false) => {
                                let f = t.int("fill")?;
                                if !(0..=255).contains(&f) {
                                    return Err(format!("[ram] `fill` {f} is not a byte"));
                                }
                                knobs.ram_fill = Some((f as u8, t.source()?));
                            }
                            (false, true) => {
                                let sd = t.int("seed")?;
                                if sd <= 0 || sd > u32::MAX as i64 {
                                    return Err(format!("[ram] `seed` {sd} is not a nonzero 32-bit value"));
                                }
                                knobs.ram_seed = Some((sd as u32, t.source()?));
                            }
                        }
                    }
                    other => return Err(format!("a table this reader does not know: [{other}] (it knows [alignment], [capture], [ram])")),
                }
                Ok(())
            })();
            r.map_err(|e| format!("{path}: {e}"))?;
        }
        Ok(knobs)
    }

    pub fn load(path: &str) -> Result<Knobs, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        Knobs::parse(&text, path)
    }

    /// `KNOBS=path`, or none set.
    pub fn from_env() -> Result<Option<Knobs>, String> {
        match std::env::var("KNOBS") {
            Ok(p) => Knobs::load(&p).map(Some),
            Err(_) => Ok(None),
        }
    }

    /// The alignment to run at: the file's, or the measured default.
    pub fn alignment(&self) -> Alignment {
        self.alignment.as_ref().map(|(a, _)| *a).unwrap_or_default()
    }

    /// One line per table, with its source: what a report prints.
    pub fn describe(&self) -> String {
        let mut lines = vec![format!("knobs {}:", self.path)];
        match &self.alignment {
            Some((a, s)) => lines.push(format!("  alignment cpu_phase {} ppu_phase {}, {}", a.cpu_phase, a.ppu_phase, s.describe())),
            None => lines.push("  alignment: not in the file; the measured default".to_string()),
        }
        if let Some(c) = &self.capture {
            lines.push(format!("  capture CH{} at {} V/div, offset {} V, {}", c.channel, c.scale_v_per_div, c.offset_v, c.source.describe()));
        }
        if let Some((f, s)) = &self.ram_fill {
            lines.push(format!("  ram fill {f:02x} at power-on, {}", s.describe()));
        }
        if let Some((sd, s)) = &self.ram_seed {
            lines.push(format!("  ram pattern from seed {sd} at power-on, {}", s.describe()));
        }
        lines.join("\n")
    }

    /// The knobs that act after construction: the work RAM's power-on
    /// fill, written into every address the CPU can reach.
    pub fn apply(&self, c: &mut crate::console::Console) {
        if let Some((f, _)) = self.ram_fill {
            let mut b = c.board.borrow_mut();
            for a in 0..0x800u16 {
                b.wram.write(a, f);
            }
        }
        if let Some((sd, _)) = self.ram_seed {
            let mut x = sd;
            let mut b = c.board.borrow_mut();
            for a in 0..0x800u16 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                b.wram.write(a, (x >> 24) as u8);
            }
        }
    }
}

/// What every runner does at its start: the alignment from `KNOBS` if
/// set (printed, so the run's report carries the source), else the
/// measured default; a file that does not parse ends the run by name.
pub fn alignment_from_env() -> Alignment {
    match Knobs::from_env() {
        Ok(Some(k)) => {
            println!("{}", k.describe());
            k.alignment()
        }
        Ok(None) => Alignment::default(),
        Err(e) => {
            eprintln!("KNOBS refused: {e}");
            std::process::exit(2);
        }
    }
}

/// What every runner does once the console exists: the knobs that act
/// on it (the RAM fill). Quiet without `KNOBS`.
pub fn configure_from_env(c: &mut crate::console::Console) {
    if let Ok(Some(k)) = Knobs::from_env() {
        k.apply(c);
    }
}
