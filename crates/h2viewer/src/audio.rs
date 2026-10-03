//! Sound output: a small mixer playing Halo 2's decoded sounds on the
//! default audio device, each voice with its own left/right gain. Without
//! an audio device the game runs silent.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

/// Most sounds playing at once; the quietest is dropped beyond this.
const MAX_VOICES: usize = 64;
/// Overall volume, leaving headroom for many sounds at once.
const MASTER: f32 = 0.55;

/// One variation of a sound, decoded.
pub struct Clip {
    /// Interleaved 16-bit samples.
    pub samples: Vec<i16>,
    pub channels: u16,
    pub rate: u32,
}

impl Clip {
    /// Seconds long.
    pub fn duration(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }

    fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    fn at(&self, frame: usize, channel: usize) -> f32 {
        let ch = self.channels.max(1) as usize;
        self.samples[frame * ch + channel.min(ch - 1)] as f32 / 32768.0
    }
}

struct Voice {
    id: u64,
    clip: Arc<Clip>,
    /// Position in source frames.
    position: f64,
    /// Source frames per output frame.
    step: f64,
    gain: [f32; 2],
    looping: bool,
}

#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    rate: u32,
}

impl Mixer {
    /// Mix every voice into `out` (stereo or more channels, interleaved).
    fn mix(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        let frames = out.len() / channels.max(1);
        self.voices.retain_mut(|v| {
            let total = v.clip.frames();
            if total < 2 {
                return false;
            }
            for f in 0..frames {
                let mut i = v.position as usize;
                if i + 1 >= total {
                    if !v.looping {
                        return false;
                    }
                    v.position -= (total - 1) as f64;
                    i = v.position as usize;
                }
                let t = (v.position - i as f64) as f32;
                let left = v.clip.at(i, 0) + (v.clip.at(i + 1, 0) - v.clip.at(i, 0)) * t;
                let right = v.clip.at(i, 1) + (v.clip.at(i + 1, 1) - v.clip.at(i, 1)) * t;
                out[f * channels] += left * v.gain[0];
                if channels > 1 {
                    out[f * channels + 1] += right * v.gain[1];
                }
                v.position += v.step;
            }
            true
        });
        for s in out.iter_mut() {
            *s = (*s * MASTER).clamp(-1.0, 1.0);
        }
    }
}

pub struct Audio {
    mixer: Arc<Mutex<Mixer>>,
    /// Where the mix goes: an audio device, a WAV recording, or nowhere.
    output: Output,
    next_id: u64,
}

enum Output {
    None,
    // Kept alive while the game plays.
    Device { _stream: cpal::Stream },
    Recording,
}

/// Mix in real time into a 44.1 kHz stereo WAV file instead of a device
/// (H2_AUDIO_WAV=path), to check the game's sound without speakers.
fn record(mixer: Arc<Mutex<Mixer>>, path: &str) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};
    const RATE: u32 = 44100;
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    mixer.lock().map_err(|e| e.to_string())?.rate = RATE;
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut written = 0u64;
        let mut out = Vec::new();
        let mut bytes = Vec::new();
        let header = |data: u32| {
            let mut h = Vec::with_capacity(44);
            h.extend_from_slice(b"RIFF");
            h.extend_from_slice(&(36 + data).to_le_bytes());
            h.extend_from_slice(b"WAVEfmt ");
            h.extend_from_slice(&16u32.to_le_bytes());
            h.extend_from_slice(&1u16.to_le_bytes());
            h.extend_from_slice(&2u16.to_le_bytes());
            h.extend_from_slice(&RATE.to_le_bytes());
            h.extend_from_slice(&(RATE * 4).to_le_bytes());
            h.extend_from_slice(&4u16.to_le_bytes());
            h.extend_from_slice(&16u16.to_le_bytes());
            h.extend_from_slice(b"data");
            h.extend_from_slice(&data.to_le_bytes());
            h
        };
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let due = (start.elapsed().as_secs_f64() * RATE as f64) as u64;
            let frames = (due - written) as usize;
            out.resize(frames * 2, 0.0);
            if let Ok(mut m) = mixer.lock() {
                m.mix(&mut out, 2);
            }
            written = due;
            bytes.clear();
            for s in &out {
                bytes.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
            }
            let data = (written * 4).min(u32::MAX as u64 - 36) as u32;
            let ok = file.seek(SeekFrom::Start(0)).is_ok()
                && file.write_all(&header(data)).is_ok()
                && file.seek(SeekFrom::End(0)).is_ok()
                && file.write_all(&bytes).is_ok();
            if !ok {
                return;
            }
        }
    });
    Ok(())
}

fn open(mixer: Arc<Mutex<Mixer>>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let channels = config.channels as usize;
    mixer.lock().map_err(|e| e.to_string())?.rate = config.sample_rate.0;
    let error = |e| println!("audio: {e}");
    let mut scratch = Vec::new();
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |out: &mut [f32], _| {
                if let Ok(mut m) = mixer.lock() {
                    m.mix(out, channels);
                }
            },
            error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |out: &mut [i16], _| {
                scratch.resize(out.len(), 0.0);
                if let Ok(mut m) = mixer.lock() {
                    m.mix(&mut scratch, channels);
                }
                for (o, s) in out.iter_mut().zip(&scratch) {
                    *o = (s * 32767.0) as i16;
                }
            },
            error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config,
            move |out: &mut [u16], _| {
                scratch.resize(out.len(), 0.0);
                if let Ok(mut m) = mixer.lock() {
                    m.mix(&mut scratch, channels);
                }
                for (o, s) in out.iter_mut().zip(&scratch) {
                    *o = ((s * 32767.0) as i32 + 32768) as u16;
                }
            },
            error,
            None,
        ),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

impl Audio {
    pub fn new() -> Audio {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        // H2_MUTE=1 starts silent (tests).
        let output = if std::env::var("H2_MUTE").is_ok() {
            Output::None
        } else if let Ok(path) = std::env::var("H2_AUDIO_WAV") {
            match record(mixer.clone(), &path) {
                Ok(()) => Output::Recording,
                Err(e) => {
                    println!("audio: can't record to {path} ({e})");
                    Output::None
                }
            }
        } else {
            match open(mixer.clone()) {
                Ok(s) => Output::Device { _stream: s },
                Err(e) => {
                    println!("audio: no sound ({e})");
                    Output::None
                }
            }
        };
        Audio {
            mixer,
            output,
            next_id: 1,
        }
    }

    /// Start a clip with left/right gains; returns its voice id.
    pub fn play(&mut self, clip: &Arc<Clip>, gain: [f32; 2], pitch: f32, looping: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if matches!(self.output, Output::None) || gain[0].max(gain[1]) < 0.002 {
            return id;
        }
        let Ok(mut m) = self.mixer.lock() else {
            return id;
        };
        let rate = m.rate.max(1) as f64;
        if m.voices.len() >= MAX_VOICES {
            // Make room by dropping the quietest.
            if let Some(k) = (0..m.voices.len())
                .min_by(|&a, &b| loudness(&m.voices[a]).total_cmp(&loudness(&m.voices[b])))
            {
                m.voices.swap_remove(k);
            }
        }
        m.voices.push(Voice {
            id,
            clip: clip.clone(),
            position: 0.0,
            step: clip.rate as f64 / rate * pitch as f64,
            gain,
            looping,
        });
        id
    }

    pub fn stop(&mut self, id: u64) {
        if let Ok(mut m) = self.mixer.lock() {
            m.voices.retain(|v| v.id != id);
        }
    }
}

fn loudness(v: &Voice) -> f32 {
    v.gain[0].max(v.gain[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_mix_resample_and_finish() {
        let mut m = Mixer {
            voices: Vec::new(),
            rate: 44100,
        };
        let clip = Arc::new(Clip {
            samples: vec![16384; 100],
            channels: 1,
            rate: 22050,
        });
        m.voices.push(Voice {
            id: 1,
            clip,
            position: 0.0,
            step: 0.5,
            gain: [1.0, 0.5],
            looping: false,
        });
        let mut out = vec![0.0; 2 * 150];
        m.mix(&mut out, 2);
        assert!((out[0] - 0.5 * MASTER).abs() < 1e-4);
        assert!((out[1] - 0.25 * MASTER).abs() < 1e-4);
        // 100 source frames at half speed last about 200 output frames.
        assert_eq!(m.voices.len(), 1);
        m.mix(&mut out, 2);
        assert!(m.voices.is_empty());
    }
}
