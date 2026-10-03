//! Reading a bitstream most significant bit first.

pub struct BitReader<'a> {
    data: &'a [u8],
    /// Position in bits.
    pos: usize,
    /// Bits that may be read; reading past them gives zeros.
    end: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8], bits: usize) -> BitReader<'a> {
        BitReader {
            data,
            pos: 0,
            end: bits.min(data.len() * 8),
        }
    }

    pub fn bit(&mut self) -> bool {
        let b = self.pos < self.end && self.data[self.pos >> 3] & (0x80 >> (self.pos & 7)) != 0;
        self.pos += 1;
        b
    }

    /// The next `n` bits (at most 32) as a number.
    pub fn bits(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit() as u32;
        }
        v
    }

    pub fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn left(&self) -> isize {
        self.end as isize - self.pos as isize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_most_significant_bit_first() {
        let mut r = BitReader::new(&[0b1010_0000, 0xFF], 12);
        assert_eq!(r.bits(3), 0b101);
        assert_eq!(r.bits(5), 0);
        assert_eq!(r.bits(4), 0xF);
        assert_eq!(r.left(), 0);
        // Past the end: zeros.
        assert_eq!(r.bits(4), 0);
    }
}
