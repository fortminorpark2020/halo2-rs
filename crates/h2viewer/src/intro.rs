//! Halo 2's intro movie, played as the game starts, before the start
//! screen: `movie\intro_60.wmv` beside the maps folder
//! (`intro_low_60.wmv` in a small window). Any key, mouse button or
//! controller button skips it; without the file, or a way to decode it,
//! the game goes straight on to the start screen.
//!
//! Windows decodes it with Media Foundation; elsewhere ffmpeg does, if
//! it's installed.

use crate::audio::{Audio, Clip};
use crate::gpu::{hud_mode, HudBatch, MOVIE_TEXTURE};
use crate::hud::HudBuilder;
use crate::{App, MUSIC_VOLUME};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Pictures decoded ahead of when they show.
const AHEAD: usize = 4;
/// How long to wait on the decoder before giving up on the movie.
const STALL: Duration = Duration::from_secs(5);

/// What the decoder sends: the whole soundtrack first, then the pictures
/// as they're wanted.
enum Decoded {
    Sound(Clip),
    Picture(Picture),
}

/// A picture of the movie: seconds in when it shows, its size, and its
/// pixels (BGRA, top row first, opaque).
pub struct Picture {
    time: f32,
    pub size: [u32; 2],
    pub bgra: Vec<u8>,
}

pub struct Intro {
    rx: Receiver<Decoded>,
    /// The next picture, waiting for its time.
    next: Option<Picture>,
    /// When the movie (and its sound) started.
    started: Option<Instant>,
    voice: Option<u64>,
    /// When the decoder was last heard from.
    heard: Instant,
    over: bool,
    /// The size of the picture showing, once one is.
    shown: Option<[u32; 2]>,
}

/// The intro for a window `height` pixels tall, if the install has it.
fn movie(maps: &Path, height: u32) -> Option<PathBuf> {
    let dir = maps.parent()?.join("movie");
    let names = match height < 720 {
        true => ["intro_low_60.wmv", "intro_60.wmv"],
        false => ["intro_60.wmv", "intro_low_60.wmv"],
    };
    names.iter().map(|n| dir.join(n)).find(|p| p.exists())
}

impl Intro {
    /// Start decoding the intro, if there's one and a way to play it.
    pub fn start(maps: &Path, height: u32) -> Option<Intro> {
        let path = movie(maps, height)?;
        if !decode::available() {
            println!("intro: no decoder for {}", path.display());
            return None;
        }
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        std::thread::spawn(move || {
            if let Err(e) = decode::run(&path, &tx) {
                println!("intro: {}: {e}", path.display());
            }
        });
        Some(Intro {
            rx,
            next: None,
            started: None,
            voice: None,
            heard: Instant::now(),
            over: false,
            shown: None,
        })
    }

    /// Play on: start the sound when it comes, and return the picture due
    /// now, if a new one is.
    pub fn update(&mut self, audio: &mut Audio, volume: f32) -> Option<Picture> {
        let mut due = None;
        loop {
            if self.next.is_none() {
                match self.rx.try_recv() {
                    Ok(Decoded::Sound(clip)) => {
                        self.heard = Instant::now();
                        self.started.get_or_insert_with(Instant::now);
                        self.voice = Some(audio.play_ui(&Arc::new(clip), volume));
                        continue;
                    }
                    Ok(Decoded::Picture(p)) => {
                        self.heard = Instant::now();
                        self.started.get_or_insert_with(Instant::now);
                        self.next = Some(p);
                    }
                    Err(TryRecvError::Empty) => {
                        self.over |= self.heard.elapsed() > STALL;
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.over = true;
                        break;
                    }
                }
            }
            let now = self.started.map_or(0.0, |t| t.elapsed().as_secs_f32());
            match self.next.take() {
                // Late pictures are passed over for the latest one due.
                Some(p) if p.time <= now => due = Some(p),
                waiting => {
                    self.next = waiting;
                    break;
                }
            }
        }
        due
    }

    /// It's played to the end (or its decoder gave up).
    pub fn over(&self) -> bool {
        self.over
    }

    /// Stop it, sound and all.
    pub fn stop(&mut self, audio: &mut Audio) {
        if let Some(voice) = self.voice.take() {
            audio.stop(voice);
        }
        self.over = true;
    }
}

/// Media Foundation, which comes with Windows (and its WMV decoders).
#[cfg(windows)]
mod decode {
    use super::{Decoded, Picture};
    use crate::audio::Clip;
    use std::path::Path;
    use std::sync::mpsc::SyncSender;
    use windows::core::{Interface, Result, GUID, HSTRING};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    pub fn available() -> bool {
        true
    }

    /// A reader of one of the file's streams, decoding it to `subtype`
    /// (with 16-bit samples, for sound).
    fn reader(path: &Path, stream: u32, major: GUID, subtype: GUID) -> Result<IMFSourceReader> {
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.ok_or_else(windows::core::Error::empty)?;
            // Let it turn the video into RGB.
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path), &attributes)?;
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(stream, true)?;
            let wanted = MFCreateMediaType()?;
            wanted.SetGUID(&MF_MT_MAJOR_TYPE, &major)?;
            wanted.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
            if major == MFMediaType_Audio {
                wanted.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            }
            reader.SetCurrentMediaType(stream, None, &wanted)?;
            Ok(reader)
        }
    }

    /// The next sample of the stream, its time (100 ns units), and the
    /// reader's flags (the stream has ended, its format has changed).
    fn next(reader: &IMFSourceReader, stream: u32) -> Result<(Option<IMFSample>, i64, u32)> {
        let (mut flags, mut time, mut sample) = (0u32, 0i64, None);
        unsafe {
            reader.ReadSample(
                stream,
                0,
                None,
                Some(&mut flags as *mut _),
                Some(&mut time as *mut _),
                Some(&mut sample as *mut _),
            )?;
        }
        Ok((sample, time, flags))
    }

    fn ended(flags: u32) -> bool {
        flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0
    }

    /// The pictures' size, as the decoder gives them now.
    fn frame_size(reader: &IMFSourceReader, stream: u32) -> Result<[u32; 2]> {
        let format = unsafe { reader.GetCurrentMediaType(stream)? };
        let packed = unsafe { format.GetUINT64(&MF_MT_FRAME_SIZE)? };
        Ok([(packed >> 32) as u32, packed as u32])
    }

    /// A sample's bytes.
    fn bytes(sample: &IMFSample) -> Result<Vec<u8>> {
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
            buffer.Lock(&mut data, None, Some(&mut len as *mut _))?;
            let out = std::slice::from_raw_parts(data, len as usize).to_vec();
            buffer.Unlock()?;
            Ok(out)
        }
    }

    /// The whole soundtrack, if the movie has one.
    fn sound(path: &Path) -> Result<Option<Clip>> {
        let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        let Ok(reader) = reader(path, stream, MFMediaType_Audio, MFAudioFormat_PCM) else {
            return Ok(None);
        };
        let format = unsafe { reader.GetCurrentMediaType(stream)? };
        let channels = unsafe { format.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? } as u16;
        let rate = unsafe { format.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)? };
        let mut samples = Vec::new();
        loop {
            let (sample, _, flags) = next(&reader, stream)?;
            if let Some(s) = sample {
                let b = bytes(&s)?;
                samples.extend(b.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)));
            }
            if ended(flags) {
                break;
            }
        }
        Ok(Some(Clip {
            samples,
            channels,
            rate,
        }))
    }

    /// A picture's pixels, top row first, opaque.
    fn pixels(sample: &IMFSample, [w, h]: [u32; 2]) -> Result<Vec<u8>> {
        let row = w as usize * 4;
        let mut out = vec![0u8; row * h as usize];
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            match buffer.cast::<IMF2DBuffer>() {
                // Rows may be padded, or stored bottom up.
                Ok(b) => {
                    let (mut line, mut pitch) = (std::ptr::null_mut(), 0i32);
                    b.Lock2D(&mut line, &mut pitch)?;
                    for (y, dst) in out.chunks_exact_mut(row).enumerate() {
                        let src = line.offset(y as isize * pitch as isize);
                        std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), row);
                    }
                    b.Unlock2D()?;
                }
                Err(_) => {
                    let b = bytes(sample)?;
                    let n = b.len().min(out.len());
                    out[..n].copy_from_slice(&b[..n]);
                }
            }
        }
        // RGB32 leaves the fourth byte unset.
        for px in out.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        Ok(out)
    }

    fn pictures(path: &Path, tx: &SyncSender<Decoded>) -> Result<()> {
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let reader = reader(path, stream, MFMediaType_Video, MFVideoFormat_RGB32)?;
        let mut size = frame_size(&reader, stream)?;
        loop {
            let (sample, time, flags) = next(&reader, stream)?;
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                size = frame_size(&reader, stream)?;
            }
            if let Some(s) = sample {
                let picture = Picture {
                    time: time as f32 / 1e7,
                    size,
                    bgra: pixels(&s, size)?,
                };
                if tx.send(Decoded::Picture(picture)).is_err() {
                    return Ok(());
                }
            }
            if ended(flags) {
                return Ok(());
            }
        }
    }

    pub fn run(path: &Path, tx: &SyncSender<Decoded>) -> std::result::Result<(), String> {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let played = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.and_then(|()| {
            let played = sound(path).and_then(|clip| {
                if let Some(c) = clip {
                    if tx.send(Decoded::Sound(c)).is_err() {
                        return Ok(());
                    }
                }
                pictures(path, tx)
            });
            let _ = unsafe { MFShutdown() };
            played
        });
        if com {
            unsafe { CoUninitialize() };
        }
        played.map_err(|e| e.to_string())
    }
}

/// ffmpeg, if it's installed (not on Windows, where Media Foundation
/// plays the movie).
#[cfg(not(windows))]
mod decode {
    use super::{Decoded, Picture};
    use crate::audio::Clip;
    use std::io::Read;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::sync::mpsc::SyncSender;

    /// The soundtrack's sample rate.
    const RATE: u32 = 48_000;

    pub fn available() -> bool {
        ["ffmpeg", "ffprobe"].iter().all(|tool| {
            Command::new(tool)
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    fn ffmpeg(path: &Path, output: &[&str]) -> Command {
        let mut c = Command::new("ffmpeg");
        c.args(["-v", "error", "-i"]).arg(path).args(output);
        c.stdin(Stdio::null()).stderr(Stdio::null());
        c
    }

    /// The video's size and pictures per second.
    fn video(path: &Path) -> Option<([u32; 2], f32)> {
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0"])
            .args(["-show_entries", "stream=width,height,r_frame_rate"])
            .args(["-of", "csv=p=0"])
            .arg(path)
            .output()
            .ok()?;
        // "1280,720,30/1"
        let text = String::from_utf8_lossy(&probe.stdout);
        let mut fields = text.trim().split(',');
        let w = fields.next()?.parse().ok()?;
        let h = fields.next()?.parse().ok()?;
        let (n, d) = fields.next()?.split_once('/')?;
        let rate = n.parse::<f32>().ok()? / d.parse::<f32>().ok()?.max(1.0);
        Some(([w, h], rate.max(1.0)))
    }

    pub fn run(path: &Path, tx: &SyncSender<Decoded>) -> Result<(), String> {
        let channels = ["-ac", "2", "-ar", "48000"];
        let raw = ffmpeg(path, &["-vn", "-f", "s16le"])
            .args(channels)
            .arg("-")
            .output()
            .map_err(|e| e.to_string())?;
        if !raw.stdout.is_empty() {
            let samples = raw.stdout.as_chunks::<2>().0;
            let clip = Clip {
                samples: samples.iter().map(|b| i16::from_le_bytes(*b)).collect(),
                channels: 2,
                rate: RATE,
            };
            if tx.send(Decoded::Sound(clip)).is_err() {
                return Ok(());
            }
        }
        let (size, rate) = video(path).ok_or("can't read the video's size")?;
        let mut child = ffmpeg(path, &["-an", "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut out = child.stdout.take().ok_or("no output")?;
        let bytes = size[0] as usize * size[1] as usize * 4;
        for k in 0.. {
            let mut bgra = vec![0u8; bytes];
            if out.read_exact(&mut bgra).is_err() {
                break;
            }
            let time = k as f32 / rate;
            if tx
                .send(Decoded::Picture(Picture { time, size, bgra }))
                .is_err()
            {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

impl App {
    /// Start the intro as the window opens (`height` pixels tall), if it's
    /// wanted and there.
    pub(crate) fn start_intro(&mut self, height: u32) {
        let maps = self.map_path.parent();
        self.intro = maps
            .filter(|_| self.intro_wanted)
            .and_then(|m| Intro::start(m, height));
    }

    /// Show the picture due, and let the intro go once it's over.
    pub(crate) fn update_intro(&mut self) {
        let Some(intro) = &mut self.intro else {
            return;
        };
        if let Some(p) = intro.update(&mut self.sound.audio, MUSIC_VOLUME) {
            intro.shown = Some(p.size);
            if let Some(g) = &mut self.gpu {
                g.set_movie_frame(p.size, &p.bgra);
            }
        }
        if intro.over() {
            self.skip_intro();
        }
    }

    /// Stop the intro, if it's playing (any key, mouse button or
    /// controller button does): on to the start screen.
    pub(crate) fn skip_intro(&mut self) -> bool {
        let Some(mut intro) = self.intro.take() else {
            return false;
        };
        intro.stop(&mut self.sound.audio);
        if let Some(g) = &mut self.gpu {
            g.clear_movie();
        }
        self.menu.reveal();
        true
    }

    /// While the intro plays, it fills the window: its picture (once
    /// there is one) on black.
    pub(crate) fn intro_overlay(&self, w: f32, h: f32) -> Option<Vec<HudBatch>> {
        let intro = self.intro.as_ref()?;
        let mut hb = HudBuilder::new(w, h);
        let black = [0.0, 0.0, 0.0, 1.0];
        let white = self.scene.hud_white;
        hb.quad(
            white,
            [0.0, 0.0, w, h],
            [0.0; 4],
            black,
            hud_mode::PLAIN,
            0.0,
        );
        if let Some(size) = intro.shown {
            let rect = letterbox(size, w, h);
            let full = [0.0, 0.0, 1.0, 1.0];
            hb.quad(MOVIE_TEXTURE, rect, full, [1.0; 4], hud_mode::PLAIN, 0.0);
        }
        Some(hb.finish())
    }
}

/// Where a `size` picture goes in a `w` by `h` window: as large as fits,
/// in the middle.
pub fn letterbox([pw, ph]: [u32; 2], w: f32, h: f32) -> [f32; 4] {
    let scale = (w / pw.max(1) as f32).min(h / ph.max(1) as f32);
    let (sw, sh) = (pw as f32 * scale, ph as f32 * scale);
    let (x, y) = ((w - sw) * 0.5, (h - sh) * 0.5);
    [x, y, x + sw, y + sh]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_fits_the_picture_in_the_middle() {
        // 4:3 in 16:9: bars at the sides.
        assert_eq!(
            letterbox([640, 480], 1280.0, 720.0),
            [160.0, 0.0, 1120.0, 720.0]
        );
        // 16:9 in 4:3: bars above and below.
        assert_eq!(
            letterbox([1280, 720], 1024.0, 768.0),
            [0.0, 96.0, 1024.0, 672.0]
        );
    }

    #[test]
    fn small_windows_get_the_low_movie() {
        let root = std::env::temp_dir().join(format!("h2intro{}", std::process::id()));
        let (maps, movies) = (root.join("maps"), root.join("movie"));
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::create_dir_all(&movies).unwrap();
        assert_eq!(movie(&maps, 720), None);
        std::fs::write(movies.join("intro_60.wmv"), b"").unwrap();
        std::fs::write(movies.join("intro_low_60.wmv"), b"").unwrap();
        assert_eq!(movie(&maps, 720), Some(movies.join("intro_60.wmv")));
        assert_eq!(movie(&maps, 480), Some(movies.join("intro_low_60.wmv")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
