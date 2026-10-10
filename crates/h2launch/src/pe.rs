//! A small reader for 64-bit PE files (DLLs and EXEs), for `--check`:
//! the headers, the section table, the export names, the DLLs named by
//! the import and delay-import tables, and the file version from the
//! version resource. Also the SHA-256 of a file's bytes.

use sha2::{Digest, Sha256};

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub name: String,
    pub rva: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub flags: u32,
}

impl Section {
    pub fn contains(&self, rva: u32) -> bool {
        rva >= self.rva && rva < self.rva + self.virtual_size.max(self.raw_size)
    }

    /// `RX`, `R`, `RW` and so on.
    pub fn access(&self) -> String {
        let mut s = String::new();
        if self.flags & 0x4000_0000 != 0 {
            s.push('R');
        }
        if self.flags & 0x8000_0000 != 0 {
            s.push('W');
        }
        if self.flags & 0x2000_0000 != 0 {
            s.push('X');
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Export {
    pub name: String,
    pub ordinal: u32,
    pub rva: u32,
}

/// Data directory indices.
pub mod dir {
    pub const EXPORT: usize = 0;
    pub const IMPORT: usize = 1;
    pub const RESOURCE: usize = 2;
    pub const EXCEPTION: usize = 3;
    pub const DELAY_IMPORT: usize = 13;
}

#[derive(Clone, Debug)]
pub struct Pe<'a> {
    data: &'a [u8],
    pub machine: u16,
    pub timestamp: u32,
    pub characteristics: u16,
    pub image_base: u64,
    pub entry_rva: u32,
    pub size_of_image: u32,
    pub subsystem: u16,
    pub sections: Vec<Section>,
    pub dirs: Vec<(u32, u32)>,
}

impl<'a> Pe<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Pe<'a>, String> {
        let bad = |what: &str| format!("not a PE file: {what}");
        if data.get(0..2) != Some(b"MZ") {
            return Err(bad("no MZ"));
        }
        let pe = u32_at(data, 0x3C).ok_or_else(|| bad("short"))? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(bad("no PE signature"));
        }
        let coff = pe + 4;
        let machine = u16_at(data, coff).ok_or_else(|| bad("short"))?;
        let nsections = u16_at(data, coff + 2).ok_or_else(|| bad("short"))? as usize;
        let timestamp = u32_at(data, coff + 4).ok_or_else(|| bad("short"))?;
        let opt_size = u16_at(data, coff + 16).ok_or_else(|| bad("short"))? as usize;
        let characteristics = u16_at(data, coff + 18).ok_or_else(|| bad("short"))?;
        let opt = coff + 20;
        if u16_at(data, opt) != Some(0x20B) {
            return Err("not a 64-bit (PE32+) image".into());
        }
        let g32 = |o: usize| u32_at(data, opt + o).ok_or_else(|| bad("short optional header"));
        let entry_rva = g32(16)?;
        let image_base = u64_at(data, opt + 24).ok_or_else(|| bad("short"))?;
        let size_of_image = g32(56)?;
        let subsystem = u16_at(data, opt + 68).ok_or_else(|| bad("short"))?;
        let ndirs = (g32(108)? as usize).min(16);
        let mut dirs = Vec::with_capacity(ndirs);
        for i in 0..ndirs {
            dirs.push((g32(112 + 8 * i)?, g32(116 + 8 * i)?));
        }
        let mut sections = Vec::with_capacity(nsections);
        let st = opt + opt_size;
        for i in 0..nsections {
            let s = st + 40 * i;
            let raw = data
                .get(s..s + 40)
                .ok_or_else(|| bad("short section table"))?;
            let end = raw[..8].iter().position(|&c| c == 0).unwrap_or(8);
            sections.push(Section {
                name: String::from_utf8_lossy(&raw[..end]).into_owned(),
                virtual_size: u32_at(raw, 8).unwrap(),
                rva: u32_at(raw, 12).unwrap(),
                raw_size: u32_at(raw, 16).unwrap(),
                raw_offset: u32_at(raw, 20).unwrap(),
                flags: u32_at(raw, 36).unwrap(),
            });
        }
        Ok(Pe {
            data,
            machine,
            timestamp,
            characteristics,
            image_base,
            entry_rva,
            size_of_image,
            subsystem,
            sections,
            dirs,
        })
    }

    pub fn dir(&self, i: usize) -> Option<(u32, u32)> {
        self.dirs.get(i).copied().filter(|d| d.0 != 0)
    }

    pub fn section_of(&self, rva: u32) -> Option<&Section> {
        self.sections.iter().find(|s| s.contains(rva))
    }

    /// File offset of an RVA, if the bytes are in the file.
    pub fn offset(&self, rva: u32) -> Option<usize> {
        let s = self.section_of(rva)?;
        let within = rva - s.rva;
        (within < s.raw_size).then(|| (s.raw_offset + within) as usize)
    }

    fn cstr(&self, rva: u32) -> Option<String> {
        let at = self.offset(rva)?;
        let rest = self.data.get(at..)?;
        let end = rest.iter().take(512).position(|&c| c == 0)?;
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    }

    fn u32_rva(&self, rva: u32) -> Option<u32> {
        u32_at(self.data, self.offset(rva)?)
    }

    pub fn exports(&self) -> Result<Vec<Export>, String> {
        let Some((d, _)) = self.dir(dir::EXPORT) else {
            return Ok(Vec::new());
        };
        let bad = || "bad export directory".to_string();
        let base = self.u32_rva(d + 16).ok_or_else(bad)?;
        let nfuncs = self.u32_rva(d + 20).ok_or_else(bad)?;
        let nnames = self.u32_rva(d + 24).ok_or_else(bad)?;
        let funcs = self.u32_rva(d + 28).ok_or_else(bad)?;
        let names = self.u32_rva(d + 32).ok_or_else(bad)?;
        let ords = self.u32_rva(d + 36).ok_or_else(bad)?;
        if nnames > 0x10000 || nfuncs > 0x10000 {
            return Err(bad());
        }
        let mut out = Vec::new();
        for i in 0..nnames {
            let name_rva = self.u32_rva(names + 4 * i).ok_or_else(bad)?;
            let ord_index = u16_at(self.data, self.offset(ords + 2 * i).ok_or_else(bad)?)
                .ok_or_else(bad)? as u32;
            let rva = self.u32_rva(funcs + 4 * ord_index).ok_or_else(bad)?;
            out.push(Export {
                name: self.cstr(name_rva).ok_or_else(bad)?,
                ordinal: base + ord_index,
                rva,
            });
        }
        Ok(out)
    }

    /// DLL names of the normal import table.
    pub fn imports(&self) -> Result<Vec<String>, String> {
        let Some((d, _)) = self.dir(dir::IMPORT) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for i in 0..4096u32 {
            let e = d + 20 * i;
            let name = self.u32_rva(e + 12).ok_or("bad import directory")?;
            let first = self.u32_rva(e + 16).ok_or("bad import directory")?;
            if name == 0 && first == 0 {
                break;
            }
            out.push(self.cstr(name).ok_or("bad import name")?);
        }
        Ok(out)
    }

    /// DLL names of the delay-import table.
    pub fn delay_imports(&self) -> Result<Vec<String>, String> {
        let Some((d, _)) = self.dir(dir::DELAY_IMPORT) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for i in 0..4096u32 {
            let e = d + 32 * i;
            let attrs = self.u32_rva(e).ok_or("bad delay-import directory")?;
            let name = self.u32_rva(e + 4).ok_or("bad delay-import directory")?;
            if name == 0 {
                break;
            }
            // Bit 0 clear: the old form, with addresses instead of RVAs.
            let name = if attrs & 1 == 0 {
                (name as u64).wrapping_sub(self.image_base) as u32
            } else {
                name
            };
            out.push(self.cstr(name).ok_or("bad delay-import name")?);
        }
        Ok(out)
    }

    /// The version resource's fixed file version and product version.
    pub fn versions(&self) -> Option<([u16; 4], [u16; 4])> {
        let (root, _) = self.dir(dir::RESOURCE)?;
        let base = self.offset(root)?;
        // A directory's entries: (name or id, offset); high bit of the
        // offset = another directory. Offsets count from the root.
        let entries = |dir_off: usize| -> Option<Vec<(u32, u32)>> {
            let named = u16_at(self.data, base + dir_off + 12)? as usize;
            let ids = u16_at(self.data, base + dir_off + 14)? as usize;
            (0..(named + ids).min(512))
                .map(|k| {
                    let e = base + dir_off + 16 + 8 * k;
                    Some((u32_at(self.data, e)?, u32_at(self.data, e + 4)?))
                })
                .collect()
        };
        let (_, types) = entries(0)?
            .into_iter()
            .find(|&(id, off)| id == 16 && off & 0x8000_0000 != 0)?;
        let (_, names) = *entries((types & 0x7FFF_FFFF) as usize)?.first()?;
        let mut leaf = names;
        // Name level, then language level, then the data entry.
        if leaf & 0x8000_0000 != 0 {
            leaf = entries((leaf & 0x7FFF_FFFF) as usize)?.first()?.1;
        }
        if leaf & 0x8000_0000 != 0 {
            return None;
        }
        let de = base + leaf as usize;
        let data_rva = u32_at(self.data, de)?;
        let size = u32_at(self.data, de + 4)? as usize;
        let at = self.offset(data_rva)?;
        let blob = self.data.get(at..at + size)?;
        let sig = 0xFEEF_04BDu32.to_le_bytes();
        let pos = (0..blob.len().saturating_sub(52))
            .step_by(4)
            .find(|&i| blob[i..i + 4] == sig)?;
        let split = |ms: u32, ls: u32| [(ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16];
        let file = split(u32_at(blob, pos + 8)?, u32_at(blob, pos + 12)?);
        let product = split(u32_at(blob, pos + 16)?, u32_at(blob, pos + 20)?);
        Some((file, product))
    }
}

pub fn version_string(v: [u16; 4]) -> String {
    format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3])
}

/// Upper-case hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{b:02X}")).collect()
}

/// What a launch logs about the `halo2.dll` it is about to load, against
/// the build the research was checked on (`crate::expected`).
#[derive(Clone, Debug, PartialEq)]
pub struct BuildFacts {
    pub version: Option<[u16; 4]>,
    pub timestamp: u32,
    pub size_of_image: u32,
    pub sha256: String,
}

impl BuildFacts {
    pub fn read(data: &[u8]) -> Result<BuildFacts, String> {
        let pe = Pe::parse(data)?;
        Ok(BuildFacts {
            version: pe.versions().map(|(file, _)| file),
            timestamp: pe.timestamp,
            size_of_image: pe.size_of_image,
            sha256: sha256_hex(data),
        })
    }

    /// The fields that differ from the researched build, empty if none.
    pub fn mismatches(&self) -> Vec<&'static str> {
        use crate::expected as e;
        let mut out = Vec::new();
        if self.version != Some(e::HALO2_VERSION) {
            out.push("FileVersion");
        }
        if self.timestamp != e::HALO2_TIMESTAMP {
            out.push("TimeDateStamp");
        }
        if self.size_of_image != e::HALO2_SIZE_OF_IMAGE {
            out.push("SizeOfImage");
        }
        if self.sha256 != e::HALO2_SHA256 {
            out.push("SHA-256");
        }
        out
    }

    pub fn describe(&self) -> String {
        format!(
            "FileVersion {}, TimeDateStamp {:#010x}, SizeOfImage {:#x}, SHA-256 {}",
            self.version
                .map_or_else(|| "not found".to_string(), version_string),
            self.timestamp,
            self.size_of_image,
            self.sha256
        )
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A synthetic 64-bit DLL: one section whose file offsets equal its
    /// RVAs, with exports, an import, a delay import and a version
    /// resource. No game bytes.
    pub fn build_test_dll() -> Vec<u8> {
        let mut b = vec![0u8; 0x3000];
        let put16 =
            |b: &mut Vec<u8>, at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
        let put32 =
            |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let put64 =
            |b: &mut Vec<u8>, at: usize, v: u64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());
        let puts =
            |b: &mut Vec<u8>, at: usize, s: &str| b[at..at + s.len()].copy_from_slice(s.as_bytes());
        b[0..2].copy_from_slice(b"MZ");
        put32(&mut b, 0x3C, 0x80);
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        put16(&mut b, coff, 0x8664);
        put16(&mut b, coff + 2, 1);
        put32(&mut b, coff + 4, 0x68A0_F0F2);
        put16(&mut b, coff + 16, 240);
        put16(&mut b, coff + 18, 0x2022);
        let opt = coff + 20;
        put16(&mut b, opt, 0x20B);
        put32(&mut b, opt + 16, 0x1234);
        put64(&mut b, opt + 24, 0x1_8000_0000);
        put32(&mut b, opt + 56, 0x3000);
        put16(&mut b, opt + 68, 2);
        put32(&mut b, opt + 108, 16);
        let dirs = opt + 112;
        put32(&mut b, dirs, 0x1000); // export
        put32(&mut b, dirs + 4, 0x100);
        put32(&mut b, dirs + 8, 0x1400); // import
        put32(&mut b, dirs + 12, 40);
        put32(&mut b, dirs + 16, 0x2000); // resource
        put32(&mut b, dirs + 20, 0x200);
        put32(&mut b, dirs + 13 * 8, 0x1600); // delay import
        put32(&mut b, dirs + 13 * 8 + 4, 64);
        let st = opt + 240;
        b[st..st + 5].copy_from_slice(b".data");
        put32(&mut b, st + 8, 0x2000); // virtual size
        put32(&mut b, st + 12, 0x1000); // rva
        put32(&mut b, st + 16, 0x2000); // raw size
        put32(&mut b, st + 20, 0x1000); // raw offset
        put32(&mut b, st + 36, 0xC000_0040);
        // Exports: base 1, two functions, names sorted.
        let e = 0x1000;
        put32(&mut b, e + 12, 0x1100);
        put32(&mut b, e + 16, 1);
        put32(&mut b, e + 20, 2);
        put32(&mut b, e + 24, 2);
        put32(&mut b, e + 28, 0x1040);
        put32(&mut b, e + 32, 0x1050);
        put32(&mut b, e + 36, 0x1060);
        put32(&mut b, 0x1040, 0x54730);
        put32(&mut b, 0x1044, 0x3AF40);
        put32(&mut b, 0x1050, 0x1110);
        put32(&mut b, 0x1054, 0x1130);
        put16(&mut b, 0x1060, 1);
        put16(&mut b, 0x1062, 0);
        puts(&mut b, 0x1100, "test.dll");
        puts(&mut b, 0x1110, "CreateDataAccess");
        puts(&mut b, 0x1130, "CreateGameEngine");
        // One import descriptor, then a zero one.
        put32(&mut b, 0x1400 + 12, 0x1500);
        put32(&mut b, 0x1400 + 16, 0x1520);
        puts(&mut b, 0x1500, "KERNEL32.dll");
        // One delay-import descriptor (RVA form), then a zero one.
        put32(&mut b, 0x1600, 1);
        put32(&mut b, 0x1604, 0x1700);
        puts(&mut b, 0x1700, "mss64.dll");
        // Resources: root -> type 16 -> name 1 -> language -> data.
        let r = 0x2000;
        put16(&mut b, r + 14, 1);
        put32(&mut b, r + 16, 16);
        put32(&mut b, r + 20, 0x8000_0018);
        put16(&mut b, r + 0x18 + 14, 1);
        put32(&mut b, r + 0x18 + 16, 1);
        put32(&mut b, r + 0x18 + 20, 0x8000_0030);
        put16(&mut b, r + 0x30 + 14, 1);
        put32(&mut b, r + 0x30 + 16, 0x409);
        put32(&mut b, r + 0x30 + 20, 0x48);
        put32(&mut b, r + 0x48, 0x2100);
        put32(&mut b, r + 0x48 + 4, 0x80);
        // VS_VERSIONINFO: header and key, then VS_FIXEDFILEINFO at +0x28.
        let v = 0x2100;
        put16(&mut b, v, 0x80);
        put16(&mut b, v + 2, 52);
        let key: Vec<u8> = "VS_VERSION_INFO"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        b[v + 6..v + 6 + key.len()].copy_from_slice(&key);
        let f = v + 0x28;
        put32(&mut b, f, 0xFEEF_04BD);
        put32(&mut b, f + 4, 0x0001_0000);
        put32(&mut b, f + 8, 0x0001_0DC8); // 1.3528
        put32(&mut b, f + 12, 0);
        put32(&mut b, f + 16, 0x0001_0DC8);
        put32(&mut b, f + 20, 0x0000_0002);
        b
    }

    #[test]
    fn headers_and_tables() {
        let b = build_test_dll();
        let pe = Pe::parse(&b).unwrap();
        assert_eq!(pe.machine, 0x8664);
        assert_eq!(pe.timestamp, 0x68A0_F0F2);
        assert_eq!(pe.image_base, 0x1_8000_0000);
        assert_eq!(pe.size_of_image, 0x3000);
        assert_eq!(pe.entry_rva, 0x1234);
        assert_eq!(pe.sections.len(), 1);
        assert_eq!(pe.sections[0].name, ".data");
        assert_eq!(pe.sections[0].access(), "RW");
        assert_eq!(pe.section_of(0x2FFF).unwrap().name, ".data");
        assert!(pe.section_of(0x3000).is_none());
        let ex = pe.exports().unwrap();
        assert_eq!(
            ex,
            vec![
                Export {
                    name: "CreateDataAccess".into(),
                    ordinal: 2,
                    rva: 0x3AF40
                },
                Export {
                    name: "CreateGameEngine".into(),
                    ordinal: 1,
                    rva: 0x54730
                },
            ]
        );
        assert_eq!(pe.imports().unwrap(), vec!["KERNEL32.dll"]);
        assert_eq!(pe.delay_imports().unwrap(), vec!["mss64.dll"]);
        let (file, product) = pe.versions().unwrap();
        assert_eq!(file, [1, 3528, 0, 0]);
        assert_eq!(product, [1, 3528, 0, 2]);
        assert_eq!(version_string(file), "1.3528.0.0");
    }

    #[test]
    fn rejects_other_files() {
        assert!(Pe::parse(b"hello").is_err());
        let mut b = build_test_dll();
        b[0x84 + 20] = 0x0B;
        b[0x84 + 21] = 0x01; // PE32
        assert!(Pe::parse(&b).unwrap_err().contains("64-bit"));
    }

    #[test]
    fn sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
    }

    #[test]
    fn build_facts() {
        let b = build_test_dll();
        let f = BuildFacts::read(&b).unwrap();
        assert_eq!(f.version, Some([1, 3528, 0, 0]));
        assert_eq!(f.timestamp, crate::expected::HALO2_TIMESTAMP);
        // The test file is small and not the real DLL.
        assert_eq!(f.mismatches(), vec!["SizeOfImage", "SHA-256"]);
        assert!(f
            .describe()
            .starts_with("FileVersion 1.3528.0.0, TimeDateStamp 0x68a0f0f2, SizeOfImage 0x3000"));
        assert!(BuildFacts::read(b"MZ").is_err());
    }
}
