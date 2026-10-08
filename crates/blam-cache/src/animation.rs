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
//!
//! After the flags comes the animation's movement ("frame info"): how far
//! each frame carries the body, which is how fast its legs were made to go.

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};
use std::collections::HashSet;

const JMAD_PARENT: usize = 0x0;
const JMAD_NODES: usize = 0xC;
const NODE_SIZE: usize = 0x20;
const JMAD_ANIMATIONS: usize = 0x2C;
const ANIMATION_SIZE: usize = 0x60;
const JMAD_SOUNDS: usize = 0x14;
const SOUND_REF_SIZE: usize = 0xC;
const ANIMATION_FRAME_EVENTS: usize = 0x40;
const ANIMATION_SOUND_EVENTS: usize = 0x48;

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
    /// Sounds to start at frames: (frame, index into the graph's `sounds`).
    pub sound_events: Vec<(u16, usize)>,
    /// Footfalls and other moments: (frame, event).
    pub frame_events: Vec<(u16, FrameEvent)>,
    /// How far each frame carries the body: forward and left (world units)
    /// and turning left (radians). Empty when it doesn't move the body.
    pub movement: Vec<[f32; 3]>,
}

/// Moments marked in an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEvent {
    LeftFoot,
    RightFoot,
    BothFeet,
    BodyImpact,
    Other(u16),
}

impl From<u16> for FrameEvent {
    fn from(v: u16) -> Self {
        match v {
            2 => FrameEvent::LeftFoot,
            3 => FrameEvent::RightFoot,
            9 => FrameEvent::BothFeet,
            10 => FrameEvent::BodyImpact,
            n => FrameEvent::Other(n),
        }
    }
}

impl Animation {
    /// Length in seconds.
    pub fn duration(&self) -> f32 {
        self.frame_count.saturating_sub(1) as f32 / FRAME_RATE
    }

    /// How fast the animation carries the body when played at its own pace,
    /// in world units a second: forward and left (`move_front` runs at
    /// about 2.26). Zero when it doesn't move it.
    pub fn speed(&self) -> [f32; 2] {
        if self.movement.is_empty() {
            return [0.0; 2];
        }
        let per_second = FRAME_RATE / self.movement.len() as f32;
        let (x, y) = self
            .movement
            .iter()
            .fold((0.0, 0.0), |(x, y), m| (x + m[0], y + m[1]));
        [x * per_second, y * per_second]
    }

    /// The first frame marked with `event` (a footfall), if any.
    pub fn event_frame(&self, event: FrameEvent) -> Option<u16> {
        self.frame_events
            .iter()
            .filter(|(_, e)| *e == event)
            .map(|(f, _)| *f)
            .min()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimationGraph {
    /// Graph this one inherits animations from.
    pub parent: Option<DatumIndex>,
    pub nodes: Vec<GraphNode>,
    pub animations: Vec<Animation>,
    /// Sounds the animations play (`snd!` tags), by sound event index.
    pub sounds: Vec<Option<DatumIndex>>,
}

impl AnimationGraph {
    pub fn find(&self, name: &str) -> Option<&Animation> {
        self.animations.iter().find(|a| a.name == name)
    }

    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Take in the animations of the graph this one inherits from that it
    /// doesn't have itself, their tracks moved onto this graph's nodes by
    /// name. (The multiplayer Elite's own graph has only a few of its
    /// animations; it runs and jumps with the ones it inherits.)
    pub fn inherit(&mut self, parent: &AnimationGraph) {
        let to_parent: Vec<Option<usize>> =
            self.nodes.iter().map(|n| parent.node(&n.name)).collect();
        let own: HashSet<String> = self.animations.iter().map(|a| a.name.clone()).collect();
        // Their sound events count from the parent's sounds, put after ours.
        let sounds = self.sounds.len();
        self.sounds.extend_from_slice(&parent.sounds);
        for a in &parent.animations {
            if own.contains(&a.name) {
                continue;
            }
            self.animations.push(Animation {
                name: a.name.clone(),
                kind: a.kind,
                frame_count: a.frame_count,
                decoded: a.decoded,
                rotations: on_nodes(&a.rotations, &to_parent),
                translations: on_nodes(&a.translations, &to_parent),
                scales: on_nodes(&a.scales, &to_parent),
                sound_events: a
                    .sound_events
                    .iter()
                    .map(|&(frame, s)| (frame, s + sounds))
                    .collect(),
                frame_events: a.frame_events.clone(),
                movement: a.movement.clone(),
            });
        }
    }
}

/// Per node tracks moved to other nodes: `from[n]` is where node `n`'s
/// come from.
fn on_nodes<T: Clone>(tracks: &[Option<T>], from: &[Option<usize>]) -> Vec<Option<T>> {
    from.iter()
        .map(|k| k.and_then(|k| tracks.get(k).cloned().flatten()))
        .collect()
}

/// The deepest chain of graphs inheriting from one another that's followed.
const MAX_INHERITANCE: usize = 8;

/// A graph with everything it inherits (see [`AnimationGraph::inherit`]):
/// its own animations first, then its parent's, its grandparent's and so on
/// (as far as they can be read).
pub fn read_inherited_animation_graph(
    set: &mut MapSet,
    jmad: DatumIndex,
) -> Result<AnimationGraph> {
    let mut graph = read_animation_graph(set, jmad)?;
    let mut seen = vec![jmad];
    let mut next = graph.parent;
    while let Some(p) = next.filter(|p| !seen.contains(p) && seen.len() <= MAX_INHERITANCE) {
        seen.push(p);
        let Ok(parent) = read_animation_graph(set, p) else {
            break;
        };
        graph.inherit(&parent);
        next = parent.parent;
    }
    Ok(graph)
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
        let sound_events = file
            .read_block(region, a, ANIMATION_SOUND_EVENTS, 8)?
            .as_chunks::<8>()
            .0
            .iter()
            .filter_map(|e| {
                let sound = usize::try_from(i16_at(e, 0)).ok()?;
                Some((i16_at(e, 2).max(0) as u16, sound))
            })
            .collect();
        let frame_events = file
            .read_block(region, a, ANIMATION_FRAME_EVENTS, 4)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|e| {
                let kind = u16::from_le_bytes([e[0], e[1]]);
                (i16_at(e, 2).max(0) as u16, FrameEvent::from(kind))
            })
            .collect();
        let mut anim = Animation {
            name,
            kind: AnimationKind::from(a[0x10]),
            frame_count: i16_at(a, 0x14).max(0) as u16,
            decoded: false,
            rotations: vec![None; nodes.len()],
            translations: vec![None; nodes.len()],
            scales: vec![None; nodes.len()],
            sound_events,
            frame_events,
            movement: read_movement(&blob, &sizes),
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
    let sounds = file
        .read_block(region, &data, JMAD_SOUNDS, SOUND_REF_SIZE)?
        .as_chunks::<SOUND_REF_SIZE>()
        .0
        .iter()
        .map(|r| Some(DatumIndex(u32_at(r, 4))).filter(|d| *d != DatumIndex::NONE))
        .collect();
    Ok(AnimationGraph {
        parent: (parent != DatumIndex::NONE).then_some(parent),
        nodes,
        animations,
        sounds,
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
    /// The frame info: what each frame's movement holds (0 nothing, 1
    /// dx dy, 2 dx dy dyaw, 3 dx dy dz dyaw), and its size in bytes.
    frame_info: u8,
    movement: usize,
}

impl DataSizes {
    fn read(a: &[u8]) -> DataSizes {
        DataSizes {
            static_flags: a[0x30] as usize,
            animated_flags: a[0x31] as usize,
            default: u16::from_le_bytes([a[0x36], a[0x37]]) as usize,
            uncompressed: u32_at(a, 0x38) as usize,
            compressed: u32_at(a, 0x3C) as usize,
            frame_info: a[0x11],
            movement: u16::from_le_bytes([a[0x32], a[0x33]]) as usize,
        }
    }
}

/// Each frame's movement (forward, left, turn), which follows the node flags.
fn read_movement(blob: &[u8], sizes: &DataSizes) -> Vec<[f32; 3]> {
    let (stride, turn) = match sizes.frame_info {
        1 => (8, None),
        2 => (12, Some(8)),
        3 => (16, Some(12)),
        _ => return Vec::new(),
    };
    let start = sizes.default
        + sizes.uncompressed
        + sizes.compressed
        + sizes.static_flags
        + sizes.animated_flags;
    let Some(data) = blob.get(start..start + sizes.movement) else {
        return Vec::new();
    };
    data.chunks_exact(stride)
        .map(|f| {
            [
                f32_at(f, 0),
                f32_at(f, 4),
                turn.map_or(0.0, |o| f32_at(f, o)),
            ]
        })
        .collect()
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

    fn animation(name: &str, nodes: usize) -> Animation {
        Animation {
            name: name.into(),
            kind: AnimationKind::Base,
            frame_count: 2,
            decoded: true,
            rotations: vec![None; nodes],
            translations: vec![None; nodes],
            scales: vec![None; nodes],
            sound_events: Vec::new(),
            frame_events: Vec::new(),
            movement: Vec::new(),
        }
    }

    fn graph(nodes: &[&str], animations: Vec<Animation>) -> AnimationGraph {
        AnimationGraph {
            parent: None,
            nodes: nodes
                .iter()
                .map(|&name| GraphNode {
                    name: name.into(),
                    parent: -1,
                })
                .collect(),
            animations,
            sounds: vec![None],
        }
    }

    #[test]
    fn inherited_animations_move_onto_the_graphs_own_nodes() {
        let mine = animation("combat:sword:melee", 2);
        let mut child = graph(&["pelvis", "head"], vec![mine.clone()]);
        // The parent lists the nodes in another order and has one more.
        let mut idle = animation("combat:rifle:idle", 3);
        idle.rotations[0] = Some(Track::constant([1.0, 0.0, 0.0, 0.0]));
        idle.translations[1] = Some(Track::constant([0.0, 0.0, 0.4]));
        idle.translations[2] = Some(Track::constant([9.0, 9.0, 9.0]));
        idle.sound_events = vec![(3, 0)];
        let mut theirs = animation("combat:sword:melee", 3);
        theirs.frame_count = 99;
        let parent = graph(&["head", "pelvis", "tail"], vec![idle, theirs]);
        child.inherit(&parent);
        assert_eq!(child.animations.len(), 2);
        assert_eq!(child.animations[0], mine, "the graph's own one wins");
        let idle = child.find("combat:rifle:idle").unwrap();
        assert_eq!(idle.rotations[0], None);
        assert_eq!(
            idle.rotations[1],
            Some(Track::constant([1.0, 0.0, 0.0, 0.0]))
        );
        assert_eq!(idle.translations[0], Some(Track::constant([0.0, 0.0, 0.4])));
        assert_eq!(idle.translations.len(), 2, "nothing for the parent's tail");
        assert_eq!(idle.sound_events, vec![(3, 1)], "the parent's sound");
    }

    /// Two frames of a run, each 0.075 forward: 2.25 a second.
    #[test]
    fn reads_how_far_frames_carry_the_body() {
        let sizes = DataSizes {
            static_flags: 3,
            animated_flags: 3,
            default: 4,
            uncompressed: 0,
            compressed: 2,
            frame_info: 2,
            movement: 24,
        };
        let mut blob = vec![0u8; 12];
        for _ in 0..2 {
            blob.extend(0.075f32.to_le_bytes());
            blob.extend(0.0f32.to_le_bytes());
            blob.extend(0.1f32.to_le_bytes());
        }
        let mut a = animation("combat:rifle:move_front", 1);
        a.movement = read_movement(&blob, &sizes);
        assert_eq!(a.movement, vec![[0.075, 0.0, 0.1]; 2]);
        let [x, y] = a.speed();
        assert!((x - 2.25).abs() < 1e-5 && y == 0.0, "{x} {y}");
        // Movement data that runs past the blob is left out.
        assert!(read_movement(&blob[..30], &sizes).is_empty());
    }
}
