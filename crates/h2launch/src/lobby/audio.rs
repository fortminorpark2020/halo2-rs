//! The lobby's sound output: a small mixer for Halo 2's menu sounds and
//! music, copied from the from-scratch game's (`h2viewer/src/audio.rs`)
//! without its game voices and their placing. It plays on the default
//! audio device through cpal (`Output::Device`), records into a WAV file
//! instead (`Output::Wav`, which works without a device or a window), or
//! goes nowhere (`Output::Null`).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Most sounds playing at once; beyond this the oldest menu sound is
/// dropped, never the music. The menus play a few at most (the music and
/// a cue or two), more only with a key held down (each repeat a press).
const MAX_VOICES: usize = 16;
/// Overall volume. The tags' own gains leave the headroom (the menu
/// music at -10 dB peaks at about a third of full scale, and a menu sound
/// at -7 dB at a twentieth), so nothing is taken off them. (Until the
/// owner's PC check after 2.2 this was the from-scratch game's 0.55, for
/// a battle's many sounds, which made the menus 5 dB quieter than the
/// tags ask.)
const MASTER: f32 = 1.0;
/// The sample rate a WAV recording is mixed at.
const WAV_RATE: u32 = 44100;
/// How often a WAV recording mixes what is due.
const WAV_STEP: Duration = Duration::from_millis(20);

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
    gain: f32,
    /// A fade out: how much gain it loses each output frame (0: none).
    /// The voice ends when its gain reaches 0.
    fade: f32,
    looping: bool,
    /// Clips to go on with when this one ends, one picked at random each
    /// time (music).
    next: Vec<Arc<Clip>>,
    /// The music (`Audio::play_music`): never dropped to make room.
    music: bool,
}

#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    rate: u32,
    rng: u32,
    /// Paused: silence, and every voice holds where it is.
    paused: bool,
}

impl Mixer {
    /// Mix every voice into `out` (stereo or more channels, interleaved).
    fn mix(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        if self.paused {
            return;
        }
        let channels = channels.max(1);
        let frames = out.len() / channels;
        let rng = &mut self.rng;
        self.voices.retain_mut(|v| {
            for f in 0..frames {
                let mut total = v.clip.frames();
                if total < 2 {
                    return false;
                }
                let mut i = v.position as usize;
                if i + 1 >= total {
                    v.position -= (total - 1) as f64;
                    if !v.next.is_empty() {
                        *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
                        let k = (*rng >> 16) as usize % v.next.len();
                        v.clip = v.next[k].clone();
                        total = v.clip.frames();
                    } else if !v.looping {
                        return false;
                    }
                    i = v.position as usize;
                    if i + 1 >= total {
                        return false;
                    }
                }
                if v.fade > 0.0 {
                    v.gain -= v.fade;
                    // Silent (below -100 dB, whatever the steps' rounding
                    // left): the fade is over.
                    if v.gain <= 1e-5 {
                        return false;
                    }
                }
                let t = (v.position - i as f64) as f32;
                let left = v.clip.at(i, 0) + (v.clip.at(i + 1, 0) - v.clip.at(i, 0)) * t;
                let right = v.clip.at(i, 1) + (v.clip.at(i + 1, 1) - v.clip.at(i, 1)) * t;
                out[f * channels] += left * v.gain;
                if channels > 1 {
                    out[f * channels + 1] += right * v.gain;
                }
                v.position += v.step;
            }
            true
        });
        for s in out.iter_mut() {
            *s = (*s * MASTER).clamp(-1.0, 1.0);
        }
    }

    /// Start a voice (`music`: the music). With `MAX_VOICES` playing, the
    /// oldest menu sound makes room (ids only grow), so the music plays on
    /// however many presses sound at once (a held key's repeats); only if
    /// every voice were music would the quietest go.
    fn start(&mut self, id: u64, clip: &Arc<Clip>, gain: f32, next: Vec<Arc<Clip>>, music: bool) {
        if self.voices.len() >= MAX_VOICES {
            let k = (0..self.voices.len())
                .filter(|&k| !self.voices[k].music)
                .min_by_key(|&k| self.voices[k].id)
                .or_else(|| {
                    (0..self.voices.len())
                        .min_by(|&a, &b| self.voices[a].gain.total_cmp(&self.voices[b].gain))
                });
            if let Some(k) = k {
                self.voices.swap_remove(k);
            }
        }
        self.voices.push(Voice {
            id,
            clip: clip.clone(),
            position: 0.0,
            step: clip.rate as f64 / self.rate.max(1) as f64,
            gain,
            fade: 0.0,
            looping: false,
            next,
            music,
        });
    }

    /// Fade voice `id` out over `seconds`, then end it (a voice silent
    /// already ends at the next mix). False if it has ended already.
    fn fade_out(&mut self, id: u64, seconds: f32) -> bool {
        let rate = self.rate.max(1) as f32;
        let Some(v) = self.voices.iter_mut().find(|v| v.id == id) else {
            return false;
        };
        v.fade = (v.gain / (seconds.max(0.01) * rate)).max(f32::MIN_POSITIVE);
        true
    }
}

/// Where the mix goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// The default audio device.
    Device,
    /// Nowhere: nothing is kept or mixed.
    Null,
    /// A 44.1 kHz stereo WAV file, mixed in real time, for checking the
    /// sound without speakers (headless too).
    Wav(PathBuf),
}

enum Sink {
    Null,
    Device(cpal::Stream),
    Wav {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    },
}

pub struct Audio {
    mixer: Arc<Mutex<Mixer>>,
    sink: Sink,
    next_id: u64,
    paused: bool,
    /// The audio device's name, when it plays to one that gave it.
    device_name: Option<String>,
}

impl Audio {
    /// Output that goes nowhere.
    pub fn null() -> Audio {
        Audio {
            mixer: Arc::new(Mutex::new(Mixer::default())),
            sink: Sink::Null,
            next_id: 1,
            paused: false,
            device_name: None,
        }
    }

    /// Open `output`, or say why it couldn't be (no device, a file that
    /// can't be written).
    pub fn open(output: &Output) -> Result<Audio, String> {
        let mut a = Audio::null();
        a.sink = match output {
            Output::Null => Sink::Null,
            Output::Device => {
                let (stream, name) = device(a.mixer.clone())?;
                a.device_name = name;
                Sink::Device(stream)
            }
            Output::Wav(path) => {
                let stop = Arc::new(AtomicBool::new(false));
                let thread = record(a.mixer.clone(), path, stop.clone())?;
                Sink::Wav {
                    stop,
                    thread: Some(thread),
                }
            }
        };
        Ok(a)
    }

    /// The audio device's name (cpal's), when it plays to one that gave
    /// it.
    pub fn device_name(&self) -> Option<&str> {
        self.device_name.as_deref()
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id - 1
    }

    /// Play a menu sound once at `gain`; returns its voice id.
    pub fn play_ui(&mut self, clip: &Arc<Clip>, gain: f32) -> u64 {
        self.play(clip, &[], gain, false)
    }

    /// Music: `first`, then one of `then` after another for as long as it
    /// plays (all at the same sample rate); returns its voice id.
    pub fn play_music(&mut self, first: &Arc<Clip>, then: &[Arc<Clip>], gain: f32) -> u64 {
        self.play(first, then, gain, true)
    }

    /// A movie's soundtrack, once at `gain`, kept as the music is (never
    /// dropped to make room); returns its voice id.
    pub fn play_movie(&mut self, clip: &Arc<Clip>, gain: f32) -> u64 {
        self.play(clip, &[], gain, true)
    }

    fn play(&mut self, first: &Arc<Clip>, then: &[Arc<Clip>], gain: f32, music: bool) -> u64 {
        let id = self.id();
        if matches!(self.sink, Sink::Null) || gain < 0.002 {
            return id;
        }
        if let Ok(mut m) = self.mixer.lock() {
            m.start(id, first, gain, then.to_vec(), music);
        }
        id
    }

    /// Fade a voice out over `seconds`, then stop it.
    pub fn fade_out(&mut self, id: u64, seconds: f32) {
        if let Ok(mut m) = self.mixer.lock() {
            m.fade_out(id, seconds);
        }
    }

    /// Fade every voice out over `seconds` (the music, one fading out
    /// already, a menu sound), then stop each.
    pub fn fade_all(&mut self, seconds: f32) {
        if let Ok(mut m) = self.mixer.lock() {
            let ids: Vec<u64> = m.voices.iter().map(|v| v.id).collect();
            for id in ids {
                m.fade_out(id, seconds);
            }
        }
    }

    /// Stop every voice at once.
    pub fn stop_all(&mut self) {
        if let Ok(mut m) = self.mixer.lock() {
            m.voices.clear();
        }
    }

    /// A voice's gain now, if it still plays.
    pub fn gain(&self, id: u64) -> Option<f32> {
        let m = self.mixer.lock().ok()?;
        m.voices.iter().find(|v| v.id == id).map(|v| v.gain)
    }

    /// Pause (silence, every voice held) or play on. The device's stream
    /// stops too, so the lobby uses no audio while a match plays.
    pub fn pause(&mut self, paused: bool) -> Result<(), String> {
        if paused == self.paused {
            return Ok(());
        }
        self.paused = paused;
        if let Ok(mut m) = self.mixer.lock() {
            m.paused = paused;
        }
        match &self.sink {
            Sink::Device(s) if paused => s.pause().map_err(|e| e.to_string()),
            Sink::Device(s) => s.play().map_err(|e| e.to_string()),
            _ => Ok(()),
        }
    }

    /// Voices playing.
    pub fn voices(&self) -> usize {
        self.mixer.lock().map_or(0, |m| m.voices.len())
    }

    /// The mixer is paused (silent, every voice held).
    #[cfg(test)]
    pub fn is_paused(&self) -> bool {
        self.mixer.lock().is_ok_and(|m| m.paused)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        if let Sink::Wav { stop, thread } = &mut self.sink {
            stop.store(true, Ordering::Relaxed);
            if let Some(t) = thread.take() {
                let _ = t.join();
            }
        }
    }
}

/// A WAV file's 44-byte header for 16-bit stereo at `WAV_RATE`, with
/// `data` bytes of samples.
fn wav_header(data: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&WAV_RATE.to_le_bytes());
    h.extend_from_slice(&(WAV_RATE * 4).to_le_bytes());
    h.extend_from_slice(&4u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

/// Mix in real time into a WAV file at `path` until `stop`, the header
/// kept up to date as it grows.
fn record(
    mixer: Arc<Mutex<Mixer>>,
    path: &std::path::Path,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, String> {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(&wav_header(0)).map_err(|e| e.to_string())?;
    mixer.lock().map_err(|e| e.to_string())?.rate = WAV_RATE;
    // The recording's time starts now, however late the thread starts.
    let start = Instant::now();
    Ok(std::thread::spawn(move || {
        let mut written = 0u64;
        let mut out = Vec::new();
        let mut bytes = Vec::new();
        loop {
            let last = stop.load(Ordering::Relaxed);
            if !last {
                std::thread::sleep(WAV_STEP);
            }
            let due = (start.elapsed().as_secs_f64() * WAV_RATE as f64) as u64;
            let frames = due.saturating_sub(written) as usize;
            out.resize(frames * 2, 0.0);
            if let Ok(mut m) = mixer.lock() {
                m.mix(&mut out, 2);
            }
            written += frames as u64;
            bytes.clear();
            for s in &out {
                bytes.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
            }
            let data = (written * 4).min(u32::MAX as u64 - 36) as u32;
            let ok = file.seek(SeekFrom::Start(0)).is_ok()
                && file.write_all(&wav_header(data)).is_ok()
                && file.seek(SeekFrom::End(0)).is_ok()
                && file.write_all(&bytes).is_ok();
            if !ok || last {
                return;
            }
        }
    }))
}

/// The default audio device's stream, mixing as it asks for samples, and
/// the device's name if it gives one.
fn device(mixer: Arc<Mutex<Mixer>>) -> Result<(cpal::Stream, Option<String>), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device")?;
    let name = device.name().ok().filter(|n| !n.trim().is_empty());
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let channels = config.channels as usize;
    mixer.lock().map_err(|e| e.to_string())?.rate = config.sample_rate.0;
    let error = |e| eprintln!("audio: {e}");
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
    Ok((stream, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixer(rate: u32) -> Mixer {
        Mixer {
            voices: Vec::new(),
            rate,
            rng: 1,
            paused: false,
        }
    }

    fn clip(value: i16, frames: usize, rate: u32) -> Arc<Clip> {
        Arc::new(Clip {
            samples: vec![value; frames],
            channels: 1,
            rate,
        })
    }

    #[test]
    fn a_clip_is_resampled_and_finishes() {
        let mut m = mixer(44100);
        // 100 frames at 22.05 kHz: about 200 output frames at 44.1 kHz.
        m.start(1, &clip(16384, 100, 22050), 1.0, Vec::new(), false);
        assert_eq!(m.voices[0].step, 0.5);
        let mut out = vec![0.0; 2 * 150];
        m.mix(&mut out, 2);
        assert!((out[0] - 0.5 * MASTER).abs() < 1e-4);
        assert!((out[1] - 0.5 * MASTER).abs() < 1e-4);
        assert_eq!(m.voices.len(), 1);
        m.mix(&mut out, 2);
        assert!(m.voices.is_empty());
        // Half way between two samples, linearly.
        let mut m = mixer(200);
        let ramp = Arc::new(Clip {
            samples: vec![0, 8192, 16384, 16384],
            channels: 1,
            rate: 100,
        });
        m.start(1, &ramp, 1.0, Vec::new(), false);
        let mut out = vec![0.0; 2 * 3];
        m.mix(&mut out, 2);
        let at = |f: usize| out[2 * f] / MASTER;
        assert!(at(0).abs() < 1e-4);
        assert!((at(1) - 0.125).abs() < 1e-4);
        assert!((at(2) - 0.25).abs() < 1e-4);
    }

    #[test]
    fn the_mix_takes_nothing_off_the_tags_gains() {
        // Nothing scales the mix after a sound's own gain (`MASTER` is
        // 1, menu.md "After P2"): a full-scale clip at gain 0.5 comes out
        // at half of full scale, not 0.5 times a master volume.
        let mut m = mixer(100);
        m.start(1, &clip(i16::MAX, 10, 100), 0.5, Vec::new(), false);
        let mut out = vec![0.0; 2 * 5];
        m.mix(&mut out, 2);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-3), "{out:?}");
    }

    #[test]
    fn two_voices_mix() {
        let mut m = mixer(100);
        m.start(1, &clip(8192, 50, 100), 1.0, Vec::new(), false);
        m.start(2, &clip(4096, 50, 100), 0.5, Vec::new(), false);
        let mut out = vec![0.0; 2 * 10];
        m.mix(&mut out, 2);
        // 0.25 and half of 0.125.
        assert!((out[0] - (0.25 + 0.0625) * MASTER).abs() < 1e-4);
        assert!((out[19] - (0.25 + 0.0625) * MASTER).abs() < 1e-4);
        // A stereo clip keeps its sides; one channel out takes the left.
        let mut m = mixer(100);
        let stereo = Arc::new(Clip {
            samples: [8192, -8192].repeat(10),
            channels: 2,
            rate: 100,
        });
        m.start(1, &stereo, 1.0, Vec::new(), false);
        let mut out = vec![0.0; 2 * 3];
        m.mix(&mut out, 2);
        assert!(out[0] > 0.0 && out[1] < 0.0);
        let mut mono = vec![0.0; 3];
        m.mix(&mut mono, 1);
        assert!(mono[0] > 0.0);
    }

    #[test]
    fn a_looping_voice_wraps() {
        let mut m = mixer(100);
        m.start(1, &clip(8192, 10, 100), 1.0, Vec::new(), false);
        m.voices[0].looping = true;
        let mut out = vec![0.0; 2 * 35];
        m.mix(&mut out, 2);
        assert_eq!(m.voices.len(), 1);
        assert!(m.voices[0].position < 10.0);
        assert!(out.iter().all(|s| (s - 0.25 * MASTER).abs() < 1e-4));
    }

    #[test]
    fn music_goes_from_its_intro_to_its_loops() {
        let mut m = mixer(100);
        m.start(
            1,
            &clip(8192, 10, 100),
            1.0,
            vec![clip(16384, 10, 100)],
            true,
        );
        let mut out = vec![0.0; 2 * 30];
        m.mix(&mut out, 2);
        // The intro, then the loop without a gap, then the loop again.
        assert!((out[0] - 0.25 * MASTER).abs() < 1e-4);
        assert!((out[2 * 12] - 0.5 * MASTER).abs() < 1e-4);
        assert!((out[2 * 25] - 0.5 * MASTER).abs() < 1e-4);
        assert_eq!(m.voices.len(), 1);
        // Picked at random among several, and still going.
        m.voices[0].next.push(clip(4096, 10, 100));
        for _ in 0..20 {
            m.mix(&mut out, 2);
        }
        assert_eq!(m.voices.len(), 1);
    }

    #[test]
    fn a_fade_out_ends_the_voice() {
        let mut m = mixer(100);
        m.start(1, &clip(8192, 1000, 100), 1.0, Vec::new(), false);
        // 0.2 s at 100 frames a second: 20 frames.
        assert!(m.fade_out(1, 0.2));
        let mut out = vec![0.0; 2 * 10];
        m.mix(&mut out, 2);
        let g = m.voices[0].gain;
        assert!((g - 0.5).abs() < 1e-3, "{g}");
        // Getting quieter frame by frame.
        assert!(out[2 * 9] < out[0]);
        m.mix(&mut out, 2);
        assert!(m.voices.is_empty());
        assert!(!m.fade_out(1, 0.2));
        // A voice silent already ends at once.
        m.start(2, &clip(8192, 1000, 100), 0.0, Vec::new(), false);
        m.fade_out(2, 1.0);
        m.mix(&mut out, 2);
        assert!(m.voices.is_empty());
    }

    #[test]
    fn the_music_is_never_dropped_to_make_room() {
        // The music (quieter than any menu sound), then a held key's
        // repeats: 20 menu sounds, each longer than the repeats' gap.
        let mut m = mixer(100);
        m.start(
            1,
            &clip(8192, 1000, 100),
            0.5,
            vec![clip(8192, 1000, 100)],
            true,
        );
        for id in 2..=21 {
            m.start(id, &clip(8192, 1000, 100), 0.71, Vec::new(), false);
        }
        assert_eq!(m.voices.len(), MAX_VOICES);
        let mut ids: Vec<u64> = m.voices.iter().map(|v| v.id).collect();
        ids.sort();
        // The music, and the newest 15 menu sounds: the oldest made room.
        assert_eq!(ids, [1].into_iter().chain(7..=21).collect::<Vec<u64>>());
        // A music without loops is music too, and a music starting with
        // every voice a menu sound gets one (the oldest's).
        let mut m = mixer(100);
        for id in 1..=16 {
            m.start(id, &clip(8192, 1000, 100), 0.71, Vec::new(), false);
        }
        m.start(17, &clip(8192, 1000, 100), 0.5, Vec::new(), true);
        assert_eq!(m.voices.len(), MAX_VOICES);
        assert!(m.voices.iter().all(|v| v.id != 1));
        for id in 18..=40 {
            m.start(id, &clip(8192, 1000, 100), 0.71, Vec::new(), false);
        }
        assert!(m.voices.iter().any(|v| v.id == 17 && v.music));
        // Only if every voice were music would the quietest go.
        let mut m = mixer(100);
        for id in 1..=16 {
            let gain = if id == 5 { 0.1 } else { 0.5 };
            m.start(id, &clip(8192, 1000, 100), gain, Vec::new(), true);
        }
        m.start(17, &clip(8192, 1000, 100), 0.5, Vec::new(), true);
        assert!(m.voices.iter().all(|v| v.id != 5));
        assert_eq!(m.voices.len(), MAX_VOICES);
    }

    #[test]
    fn paused_is_silent_and_holds_every_voice() {
        let mut m = mixer(100);
        m.start(1, &clip(8192, 1000, 100), 1.0, Vec::new(), false);
        m.fade_out(1, 1.0);
        m.paused = true;
        let mut out = vec![1.0; 2 * 20];
        m.mix(&mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(m.voices[0].position, 0.0);
        assert_eq!(m.voices[0].gain, 1.0);
        m.paused = false;
        m.mix(&mut out, 2);
        assert_eq!(m.voices[0].position, 20.0);
        assert!(out[0] > 0.0);
    }

    #[test]
    fn the_null_output_keeps_nothing() {
        let mut a = Audio::open(&Output::Null).unwrap();
        let c = clip(8192, 100, 22050);
        let one = a.play_ui(&c, 1.0);
        let two = a.play_music(&c, std::slice::from_ref(&c), 0.5);
        assert_ne!(one, two);
        assert_eq!(a.voices(), 0);
        assert_eq!(a.gain(two), None);
        a.fade_out(two, 1.0);
        assert!(a.pause(true).is_ok() && a.pause(false).is_ok());
    }

    #[test]
    fn a_wav_recording_holds_what_was_played() {
        let dir = std::env::temp_dir().join(format!("h2audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.wav");
        let mut a = Audio::open(&Output::Wav(path.clone())).unwrap();
        let id = a.play_music(&clip(8192, 44100, 44100), &[], 1.0);
        assert_eq!(a.gain(id), Some(1.0));
        std::thread::sleep(Duration::from_millis(120));
        drop(a);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 44 + data);
        // At least the 0.12 s slept, and not silent: the voice was in
        // the mixer before the sleep, so blocks mixed after it hold it,
        // however the recorder's first block fell.
        assert!(data >= 4 * 5292, "{data}");
        let loud = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .any(|b| i16::from_le_bytes(*b) > 0);
        assert!(loud);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
