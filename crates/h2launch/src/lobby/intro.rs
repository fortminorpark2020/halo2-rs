//! Halo 2's intro movie (menu.md, "What Phase 2.3 built"): played once as
//! the lobby opens, before the start screen, with its sound, from Halo 2
//! Vista's `movie` folder as Project Cartographer installs it
//! (`intro_60.wmv`, or `intro_low_60.wmv` in a window under 720 pixels
//! tall). Any button skips it (`App`); at its end, on a skip, or when it
//! can't be played, the start screen comes in.
//!
//! The movie is decoded on a thread of its own by a `MovieSource`: Media
//! Foundation on Windows (`MfSource`, a close port of `menu-preview`'s
//! `h2viewer/src/intro.rs`), a made-up one in the tests (`FakeSource`).
//! Off Windows there is no movie. The thread sends the whole soundtrack
//! first, then the pictures in order, at most `AHEAD` ahead of the one
//! showing; `Intro` shows each when its time comes, passing over late
//! ones, and gives up after `STALL` without a word from the thread.

use super::audio::Clip;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `off` opens the lobby on the start screen, without the intro.
pub const ENV: &str = "H2LOBBY_INTRO";
/// Pictures decoded ahead of the one showing.
pub const AHEAD: usize = 4;
/// How long the decoder may send nothing before the movie is given up.
pub const STALL: Duration = Duration::from_secs(5);
/// The intro, and the smaller one for small windows, in Vista's `movie`
/// folder.
const FULL: &str = "intro_60.wmv";
const LOW: &str = "intro_low_60.wmv";
/// Windows shorter than this (pixels) get the smaller movie first.
const LOW_BELOW: usize = 720;
/// How long the last picture stays up once the decoder has finished,
/// unless the sound goes on longer: about two pictures at 60 a second (an
/// estimate; nothing to read it from).
const LAST_HOLD: f32 = 1.0 / 30.0;

/// What the decoder thread sends.
pub enum Decoded {
    /// The whole soundtrack, first (none if the movie has no sound).
    Sound(Clip),
    /// A picture, in order.
    Picture(Picture),
    /// Why the movie can't be played (the thread's last word).
    Failed(String),
}

/// One picture of the movie.
pub struct Picture {
    /// Seconds into the movie when it shows.
    pub time: f32,
    pub w: u32,
    pub h: u32,
    /// Its pixels as RGBA, top row first, opaque (the decoder's BGRA,
    /// swapped on its thread so drawing needn't).
    pub rgba: Vec<u8>,
}

/// Decodes a movie on its own thread, sending its sound and pictures down
/// `tx` until it ends or nobody listens (a send fails: then it stops and
/// returns `Ok`).
pub trait MovieSource: Send {
    fn run(self: Box<Self>, tx: &SyncSender<Decoded>) -> Result<(), String>;
}

/// Makes the source for a movie file: Media Foundation's on Windows, a
/// made-up one in tests.
pub type Opener = Arc<dyn Fn(&Path) -> Box<dyn MovieSource>>;

/// The intro the lobby plays: the movie files found, and how to decode
/// them.
#[derive(Clone)]
pub struct Plan {
    pub full: Option<PathBuf>,
    pub low: Option<PathBuf>,
    pub open: Opener,
}

impl Plan {
    /// The intro in Vista's movie folder (`sources::Vista::movies`), if
    /// either file is there.
    pub fn find(movies: &Path, open: Opener) -> Option<Plan> {
        let file = |name: &str| Some(movies.join(name)).filter(|p| p.is_file());
        let (full, low) = (file(FULL), file(LOW));
        (full.is_some() || low.is_some()).then_some(Plan { full, low, open })
    }

    /// The file for a window `height` pixels tall: the smaller movie under
    /// `LOW_BELOW`, the full one otherwise, or the other one when that is
    /// missing.
    pub fn file(&self, height: usize) -> Option<&Path> {
        let (first, second) = match height < LOW_BELOW {
            true => (&self.low, &self.full),
            false => (&self.full, &self.low),
        };
        first.as_deref().or(second.as_deref())
    }
}

/// Why the lobby opens without the intro, if it does: headless, switched
/// off (`H2LOBBY_INTRO=off`, `intro = off` in `lobby.txt`), or no movie it
/// can play (`playable`: Windows, and Vista's movie folder holds one).
pub fn skip_reason(
    headless: bool,
    env: Option<&OsStr>,
    pref: Option<bool>,
    playable: bool,
) -> Option<&'static str> {
    if headless {
        Some("headless")
    } else if env.is_some_and(|v| v.eq_ignore_ascii_case("off")) {
        Some("H2LOBBY_INTRO=off")
    } else if pref == Some(false) {
        Some("intro = off in lobby.txt")
    } else if !playable {
        Some("no intro movie")
    } else {
        None
    }
}

/// How the intro ended.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// Played to its end.
    Played,
    /// The decoder sent nothing for `STALL`.
    Stalled,
    /// It couldn't be played (the decoder's reason).
    Failed(String),
}

/// What `Intro::update` found.
#[derive(Default)]
pub struct Update {
    /// The soundtrack came: play it now.
    pub sound: Option<Arc<Clip>>,
    /// The movie is over.
    pub end: Option<End>,
}

/// The intro playing: the pictures as they come, the one showing, and
/// the movie's clock.
pub struct Intro {
    rx: Receiver<Decoded>,
    /// The next picture, waiting for its time.
    next: Option<Picture>,
    /// The picture showing.
    shown: Option<Picture>,
    /// Pictures that came due (shown or passed over), and those passed
    /// over because a later one was due as well.
    pictures: u32,
    late: u32,
    /// Bumped each time a new picture shows, so it is drawn once.
    serial: u64,
    /// When the movie's clock started: the decoder's first word.
    started: Option<Instant>,
    /// When the decoder was last heard from.
    heard: Instant,
    /// The soundtrack's length, once it came.
    sound_seconds: f32,
    /// The decoder has finished: the movie ends at this time on its clock.
    ends_at: Option<f32>,
    end: Option<End>,
}

impl Intro {
    /// Start decoding `source` on a thread of its own.
    pub fn start(source: Box<dyn MovieSource>, now: Instant) -> Result<Intro, String> {
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        std::thread::Builder::new()
            .name("intro movie".into())
            .spawn(move || {
                if let Err(e) = source.run(&tx) {
                    let _ = tx.send(Decoded::Failed(e));
                }
            })
            .map_err(|e| format!("couldn't start decoding: {e}"))?;
        Ok(Intro::receiving(rx, now))
    }

    /// An intro taking what comes down `rx` (`start`'s, or a test's).
    pub fn receiving(rx: Receiver<Decoded>, now: Instant) -> Intro {
        Intro {
            rx,
            next: None,
            shown: None,
            pictures: 0,
            late: 0,
            serial: 0,
            started: None,
            heard: now,
            sound_seconds: 0.0,
            ends_at: None,
            end: None,
        }
    }

    /// Seconds into the movie at `now` (0 until the decoder's first word).
    pub fn clock(&self, now: Instant) -> f32 {
        self.started
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32())
    }

    /// The picture showing, and its serial number (new pictures get new
    /// numbers).
    pub fn showing(&self) -> Option<(&Picture, u64)> {
        self.shown.as_ref().map(|p| (p, self.serial))
    }

    /// Pictures that came due, and those of them passed over as late.
    pub fn counts(&self) -> (u32, u32) {
        (self.pictures, self.late)
    }

    /// Play on to `now`: take what the decoder sent, show the latest
    /// picture due (passing over earlier ones also due), and say whether
    /// the sound came and whether the movie is over.
    pub fn update(&mut self, now: Instant) -> Update {
        let mut out = Update::default();
        let due_before = self.pictures;
        while self.end.is_none() {
            if self.next.is_none() && self.ends_at.is_none() {
                match self.rx.try_recv() {
                    Ok(Decoded::Sound(clip)) => {
                        self.heard = now;
                        self.started.get_or_insert(now);
                        self.sound_seconds = clip.duration();
                        out.sound = Some(Arc::new(clip));
                        continue;
                    }
                    Ok(Decoded::Picture(p)) => {
                        self.heard = now;
                        self.started.get_or_insert(now);
                        self.next = Some(p);
                    }
                    Ok(Decoded::Failed(e)) => {
                        self.end = Some(End::Failed(e));
                        break;
                    }
                    Err(TryRecvError::Empty) => {
                        if now.saturating_duration_since(self.heard) > STALL {
                            self.end = Some(End::Stalled);
                        }
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        if self.pictures == 0 && self.next.is_none() {
                            let why = "the movie had no pictures".to_string();
                            self.end = Some(End::Failed(why));
                            break;
                        }
                        // The last picture stays a moment, and the sound
                        // plays out.
                        let last = self.shown.as_ref().map_or(0.0, |p| p.time + LAST_HOLD);
                        self.ends_at = Some(last.max(self.sound_seconds));
                        continue;
                    }
                }
            }
            let t = self.clock(now);
            if let Some(at) = self.ends_at {
                if t >= at {
                    self.end = Some(End::Played);
                }
                break;
            }
            match self.next.take() {
                Some(p) if p.time <= t => {
                    self.shown = Some(p);
                    self.pictures += 1;
                }
                waiting => {
                    self.next = waiting;
                    break;
                }
            }
        }
        // Of the pictures that came due now only the latest is shown: the
        // others were late.
        let due = self.pictures - due_before;
        if due > 0 {
            self.serial += 1;
            self.late += due - 1;
        }
        out.end = self.end.clone();
        out
    }
}

/// Where a `pw` by `ph` picture goes in a `w` by `h` window (left, top,
/// right, bottom): as large as fits, in the middle, black around it.
pub fn letterbox(pw: u32, ph: u32, w: f32, h: f32) -> [f32; 4] {
    let scale = (w / pw.max(1) as f32).min(h / ph.max(1) as f32);
    let (sw, sh) = (pw as f32 * scale, ph as f32 * scale);
    let (x, y) = ((w - sw) * 0.5, (h - sh) * 0.5);
    [x, y, x + sw, y + sh]
}

/// The decoder for a movie file on this PC: Media Foundation on Windows;
/// none elsewhere.
pub fn opener() -> Option<Opener> {
    #[cfg(windows)]
    {
        let open: Opener = Arc::new(|path: &Path| {
            Box::new(mf::MfSource {
                path: path.to_path_buf(),
            }) as Box<dyn MovieSource>
        });
        Some(open)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Media Foundation, which comes with Windows (with its WMV decoders; the
/// N editions need the Media Feature Pack, or the intro is skipped). Ported
/// closely from `menu-preview`'s `h2viewer/src/intro.rs`.
#[cfg(windows)]
mod mf {
    use super::{Decoded, MovieSource, Picture};
    use crate::lobby::audio::Clip;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::SyncSender;
    use windows::core::{Interface, Result, GUID, HSTRING};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    /// A movie file, decoded by Media Foundation.
    pub struct MfSource {
        pub path: PathBuf,
    }

    impl MovieSource for MfSource {
        fn run(self: Box<Self>, tx: &SyncSender<Decoded>) -> std::result::Result<(), String> {
            run(&self.path, tx)
        }
    }

    /// A reader of one of the file's streams, decoding it to `subtype`
    /// (with 16-bit samples, for sound).
    fn reader(path: &Path, stream: u32, major: GUID, subtype: GUID) -> Result<IMFSourceReader> {
        // SAFETY: Media Foundation is started on this thread (`run`), and
        // every pointer passed is to a live local.
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
        // SAFETY: the out pointers are to live locals.
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
        // SAFETY: plain COM calls on a live reader.
        let format = unsafe { reader.GetCurrentMediaType(stream)? };
        let packed = unsafe { format.GetUINT64(&MF_MT_FRAME_SIZE)? };
        Ok([(packed >> 32) as u32, packed as u32])
    }

    /// A sample's bytes.
    fn bytes(sample: &IMFSample) -> Result<Vec<u8>> {
        // SAFETY: the buffer is locked while its `len` bytes are copied.
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
        // SAFETY: plain COM calls on a live reader.
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

    /// A picture's pixels as RGBA, top row first, opaque.
    fn pixels(sample: &IMFSample, [w, h]: [u32; 2]) -> Result<Vec<u8>> {
        let row = w as usize * 4;
        let mut out = vec![0u8; row * h as usize];
        // SAFETY: the 2D buffer is locked while its rows are copied, each
        // `row` bytes from the row pointer stepped by the pitch it gave.
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
        // RGB32 is blue, green, red and an unset byte: to RGBA, opaque.
        for px in out.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
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
                    w: size[0],
                    h: size[1],
                    rgba: pixels(&s, size)?,
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
        // SAFETY: COM and Media Foundation are started and stopped on this
        // thread, around everything that uses them.
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
            // SAFETY: started above.
            let _ = unsafe { MFShutdown() };
            played
        });
        if com {
            // SAFETY: initialised above on this thread.
            unsafe { CoUninitialize() };
        }
        played.map_err(|e| format!("Media Foundation: {e}"))
    }
}

/// A made-up movie for tests: `pictures` solid pictures `step` seconds
/// apart, after a short soundtrack; then, with `hold`, it says nothing
/// more until `hold`'s sender goes (a stalled decoder), or with `fail`
/// it fails.
#[cfg(test)]
pub struct FakeSource {
    pub pictures: u32,
    pub step: f32,
    pub size: (u32, u32),
    pub sound_seconds: f32,
    pub hold: Option<Receiver<()>>,
    pub fail: Option<String>,
}

#[cfg(test)]
impl FakeSource {
    /// About a second: 30 pictures at 30 a second, 4 by 3 pixels, and a
    /// tenth of a second of sound.
    pub fn second() -> FakeSource {
        FakeSource {
            pictures: 30,
            step: 1.0 / 30.0,
            size: (4, 3),
            sound_seconds: 0.1,
            hold: None,
            fail: None,
        }
    }

    /// Picture `k`: its time, and every pixel `k` in red.
    pub fn picture(&self, k: u32) -> Picture {
        let (w, h) = self.size;
        let px = [k.min(255) as u8, 0, 0, 255];
        Picture {
            time: k as f32 * self.step,
            w,
            h,
            rgba: px.repeat((w * h) as usize),
        }
    }
}

#[cfg(test)]
impl MovieSource for FakeSource {
    fn run(self: Box<Self>, tx: &SyncSender<Decoded>) -> Result<(), String> {
        if self.sound_seconds > 0.0 {
            let rate = 100;
            let clip = Clip {
                samples: vec![1000; (self.sound_seconds * rate as f32) as usize],
                channels: 1,
                rate,
            };
            if tx.send(Decoded::Sound(clip)).is_err() {
                return Ok(());
            }
        }
        for k in 0..self.pictures {
            if tx.send(Decoded::Picture(self.picture(k))).is_err() {
                return Ok(());
            }
        }
        if let Some(hold) = &self.hold {
            let _ = hold.recv();
        }
        match self.fail {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An intro fed by `source` on this thread, everything sent before the
    /// first update (a channel big enough for all of it), its failure last
    /// as `Intro::start` sends it.
    fn fed(source: FakeSource, now: Instant) -> Intro {
        let (tx, rx) = mpsc::sync_channel(1000);
        if let Err(e) = Box::new(source).run(&tx) {
            tx.send(Decoded::Failed(e)).unwrap();
        }
        Intro::receiving(rx, now)
    }

    fn secs(s: f32) -> Duration {
        Duration::from_secs_f32(s)
    }

    #[test]
    fn pictures_show_by_their_time_and_late_ones_are_passed_over() {
        let t0 = Instant::now();
        let mut intro = fed(FakeSource::second(), t0);
        // The sound first, and the clock starts: picture 0 is due.
        let u = intro.update(t0);
        let sound = u.sound.expect("the sound");
        assert!((sound.duration() - 0.1).abs() < 1e-3);
        assert!(u.end.is_none());
        let (p, first) = intro.showing().unwrap();
        assert_eq!(p.time, 0.0);
        assert_eq!(&p.rgba[..4], &[0, 0, 0, 255]);
        // A little later, nothing new is due: the same picture.
        assert!(intro.update(t0 + secs(0.02)).sound.is_none());
        assert_eq!(intro.showing().unwrap().1, first);
        // 0.25 s in: pictures 1 to 7 came due, 7 shows, 1 to 6 were late.
        intro.update(t0 + secs(0.25));
        let (p, second) = intro.showing().unwrap();
        assert!(second > first);
        assert_eq!(p.rgba[0], 7);
        assert_eq!(intro.counts(), (8, 6));
        // Exactly on time, one at a time: none late.
        intro.update(t0 + secs(8.0 / 30.0 + 0.001));
        assert_eq!(intro.showing().unwrap().0.rgba[0], 8);
        assert_eq!(intro.counts(), (9, 6));
    }

    #[test]
    fn the_movie_ends_after_its_last_picture_and_its_sound() {
        let t0 = Instant::now();
        let mut intro = fed(FakeSource::second(), t0);
        intro.update(t0);
        // The last picture (29, at 29/30 s) shows, and stays a moment.
        assert!(intro.update(t0 + secs(0.97)).end.is_none());
        assert_eq!(intro.showing().unwrap().0.rgba[0], 29);
        assert_eq!(intro.update(t0 + secs(1.1)).end, Some(End::Played));
        // A soundtrack longer than the pictures plays out first.
        let long = FakeSource {
            sound_seconds: 2.0,
            ..FakeSource::second()
        };
        let mut intro = fed(long, t0);
        intro.update(t0);
        assert!(intro.update(t0 + secs(1.5)).end.is_none());
        assert_eq!(intro.update(t0 + secs(2.01)).end, Some(End::Played));
        // It stays over.
        assert_eq!(intro.update(t0 + secs(3.0)).end, Some(End::Played));
    }

    #[test]
    fn a_silent_stalled_or_failing_decoder_ends_it() {
        let t0 = Instant::now();
        // Nothing at all: no pictures.
        let none = FakeSource {
            pictures: 0,
            sound_seconds: 0.0,
            ..FakeSource::second()
        };
        let end = fed(none, t0).update(t0).end;
        assert!(matches!(end, Some(End::Failed(_))), "{end:?}");
        // A failure says why.
        let failing = FakeSource {
            fail: Some("no decoder".into()),
            pictures: 2,
            ..FakeSource::second()
        };
        let mut intro = fed(failing, t0);
        // The two pictures, then its word.
        let mut end = None;
        for k in 0..10 {
            end = intro.update(t0 + secs(k as f32 * 0.1)).end;
        }
        assert_eq!(end, Some(End::Failed("no decoder".into())));
    }

    #[test]
    fn a_stalled_decoder_is_given_up_after_the_stall() {
        // On its own thread, as in the lobby: two pictures, then nothing
        // (it holds the channel open).
        let (keep, hold) = mpsc::channel();
        let source = FakeSource {
            pictures: 2,
            hold: Some(hold),
            ..FakeSource::second()
        };
        let t0 = Instant::now();
        let mut intro = Intro::start(Box::new(source), t0).unwrap();
        // Wait for its first word, on the test's clock.
        let deadline = Instant::now() + Duration::from_secs(5);
        while intro.showing().is_none() && Instant::now() < deadline {
            assert!(intro.update(t0).end.is_none());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(intro.showing().is_some());
        // Within the stall it waits; past it, it gives up.
        assert!(intro.update(t0 + STALL - secs(0.1)).end.is_none());
        assert_eq!(intro.update(t0 + STALL + secs(0.1)).end, Some(End::Stalled));
        drop(keep);
    }

    #[test]
    fn letterbox_fits_the_movie_in_the_middle() {
        // A 16:9 movie in a 4:3 window: bars above and below.
        assert_eq!(
            letterbox(1280, 720, 1024.0, 768.0),
            [0.0, 96.0, 1024.0, 672.0]
        );
        // In a 16:9 window: all of it.
        assert_eq!(
            letterbox(1280, 720, 1920.0, 1080.0),
            [0.0, 0.0, 1920.0, 1080.0]
        );
        // In a 21:9 window: bars at the sides.
        assert_eq!(
            letterbox(1280, 720, 2560.0, 1080.0),
            [320.0, 0.0, 2240.0, 1080.0]
        );
        // Nonsense sizes don't divide by nothing.
        let r = letterbox(0, 0, 100.0, 100.0);
        assert!(r.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn small_windows_get_the_small_movie_or_the_other_if_missing() {
        let dir = std::env::temp_dir().join(format!("h2intro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let open: Opener = Arc::new(|_: &Path| Box::new(FakeSource::second()) as _);
        assert!(Plan::find(&dir, open.clone()).is_none());
        std::fs::write(dir.join(FULL), b"").unwrap();
        let plan = Plan::find(&dir, open.clone()).unwrap();
        assert_eq!(plan.file(1080), Some(dir.join(FULL).as_path()));
        assert_eq!(plan.file(480), Some(dir.join(FULL).as_path()));
        std::fs::write(dir.join(LOW), b"").unwrap();
        let plan = Plan::find(&dir, open.clone()).unwrap();
        assert_eq!(plan.file(720), Some(dir.join(FULL).as_path()));
        assert_eq!(plan.file(719), Some(dir.join(LOW).as_path()));
        std::fs::remove_file(dir.join(FULL)).unwrap();
        let plan = Plan::find(&dir, open).unwrap();
        assert_eq!(plan.file(1080), Some(dir.join(LOW).as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_intro_is_skipped_headless_switched_off_or_without_a_movie() {
        let off = Some(OsStr::new("off"));
        assert_eq!(skip_reason(false, None, None, true), None);
        assert_eq!(
            skip_reason(false, Some(OsStr::new("")), Some(true), true),
            None
        );
        assert_eq!(skip_reason(true, None, None, true), Some("headless"));
        assert_eq!(
            skip_reason(false, off, None, true),
            Some("H2LOBBY_INTRO=off")
        );
        assert_eq!(
            skip_reason(false, None, Some(false), true),
            Some("intro = off in lobby.txt")
        );
        assert_eq!(
            skip_reason(false, None, None, false),
            Some("no intro movie")
        );
    }

    #[test]
    fn there_is_a_movie_player_only_on_windows() {
        assert_eq!(opener().is_some(), cfg!(windows));
    }
}
