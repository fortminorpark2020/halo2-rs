//! Bitmap (`bitm`) tags: textures, decoded to RGBA8.
//!
//! On Halo 2 PC each bitmap's pixel data is a zlib stream stored in the map (or
//! a shared map) at the address given by its data pointer.

use crate::mapset::{pointer_offset, MapSet};
use crate::{f32_at, i16_at, i32_at, u32_at, DatumIndex, Error, Result};
use std::io::Read;

const BITM_SEQUENCES: usize = 0x3C;
const SEQUENCE_SIZE: usize = 0x3C;
const SPRITE_SIZE: usize = 0x20;
const BITM_BITMAPS: usize = 68;
const BITMAP_DATA_SIZE: usize = 116;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    A8,
    Y8,
    AY8,
    A8Y8,
    R5G6B5,
    A1R5G5B5,
    A4R4G4B4,
    X8R8G8B8,
    A8R8G8B8,
    Dxt1,
    Dxt3,
    Dxt5,
    P8Bump,
    Other(i16),
}

impl From<i16> for Format {
    fn from(v: i16) -> Self {
        match v {
            0 => Format::A8,
            1 => Format::Y8,
            2 => Format::AY8,
            3 => Format::A8Y8,
            6 => Format::R5G6B5,
            8 => Format::A1R5G5B5,
            9 => Format::A4R4G4B4,
            10 => Format::X8R8G8B8,
            11 => Format::A8R8G8B8,
            14 => Format::Dxt1,
            15 => Format::Dxt3,
            16 => Format::Dxt5,
            17 => Format::P8Bump,
            o => Format::Other(o),
        }
    }
}

impl Format {
    /// Bytes needed for the top mip level.
    fn top_level_size(self, w: usize, h: usize) -> Option<usize> {
        let blocks = w.div_ceil(4) * h.div_ceil(4);
        Some(match self {
            Format::A8 | Format::Y8 | Format::AY8 | Format::P8Bump => w * h,
            Format::A8Y8 | Format::R5G6B5 | Format::A1R5G5B5 | Format::A4R4G4B4 => w * h * 2,
            Format::X8R8G8B8 | Format::A8R8G8B8 => w * h * 4,
            Format::Dxt1 => blocks * 8,
            Format::Dxt3 | Format::Dxt5 => blocks * 16,
            Format::Other(_) => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, top row first.
    pub rgba: Vec<u8>,
}

/// A sprite inside one of the bitmap's images, in 0..1 texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    pub bitmap: i16,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub registration: [f32; 2],
}

/// A named run of images (animation frames, HUD variants) in a bitmap tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    pub name: String,
    pub first_bitmap: i16,
    pub bitmap_count: i16,
    pub sprites: Vec<Sprite>,
}

pub fn read_sequences(set: &mut MapSet, bitmap: DatumIndex) -> Result<Vec<Sequence>> {
    let (src, _, data) = set.tag_data(bitmap)?;
    let file = set.get(src);
    let region = file.meta_region();
    let raw = file.read_block(region, &data, BITM_SEQUENCES, SEQUENCE_SIZE)?;
    let mut out = Vec::new();
    for s in raw.as_chunks::<SEQUENCE_SIZE>().0 {
        let name_end = s[..0x20].iter().position(|&b| b == 0).unwrap_or(0x20);
        let sprites = file
            .read_block(region, s, 0x34, SPRITE_SIZE)?
            .as_chunks::<SPRITE_SIZE>()
            .0
            .iter()
            .map(|p| Sprite {
                bitmap: i16_at(p, 0),
                left: f32_at(p, 0x8),
                right: f32_at(p, 0xC),
                top: f32_at(p, 0x10),
                bottom: f32_at(p, 0x14),
                registration: [f32_at(p, 0x18), f32_at(p, 0x1C)],
            })
            .collect();
        out.push(Sequence {
            name: String::from_utf8_lossy(&s[..name_end]).into_owned(),
            first_bitmap: i16_at(s, 0x20),
            bitmap_count: i16_at(s, 0x22),
            sprites,
        });
    }
    Ok(out)
}

/// How many images a bitmap tag holds.
pub fn image_count(set: &mut MapSet, bitmap: DatumIndex) -> Result<usize> {
    let (src, _, data) = set.tag_data(bitmap)?;
    let file = set.get(src);
    let region = file.meta_region();
    let entries = file.read_block(region, &data, BITM_BITMAPS, BITMAP_DATA_SIZE)?;
    Ok(entries.len() / BITMAP_DATA_SIZE)
}

/// Decode the first 2D image of a bitmap tag.
pub fn read_bitmap(set: &mut MapSet, bitmap: DatumIndex) -> Result<Image> {
    read_bitmap_at(set, bitmap, 0)
}

/// Decode image `index` of a bitmap tag (top mip level only).
pub fn read_bitmap_at(set: &mut MapSet, bitmap: DatumIndex, index: usize) -> Result<Image> {
    let (src, _, data) = set.tag_data(bitmap)?;
    let file = set.get(src);
    let region = file.meta_region();
    let entries = file.read_block(region, &data, BITM_BITMAPS, BITMAP_DATA_SIZE)?;
    let e = entries
        .get(index * BITMAP_DATA_SIZE..(index + 1) * BITMAP_DATA_SIZE)
        .ok_or_else(|| Error::Corrupt(format!("bitmap has no image {index}")))?;
    let width = i16_at(e, 4).max(1) as usize;
    let height = i16_at(e, 6).max(1) as usize;
    let format = Format::from(i16_at(e, 12));
    let pointer = u32_at(e, 28);
    let compressed = i32_at(e, 52);
    let need = format
        .top_level_size(width, height)
        .ok_or_else(|| Error::Corrupt(format!("unsupported bitmap format {format:?}")))?;
    if compressed <= 0 {
        return Err(Error::Corrupt("bitmap has no pixel data".into()));
    }

    let file = set.resource_file(src, pointer)?;
    let raw = file.read_raw(pointer_offset(pointer), compressed as usize)?;
    let mut pixels = Vec::with_capacity(need);
    flate2::read::ZlibDecoder::new(&raw[..])
        .take(need as u64)
        .read_to_end(&mut pixels)?;
    if pixels.len() < need {
        return Err(Error::Corrupt(format!(
            "bitmap data short: {} of {need} bytes",
            pixels.len()
        )));
    }
    Ok(Image {
        width: width as u32,
        height: height as u32,
        rgba: decode(format, width, height, &pixels),
    })
}

fn expand(v: u32, bits: u32) -> u8 {
    ((v * 255 + ((1 << bits) - 1) / 2) / ((1 << bits) - 1)) as u8
}

fn rgb565(c: u16) -> [u8; 3] {
    [
        expand((c >> 11) as u32 & 31, 5),
        expand((c >> 5) as u32 & 63, 6),
        expand(c as u32 & 31, 5),
    ]
}

pub fn decode(format: Format, w: usize, h: usize, src: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let px = |i: usize| -> &[u8] { &src[i..] };
    match format {
        Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => decode_bc(format, w, h, src, &mut out),
        _ => {
            for i in 0..w * h {
                let rgba: [u8; 4] = match format {
                    Format::A8 => [255, 255, 255, src[i]],
                    Format::Y8 => [src[i], src[i], src[i], 255],
                    Format::AY8 => [src[i]; 4],
                    Format::P8Bump => [128, 128, 255, 255],
                    Format::A8Y8 => [px(i * 2)[0], px(i * 2)[0], px(i * 2)[0], px(i * 2)[1]],
                    Format::R5G6B5 => {
                        let [r, g, b] = rgb565(u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]));
                        [r, g, b, 255]
                    }
                    Format::A1R5G5B5 => {
                        let c = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]) as u32;
                        let a = if c >> 15 != 0 { 255 } else { 0 };
                        [
                            expand((c >> 10) & 31, 5),
                            expand((c >> 5) & 31, 5),
                            expand(c & 31, 5),
                            a,
                        ]
                    }
                    Format::A4R4G4B4 => {
                        let c = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]) as u32;
                        [
                            expand((c >> 8) & 15, 4),
                            expand((c >> 4) & 15, 4),
                            expand(c & 15, 4),
                            expand(c >> 12, 4),
                        ]
                    }
                    // stored as B, G, R, A
                    Format::X8R8G8B8 => [src[i * 4 + 2], src[i * 4 + 1], src[i * 4], 255],
                    Format::A8R8G8B8 => {
                        [src[i * 4 + 2], src[i * 4 + 1], src[i * 4], src[i * 4 + 3]]
                    }
                    _ => [255, 0, 255, 255],
                };
                out[i * 4..i * 4 + 4].copy_from_slice(&rgba);
            }
        }
    }
    out
}

fn decode_bc(format: Format, w: usize, h: usize, src: &[u8], out: &mut [u8]) {
    let block_size = if format == Format::Dxt1 { 8 } else { 16 };
    let bw = w.div_ceil(4);
    for by in 0..h.div_ceil(4) {
        for bx in 0..bw {
            let b = &src[(by * bw + bx) * block_size..][..block_size];
            let (alpha, color) = if format == Format::Dxt1 {
                (None, b)
            } else {
                (Some(&b[..8]), &b[8..])
            };
            let c0 = u16::from_le_bytes([color[0], color[1]]);
            let c1 = u16::from_le_bytes([color[2], color[3]]);
            let (p0, p1) = (rgb565(c0), rgb565(c1));
            let mix = |a: u8, b: u8, wa: u16, wb: u16| {
                ((a as u16 * wa + b as u16 * wb) / (wa + wb)) as u8
            };
            let mut palette = [[0u8; 4]; 4];
            palette[0] = [p0[0], p0[1], p0[2], 255];
            palette[1] = [p1[0], p1[1], p1[2], 255];
            if c0 > c1 || format != Format::Dxt1 {
                palette[2] = [
                    mix(p0[0], p1[0], 2, 1),
                    mix(p0[1], p1[1], 2, 1),
                    mix(p0[2], p1[2], 2, 1),
                    255,
                ];
                palette[3] = [
                    mix(p0[0], p1[0], 1, 2),
                    mix(p0[1], p1[1], 1, 2),
                    mix(p0[2], p1[2], 1, 2),
                    255,
                ];
            } else {
                palette[2] = [
                    mix(p0[0], p1[0], 1, 1),
                    mix(p0[1], p1[1], 1, 1),
                    mix(p0[2], p1[2], 1, 1),
                    255,
                ];
                palette[3] = [0, 0, 0, 0];
            }
            let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
            let alphas: Option<[u8; 16]> = alpha.map(|a| {
                let mut v = [255u8; 16];
                if format == Format::Dxt3 {
                    for (i, x) in v.iter_mut().enumerate() {
                        *x = ((a[i / 2] >> ((i % 2) * 4)) & 15) * 17;
                    }
                } else {
                    let (a0, a1) = (a[0] as u16, a[1] as u16);
                    let mut table = [0u8; 8];
                    table[0] = a0 as u8;
                    table[1] = a1 as u8;
                    for k in 2..8u16 {
                        table[k as usize] = if a0 > a1 {
                            ((a0 * (8 - k) + a1 * (k - 1)) / 7) as u8
                        } else if k < 6 {
                            ((a0 * (6 - k) + a1 * (k - 1)) / 5) as u8
                        } else if k == 6 {
                            0
                        } else {
                            255
                        };
                    }
                    let mut abits = 0u64;
                    for (i, byte) in a[2..8].iter().enumerate() {
                        abits |= (*byte as u64) << (8 * i);
                    }
                    for (i, x) in v.iter_mut().enumerate() {
                        *x = table[((abits >> (3 * i)) & 7) as usize];
                    }
                }
                v
            });
            for py in 0..4 {
                for pxl in 0..4 {
                    let (x, y) = (bx * 4 + pxl, by * 4 + py);
                    if x >= w || y >= h {
                        continue;
                    }
                    let i = py * 4 + pxl;
                    let mut c = palette[((bits >> (2 * i)) & 3) as usize];
                    if let Some(a) = &alphas {
                        c[3] = a[i];
                    }
                    out[(y * w + x) * 4..][..4].copy_from_slice(&c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxt1_solid_red_block() {
        // c0 = c1 = pure red (0xF800), all indices 0
        let block = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0];
        let rgba = decode(Format::Dxt1, 4, 4, &block);
        assert!(rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 0, 0, 255]));
    }

    #[test]
    fn argb_is_stored_bgra() {
        assert_eq!(
            decode(Format::A8R8G8B8, 1, 1, &[1, 2, 3, 4]),
            vec![3, 2, 1, 4]
        );
    }
}
