//! The iNES container, enough of it for the boards the console has: a
//! 16-byte header, PRG in 16 KiB units, CHR in 8 KiB units, the
//! mirroring bit, the mapper number. Mappers 0 (NROM), 1 (MMC1),
//! 2 (UxROM), 3 (CNROM), 4 (MMC3), 9 (MMC2) and 66 (GxROM, the bench's
//! own cartridge) are known; anything else (a trainer, four-screen, PAL,
//! every other mapper) is refused by name: a silent fallback here would
//! be a plausible wrong console.

use nes_bus::cart::{Cartridge, Cnrom, Gxrom, Mirroring, Mmc1, Mmc2, Mmc3, Nrom, Uxrom};

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
    /// Flags 6 bit 1: the cartridge has a battery behind its RAM, so what
    /// a game writes at $6000..$7FFF is meant to outlive power-off.
    pub battery: bool,
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
        battery: f6 & 0x02 != 0,
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
    /// as `Nrom`, 1 as `Mmc1`, 2 as `Uxrom`, 3 as `Cnrom`, 4 as `Mmc3`,
    /// 9 as `Mmc2`, 66 as `Gxrom`, anything else refused by name.
    pub fn cart(&self) -> Result<Box<dyn Cartridge>, String> {
        match self.mapper {
            0 => Ok(Box::new(self.nrom()?)),
            1 => {
                // As MMC3: the CHR RAM board hands MMC1 an empty CHR and
                // it keeps and banks the 8 KiB itself.
                let chr = if self.chr_ram { Vec::new() } else { self.chr.clone() };
                Ok(Box::new(Mmc1::new(self.prg.clone(), chr, self.mirroring)?))
            }
            2 => {
                if !self.chr_ram {
                    return Err(format!("mapper 2 with {} KiB of CHR ROM is not a UxROM board; UxROM carries CHR RAM", self.chr.len() / 1024));
                }
                Ok(Box::new(Uxrom::new(self.prg.clone(), Vec::new(), self.mirroring)?))
            }
            3 => {
                if self.chr_ram {
                    return Err("mapper 3 with CHR RAM is not a board this console has".into());
                }
                Ok(Box::new(Cnrom::new(self.prg.clone(), self.chr.clone(), self.mirroring)?))
            }
            4 => {
                // The CHR RAM board hands MMC3 an empty CHR and it keeps
                // the 8 KiB itself, banked: `owns_chr_ram` then tells the
                // Board not to keep a second copy outside the cartridge.
                let chr = if self.chr_ram { Vec::new() } else { self.chr.clone() };
                Ok(Box::new(Mmc3::new(self.prg.clone(), chr, self.mirroring)?))
            }
            9 => {
                if self.chr_ram {
                    return Err("mapper 9 with CHR RAM is not a board this console has: MMC2's latches switch between banks of CHR ROM".into());
                }
                Ok(Box::new(Mmc2::new(self.prg.clone(), self.chr.clone(), self.mirroring)?))
            }
            66 => {
                if self.chr_ram {
                    return Err("mapper 66 with CHR RAM is not a board this console has".into());
                }
                Ok(Box::new(Gxrom::new(self.prg.clone(), self.chr.clone(), self.mirroring)?))
            }
            m => Err(format!("mapper {m} is out of scope; this console has NROM (0), MMC1 (1), UxROM (2), CNROM (3), MMC3 (4), MMC2 (9) and GxROM (66)")),
        }
    }
}
