//! `--watch`: engine values read (never written) at RVAs in halo2.dll and
//! logged when they change, to see the engine's own state (its network
//! session, its load step) from the outside. Reading memory calls no engine
//! code, so this stays within the launcher's rule of no patches, hooks or
//! calls into the engine.
//!
//! A watch is `<name>=<path>:<type>`. The path starts with an RVA in
//! halo2.dll; each `->off` reads a pointer there and moves to that pointer
//! plus `off`, and a final `+off` adds without reading. Examples:
//! `net=0xE14FF0:u8`, `state=0xE15048+0x90A8:i32`,
//! `sstate=0xE15048->0x90A8:i32` (if 0xE15048 holds a pointer).

/// What a watch reads at its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    /// That many bytes, shown as hex (1 to 64).
    Hex(usize),
}

impl Kind {
    pub fn size(self) -> usize {
        match self {
            Kind::U8 | Kind::I8 => 1,
            Kind::U16 | Kind::I16 => 2,
            Kind::U32 | Kind::I32 | Kind::F32 => 4,
            Kind::U64 | Kind::I64 => 8,
            Kind::Hex(n) => n,
        }
    }

    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "u8" => Kind::U8,
            "i8" => Kind::I8,
            "u16" => Kind::U16,
            "i16" => Kind::I16,
            "u32" => Kind::U32,
            "i32" => Kind::I32,
            "u64" => Kind::U64,
            "i64" => Kind::I64,
            "f32" => Kind::F32,
            _ => {
                let n: usize = s.strip_prefix("hex")?.parse().ok()?;
                if !(1..=64).contains(&n) {
                    return None;
                }
                Kind::Hex(n)
            }
        })
    }

    /// The bytes read, as text.
    pub fn show(self, b: &[u8]) -> String {
        let le = |n: usize| -> u64 {
            let mut v = [0u8; 8];
            v[..n].copy_from_slice(&b[..n]);
            u64::from_le_bytes(v)
        };
        match self {
            Kind::U8 => format!("{}", b[0]),
            Kind::I8 => format!("{}", b[0] as i8),
            Kind::U16 => format!("{}", le(2) as u16),
            Kind::I16 => format!("{}", le(2) as u16 as i16),
            Kind::U32 => format!("{:#x}", le(4) as u32),
            Kind::I32 => format!("{}", le(4) as u32 as i32),
            Kind::U64 => format!("{:#x}", le(8)),
            Kind::I64 => format!("{}", le(8) as i64),
            Kind::F32 => format!("{}", f32::from_bits(le(4) as u32)),
            Kind::Hex(n) => b[..n]
                .iter()
                .map(|x| format!("{x:02x}"))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// One step after the RVA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Read a pointer at the current address, then add this.
    Deref(u64),
    /// Add this.
    Add(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub name: String,
    pub rva: u64,
    pub steps: Vec<Step>,
    pub kind: Kind,
    pub text: String,
}

impl Watch {
    pub fn parse(s: &str) -> Result<Watch, String> {
        let bad = |why: &str| format!("--watch {s:?}: {why} (see --help)");
        let (name, rest) = s
            .split_once('=')
            .ok_or_else(|| bad("expected <name>=<path>:<type>"))?;
        let name = name.trim();
        if name.is_empty()
            || name.len() > 24
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(bad("the name should be 1 to 24 letters, digits, - or _"));
        }
        let (path, ty) = rest
            .rsplit_once(':')
            .ok_or_else(|| bad("expected <path>:<type>"))?;
        let kind = Kind::parse(ty.trim()).ok_or_else(|| {
            bad("the type should be u8 i8 u16 i16 u32 i32 u64 i64 f32 or hex1..hex64")
        })?;
        // Split the path into the RVA and its steps, keeping each step's
        // operator.
        let path = path.trim();
        let mut parts: Vec<(Option<&str>, &str)> = Vec::new();
        let mut rest = path;
        let mut op: Option<&str> = None;
        loop {
            let next = [
                rest.find("->").map(|i| (i, "->")),
                rest.find('+').map(|i| (i, "+")),
            ]
            .into_iter()
            .flatten()
            .min_by_key(|(i, _)| *i);
            match next {
                Some((i, o)) => {
                    parts.push((op, &rest[..i]));
                    op = Some(o);
                    rest = &rest[i + o.len()..];
                }
                None => {
                    parts.push((op, rest));
                    break;
                }
            }
        }
        let num = |t: &str| {
            crate::util::parse_u64(t).ok_or_else(|| bad(&format!("{t:?} is not a number")))
        };
        let rva = num(parts[0].1)?;
        if rva >= crate::expected::HALO2_SIZE_OF_IMAGE as u64 {
            return Err(bad("the RVA is past the end of halo2.dll"));
        }
        let mut steps = Vec::new();
        for (o, t) in &parts[1..] {
            let v = num(t)?;
            steps.push(match *o {
                Some("->") => Step::Deref(v),
                _ => Step::Add(v),
            });
        }
        if steps.len() > 8 {
            return Err(bad("at most 8 steps"));
        }
        Ok(Watch {
            name: name.to_string(),
            rva,
            steps,
            kind,
            text: s.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paths() {
        let w = Watch::parse("net=0xE14FF0:u8").unwrap();
        assert_eq!(w.rva, 0xE14FF0);
        assert!(w.steps.is_empty());
        assert_eq!(w.kind, Kind::U8);

        let w = Watch::parse("state=0xE15048+0x90A8:i32").unwrap();
        assert_eq!(w.steps, vec![Step::Add(0x90A8)]);

        let w = Watch::parse("s=0xE15048->0x90A8->0+8:hex16").unwrap();
        assert_eq!(
            w.steps,
            vec![Step::Deref(0x90A8), Step::Deref(0), Step::Add(8)]
        );
        assert_eq!(w.kind, Kind::Hex(16));
    }

    #[test]
    fn rejects_bad_input() {
        for s in [
            "0xE14FF0:u8",
            "=0x10:u8",
            "a=0x10",
            "a=0x10:u128",
            "a=0x10:hex0",
            "a=0x10:hex65",
            "a=zz:u8",
            "a=0x10->zz:u8",
            "a=0xFFFFFFFF:u8",
            "a b=0x10:u8",
        ] {
            assert!(Watch::parse(s).is_err(), "{s} should fail");
        }
    }

    #[test]
    fn shows_values() {
        assert_eq!(Kind::I32.show(&[0xFF, 0xFF, 0xFF, 0xFF]), "-1");
        assert_eq!(Kind::U32.show(&[9, 0, 0, 0]), "0x9");
        assert_eq!(Kind::Hex(3).show(&[1, 2, 0xAB]), "01 02 ab");
        assert_eq!(Kind::F32.show(&1.5f32.to_le_bytes()), "1.5");
    }
}
