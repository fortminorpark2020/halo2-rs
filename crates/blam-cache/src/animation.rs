//! Animation graphs (`jmad`): a skeleton and keyframed node animations.
//!
//! Each animation's data blob holds, in order: the "default" data (one value
//! per node that never moves), the animated data (a codec header followed by
//! per node tracks), then two sets of node flags saying which nodes the static
//! and the animated parts cover. Halo 2 uses four codecs for these:
//!
//! - 1: static, one value per node
//! - 3: every frame, 8-byte quantized rotations and float translations
//! - 4 / 6: keyframes with byte frame indices (6 stores the nodes in reverse)
//!
//! Codec 8 (blend screens, used for aiming) is not decoded yet.

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};

const JMAD_PARENT: usize = 0x0;
const JMAD_NODES: usize = 0xC;
const NODE_SIZE: usize = 0x20;
const JMAD_ANIMATIONS: usize = 0x2C;
const ANIMATION_SIZE: usize = 0x60;

const CODEC_STATIC: u8 = 1;
const CODEC_FULL_FRAMES: u8 = 3;
const CODEC_BYTE_KEYFRAMES: u8 = 4;
const CODEC_WORD_KEYFRAMES: u8 = 5;
const CODEC_REVERSE_BYTE_KEYFRAMES: u8 = 6;
const CODEC_REVERSE_WORD_KEYFRAMES: u8 = 7;

/// Halo animations play at 30 frames a second.
pub const FRAME_RATE: f32 = 30.0;

#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    pub name: String,
    pub parent: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationKind {
    /// A full pose.
    Base,
    /// Added on top of whatever base animation is playing.
    Overlay,
    /// Replaces the nodes it animates.
    Replacement,
    Other(u8),
}

impl From<u8> for AnimationKind {
    fn from(v: u8) -> Self {
        match v {
            0 => AnimationKind::Base,
            1 => AnimationKind::Overlay,
            2 => AnimationKind::Replacement,
            n => AnimationKind::Other(n),
        }
    }
}

/// Values at increasing frame numbers; between keys the value is interpolated.
#[derive(Debug, Clone, PartialEq)]
pub struct Track<T> {
    pub frames: Vec<u16>,
    pub values: Vec<T>,
}

impl<T: Copy> Track<T> {
    #[cfg(test)]
    fn constant(v: T) -> Self {
        Track {
            frames: vec![0],
            values: vec![v],
        }
    }

    /// The two keys around `frame` and how far between them it is.
    pub fn around(&self, frame: f32) -> (T, T, f32) {
        let first = self.values[0];
        if self.values.len() == 1 || frame <= self.frames[0] as f32 {
            return (first, first, 0.0);
        }
        for k in 1..self.frames.len() {
            let (f0, f1) = (self.frames[k - 1] as f32, self.frames[k] as f32);
            if frame <= f1 {
                let t = if f1 > f0 {
                    (frame - f0) / (f1 - f0)
                } else {
                    1.0
                };
                return (self.values[k - 1], self.values[k], t);
            }
        }
        let last = *self.values.last().unwrap();
        (last, last, 0.0)
    }
}

impl Track<[f32; 4]> {
    /// Normalised-lerp between keys (taking the short way round).
    pub fn sample(&self, frame: f32) -> [f32; 4] {
        let (a, mut b, t) = self.around(frame);
        if dot4(a, b) < 0.0 {
            b = b.map(|x| -x);
        }
        let mut q = [0.0; 4];
        for k in 0..4 {
            q[k] = a[k] + (b[k] - a[k]) * t;
        }
        normalize4(q)
    }
}

impl Track<[f32; 3]> {
    pub fn sample(&self, frame: f32) -> [f32; 3] {
        let (a, b, t) = self.around(frame);
        [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
    }
}

impl Track<f32> {
    pub fn sample(&self, frame: f32) -> f32 {
        let (a, b, t) = self.around(frame);
        a + (b - a) * t
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub name: String,
    pub kind: AnimationKind,
    pub frame_count: u16,
    /// False when the data uses a codec this reader doesn't know; every track
    /// is then empty.
    pub decoded: bool,
    /// Per graph node; `None` where the animation leaves the node alone.
    /// Quaternions are (i, j, k, w).
    pub rotations: Vec<Option<Track<[f32; 4]>>>,
    pub translations: Vec<Option<Track<[f32; 3]>>>,
    pub scales: Vec<Option<Track<f32>>>,
}

impl Animation {
    /// Length in seconds.
    pub fn duration(&self) -> f32 {
        self.frame_count.saturating_sub(1) as f32 / FRAME_RATE
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimationGraph {
    /// Graph this one inherits animations from.
    pub parent: Option<DatumIndex>,
    pub nodes: Vec<GraphNode>,
    pub animations: Vec<Animation>,
}

impl AnimationGraph {
    pub fn find(&self, name: &str) -> Option<&Animation> {
        self.animations.iter().find(|a| a.name == name)
    }

    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }
}

pub fn read_animation_graph(set: &mut MapSet, jmad: DatumIndex) -> Result<AnimationGraph> {
    let (src, _, data) = set.tag_data(jmad)?;
    let file = set.get(src);
    let region = file.meta_region();
    let parent = DatumIndex(u32_at(&data, JMAD_PARENT + 4));
    let nodes_raw = file.read_block(region, &data, JMAD_NODES, NODE_SIZE)?;
    let nodes: Vec<GraphNode> = nodes_raw
        .as_chunks::<NODE_SIZE>()
        .0
        .iter()
        .map(|n| GraphNode {
            name: file.string_id(u32_at(n, 0)).unwrap_or("").to_string(),
            parent: i16_at(n, 8),
        })
        .collect();
    let anims = file.read_block(region, &data, JMAD_ANIMATIONS, ANIMATION_SIZE)?;
    let mut animations = Vec::new();
    for a in anims.as_chunks::<ANIMATION_SIZE>().0 {
        let name = file.string_id(u32_at(a, 0)).unwrap_or("").to_string();
        let size = u32_at(a, 0x28) as usize;
        let blob = if size > 0 {
            file.read_in(region, u32_at(a, 0x2C), size)?
        } else {
            Vec::new()
        };
        let sizes = DataSizes::read(a);
        let mut anim = Animation {
            name,
            kind: AnimationKind::from(a[0x10]),
            frame_count: i16_at(a, 0x14).max(0) as u16,
            decoded: false,
            rotations: vec![None; nodes.len()],
            translations: vec![None; nodes.len()],
            scales: vec![None; nodes.len()],
        };
        if decode_animation(&mut anim, &blob, &sizes).is_ok() {
            anim.decoded = true;
        } else {
            anim.rotations.iter_mut().for_each(|r| *r = None);
            anim.translations.iter_mut().for_each(|t| *t = None);
            anim.scales.iter_mut().for_each(|s| *s = None);
        }
        animations.push(anim);
    }
    Ok(AnimationGraph {
        parent: (parent != DatumIndex::NONE).then_some(parent),
        nodes,
        animations,
    })
}

/// How the animation's data blob is divided up.
#[derive(Debug, Clone, Copy)]
struct DataSizes {
    static_flags: usize,
    animated_flags: usize,
    default: usize,
    uncompressed: usize,
    compressed: usize,
}

impl DataSizes {
    fn read(a: &[u8]) -> DataSizes {
        DataSizes {
            static_flags: a[0x30] as usize,
            animated_flags: a[0x31] as usize,
            default: u16::from_le_bytes([a[0x36], a[0x37]]) as usize,
            uncompressed: u32_at(a, 0x38) as usize,
            compressed: u32_at(a, 0x3C) as usize,
        }
    }
}

/// Decoded codec data, in the codec's own node order.
#[derive(Debug, Default)]
struct Tracks {
    rotations: Vec<Track<[f32; 4]>>,
    translations: Vec<Track<[f32; 3]>>,
    scales: Vec<Track<f32>>,
}

fn decode_animation(anim: &mut Animation, blob: &[u8], sizes: &DataSizes) -> Result<()> {
    let corrupt = || Error::Corrupt(format!("animation {} data doesn't fit", anim.name));
    let animated_start = sizes.default + sizes.uncompressed;
    let flags_start = animated_start + sizes.compressed;
    let static_flags = blob
        .get(flags_start..flags_start + sizes.static_flags)
        .ok_or_else(corrupt)?;
    let animated_flags = blob
        .get(
            flags_start + sizes.static_flags
                ..flags_start + sizes.static_flags + sizes.animated_flags,
        )
        .ok_or_else(corrupt)?;
    let frames = anim.frame_count as usize;
    if sizes.default > 0 {
        let tracks = decode_codec(&blob[..sizes.default], frames)?;
        assign(anim, &tracks, static_flags)?;
    }
    if sizes.compressed > 0 {
        let tracks = decode_codec(&blob[animated_start..flags_start], frames)?;
        assign(anim, &tracks, animated_flags)?;
    } else if sizes.uncompressed > 0 {
        let tracks = decode_codec(&blob[sizes.default..animated_start], frames)?;
        assign(anim, &tracks, animated_flags)?;
    }
    Ok(())
}

/// The node indices whose bits are set in a little-endian bit field.
fn flagged_nodes(bits: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    for (byte, b) in bits.iter().enumerate() {
        for bit in 0..8 {
            if b & (1 << bit) != 0 {
                out.push(byte * 8 + bit);
            }
        }
    }
    out
}

/// Hand the codec's tracks to the nodes its flags name (rotation, translation
/// and scale flags, one bit field each).
fn assign(anim: &mut Animation, tracks: &Tracks, flags: &[u8]) -> Result<()> {
    let n = flags.len() / 3;
    let field = |k: usize| flagged_nodes(&flags[k * n..(k + 1) * n]);
    let (rot, trans, scale) = (field(0), field(1), field(2));
    if rot.len() != tracks.rotations.len()
        || trans.len() != tracks.translations.len()
        || scale.len() != tracks.scales.len()
    {
        return Err(Error::Corrupt(format!(
            "animation {}: node flags don't match its data",
            anim.name
        )));
    }
    let nodes = anim.rotations.len();
    for (node, track) in rot.into_iter().zip(&tracks.rotations) {
        if node < nodes {
            anim.rotations[node] = Some(track.clone());
        }
    }
    for (node, track) in trans.into_iter().zip(&tracks.translations) {
        if node < nodes {
            anim.translations[node] = Some(track.clone());
        }
    }
    for (node, track) in scale.into_iter().zip(&tracks.scales) {
        if node < nodes {
            anim.scales[node] = Some(track.clone());
        }
    }
    Ok(())
}

fn quaternion(b: &[u8], o: usize) -> [f32; 4] {
    normalize4([0, 1, 2, 3].map(|k| i16_at(b, o + k * 2) as f32 / 32767.0))
}

fn vector(b: &[u8], o: usize) -> [f32; 3] {
    [f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8)]
}

fn decode_codec(data: &[u8], frame_count: usize) -> Result<Tracks> {
    let corrupt = |what: &str| Error::Corrupt(format!("animation codec data: {what}"));
    if data.len() < 0x20 {
        return Err(corrupt("too short"));
    }
    let (codec, nr, nt, ns) = (
        data[0],
        data[1] as usize,
        data[2] as usize,
        data[3] as usize,
    );
    let off = |o: usize| u32_at(data, o) as usize;
    let need = |end: usize| {
        if end <= data.len() {
            Ok(())
        } else {
            Err(corrupt("track data past the end"))
        }
    };
    let mut out = Tracks::default();
    match codec {
        CODEC_STATIC | CODEC_FULL_FRAMES => {
            let keys = if codec == CODEC_STATIC {
                1
            } else {
                frame_count.max(1)
            };
            let (trans_at, scale_at) = (off(0x0C), off(0x10));
            need(0x20 + nr * keys * 8)?;
            need(trans_at + nt * keys * 12)?;
            need(scale_at + ns * keys * 4)?;
            let frames: Vec<u16> = (0..keys as u16).collect();
            for n in 0..nr {
                let values = (0..keys)
                    .map(|k| quaternion(data, 0x20 + (n * keys + k) * 8))
                    .collect();
                out.rotations.push(Track {
                    frames: frames.clone(),
                    values,
                });
            }
            for n in 0..nt {
                let values = (0..keys)
                    .map(|k| vector(data, trans_at + (n * keys + k) * 12))
                    .collect();
                out.translations.push(Track {
                    frames: frames.clone(),
                    values,
                });
            }
            for n in 0..ns {
                let values = (0..keys)
                    .map(|k| f32_at(data, scale_at + (n * keys + k) * 4))
                    .collect();
                out.scales.push(Track {
                    frames: frames.clone(),
                    values,
                });
            }
        }
        CODEC_BYTE_KEYFRAMES
        | CODEC_WORD_KEYFRAMES
        | CODEC_REVERSE_BYTE_KEYFRAMES
        | CODEC_REVERSE_WORD_KEYFRAMES => {
            if data.len() < 0x30 {
                return Err(corrupt("keyframe header too short"));
            }
            let index_size = if matches!(codec, CODEC_WORD_KEYFRAMES | CODEC_REVERSE_WORD_KEYFRAMES)
            {
                2
            } else {
                1
            };
            let tables = [0x30, off(0x0C), off(0x10)];
            let indices = [off(0x14), off(0x18), off(0x1C)];
            let values = [off(0x20), off(0x24), off(0x28)];
            let counts = [nr, nt, ns];
            let value_size = [8, 12, 4];
            for c in 0..3 {
                need(tables[c] + counts[c] * 4)?;
                for n in 0..counts[c] {
                    let entry = u32_at(data, tables[c] + n * 4);
                    let (count, first) = ((entry & 0xFFF) as usize, (entry >> 12) as usize);
                    if count == 0 {
                        return Err(corrupt("track without keys"));
                    }
                    need(indices[c] + (first + count) * index_size)?;
                    need(values[c] + (first + count) * value_size[c])?;
                    let frames: Vec<u16> = (first..first + count)
                        .map(|k| {
                            let at = indices[c] + k * index_size;
                            if index_size == 1 {
                                data[at] as u16
                            } else {
                                u16::from_le_bytes([data[at], data[at + 1]])
                            }
                        })
                        .collect();
                    let at = |k: usize| values[c] + k * value_size[c];
                    match c {
                        0 => out.rotations.push(Track {
                            frames,
                            values: (first..first + count)
                                .map(|k| quaternion(data, at(k)))
                                .collect(),
                        }),
                        1 => out.translations.push(Track {
                            frames,
                            values: (first..first + count)
                                .map(|k| vector(data, at(k)))
                                .collect(),
                        }),
                        _ => out.scales.push(Track {
                            frames,
                            values: (first..first + count)
                                .map(|k| f32_at(data, at(k)))
                                .collect(),
                        }),
                    }
                }
            }
        }
        other => return Err(corrupt(&format!("codec {other} not supported"))),
    }
    Ok(out)
}

fn dot4(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn normalize4(q: [f32; 4]) -> [f32; 4] {
    let len = dot4(q, q).sqrt();
    if len > 1e-6 {
        q.map(|x| x / len)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn keyframe_tracks_interpolate() {
        let t = Track {
            frames: vec![0, 10],
            values: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
        };
        assert_eq!(t.sample(5.0), [5.0, 0.0, 0.0]);
        assert_eq!(t.sample(20.0), [10.0, 0.0, 0.0]);
        assert_eq!(Track::constant(2.0f32).sample(3.0), 2.0);
    }

    #[test]
    fn rotations_take_the_short_way() {
        let t = Track {
            frames: vec![0, 2],
            values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -1.0]],
        };
        let q = t.sample(1.0);
        assert!((q[3].abs() - 1.0).abs() < 1e-5);
    }

    /// A byte-keyframe blob with one rotation track (keys at frames 0 and 4).
    #[test]
    fn decodes_byte_keyframes() {
        let mut d = vec![0u8; 0x48];
        d[0] = CODEC_BYTE_KEYFRAMES;
        d[1] = 1;
        put_u32(&mut d, 0x0C, 0x34);
        put_u32(&mut d, 0x10, 0x34);
        put_u32(&mut d, 0x14, 0x34);
        put_u32(&mut d, 0x18, 0x36);
        put_u32(&mut d, 0x1C, 0x36);
        put_u32(&mut d, 0x20, 0x38);
        put_u32(&mut d, 0x24, 0x48);
        put_u32(&mut d, 0x28, 0x48);
        put_u32(&mut d, 0x30, 2); // two keys from key 0
        d[0x34] = 0;
        d[0x35] = 4;
        d[0x3E..0x40].copy_from_slice(&32767i16.to_le_bytes()); // key 0: w = 1
        d[0x40..0x42].copy_from_slice(&32767i16.to_le_bytes()); // key 1: i = 1
        let t = decode_codec(&d, 5).unwrap();
        assert_eq!(t.rotations.len(), 1);
        assert_eq!(t.rotations[0].frames, vec![0, 4]);
        assert_eq!(t.rotations[0].values[1], [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn flags_list_nodes_in_order() {
        assert_eq!(flagged_nodes(&[0b1000_0010, 0x01]), vec![1, 7, 8]);
    }
}
