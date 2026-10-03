//! Windows Media Audio 2 decoding.
//!
//! A Rust port of FFmpeg's WMA decoder (libavcodec/wma.c and wmadec.c,
//! copyright the FFmpeg project), and like it licensed under the GNU Lesser
//! General Public License, version 2.1 or later. Halo 2 stores its announcer
//! and other dialogue this way.
//!
//! Not supported: WMA 1, and exponents coded as line spectral pairs (WMA 2
//! streams without the exponent VLC flag).

// Per-channel state lives in parallel arrays, walked by channel number.
#![allow(clippy::needless_range_loop)]

mod bits;
mod mdct;
mod tables;
mod vlc;

use bits::BitReader;
use mdct::Imdct;
use std::fmt;
use tables::*;
use vlc::Vlc;

const BLOCK_MIN_BITS: u32 = 7;
const HIGH_BAND_MAX_SIZE: usize = 16;
const NOISE_TAB_SIZE: usize = 8192;
const MAX_CODED_SUPERFRAME_SIZE: usize = 32768;
/// Largest `byte_offset_bits + 3` FFmpeg's bit reader allows.
const MIN_CACHE_BITS: u32 = 25;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Unsupported(String),
    Invalid(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Unsupported(what) => write!(f, "unsupported WMA stream: {what}"),
            Error::Invalid(what) => write!(f, "invalid WMA data: {what}"),
        }
    }
}

impl std::error::Error for Error {}

/// A stream's format, from its WAVEFORMATEX.
#[derive(Debug, Clone)]
pub struct Format {
    /// 0x161 for WMA 2.
    pub tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub byte_rate: u32,
    /// Bytes per packet.
    pub block_align: u16,
    /// The codec's own bytes after the WAVEFORMATEX.
    pub extra: Vec<u8>,
}

impl Format {
    /// Read a WAVEFORMATEX structure.
    pub fn parse(b: &[u8]) -> Result<Format, Error> {
        if b.len() < 18 {
            return Err(Error::Invalid("WAVEFORMATEX too short"));
        }
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let extra_len = u16_at(16) as usize;
        let extra = b
            .get(18..18 + extra_len)
            .ok_or(Error::Invalid("WAVEFORMATEX extra bytes missing"))?;
        Ok(Format {
            tag: u16_at(0),
            channels: u16_at(2),
            sample_rate: u32_at(4),
            byte_rate: u32_at(8),
            block_align: u16_at(12),
            extra: extra.to_vec(),
        })
    }
}

/// floor(log2(v)), 0 for 0.
fn log2(v: u32) -> u32 {
    31 - (v | 1).leading_zeros()
}

/// One of the two coefficient code tables in use.
struct CoefTable {
    vlc: Vlc,
    runs: Vec<u16>,
    levels: Vec<f32>,
}

impl CoefTable {
    fn new(index: usize) -> CoefTable {
        let (codes, bits, runs_per_level) = COEF_TABLES[index];
        let n = codes.len();
        let mut runs = vec![0u16; n];
        let mut levels = vec![0f32; n];
        // Codes 0 and 1 are escape and end of block; then each level's runs.
        let mut i = 2;
        for (level, &count) in runs_per_level.iter().enumerate() {
            for run in 0..count {
                if i < n {
                    runs[i] = run;
                    levels[i] = (level + 1) as f32;
                }
                i += 1;
            }
        }
        CoefTable {
            vlc: Vlc::new(codes, bits),
            runs,
            levels,
        }
    }
}

/// Decodes a WMA 2 stream packet by packet.
pub struct Decoder {
    channels: usize,
    sample_rate: u32,
    block_align: usize,
    use_bit_reservoir: bool,
    use_variable_block_len: bool,
    use_noise_coding: bool,
    byte_offset_bits: u32,
    exp_vlc: Vlc,
    hgain_vlc: Vlc,
    coef: [CoefTable; 2],
    /// Per block size (largest first):
    exponent_bands: Vec<Vec<usize>>,
    high_band_start: Vec<usize>,
    coefs_end: Vec<usize>,
    exponent_high_bands: Vec<Vec<usize>>,
    imdct: Vec<Imdct>,
    windows: Vec<Vec<f32>>,

    frame_len: usize,
    frame_len_bits: u32,
    nb_block_sizes: usize,
    reset_block_lengths: bool,
    block_len_bits: u32,
    next_block_len_bits: u32,
    prev_block_len_bits: u32,
    block_len: usize,
    block_pos: usize,
    ms_stereo: bool,
    channel_coded: [bool; 2],
    high_band_coded: [[bool; HIGH_BAND_MAX_SIZE]; 2],
    high_band_values: [[i32; HIGH_BAND_MAX_SIZE]; 2],
    exponents_bsize: [usize; 2],
    exponents: [Vec<f32>; 2],
    max_exponent: [f32; 2],
    exponents_initialized: [bool; 2],
    coefs1: [Vec<f32>; 2],
    coefs: [Vec<f32>; 2],
    output: Vec<f32>,
    /// Two frames per channel: the one being finished and the overlap.
    frame_out: [Vec<f32>; 2],
    last_superframe: Vec<u8>,
    last_bitoffset: usize,
    noise_table: Vec<f32>,
    noise_index: usize,
    noise_mult: f32,
}

/// 10^(i/16) for i in -60..96.
fn pow_tab(i: i32) -> f32 {
    10f64.powf(i as f64 / 16.0) as f32
}

impl Decoder {
    pub fn new(format: &Format) -> Result<Decoder, Error> {
        if format.tag != 0x161 {
            return Err(Error::Unsupported(format!(
                "audio format 0x{:x} (only WMA 2, 0x161)",
                format.tag
            )));
        }
        let channels = format.channels as usize;
        let sample_rate = format.sample_rate;
        let bit_rate = format.byte_rate as u64 * 8;
        if sample_rate == 0 || sample_rate > 50000 || channels == 0 || channels > 2 || bit_rate == 0
        {
            return Err(Error::Unsupported(format!(
                "{channels} channels at {sample_rate} Hz, {bit_rate} bits/s"
            )));
        }
        if format.block_align == 0 {
            return Err(Error::Invalid("no packet size"));
        }
        let e = &format.extra;
        let flags2 = if e.len() >= 6 {
            u16::from_le_bytes([e[4], e[5]])
        } else {
            0
        };
        let use_exp_vlc = flags2 & 1 != 0;
        let use_bit_reservoir = flags2 & 2 != 0;
        let mut use_variable_block_len = flags2 & 4 != 0;
        if e.len() >= 8 && flags2 == 0xd && use_variable_block_len {
            // FFmpeg's workaround for a known broken encoder.
            use_variable_block_len = false;
        }
        if !use_exp_vlc {
            return Err(Error::Unsupported(
                "exponents coded as line spectral pairs".into(),
            ));
        }

        let frame_len_bits: u32 = if sample_rate <= 16000 {
            9
        } else if sample_rate <= 22050 {
            10
        } else {
            11
        };
        let frame_len = 1usize << frame_len_bits;
        let nb_block_sizes = if use_variable_block_len {
            let mut nb = ((flags2 >> 3) & 3) as u32 + 1;
            if bit_rate / channels as u64 >= 32000 {
                nb += 2;
            }
            nb.min(frame_len_bits - BLOCK_MIN_BITS) as usize + 1
        } else {
            1
        };

        // Rate dependent choices.
        let mut use_noise_coding = true;
        let mut high_freq = sample_rate as f32 * 0.5;
        let sample_rate1 = match sample_rate {
            r if r >= 44100 => 44100,
            r if r >= 22050 => 22050,
            r if r >= 16000 => 16000,
            r if r >= 11025 => 11025,
            r if r >= 8000 => 8000,
            r => r,
        };
        let bps = bit_rate as f32 / (channels as u32 * sample_rate) as f32;
        let byte_offset_bits =
            log2(((bps * frame_len as f32) as f64 / 8.0 + 0.5) as i32 as u32) + 2;
        if byte_offset_bits + 3 > MIN_CACHE_BITS {
            return Err(Error::Unsupported(format!(
                "byte offset bits {byte_offset_bits}"
            )));
        }
        let bps1 = if channels == 2 {
            (bps as f64 * 1.6) as f32
        } else {
            bps
        };
        let scale = |f: f32, by: f64| (f as f64 * by) as f32;
        match sample_rate1 {
            44100 => {
                if bps1 >= 0.61 {
                    use_noise_coding = false;
                } else {
                    high_freq = scale(high_freq, 0.4);
                }
            }
            22050 => {
                if bps1 >= 1.16 {
                    use_noise_coding = false;
                } else if bps1 >= 0.72 {
                    high_freq = scale(high_freq, 0.7);
                } else {
                    high_freq = scale(high_freq, 0.6);
                }
            }
            16000 => high_freq = scale(high_freq, if bps > 0.5 { 0.5 } else { 0.3 }),
            11025 => high_freq = scale(high_freq, 0.7),
            8000 => {
                if bps <= 0.625 {
                    high_freq = scale(high_freq, 0.5);
                } else if bps > 0.75 {
                    use_noise_coding = false;
                } else {
                    high_freq = scale(high_freq, 0.65);
                }
            }
            _ => {
                let by = if bps >= 0.8 {
                    0.75
                } else if bps >= 0.6 {
                    0.6
                } else {
                    0.5
                };
                high_freq = scale(high_freq, by);
            }
        }

        // Scale factor bands for each block size.
        let mut exponent_bands = Vec::new();
        let mut high_band_start = Vec::new();
        let mut coefs_end = Vec::new();
        let mut exponent_high_bands = Vec::new();
        for k in 0..nb_block_sizes {
            let block_len = frame_len >> k;
            let a = (frame_len_bits - BLOCK_MIN_BITS) as usize - k;
            let table = match sample_rate {
                _ if a >= 3 => None,
                r if r >= 44100 => Some(EXPONENT_BANDS_44100[a]),
                r if r >= 32000 => Some(EXPONENT_BANDS_32000[a]),
                r if r >= 22050 => Some(EXPONENT_BANDS_22050[a]),
                _ => None,
            };
            let bands: Vec<usize> = match table {
                Some(t) => t.iter().map(|&b| b as usize).collect(),
                None => {
                    let mut bands = Vec::new();
                    let mut lpos = 0;
                    for &f in &CRITICAL_FREQS {
                        let (a, b) = (f as usize, sample_rate as usize);
                        let pos =
                            ((((block_len * 2 * a) + (b << 1)) / (4 * b)) << 2).min(block_len);
                        if pos > lpos {
                            bands.push(pos - lpos);
                        }
                        if pos >= block_len {
                            break;
                        }
                        lpos = pos;
                    }
                    bands
                }
            };
            let end = (frame_len - (frame_len * 9) / 100) >> k;
            let start =
                ((block_len as f32 * 2.0 * high_freq / sample_rate as f32) as f64 + 0.5) as usize;
            let mut high = Vec::new();
            let mut pos = 0;
            for &b in &bands {
                let (s, e) = (pos.max(start), (pos + b).min(end));
                pos += b;
                if e > s {
                    high.push(e - s);
                }
            }
            if high.len() > HIGH_BAND_MAX_SIZE {
                return Err(Error::Unsupported("too many high bands".into()));
            }
            exponent_bands.push(bands);
            high_band_start.push(start);
            coefs_end.push(end);
            exponent_high_bands.push(high);
        }

        let windows = (0..nb_block_sizes)
            .map(|i| {
                let n = 1usize << (frame_len_bits as usize - i);
                (0..n)
                    .map(|j| ((j as f64 + 0.5) * (std::f64::consts::PI / (2.0 * n as f64))) as f32)
                    .map(f32::sin)
                    .collect()
            })
            .collect();
        let imdct = (0..nb_block_sizes)
            .map(|i| Imdct::new(1 << (frame_len_bits as usize - i), 1.0 / 32768.0))
            .collect();

        let noise_mult = 0.02f32;
        let noise_table = if use_noise_coding {
            let norm =
                ((1.0 / (1u64 << 31) as f32) as f64 * 3f64.sqrt() * noise_mult as f64) as f32;
            let mut seed = 1u32;
            (0..NOISE_TAB_SIZE)
                .map(|_| {
                    seed = seed.wrapping_mul(314159).wrapping_add(1);
                    seed as i32 as f32 * norm
                })
                .collect()
        } else {
            Vec::new()
        };

        let coef_table = match () {
            _ if sample_rate >= 32000 && bps1 < 0.72 => 0,
            _ if sample_rate >= 32000 && bps1 < 1.16 => 1,
            _ => 2,
        };
        let buffer = || vec![0f32; frame_len];
        Ok(Decoder {
            channels,
            sample_rate,
            block_align: format.block_align as usize,
            use_bit_reservoir,
            use_variable_block_len,
            use_noise_coding,
            byte_offset_bits,
            exp_vlc: Vlc::new(&SCALEFACTOR_CODES, &SCALEFACTOR_BITS),
            hgain_vlc: if use_noise_coding {
                Vlc::from_lengths(&HGAIN_HUFFTAB)
            } else {
                Vlc::default()
            },
            coef: [
                CoefTable::new(coef_table * 2),
                CoefTable::new(coef_table * 2 + 1),
            ],
            exponent_bands,
            high_band_start,
            coefs_end,
            exponent_high_bands,
            imdct,
            windows,
            frame_len,
            frame_len_bits,
            nb_block_sizes,
            reset_block_lengths: true,
            block_len_bits: frame_len_bits,
            next_block_len_bits: frame_len_bits,
            prev_block_len_bits: frame_len_bits,
            block_len: frame_len,
            block_pos: 0,
            ms_stereo: false,
            channel_coded: [false; 2],
            high_band_coded: [[false; HIGH_BAND_MAX_SIZE]; 2],
            high_band_values: [[0; HIGH_BAND_MAX_SIZE]; 2],
            exponents_bsize: [0; 2],
            exponents: [buffer(), buffer()],
            max_exponent: [1.0; 2],
            exponents_initialized: [false; 2],
            coefs1: [buffer(), buffer()],
            coefs: [buffer(), buffer()],
            output: vec![0.0; frame_len * 2],
            frame_out: [vec![0.0; frame_len * 2], vec![0.0; frame_len * 2]],
            last_superframe: Vec::new(),
            last_bitoffset: 0,
            noise_table,
            noise_index: 0,
            noise_mult,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Samples per channel in each frame.
    pub fn frame_len(&self) -> usize {
        self.frame_len
    }

    fn noise(&mut self) -> f32 {
        let v = self.noise_table[self.noise_index];
        self.noise_index = (self.noise_index + 1) & (NOISE_TAB_SIZE - 1);
        v
    }

    /// Exponents (the spectral envelope) coded as deltas between bands.
    fn decode_exp_vlc(&mut self, ch: usize, r: &mut BitReader) -> Result<(), Error> {
        let bsize = (self.frame_len_bits - self.block_len_bits) as usize;
        let mut q = 0;
        let mut band = 0;
        let mut last_exp = 36i32;
        let mut max_scale = 0f32;
        while q < self.block_len {
            let code = self
                .exp_vlc
                .decode(r)
                .ok_or(Error::Invalid("bad exponent code"))?;
            last_exp += code as i32 - 60;
            if !(-60..96).contains(&last_exp) {
                return Err(Error::Invalid("exponent out of range"));
            }
            let v = pow_tab(last_exp);
            max_scale = max_scale.max(v);
            let n = *self.exponent_bands[bsize]
                .get(band)
                .ok_or(Error::Invalid("exponent bands overrun"))?;
            band += 1;
            // (An empty band, which no encoder writes, fills four.)
            let n = if n == 0 { 4 } else { n };
            let end = (q + n).min(self.block_len);
            self.exponents[ch][q..end].fill(v);
            q = end;
        }
        self.max_exponent[ch] = max_scale;
        Ok(())
    }

    /// Run/level coded spectral coefficients into `coefs1[ch]`.
    fn decode_coefs(
        &mut self,
        ch: usize,
        table: usize,
        num_coefs: usize,
        coef_nb_bits: u32,
        r: &mut BitReader,
    ) -> Result<(), Error> {
        let mask = self.block_len - 1;
        let t = &self.coef[table];
        let out = &mut self.coefs1[ch];
        out[..self.block_len].fill(0.0);
        let mut offset = 0;
        while offset < num_coefs {
            let code = t
                .vlc
                .decode(r)
                .ok_or(Error::Invalid("bad coefficient code"))?;
            if code > 1 {
                offset += t.runs[code] as usize;
                let level = t.levels[code];
                out[offset & mask] = if r.bit() { level } else { -level };
            } else if code == 1 {
                // End of block.
                break;
            } else {
                // Escape: level and run written out.
                let level = r.bits(coef_nb_bits) as f32;
                offset += r.bits(self.frame_len_bits) as usize;
                out[offset & mask] = if r.bit() { level } else { -level };
            }
            offset += 1;
        }
        if offset > num_coefs {
            return Err(Error::Invalid("coefficients overrun"));
        }
        Ok(())
    }

    /// Decode one block; true when it was the frame's last.
    fn decode_block(&mut self, r: &mut BitReader) -> Result<bool, Error> {
        let flb = self.frame_len_bits;
        if self.use_variable_block_len {
            let sizes = self.nb_block_sizes;
            let n = log2(sizes as u32 - 1) + 1;
            let size = |r: &mut BitReader| {
                let v = r.bits(n);
                if v as usize >= sizes {
                    Err(Error::Invalid("block size out of range"))
                } else {
                    Ok(flb - v)
                }
            };
            if self.reset_block_lengths {
                self.reset_block_lengths = false;
                self.prev_block_len_bits = size(r)?;
                self.block_len_bits = size(r)?;
            } else {
                self.prev_block_len_bits = self.block_len_bits;
                self.block_len_bits = self.next_block_len_bits;
            }
            self.next_block_len_bits = size(r)?;
        } else {
            self.next_block_len_bits = flb;
            self.prev_block_len_bits = flb;
            self.block_len_bits = flb;
        }
        if (flb - self.block_len_bits) as usize >= self.nb_block_sizes {
            return Err(Error::Invalid("block size out of range"));
        }
        self.block_len = 1 << self.block_len_bits;
        if self.block_pos + self.block_len > self.frame_len {
            return Err(Error::Invalid("blocks overrun the frame"));
        }
        let channels = self.channels;
        if channels == 2 {
            self.ms_stereo = r.bit();
        }
        let mut any = false;
        for ch in 0..channels {
            self.channel_coded[ch] = r.bit();
            any |= self.channel_coded[ch];
        }
        let bsize = (flb - self.block_len_bits) as usize;
        if any {
            self.decode_spectrum(bsize, r)?;
        }

        // Back to samples, overlapping the previous block.
        for ch in 0..channels {
            let n4 = self.block_len / 2;
            if self.channel_coded[ch] {
                let len = self.block_len;
                self.imdct[bsize].run(&self.coefs[ch][..len], &mut self.output[..len * 2]);
            } else if !(self.ms_stereo && ch == 1) {
                self.output.fill(0.0);
            }
            let index = self.frame_len / 2 + self.block_pos - n4;
            self.window(ch, index);
        }
        self.block_pos += self.block_len;
        Ok(self.block_pos >= self.frame_len)
    }

    /// Gain, exponents and coefficients of a block with coded channels,
    /// leaving the MDCT coefficients in `coefs`.
    fn decode_spectrum(&mut self, bsize: usize, r: &mut BitReader) -> Result<(), Error> {
        let channels = self.channels;
        let mut total_gain = 1i32;
        loop {
            if r.left() < 7 {
                return Err(Error::Invalid("total gain overread"));
            }
            let a = r.bits(7) as i32;
            total_gain += a;
            if a != 127 {
                break;
            }
        }
        let coef_nb_bits = match total_gain {
            g if g < 15 => 13,
            g if g < 32 => 12,
            g if g < 40 => 11,
            g if g < 45 => 10,
            _ => 9,
        };
        let mut nb_coefs = [self.coefs_end[bsize]; 2];
        let high_count = self.exponent_high_bands[bsize].len();
        if self.use_noise_coding {
            for ch in 0..channels {
                if self.channel_coded[ch] {
                    for i in 0..high_count {
                        let a = r.bit();
                        self.high_band_coded[ch][i] = a;
                        // Noise in place of these coefficients.
                        if a {
                            nb_coefs[ch] -= self.exponent_high_bands[bsize][i];
                        }
                    }
                }
            }
            for ch in 0..channels {
                if self.channel_coded[ch] {
                    let mut val: Option<i32> = None;
                    for i in 0..high_count {
                        if self.high_band_coded[ch][i] {
                            let v = match val {
                                None => r.bits(7) as i32 - 19,
                                Some(v) => {
                                    let d = self
                                        .hgain_vlc
                                        .decode(r)
                                        .ok_or(Error::Invalid("bad high band gain"))?;
                                    v + d as i32 - 18
                                }
                            };
                            val = Some(v);
                            self.high_band_values[ch][i] = v;
                        }
                    }
                }
            }
        }

        // Exponents can be reused in short blocks.
        if self.block_len_bits == self.frame_len_bits || r.bit() {
            for ch in 0..channels {
                if self.channel_coded[ch] {
                    self.decode_exp_vlc(ch, r)?;
                    self.exponents_bsize[ch] = bsize;
                    self.exponents_initialized[ch] = true;
                }
            }
        }
        for ch in 0..channels {
            if self.channel_coded[ch] && !self.exponents_initialized[ch] {
                return Err(Error::Invalid("exponents missing"));
            }
        }

        for ch in 0..channels {
            if self.channel_coded[ch] {
                // The side channel of mid/side stereo has its own table.
                let table = (ch == 1 && self.ms_stereo) as usize;
                self.decode_coefs(ch, table, nb_coefs[ch], coef_nb_bits, r)?;
            }
        }

        let mdct_norm = 1.0 / (self.block_len / 2) as f32;
        for ch in 0..channels {
            if !self.channel_coded[ch] {
                continue;
            }
            let esize = self.exponents_bsize[ch];
            let mult = ((10f64.powf(total_gain as f64 * 0.05) / self.max_exponent[ch] as f64)
                as f32)
                * mdct_norm;
            // Exponent for coefficient i, counted from exponent index `base`.
            let at = |base: usize, i: usize| base + ((i << bsize) >> esize);
            if self.use_noise_coding {
                self.noisy_coefs(ch, bsize, esize, mult, mdct_norm);
            } else {
                let n = nb_coefs[ch];
                for i in 0..n {
                    self.coefs[ch][i] = self.coefs1[ch][i] * self.exponents[ch][at(0, i)] * mult;
                }
                self.coefs[ch][n..self.block_len].fill(0.0);
            }
        }

        if self.ms_stereo && self.channel_coded[1] {
            if !self.channel_coded[0] {
                self.coefs[0][..self.block_len].fill(0.0);
                self.channel_coded[0] = true;
            }
            let [left, right] = &mut self.coefs;
            for (a, b) in left[..self.block_len]
                .iter_mut()
                .zip(&mut right[..self.block_len])
            {
                let t = *a - *b;
                *a += *b;
                *b = t;
            }
        }
        Ok(())
    }

    /// Coefficients with perceptual noise filling the high bands.
    fn noisy_coefs(&mut self, ch: usize, bsize: usize, esize: usize, mult: f32, mdct_norm: f32) {
        let at = |base: usize, i: usize| base + ((i << bsize) >> esize);
        let high = self.exponent_high_bands[bsize].clone();
        let start = self.high_band_start[bsize];
        let mut exp_power = [0f32; HIGH_BAND_MAX_SIZE];
        let mut last_high_band = 0;
        let mut e = (start << bsize) >> esize;
        for (j, &n) in high.iter().enumerate() {
            if self.high_band_coded[ch][j] {
                let mut e2 = 0f32;
                for i in 0..n {
                    let v = self.exponents[ch][at(e, i)];
                    e2 += v * v;
                }
                exp_power[j] = e2 / n as f32;
                last_high_band = j;
            }
            e += (n << bsize) >> esize;
        }

        let mut out = 0;
        let mut c1 = 0;
        let mut e = 0;
        for j in -1..high.len() as i32 {
            let n = if j < 0 { start } else { high[j as usize] };
            if j >= 0 && self.high_band_coded[ch][j as usize] {
                let j = j as usize;
                let mut mult1 = ((exp_power[j] / exp_power[last_high_band]) as f64).sqrt() as f32;
                mult1 =
                    (mult1 as f64 * 10f64.powf(self.high_band_values[ch][j] as f64 * 0.05)) as f32;
                mult1 /= self.max_exponent[ch] * self.noise_mult;
                mult1 *= mdct_norm;
                for i in 0..n {
                    let noise = self.noise();
                    self.coefs[ch][out] = noise * self.exponents[ch][at(e, i)] * mult1;
                    out += 1;
                }
            } else {
                for i in 0..n {
                    let noise = self.noise();
                    self.coefs[ch][out] =
                        (self.coefs1[ch][c1] + noise) * self.exponents[ch][at(e, i)] * mult;
                    c1 += 1;
                    out += 1;
                }
            }
            e += (n << bsize) >> esize;
        }
        // The very highest frequencies: noise at the last band's level.
        let n = self.block_len - self.coefs_end[bsize];
        let back = (-(1i64 << bsize)) >> esize;
        let mult1 = mult * self.exponents[ch][(e as i64 + back).max(0) as usize];
        for _ in 0..n {
            let noise = self.noise();
            self.coefs[ch][out] = noise * mult1;
            out += 1;
        }
    }

    /// Window the block's IMDCT output into the frame, overlapping its
    /// neighbours by the smaller of the two block sizes.
    fn window(&mut self, ch: usize, index: usize) {
        let flb = self.frame_len_bits;
        let block_len = self.block_len;
        let out = &mut self.frame_out[ch][index..];
        let input = &self.output;
        // Left half: fade in, added to the previous block's tail.
        if self.block_len_bits <= self.prev_block_len_bits {
            let w = &self.windows[(flb - self.block_len_bits) as usize];
            for i in 0..block_len {
                out[i] += input[i] * w[i];
            }
        } else {
            let bl = 1 << self.prev_block_len_bits;
            let n = (block_len - bl) / 2;
            let w = &self.windows[(flb - self.prev_block_len_bits) as usize];
            for i in 0..bl {
                out[n + i] += input[n + i] * w[i];
            }
            out[n + bl..n + bl + n].copy_from_slice(&input[n + bl..n + bl + n]);
        }
        // Right half: fade out, replacing what was there.
        let out = &mut out[block_len..];
        let input = &input[block_len..];
        if self.block_len_bits <= self.next_block_len_bits {
            let w = &self.windows[(flb - self.block_len_bits) as usize];
            for i in 0..block_len {
                out[i] = input[i] * w[block_len - 1 - i];
            }
        } else {
            let bl = 1 << self.next_block_len_bits;
            let n = (block_len - bl) / 2;
            let w = &self.windows[(flb - self.next_block_len_bits) as usize];
            out[..n].copy_from_slice(&input[..n]);
            for i in 0..bl {
                out[n + i] = input[n + i] * w[bl - 1 - i];
            }
            out[n + bl..n + bl + n].fill(0.0);
        }
    }

    /// Decode one frame, appending its samples (interleaved) to `out`.
    fn decode_frame(&mut self, r: &mut BitReader, out: &mut Vec<f32>) -> Result<(), Error> {
        self.block_pos = 0;
        while !self.decode_block(r)? {}
        let n = self.frame_len;
        for i in 0..n {
            for ch in 0..self.channels {
                out.push(self.frame_out[ch][i]);
            }
        }
        for ch in 0..self.channels {
            self.frame_out[ch].copy_within(n..2 * n, 0);
        }
        Ok(())
    }

    /// Decode one packet (`block_align` bytes), appending its samples
    /// (interleaved, -1..1) to `out`. A packet that fails adds nothing.
    pub fn decode_packet(&mut self, packet: &[u8], out: &mut Vec<f32>) -> Result<(), Error> {
        if packet.len() < self.block_align {
            return Err(Error::Invalid("packet too small"));
        }
        let before = out.len();
        let result = self.superframe(&packet[..self.block_align], out);
        if result.is_err() {
            out.truncate(before);
            self.last_superframe.clear();
        }
        result
    }

    fn superframe(&mut self, buf: &[u8], out: &mut Vec<f32>) -> Result<(), Error> {
        if !self.use_bit_reservoir {
            let mut r = BitReader::new(buf, buf.len() * 8);
            return self.decode_frame(&mut r, out);
        }
        let mut r = BitReader::new(buf, buf.len() * 8);
        // Superframe index, then how many frames end in this packet.
        r.skip(4);
        let count = r.bits(4) as i32 - self.last_superframe.is_empty() as i32;
        if count <= 0 {
            if count < 0 || r.left() <= 8 {
                return Err(Error::Invalid("no frames in packet"));
            }
            // All of it continues a frame.
            if self.last_superframe.len() + buf.len() - 1 > MAX_CODED_SUPERFRAME_SIZE {
                return Err(Error::Invalid("frame too large"));
            }
            self.last_superframe.extend_from_slice(&buf[1..]);
            return Ok(());
        }
        let mut frames = count as usize;
        let header = 4 + 4 + self.byte_offset_bits as usize + 3;
        let bit_offset = r.bits(self.byte_offset_bits + 3) as usize;
        if bit_offset as isize > r.left() {
            return Err(Error::Invalid("frame offset past the packet"));
        }
        if !self.last_superframe.is_empty() {
            // The first frame started in the previous packet.
            let mut last = std::mem::take(&mut self.last_superframe);
            let had = last.len();
            if had + bit_offset.div_ceil(8) > MAX_CODED_SUPERFRAME_SIZE {
                return Err(Error::Invalid("frame too large"));
            }
            let mut len = bit_offset;
            while len > 7 {
                last.push(r.bits(8) as u8);
                len -= 8;
            }
            if len > 0 {
                last.push((r.bits(len as u32) << (8 - len)) as u8);
            }
            let mut fr = BitReader::new(&last, had * 8 + bit_offset);
            fr.skip(self.last_bitoffset);
            self.decode_frame(&mut fr, out)?;
            frames -= 1;
        }

        // The frames starting in this packet.
        let pos = bit_offset + header;
        if pos > buf.len() * 8 {
            return Err(Error::Invalid("frame offset past the packet"));
        }
        let mut r = BitReader::new(&buf[pos >> 3..], (buf.len() - (pos >> 3)) * 8);
        r.skip(pos & 7);
        self.reset_block_lengths = true;
        for _ in 0..frames {
            self.decode_frame(&mut r, out)?;
        }
        // Keep the start of the next frame for the next packet.
        let pos = r.position() + (pos & !7);
        self.last_bitoffset = pos & 7;
        let pos = pos >> 3;
        if pos > buf.len() {
            return Err(Error::Invalid("frames overrun the packet"));
        }
        self.last_superframe = buf[pos..].to_vec();
        Ok(())
    }

    /// The last samples, still waiting on an overlap that will never come,
    /// at the end of the stream.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        for i in 0..self.frame_len {
            for ch in 0..self.channels {
                out.push(self.frame_out[ch][i]);
            }
        }
    }
}

/// Decode a whole stream of packets to interleaved samples (-1..1), less
/// the decoder's start-up delay of two frames. Packets that fail to decode
/// are skipped.
pub fn decode(format: &Format, data: &[u8]) -> Result<Vec<f32>, Error> {
    let mut d = Decoder::new(format)?;
    let mut out = Vec::new();
    let mut first_error = None;
    for packet in data.chunks_exact(d.block_align) {
        if let Err(e) = d.decode_packet(packet, &mut out) {
            first_error.get_or_insert(e);
        }
    }
    if out.is_empty() {
        if let Some(e) = first_error {
            return Err(e);
        }
    }
    d.finish(&mut out);
    let delay = (2 * d.frame_len * d.channels).min(out.len());
    out.drain(..delay);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_consistent() {
        for (codes, bits, levels) in COEF_TABLES {
            assert_eq!(codes.len(), bits.len());
            assert_eq!(
                levels.iter().map(|&l| l as usize).sum::<usize>(),
                codes.len() - 2
            );
        }
        for bands in [
            EXPONENT_BANDS_22050,
            EXPONENT_BANDS_32000,
            EXPONENT_BANDS_44100,
        ] {
            for (a, b) in bands.iter().enumerate() {
                assert_eq!(b.iter().map(|&v| v as usize).sum::<usize>(), 128 << a);
            }
        }
    }

    #[test]
    fn rejects_what_it_cannot_decode() {
        let format = Format {
            tag: 0x160,
            channels: 2,
            sample_rate: 44100,
            byte_rate: 12000,
            block_align: 4459,
            extra: vec![0; 10],
        };
        assert!(matches!(Decoder::new(&format), Err(Error::Unsupported(_))));
    }
}
