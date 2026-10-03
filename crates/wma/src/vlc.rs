//! Variable length (Huffman) codes, decoded by walking a binary tree.

use crate::bits::BitReader;

/// Child links: a positive node index, a leaf `-(symbol + 1)`, or 0 for none.
#[derive(Default)]
pub struct Vlc {
    nodes: Vec<[i32; 2]>,
}

impl Vlc {
    /// Symbol `i` has code `codes[i]`, `lengths[i]` bits long (0: unused).
    pub fn new(codes: &[u32], lengths: &[u8]) -> Vlc {
        let mut vlc = Vlc {
            nodes: vec![[0, 0]],
        };
        for (symbol, (&code, &len)) in codes.iter().zip(lengths).enumerate() {
            vlc.insert(code, len, symbol);
        }
        vlc
    }

    /// Codes assigned in order from their lengths (each code follows the
    /// previous one), as `(symbol, length)` pairs.
    pub fn from_lengths(entries: &[(u8, u8)]) -> Vlc {
        let mut vlc = Vlc {
            nodes: vec![[0, 0]],
        };
        // Left-aligned in 32 bits.
        let mut code = 0u64;
        for &(symbol, len) in entries {
            if len == 0 {
                continue;
            }
            vlc.insert((code >> (32 - len)) as u32, len, symbol as usize);
            code += 1 << (32 - len);
        }
        vlc
    }

    fn insert(&mut self, code: u32, len: u8, symbol: usize) {
        if len == 0 {
            return;
        }
        let mut node = 0;
        for i in (0..len).rev() {
            let b = ((code >> i) & 1) as usize;
            if i == 0 {
                self.nodes[node][b] = -(symbol as i32 + 1);
            } else {
                if self.nodes[node][b] <= 0 {
                    self.nodes.push([0, 0]);
                    self.nodes[node][b] = (self.nodes.len() - 1) as i32;
                }
                node = self.nodes[node][b] as usize;
            }
        }
    }

    /// The next symbol, or `None` for a code that isn't in the table.
    pub fn decode(&self, r: &mut BitReader) -> Option<usize> {
        let mut node = 0;
        loop {
            match self.nodes[node][r.bit() as usize] {
                0 => return None,
                n if n < 0 => return Some((-n - 1) as usize),
                n => node = n as usize,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_codes_and_codes_from_lengths() {
        // a = 0, b = 10, c = 11
        let vlc = Vlc::new(&[0b0, 0b10, 0b11], &[1, 2, 2]);
        let data = [0b0101_1000];
        let mut r = BitReader::new(&data, 8);
        assert_eq!(vlc.decode(&mut r), Some(0));
        assert_eq!(vlc.decode(&mut r), Some(1));
        assert_eq!(vlc.decode(&mut r), Some(2));
        assert_eq!(vlc.decode(&mut r), Some(0));
        let vlc = Vlc::from_lengths(&[(7, 1), (8, 2), (9, 2)]);
        let mut r = BitReader::new(&data, 8);
        assert_eq!(vlc.decode(&mut r), Some(7));
        assert_eq!(vlc.decode(&mut r), Some(8));
        assert_eq!(vlc.decode(&mut r), Some(9));
    }
}
