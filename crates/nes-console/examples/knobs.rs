//! A knobs file read and described, or refused by name: what the bench's
//! `tools/knobs.py check` runs.
//!
//!   cargo run -p nes-console --example knobs -- runs/<stamp>/knobs.toml

use nes_console::Knobs;

fn main() {
    let path = std::env::args().nth(1).expect("knobs.toml");
    match Knobs::load(&path) {
        Ok(k) => println!("{}", k.describe()),
        Err(e) => {
            eprintln!("refused: {e}");
            std::process::exit(1);
        }
    }
}
