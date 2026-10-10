//! Small helpers: number parsing, hex dumps, buffer diffs, picture stats
//! and UTF-16 strings.

use std::fmt::Write as _;

/// A number in decimal or `0x` hex.
pub fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim().replace('_', "");
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// A signed number in decimal or `0x` hex (a leading `-` allowed on both).
pub fn parse_i128(s: &str) -> Option<i128> {
    let s = s.trim();
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let v = parse_u64(rest)? as i128;
    Some(if neg { -v } else { v })
}

/// Bytes written as hex, with or without spaces: `01 02 ff` or `0102ff`.
pub fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let digits: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).ok())
        .collect()
}

/// A classic hex dump, 16 bytes a line, offsets starting at `base`.
pub fn hexdump(bytes: &[u8], base: usize) -> String {
    let mut out = String::new();
    for (i, row) in bytes.chunks(16).enumerate() {
        let _ = write!(out, "{:06x}:", base + i * 16);
        for b in row {
            let _ = write!(out, " {b:02x}");
        }
        for _ in row.len()..16 {
            out.push_str("   ");
        }
        out.push_str("  ");
        for &b in row {
            out.push(if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '.'
            });
        }
        out.push('\n');
    }
    out
}

/// The byte ranges `[start, end)` where `a` and `b` differ, with gaps of
/// fewer than `merge` equal bytes joined up.
pub fn changed_ranges(a: &[u8], b: &[u8], merge: usize) -> Vec<(usize, usize)> {
    let n = a.len().min(b.len());
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        if a[i] == b[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && a[i] != b[i] {
            i += 1;
        }
        match out.last_mut() {
            Some(last) if start - last.1 < merge => last.1 = i,
            _ => out.push((start, i)),
        }
    }
    out
}

/// Ranges of non-zero bytes, merged like `changed_ranges`.
pub fn nonzero_ranges(a: &[u8], merge: usize) -> Vec<(usize, usize)> {
    changed_ranges(a, &vec![0; a.len()], merge)
}

pub fn format_ranges(r: &[(usize, usize)]) -> String {
    if r.is_empty() {
        return "none".into();
    }
    let mut s = String::new();
    for (k, (a, b)) in r.iter().enumerate() {
        if k > 0 {
            s.push_str(", ");
        }
        if k == 24 {
            let _ = write!(s, "... ({} more)", r.len() - k);
            break;
        }
        let _ = write!(s, "{a:#x}-{:#x}", b - 1);
    }
    s
}

/// What a screenshot looks like, in numbers, for a log that is read as
/// text: the mean colour, how much of it is not black, how much is one
/// colour and roughly how many colours it has.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureStats {
    pub width: u32,
    pub height: u32,
    pub mean: [f32; 3],
    /// Share of pixels with any channel above 16.
    pub non_black: f32,
    /// Share of pixels equal to the most common colour (of a sample).
    pub top_colour: f32,
    /// Distinct colours in a sample of up to 4096 pixels, 5 bits a channel.
    pub colours: usize,
}

impl PictureStats {
    /// `rgba` is width x height pixels of 4 bytes, rows packed.
    pub fn of_rgba(rgba: &[u8], width: u32, height: u32) -> PictureStats {
        let n = (width as usize * height as usize).min(rgba.len() / 4);
        let mut sum = [0u64; 3];
        let mut lit = 0usize;
        for p in rgba.as_chunks::<4>().0.iter().take(n) {
            sum[0] += p[0] as u64;
            sum[1] += p[1] as u64;
            sum[2] += p[2] as u64;
            if p[0] > 16 || p[1] > 16 || p[2] > 16 {
                lit += 1;
            }
        }
        let step = (n / 4096).max(1);
        let mut seen = std::collections::HashMap::new();
        let mut sampled = 0usize;
        for i in (0..n).step_by(step) {
            let p = &rgba[i * 4..i * 4 + 3];
            let key = ((p[0] as u32 >> 3) << 10) | ((p[1] as u32 >> 3) << 5) | (p[2] as u32 >> 3);
            *seen.entry(key).or_insert(0usize) += 1;
            sampled += 1;
        }
        let top = seen.values().copied().max().unwrap_or(0);
        let d = n.max(1) as f32;
        PictureStats {
            width,
            height,
            mean: [sum[0] as f32 / d, sum[1] as f32 / d, sum[2] as f32 / d],
            non_black: lit as f32 / d,
            top_colour: top as f32 / sampled.max(1) as f32,
            colours: seen.len(),
        }
    }

    pub fn describe(&self) -> String {
        let verdict = if self.non_black < 0.01 {
            "black"
        } else if self.top_colour > 0.97 {
            "one flat colour"
        } else if self.colours < 8 {
            "nearly flat"
        } else {
            "has a picture"
        };
        format!(
            "{}x{} mean rgb ({:.0},{:.0},{:.0}) non-black {:.0}% top colour {:.0}% colours~{} => {verdict}",
            self.width,
            self.height,
            self.mean[0],
            self.mean[1],
            self.mean[2],
            self.non_black * 100.0,
            self.top_colour * 100.0,
            self.colours
        )
    }
}

/// UTF-16 with a terminating zero.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// UTF-16 up to the first zero (or the end).
pub fn from_wide(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// Bytes up to the first zero, shown with non-ASCII escaped.
pub fn from_narrow(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].escape_ascii().to_string()
}

/// A random-looking non-zero 64-bit id from 16 bytes of seed (a GUID):
/// low half XOR high half, as HaloX and AlphaRing make their XUIDs, never 0.
pub fn xuid_from_seed(seed: [u8; 16]) -> u64 {
    let lo = u64::from_le_bytes(seed[..8].try_into().unwrap());
    let hi = u64::from_le_bytes(seed[8..].try_into().unwrap());
    match lo ^ hi {
        0 => 0x1000_0000,
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_u64("0x2C"), Some(44));
        assert_eq!(parse_u64("44"), Some(44));
        assert_eq!(parse_u64("0x1_0000"), Some(0x10000));
        assert_eq!(parse_u64("zz"), None);
        assert_eq!(parse_i128("-1"), Some(-1));
        assert_eq!(parse_i128("-0x10"), Some(-16));
        assert_eq!(parse_hex_bytes("01 02 ff"), Some(vec![1, 2, 255]));
        assert_eq!(parse_hex_bytes("0102FF"), Some(vec![1, 2, 255]));
        assert_eq!(parse_hex_bytes("012"), None);
    }

    #[test]
    fn hexdump_lines() {
        let d = hexdump(b"Halo 2\0\x01", 0x300);
        assert!(d.starts_with("000300: 48 61 6c 6f 20 32 00 01 "));
        // The text column starts after 16 three-character cells and a gap.
        assert_eq!(d.find("Halo 2..\n"), Some(7 + 16 * 3 + 2));
        assert_eq!(hexdump(&[0; 32], 0).lines().count(), 2);
    }

    #[test]
    fn ranges() {
        let a = [0u8; 64];
        let mut b = [0u8; 64];
        b[3] = 1;
        b[5] = 1;
        b[40] = 2;
        assert_eq!(changed_ranges(&a, &b, 4), vec![(3, 6), (40, 41)]);
        assert_eq!(changed_ranges(&a, &b, 1), vec![(3, 4), (5, 6), (40, 41)]);
        assert_eq!(nonzero_ranges(&b, 64), vec![(3, 41)]);
        assert_eq!(format_ranges(&[(3, 6), (40, 41)]), "0x3-0x5, 0x28-0x28");
        assert_eq!(format_ranges(&[]), "none");
    }

    #[test]
    fn picture_stats() {
        let black = vec![0u8; 4 * 16];
        let s = PictureStats::of_rgba(&black, 4, 4);
        assert_eq!(s.non_black, 0.0);
        assert!(s.describe().ends_with("black"));
        let mut img = Vec::new();
        for i in 0..64u32 {
            img.extend_from_slice(&[(i * 4) as u8, (255 - i * 4) as u8, (i * 2) as u8, 255]);
        }
        let s = PictureStats::of_rgba(&img, 8, 8);
        assert!(s.non_black > 0.9);
        assert!(s.colours > 30);
        assert!(s.describe().ends_with("has a picture"));
    }

    #[test]
    fn strings() {
        assert_eq!(wide("ab"), vec![97, 98, 0]);
        assert_eq!(from_wide(&[72, 105, 0, 33]), "Hi");
        assert_eq!(from_narrow(b"scenarios\\x\0junk"), "scenarios\\\\x");
        assert_eq!(xuid_from_seed([0; 16]), 0x1000_0000);
        let mut s = [0u8; 16];
        s[0] = 1;
        s[8] = 3;
        assert_eq!(xuid_from_seed(s), 2);
    }
}
