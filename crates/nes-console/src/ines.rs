//! The iNES container, enough of it for the boards the console has: a
//! 16-byte header, PRG in 16 KiB units, CHR in 8 KiB units, the
//! mirroring bit, the mapper number. Mapper 0 (NROM) and 66 (GxROM, the
//! bench's own cartridge, added 2026-09-12) are known; anything else (a
//! trainer, four-screen, PAL, every other mapper) is refused by name: a
//! silent fallback here would be a plausible wrong console.

use nes_bus::cart::{Cartridge, Gxrom, Mirroring, Nrom};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ines {
    pub prg: Vec<u8>,
    pub chr: Vec<u8>,
    pub mirroring: Mirroring,
    pub mapper: u8,
    /// True when the header declares no CHR banks: the cartridge carries
    /// CHR RAM, 8 KiB, which NROM-with-RAM boards (and most of blargg's
    /// tests) use.
    pub chr_ram: bool,
}

pub fn parse(bytes: &[u8]) -> Result<Ines, String> {
    if bytes.len() < 16 || &bytes[0..4] != b"NES\x1a" {
        return Err("not an iNES file (no NES<EOF> magic)".into());
    }
    let prg_banks = bytes[4] as usize;
    let chr_banks = bytes[5] as usize;
    let f6 = bytes[6];
    let f7 = bytes[7];
    let mapper = (f6 >> 4) | (f7 & 0xf0);
    if f6 & 0x04 != 0 {
        return Err("a trainer is present; out of scope".into());
    }
    if f6 & 0x08 != 0 {
        return Err("four-screen VRAM; out of scope".into());
    }
    let mirroring = if f6 & 1 != 0 { Mirroring::Vertical } else { Mirroring::Horizontal };
    let prg_len = prg_banks * 0x4000;
    let chr_len = chr_banks * 0x2000;
    if bytes.len() < 16 + prg_len + chr_len {
        return Err(format!("file is {} bytes, header promises {}", bytes.len(), 16 + prg_len + chr_len));
    }
    Ok(Ines {
        prg: bytes[16..16 + prg_len].to_vec(),
        chr: bytes[16 + prg_len..16 + prg_len + chr_len].to_vec(),
        mirroring,
        mapper,
        chr_ram: chr_banks == 0,
    })
}

impl Ines {
    /// The cartridge as nes-bus's NROM. A CHR RAM board gets 8 KiB of
    /// zeroed CHR and `Board` routes PPU writes into it; refuses any
    /// mapper but 0.
    pub fn nrom(&self) -> Result<Nrom, String> {
        if self.mapper != 0 {
            return Err(format!("mapper {} is out of scope; NROM only", self.mapper));
        }
        let chr = if self.chr_ram { vec![0u8; 0x2000] } else { self.chr.clone() };
        Nrom::new(self.prg.clone(), chr, self.mirroring)
    }

    /// The cartridge the header names, boxed for the console: mapper 0
    /// as `Nrom`, mapper 66 as `Gxrom`, anything else refused by name.
    pub fn cart(&self) -> Result<Box<dyn Cartridge>, String> {
        match self.mapper {
            0 => Ok(Box::new(self.nrom()?)),
            66 => {
                if self.chr_ram {
                    return Err("mapper 66 with CHR RAM is not a board this console has".into());
                }
                Ok(Box::new(Gxrom::new(self.prg.clone(), self.chr.clone(), self.mirroring)?))
            }
            m => Err(format!("mapper {m} is out of scope; this console has NROM (0) and GxROM (66)")),
        }
    }
}
