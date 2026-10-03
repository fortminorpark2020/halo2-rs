//! Sounds. A `snd!` tag is a small header pointing into the map's sound
//! gestalt (`ugh!`), which holds every sound's pitch ranges, permutations
//! (variations picked at random) and the chunks of raw sample data. Most
//! Halo 2 PC effects are Xbox ADPCM; dialogue (the announcer) is WMA 2.

use crate::mapset::{MapSet, Source};
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, GroupTag, Result};

const GESTALT_PLAYBACK: usize = 0x0;
const PLAYBACK_SIZE: usize = 0x38;
const GESTALT_PITCH_RANGES: usize = 0x20;
const PITCH_RANGE_SIZE: usize = 0xC;
const GESTALT_PERMUTATIONS: usize = 0x28;
const PERMUTATION_SIZE: usize = 0x10;
const GESTALT_CHUNKS: usize = 0x40;
const CHUNK_SIZE: usize = 0xC;
/// Xbox ADPCM block: 4-byte header and 32 bytes of nibbles per channel.
const ADPCM_BLOCK: usize = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    BigEndianPcm,
    XboxAdpcm,
    ImaAdpcm,
    LittleEndianPcm,
    Wma,
    Unknown(u8),
}

impl Codec {
    fn from_byte(b: u8) -> Codec {
        match b {
            0 => Codec::BigEndianPcm,
            1 => Codec::XboxAdpcm,
            2 => Codec::ImaAdpcm,
            3 => Codec::LittleEndianPcm,
            4 => Codec::Wma,
            n => Codec::Unknown(n),
        }
    }

    pub fn decodable(self) -> bool {
        !matches!(self, Codec::Unknown(_))
    }
}

/// A decoded sound.
#[derive(Debug, Clone)]
pub struct Sound {
    pub name: String,
    /// Halo's sound class (weapon_fire = 4, unit_footsteps = 0x12, ...).
    pub class: u8,
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: Codec,
    /// Full volume within the first distance, silent beyond the second
    /// (world units). Zero when the tag leaves it to the class default.
    pub distance: (f32, f32),
    /// Gain in decibels added to every play.
    pub gain_db: f32,
    /// Variations, one picked at random each play: interleaved 16-bit samples.
    pub permutations: Vec<Vec<i16>>,
    /// The raw data of variations that couldn't be decoded.
    pub encoded: Vec<Vec<u8>>,
}

impl Sound {
    pub fn duration(&self) -> f32 {
        let longest = self.permutations.iter().map(Vec::len).max().unwrap_or(0);
        longest as f32 / self.channels.max(1) as f32 / self.sample_rate.max(1) as f32
    }
}

/// The sound gestalt of one map file.
struct Gestalt {
    playback: Vec<u8>,
    pitch_ranges: Vec<u8>,
    permutations: Vec<u8>,
    chunks: Vec<u8>,
}

/// Reads sounds, keeping each file's gestalt once it is loaded.
#[derive(Default)]
pub struct SoundReader {
    gestalts: Vec<(Source, Option<Gestalt>)>,
}

fn load_gestalt(set: &mut MapSet, src: Source) -> Result<Option<Gestalt>> {
    let ugh = GroupTag::parse("ugh!").expect("valid group");
    let file = set.get(src);
    let Some(tag) = file
        .tags
        .iter()
        .find(|t| t.group == ugh && t.has_data())
        .cloned()
    else {
        return Ok(None);
    };
    let d = file.read_tag_data(&tag)?;
    let region = file.meta_region();
    Ok(Some(Gestalt {
        playback: file.read_block(region, &d, GESTALT_PLAYBACK, PLAYBACK_SIZE)?,
        pitch_ranges: file.read_block(region, &d, GESTALT_PITCH_RANGES, PITCH_RANGE_SIZE)?,
        permutations: file.read_block(region, &d, GESTALT_PERMUTATIONS, PERMUTATION_SIZE)?,
        chunks: file.read_block(region, &d, GESTALT_CHUNKS, CHUNK_SIZE)?,
    }))
}

fn element(block: &[u8], size: usize, i: i32) -> Option<&[u8]> {
    let i = usize::try_from(i).ok()?;
    block.get(i * size..(i + 1) * size)
}

impl SoundReader {
    pub fn new() -> SoundReader {
        SoundReader::default()
    }

    fn gestalt(&mut self, set: &mut MapSet, src: Source) -> Result<Option<&Gestalt>> {
        if !self.gestalts.iter().any(|g| g.0 == src) {
            let g = load_gestalt(set, src)?;
            self.gestalts.push((src, g));
        }
        Ok(self
            .gestalts
            .iter()
            .find(|g| g.0 == src)
            .and_then(|g| g.1.as_ref()))
    }

    /// Read and decode a `snd!` tag. Variations that can't be decoded come
    /// back in `encoded` instead of `permutations`.
    pub fn read(&mut self, set: &mut MapSet, snd: DatumIndex) -> Result<Sound> {
        let (src, tag, d) = set.tag_data(snd)?;
        if d.len() < 0x14 {
            return Err(Error::Corrupt(format!("sound {} too short", tag.name)));
        }
        let class = d[2];
        let mut sample_rate = match d[3] {
            1 => 44100,
            2 => 32000,
            _ => 22050,
        };
        let mut channels = if d[4] == 1 { 2 } else { 1 };
        let codec = Codec::from_byte(d[5]);
        let playback_index = i16_at(&d, 0x6) as i32;
        let first_pitch_range = i16_at(&d, 0x8) as i32;
        let pitch_range_count = d[0xA] as i32;

        // The chunk list must be copied out before reading sample data.
        let (distance, gain_db, chunk_lists) = {
            let Some(g) = self.gestalt(set, src)? else {
                return Err(Error::Corrupt(format!("no sound gestalt for {}", tag.name)));
            };
            let (distance, gain_db) = element(&g.playback, PLAYBACK_SIZE, playback_index)
                .map_or(((0.0, 0.0), 0.0), |p| {
                    ((f32_at(p, 0x0), f32_at(p, 0x4)), f32_at(p, 0x10))
                });
            let mut lists = Vec::new();
            // Only the first pitch range: the sound at its natural pitch.
            for r in first_pitch_range..first_pitch_range + pitch_range_count.min(1) {
                let Some(range) = element(&g.pitch_ranges, PITCH_RANGE_SIZE, r) else {
                    continue;
                };
                let first = i16_at(range, 0x8) as i32;
                let count = i16_at(range, 0xA) as i32;
                for p in first..first + count.max(0) {
                    let Some(perm) = element(&g.permutations, PERMUTATION_SIZE, p) else {
                        continue;
                    };
                    let first_chunk = i16_at(perm, 0xC) as u16 as i32;
                    let chunk_count = i16_at(perm, 0xE) as i32;
                    let chunks: Vec<(u32, usize)> = (first_chunk..first_chunk + chunk_count.max(0))
                        .filter_map(|c| element(&g.chunks, CHUNK_SIZE, c))
                        .map(|c| (u32_at(c, 0), (u32_at(c, 4) & 0x00FF_FFFF) as usize))
                        .collect();
                    lists.push(chunks);
                }
            }
            (distance, gain_db, lists)
        };

        let mut permutations = Vec::new();
        let mut encoded = Vec::new();
        for chunks in chunk_lists {
            let mut data = Vec::new();
            for (pointer, size) in chunks {
                data.extend(set.read_resource(src, pointer, size)?);
            }
            // Music marked as ADPCM can still be WMA: the data says which.
            if codec == Codec::Wma || data.starts_with(&MEDIATYPE_AUDIO) {
                // The stream says how many channels it has (the tag may not).
                match decode_wma(&data) {
                    Ok((samples, ch, rate)) => {
                        permutations.push(samples);
                        channels = ch;
                        sample_rate = rate;
                    }
                    Err(_) => encoded.push(data),
                }
            } else if codec.decodable() {
                permutations.push(decode(codec, &data, channels as usize));
            } else {
                encoded.push(data);
            }
        }
        Ok(Sound {
            name: tag.name,
            class,
            sample_rate,
            channels,
            codec,
            distance,
            gain_db,
            permutations,
            encoded,
        })
    }
}

const EFFECT_EVENTS: usize = 0x14;
const EVENT_SIZE: usize = 0x38;
const EVENT_PARTS: usize = 0x18;
const PART_SIZE: usize = 0x38;
const PART_TYPE: usize = 0xC;

/// The sounds an effect plays: the tag itself when it is a sound, or the
/// sound parts of an `effe`'s events.
pub fn effect_sounds(set: &mut MapSet, effect: DatumIndex) -> Result<Vec<DatumIndex>> {
    let snd = GroupTag::parse("snd!").expect("valid group");
    let effe = GroupTag::parse("effe").expect("valid group");
    let Some((src, tag)) = set.locate(effect) else {
        return Ok(Vec::new());
    };
    if tag.group == snd {
        return Ok(vec![effect]);
    }
    if tag.group != effe {
        return Ok(Vec::new());
    }
    let file = set.get(src);
    let d = file.read_tag_data(&tag)?;
    let region = file.meta_region();
    let events = file.read_block(region, &d, EFFECT_EVENTS, EVENT_SIZE)?;
    let mut parts = Vec::new();
    for e in events.as_chunks::<EVENT_SIZE>().0 {
        parts.extend(file.read_block(region, e, EVENT_PARTS, PART_SIZE)?);
    }
    let mut out = Vec::new();
    for p in parts.as_chunks::<PART_SIZE>().0 {
        let datum = DatumIndex(u32_at(p, PART_TYPE + 4));
        if set.locate(datum).is_some_and(|(_, t)| t.group == snd) && !out.contains(&datum) {
            out.push(datum);
        }
    }
    Ok(out)
}

/// DirectShow's MEDIATYPE_Audio and FORMAT_WaveFormatEx.
const MEDIATYPE_AUDIO: [u8; 16] = *b"auds\x00\x00\x10\x00\x80\x00\x00\xaa\x00\x38\x9b\x71";
const FORMAT_WAVE_FORMAT_EX: [u8; 16] = [
    0x81, 0x9f, 0x58, 0x05, 0x56, 0xc3, 0xce, 0x11, 0xbf, 0x01, 0x00, 0xaa, 0x00, 0x55, 0x59, 0x5a,
];

/// Decode a WMA sound: a DirectShow media type (AM_MEDIA_TYPE, stored
/// without its pointers) holding a WAVEFORMATEX, then the packets. Returns
/// interleaved samples, channels and sample rate.
pub fn decode_wma(data: &[u8]) -> Result<(Vec<i16>, u16, u32)> {
    let bad = |what: &str| Error::Corrupt(format!("WMA sound: {what}"));
    if data.len() < 0x40
        || data[..16] != MEDIATYPE_AUDIO
        || data[0x2C..0x3C] != FORMAT_WAVE_FORMAT_EX
    {
        return Err(bad("unknown media type"));
    }
    let format_len = u32_at(data, 0x3C) as usize;
    let format = data
        .get(0x40..0x40 + format_len)
        .ok_or_else(|| bad("truncated format"))?;
    let format = wma::Format::parse(format).map_err(|e| bad(&e.to_string()))?;
    let samples =
        wma::decode(&format, &data[0x40 + format_len..]).map_err(|e| bad(&e.to_string()))?;
    let samples = samples
        .iter()
        .map(|&s| (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
        .collect();
    Ok((samples, format.channels, format.sample_rate))
}

/// Decode raw sample data to interleaved 16-bit samples.
pub fn decode(codec: Codec, data: &[u8], channels: usize) -> Vec<i16> {
    match codec {
        Codec::XboxAdpcm | Codec::ImaAdpcm => decode_xbox_adpcm(data, channels),
        Codec::LittleEndianPcm => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect(),
        Codec::BigEndianPcm => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_be_bytes(*b))
            .collect(),
        Codec::Wma => decode_wma(data).map(|w| w.0).unwrap_or_default(),
        Codec::Unknown(_) => Vec::new(),
    }
}

const STEP_SIZES: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_STEPS: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

struct AdpcmChannel {
    predictor: i32,
    index: i32,
}

impl AdpcmChannel {
    fn next(&mut self, nibble: u8) -> i16 {
        let step = STEP_SIZES[self.index as usize];
        let mut diff = step >> 3;
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 8 != 0 {
            self.predictor -= diff;
        } else {
            self.predictor += diff;
        }
        self.predictor = self.predictor.clamp(-32768, 32767);
        self.index = (self.index + INDEX_STEPS[(nibble & 7) as usize]).clamp(0, 88);
        self.predictor as i16
    }
}

/// Xbox ADPCM (IMA ADPCM in 36-byte blocks per channel): each block starts
/// with a sample and step index per channel, then 4-byte runs of nibbles
/// taking turns between channels. 65 samples per block.
pub fn decode_xbox_adpcm(data: &[u8], channels: usize) -> Vec<i16> {
    let channels = channels.clamp(1, 2);
    let block = ADPCM_BLOCK * channels;
    let mut out = Vec::with_capacity(data.len() / block * 65 * channels);
    let mut frame = [[0i16; 2]; 64];
    for b in data.chunks_exact(block) {
        let mut state: Vec<AdpcmChannel> = (0..channels)
            .map(|c| AdpcmChannel {
                predictor: i16::from_le_bytes([b[4 * c], b[4 * c + 1]]) as i32,
                index: (b[4 * c + 2] as i32).clamp(0, 88),
            })
            .collect();
        for s in &state {
            out.push(s.predictor as i16);
        }
        let body = &b[4 * channels..];
        for group in 0..8 {
            for (c, s) in state.iter_mut().enumerate() {
                let word = &body[(group * channels + c) * 4..][..4];
                for k in 0..8 {
                    let byte = word[k / 2];
                    let nibble = if k % 2 == 0 { byte & 0xF } else { byte >> 4 };
                    frame[group * 8 + k][c] = s.next(nibble);
                }
            }
        }
        for f in &frame {
            out.extend_from_slice(&f[..channels]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adpcm_blocks_decode_to_65_samples_per_channel() {
        // A mono block: start at 1000, step index 20, all nibbles 0x7 (up).
        let mut block = vec![0u8; 36];
        block[0..2].copy_from_slice(&1000i16.to_le_bytes());
        block[2] = 20;
        for b in &mut block[4..] {
            *b = 0x77;
        }
        let s = decode_xbox_adpcm(&block, 1);
        assert_eq!(s.len(), 65);
        assert_eq!(s[0], 1000);
        assert!(s.windows(2).all(|w| w[1] >= w[0]));
        // Stereo: channels decode independently.
        let mut stereo = vec![0u8; 72];
        stereo[0..2].copy_from_slice(&500i16.to_le_bytes());
        stereo[4..6].copy_from_slice(&(-500i16).to_le_bytes());
        let s = decode_xbox_adpcm(&stereo, 2);
        assert_eq!(s.len(), 130);
        assert_eq!((s[0], s[1]), (500, -500));
    }
}
