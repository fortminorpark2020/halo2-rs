//! Halo 2's menu music and sounds in the lobby (menu.md section 5, phase
//! 2.2), read from Halo 2 Vista's mainmenu.map (Project Cartographer's) on
//! a thread of their own and played through `audio` as `App` asks: the
//! cues it gives (`App::take_cues`) and whether the music is wanted
//! (`App::music_on`). Without Vista's files, with `H2LOBBY_SOUND=off` or
//! with `menu_sound = off` in `lobby.txt`, the lobby is silent and never
//! opens an audio device. Nothing plays while a match's engine runs.

use super::audio::{Audio, Clip, Output};
use super::settings::MenuPrefs;
use super::sources::Vista;
use crate::live::Log;
use blam_cache::mapset::MapSet;
use blam_cache::sound::{self, SoundReader};
use blam_cache::ui::{Menus, Sounds};
use blam_cache::{DatumIndex, GroupTag};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `off` silences the menus.
pub const ENV: &str = "H2LOBBY_SOUND";
/// A file to record the menus' sound into (a WAV) instead of playing it.
pub const WAV_ENV: &str = "H2LOBBY_AUDIO_WAV";
/// Menu sounds over the music: the from-scratch game's gain for them
/// (`origin/menu-preview`'s soundscape), an estimate.
const UI_GAIN: f32 = 1.6;
/// The music's gain: the from-scratch game's (`origin/menu-preview`), an
/// estimate. As there, it is the music's whole gain: its looping sound's
/// track gain (-6 dB in Halo 2 Vista's) and its sounds' own aren't
/// applied on top.
const MUSIC_GAIN: f32 = 0.5;
/// How long the music takes to fade when a match's engine starts, before
/// the audio stream pauses: an estimate.
const ENGINE_FADE: Duration = Duration::from_millis(500);
/// The music's fade when the globals give none: theirs in Halo 2 Vista's
/// mainmenu.map.
const MUSIC_FADE_MS: i32 = 5500;
/// The music's two tracks by name, where the globals' looping sound
/// doesn't give them.
const MUSIC_IN: &str = r"sound\ui\main_menu_music\menu_music\in";
const MUSIC_LOOP: &str = r"sound\ui\main_menu_music\menu_music\loop";

/// What the lobby asks to hear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// The focus moved.
    Cursor,
    /// A choice was made (a screen or popup opens or closes).
    Select,
    /// Back (B).
    Back,
    /// A choice that can't be made.
    Error,
    /// A search started, or a custom game was asked for.
    Advance,
    /// An invitation arrived.
    Notify,
    /// A second of a countdown (for later).
    Countdown,
    /// A key of the on-screen keyboard (for later).
    KeyClick,
}

impl Cue {
    pub const ALL: [Cue; 8] = [
        Cue::Cursor,
        Cue::Select,
        Cue::Back,
        Cue::Error,
        Cue::Advance,
        Cue::Notify,
        Cue::Countdown,
        Cue::KeyClick,
    ];

    fn index(self) -> usize {
        Cue::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }

    fn name(self) -> &'static str {
        match self {
            Cue::Cursor => "cursor",
            Cue::Select => "select",
            Cue::Back => "back",
            Cue::Error => "error",
            Cue::Advance => "advance",
            Cue::Notify => "message",
            Cue::Countdown => "countdown",
            Cue::KeyClick => "key click",
        }
    }

    /// The menus' globals' sound for it, by tag name (menu.md section 5).
    fn sound(self, s: &Sounds) -> Option<&str> {
        match self {
            Cue::Cursor => s.cursor.as_deref(),
            Cue::Select => s.select.as_deref(),
            Cue::Back => s.retreat.as_deref(),
            Cue::Error => s.error.as_deref(),
            Cue::Advance => s.advance.as_deref(),
            Cue::Notify => s.receive_message(),
            Cue::Countdown => s.countdown(),
            Cue::KeyClick => s.keyboard_click(),
        }
    }
}

/// A sound decoded, with its tag's gain.
pub struct Effect {
    pub clip: Arc<Clip>,
    pub gain: f32,
}

/// The main menu's music: its start, then its loops one after another
/// (picked at random where there are several).
pub struct Music {
    pub intro: Arc<Clip>,
    pub loops: Vec<Arc<Clip>>,
}

/// The menus' sounds by `Cue`, the music, and how long it fades.
#[derive(Default)]
pub struct MenuSounds {
    pub cues: [Option<Effect>; 8],
    pub music: Option<Music>,
    pub fade: Duration,
}

impl MenuSounds {
    pub fn is_empty(&self) -> bool {
        self.music.is_none() && self.cues.iter().all(Option::is_none)
    }

    pub fn cue(&self, c: Cue) -> Option<&Effect> {
        self.cues[c.index()].as_ref()
    }
}

/// Decibels to a gain.
fn gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Read the menus' sounds and music from `mainmenu` (Halo 2 Vista's, with
/// shared.map beside it). Never panics: anything that fails gives empty
/// sounds. Logs one line: what loaded, how long, and how long decoding
/// took; or why nothing did.
pub fn load(mainmenu: &Path, log: &Log) -> MenuSounds {
    let started = Instant::now();
    let read = std::panic::catch_unwind(|| read(mainmenu));
    let ms = started.elapsed().as_millis();
    match read {
        Ok(Ok((s, missing))) if !s.is_empty() => {
            log(&describe(mainmenu, &s, &missing, ms));
            s
        }
        Ok(Ok(_)) => {
            log(&format!(
                "sound: no menu sounds or music read from {}; the menus are silent",
                mainmenu.display()
            ));
            MenuSounds::default()
        }
        Ok(Err(e)) => {
            log(&format!(
                "sound: none from {} ({e}); the menus are silent",
                mainmenu.display()
            ));
            MenuSounds::default()
        }
        Err(_) => {
            log(&format!(
                "sound: reading {} failed; the menus are silent",
                mainmenu.display()
            ));
            MenuSounds::default()
        }
    }
}

/// The log line for sounds that loaded.
fn describe(mainmenu: &Path, s: &MenuSounds, missing: &[String], ms: u128) -> String {
    let cues: Vec<String> = Cue::ALL
        .iter()
        .filter_map(|&c| {
            let e = s.cue(c)?;
            Some(format!("{} {:.2} s", c.name(), e.clip.duration()))
        })
        .collect();
    let music = match &s.music {
        Some(m) => {
            let loops: Vec<String> = m
                .loops
                .iter()
                .map(|l| format!("{:.1} s", l.duration()))
                .collect();
            format!(
                "music {:.1} s then loops of {} (fade {:.1} s)",
                m.intro.duration(),
                if loops.is_empty() {
                    "none".to_string()
                } else {
                    loops.join(", ")
                },
                s.fade.as_secs_f32()
            )
        }
        None => "no music".into(),
    };
    let missing = match missing.is_empty() {
        true => String::new(),
        false => format!("; not read: {}", missing.join(", ")),
    };
    format!(
        "sound: from {}: {}; {music}; decoded in {ms} ms{missing}",
        mainmenu.display(),
        cues.join(", ")
    )
}

/// The tag of `group` called `name` in the map's tag table (the same in
/// every map), if there is one.
fn find(set: &MapSet, group: &str, name: &str) -> Option<DatumIndex> {
    let group = GroupTag::parse(group)?;
    set.map
        .tags
        .iter()
        .find(|t| t.group == group && t.name.eq_ignore_ascii_case(name))
        .map(|t| t.datum)
}

/// A sound's variations decoded, and its gain.
fn clips(
    set: &mut MapSet,
    reader: &mut SoundReader,
    datum: DatumIndex,
) -> Result<(Vec<Arc<Clip>>, f32), String> {
    let s = reader.read(set, datum).map_err(|e| e.to_string())?;
    if s.permutations.is_empty() {
        return Err(format!("{} didn't decode", s.name));
    }
    let (channels, rate) = (s.channels, s.sample_rate);
    let clips = s
        .permutations
        .into_iter()
        .map(|samples| {
            Arc::new(Clip {
                samples,
                channels,
                rate,
            })
        })
        .collect();
    Ok((clips, gain(s.gain_db)))
}

/// Everything `load` reads, and what didn't read (a word or two each).
fn read(mainmenu: &Path) -> Result<(MenuSounds, Vec<String>), String> {
    let menus = Menus::open(mainmenu).map_err(|e| e.to_string())?;
    if menus.mcc {
        return Err("MCC's maps' sounds can't be read yet".into());
    }
    let globals = menus.ui.map_err(|e| e.to_string())?.globals;
    let mut set = MapSet::open(mainmenu).map_err(|e| e.to_string())?;
    let mut reader = SoundReader::new();
    let mut out = MenuSounds {
        fade: Duration::from_millis(match globals.music_fade_ms {
            ms if ms > 0 => ms as u64,
            _ => MUSIC_FADE_MS as u64,
        }),
        ..MenuSounds::default()
    };
    let mut missing = Vec::new();
    for c in Cue::ALL {
        let Some(name) = c.sound(&globals.sounds) else {
            missing.push(format!("{} (none named)", c.name()));
            continue;
        };
        let Some(datum) = find(&set, "snd!", name) else {
            missing.push(format!("{} (no such sound)", c.name()));
            continue;
        };
        match clips(&mut set, &mut reader, datum) {
            Ok((clips, gain)) => {
                let clip = clips.into_iter().next().expect("not empty");
                out.cues[c.index()] = Some(Effect { clip, gain });
            }
            Err(e) => missing.push(format!("{} ({e})", c.name())),
        }
    }
    let lsnd = globals.music.as_deref().and_then(|n| find(&set, "lsnd", n));
    out.music = match lsnd.map(|d| music_of(&mut set, &mut reader, d)) {
        Some(Ok(m)) => Some(m),
        other => {
            if let Some(Err(e)) = other {
                missing.push(format!("the music's looping sound ({e})"));
            }
            match music_by_name(&mut set, &mut reader) {
                Ok(m) => Some(m),
                Err(e) => {
                    missing.push(format!("music ({e})"));
                    None
                }
            }
        }
    };
    Ok((out, missing))
}

/// The music of looping sound `lsnd`: its first track's start, then its
/// loop's variations.
fn music_of(set: &mut MapSet, reader: &mut SoundReader, lsnd: DatumIndex) -> Result<Music, String> {
    let l = sound::looping_sound(set, lsnd).map_err(|e| e.to_string())?;
    let track = *l.tracks.first().ok_or("no tracks")?;
    let start = track.start.ok_or("no start")?;
    let (intro, _) = clips(set, reader, start)?;
    let loops = match track.repeat {
        Some(r) => clips(set, reader, r)?.0,
        None => Vec::new(),
    };
    Ok(Music {
        intro: intro[0].clone(),
        loops,
    })
}

/// The music by its tracks' names.
fn music_by_name(set: &mut MapSet, reader: &mut SoundReader) -> Result<Music, String> {
    let start = find(set, "snd!", MUSIC_IN).ok_or("no start by name")?;
    let (intro, _) = clips(set, reader, start)?;
    let loops = match find(set, "snd!", MUSIC_LOOP) {
        Some(r) => clips(set, reader, r)?.0,
        None => Vec::new(),
    };
    Ok(Music {
        intro: intro[0].clone(),
        loops,
    })
}

/// Why the menus are silent, if they are: no Halo 2 Vista files,
/// `H2LOBBY_SOUND=off` (`env`), or `menu_sound = off` in `lobby.txt`.
pub fn silent(vista: bool, env: Option<&OsStr>, prefs: &MenuPrefs) -> Option<&'static str> {
    if env.is_some_and(|v| v.eq_ignore_ascii_case("off")) {
        Some("H2LOBBY_SOUND=off")
    } else if prefs.sound == Some(false) {
        Some("menu_sound = off in lobby.txt")
    } else if !vista {
        Some("no Halo 2 Vista files")
    } else {
        None
    }
}

/// Where the sound goes: a WAV file when `wav` (`H2LOBBY_AUDIO_WAV`)
/// names one, else the audio device with a window, else nowhere.
pub fn output(window: bool, wav: Option<&OsStr>) -> Output {
    match wav.filter(|w| !w.is_empty()) {
        Some(w) => Output::Wav(PathBuf::from(w)),
        None if window => Output::Device,
        None => Output::Null,
    }
}

/// The lobby's sound: the sounds loading or loaded, and the output, which
/// opens only once they have come.
pub struct MenuSound {
    log: Log,
    loading: Option<Receiver<MenuSounds>>,
    sounds: MenuSounds,
    output: Output,
    audio: Option<Audio>,
    /// The music's voice, and whether it is wanted (playing, or coming
    /// back) rather than fading out.
    music: Option<u64>,
    music_wanted: bool,
    /// An engine runs: the music fades, cues are dropped, and the stream
    /// pauses at `pause_at`.
    engine: bool,
    pause_at: Option<Instant>,
}

impl MenuSound {
    /// Silent, with nothing to load (tests, and the menus switched off).
    pub fn off(log: &Log) -> MenuSound {
        MenuSound {
            log: log.clone(),
            loading: None,
            sounds: MenuSounds::default(),
            output: Output::Null,
            audio: None,
            music: None,
            music_wanted: false,
            engine: false,
            pause_at: None,
        }
    }

    /// Start reading the sounds from `vista`'s mainmenu.map on a thread,
    /// unless the menus are silent (`silent`, with `H2LOBBY_SOUND` and
    /// `prefs`); `window` picks the device as the output, else nowhere
    /// (`output`, with `H2LOBBY_AUDIO_WAV`).
    pub fn start(vista: Option<&Vista>, prefs: &MenuPrefs, window: bool, log: &Log) -> MenuSound {
        let mut s = MenuSound::off(log);
        let env = std::env::var_os(ENV);
        if let Some(why) = silent(vista.is_some(), env.as_deref(), prefs) {
            log(&format!("sound: the menus are silent ({why})"));
            return s;
        }
        let Some(vista) = vista else {
            return s;
        };
        s.output = output(window, std::env::var_os(WAV_ENV).as_deref());
        let map = vista.mainmenu();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_log = log.clone();
        let spawned = std::thread::Builder::new()
            .name("menu sounds".into())
            .spawn(move || {
                let _ = tx.send(load(&map, &thread_log));
            });
        match spawned {
            Ok(_) => s.loading = Some(rx),
            Err(e) => log(&format!("sound: couldn't start reading the sounds: {e}")),
        }
        s
    }

    /// Take the sounds once they have come, and open the output for them.
    fn poll(&mut self) {
        let Some(rx) = &self.loading else {
            return;
        };
        let sounds = match rx.try_recv() {
            Ok(s) => s,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => MenuSounds::default(),
        };
        self.loading = None;
        if sounds.is_empty() {
            return;
        }
        match Audio::open(&self.output) {
            Ok(a) => {
                let to = match &self.output {
                    Output::Device => "the audio device".to_string(),
                    Output::Null => "nowhere (no window)".to_string(),
                    Output::Wav(p) => p.display().to_string(),
                };
                (self.log)(&format!("sound: playing to {to}"));
                self.audio = Some(a);
                self.sounds = sounds;
            }
            Err(e) => (self.log)(&format!("sound: no sound ({e})")),
        }
    }

    /// Each tick: play `cues`, start or fade the music as `music_on` says,
    /// and while `engine_running` fade it, drop the cues and pause the
    /// stream (it plays again after).
    pub fn update(&mut self, cues: &[Cue], music_on: bool, engine_running: bool) {
        self.poll();
        let Some(audio) = &mut self.audio else {
            return;
        };
        let now = Instant::now();
        if engine_running {
            if !self.engine {
                self.engine = true;
                self.pause_at = Some(now + ENGINE_FADE);
                if let Some(id) = self.music.take() {
                    audio.fade_out(id, ENGINE_FADE.as_secs_f32());
                }
                self.music_wanted = false;
            }
            if self.pause_at.is_some_and(|t| now >= t) {
                self.pause_at = None;
                if let Err(e) = audio.pause(true) {
                    (self.log)(&format!("sound: the stream didn't pause ({e})"));
                }
            }
            return;
        }
        if self.engine {
            self.engine = false;
            self.pause_at = None;
            if let Err(e) = audio.pause(false) {
                (self.log)(&format!("sound: the stream didn't play again ({e})"));
            }
        }
        for &c in cues {
            if let Some(e) = self.sounds.cue(c) {
                audio.play_ui(&e.clip, e.gain * UI_GAIN);
            }
        }
        self.set_music(music_on);
    }

    /// Start the music, bring it back up while it fades, or fade it out.
    fn set_music(&mut self, on: bool) {
        let (Some(audio), Some(m)) = (&mut self.audio, &self.sounds.music) else {
            return;
        };
        let fade = self.sounds.fade.as_secs_f32();
        let target = MUSIC_GAIN;
        match (on, self.music_wanted) {
            (true, false) => {
                // Back up at the speed it was fading, if it still plays.
                let raised = self.music.is_some_and(|id| {
                    let now = audio.gain(id).unwrap_or(0.0);
                    let left = (1.0 - now / target.max(1e-6)).clamp(0.0, 1.0);
                    audio.fade_to(id, target, fade * left)
                });
                if !raised {
                    self.music = Some(audio.play_music(&m.intro, &m.loops, target));
                }
                self.music_wanted = true;
            }
            (false, true) => {
                if let Some(id) = self.music {
                    audio.fade_out(id, fade);
                }
                self.music_wanted = false;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn logged() -> (Log, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let kept = lines.clone();
        let log: Log = Arc::new(move |l: &str| kept.lock().unwrap().push(l.to_string()));
        (log, lines)
    }

    #[test]
    fn a_missing_or_junk_map_gives_no_sounds() {
        let dir = std::env::temp_dir().join(format!("h2sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (log, lines) = logged();
        let s = load(&dir.join("mainmenu.map"), &log);
        assert!(s.is_empty());
        let junk = dir.join("junk.map");
        std::fs::write(&junk, vec![0x5A; 5000]).unwrap();
        assert!(load(&junk, &log).is_empty());
        std::fs::write(&junk, b"head").unwrap();
        assert!(load(&junk, &log).is_empty());
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines.iter().all(|l| l.contains("the menus are silent")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn silent_without_vista_or_when_switched_off() {
        let on = MenuPrefs::default();
        let off = MenuPrefs {
            sound: Some(false),
            ..MenuPrefs::default()
        };
        assert_eq!(silent(true, None, &on), None);
        assert_eq!(silent(true, Some(OsStr::new("")), &on), None);
        assert_eq!(silent(false, None, &on), Some("no Halo 2 Vista files"));
        assert_eq!(
            silent(true, Some(OsStr::new("off")), &on),
            Some("H2LOBBY_SOUND=off")
        );
        assert_eq!(
            silent(true, None, &off),
            Some("menu_sound = off in lobby.txt")
        );
        // Without Vista nothing is read and no output is opened.
        let (log, lines) = logged();
        let mut s = MenuSound::start(None, &on, true, &log);
        assert!(s.loading.is_none());
        s.update(&[Cue::Select], true, false);
        assert!(s.audio.is_none());
        assert_eq!(
            *lines.lock().unwrap(),
            ["sound: the menus are silent (no Halo 2 Vista files)"]
        );
    }

    #[test]
    fn the_output_is_a_wav_the_device_or_nowhere() {
        let wav = Some(OsStr::new("menu.wav"));
        assert_eq!(output(true, wav), Output::Wav("menu.wav".into()));
        assert_eq!(output(false, wav), Output::Wav("menu.wav".into()));
        assert_eq!(output(true, None), Output::Device);
        assert_eq!(output(true, Some(OsStr::new(""))), Output::Device);
        assert_eq!(output(false, None), Output::Null);
    }

    /// Made-up sounds: every cue and a music of a start and a loop.
    fn sounds() -> MenuSounds {
        let clip = |frames: usize| {
            Arc::new(Clip {
                samples: vec![8192; frames],
                channels: 1,
                rate: 100,
            })
        };
        let mut s = MenuSounds {
            fade: Duration::from_millis(5500),
            music: Some(Music {
                intro: clip(100_000),
                loops: vec![clip(100_000)],
            }),
            ..MenuSounds::default()
        };
        for c in Cue::ALL {
            s.cues[c.index()] = Some(Effect {
                clip: clip(1000),
                gain: 1.0,
            });
        }
        s
    }

    /// A MenuSound with `sounds` arrived, playing nowhere but keeping its
    /// voices (a WAV in a scratch folder).
    fn playing(name: &str) -> (MenuSound, PathBuf) {
        let dir = std::env::temp_dir().join(format!("h2sound-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (log, _) = logged();
        let mut s = MenuSound::off(&log);
        s.output = Output::Wav(dir.join("out.wav"));
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(sounds()).unwrap();
        s.loading = Some(rx);
        (s, dir)
    }

    #[test]
    fn music_starts_fades_comes_back_and_stops_for_a_match() {
        let (mut s, dir) = playing("music");
        s.update(&[], true, false);
        let id = s.music.unwrap();
        let a = s.audio.as_ref().unwrap();
        assert_eq!(a.gain(id), Some(MUSIC_GAIN));
        // Cues play over it.
        s.update(&[Cue::Cursor, Cue::Select], true, false);
        assert_eq!(s.audio.as_ref().unwrap().voices(), 3);
        // Off: it fades (over 5.5 s), and on again it comes back up,
        // the same voice.
        s.update(&[], false, false);
        assert!(s.audio.as_ref().unwrap().gain(id).is_some());
        s.update(&[], true, false);
        assert_eq!(s.music, Some(id));
        assert!(s.music_wanted);
        // A match: it fades, cues are dropped, and the stream pauses.
        s.update(&[], true, true);
        assert!(s.music.is_none() && s.engine);
        s.update(&[Cue::Back], false, true);
        let voices = s.audio.as_ref().unwrap().voices();
        s.pause_at = Some(Instant::now());
        s.update(&[Cue::Back], false, true);
        assert_eq!(s.audio.as_ref().unwrap().voices(), voices);
        assert!(s.pause_at.is_none());
        // After it, the music starts again when it's wanted.
        s.update(&[], false, false);
        assert!(!s.engine && s.music.is_none());
        s.update(&[], true, false);
        assert!(s.music.is_some_and(|m| m != id));
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real sounds, from the mainmenu.map in `H2_MAPS` (Halo 2
    /// Vista's, with its shared.map).
    #[test]
    #[ignore]
    fn the_real_menu_sounds_and_music_decode() {
        let dir = std::env::var("H2_MAPS").expect("H2_MAPS");
        let (log, lines) = logged();
        let started = Instant::now();
        let s = load(&Path::new(&dir).join("mainmenu.map"), &log);
        let ms = started.elapsed().as_millis();
        println!("{} ({ms} ms)", lines.lock().unwrap().join("\n"));
        let peak = |c: &Clip| {
            c.samples
                .iter()
                .map(|v| v.unsigned_abs())
                .max()
                .unwrap_or(0)
        };
        for c in [
            Cue::Cursor,
            Cue::Select,
            Cue::Back,
            Cue::Error,
            Cue::Advance,
        ] {
            let e = s.cue(c).unwrap_or_else(|| panic!("{c:?}"));
            assert!(e.clip.duration() > 0.0, "{c:?}");
            let (ch, rate) = (e.clip.channels, e.clip.rate);
            println!(
                "{c:?}: gain {:.2}, peak {}, {ch} ch at {rate}",
                e.gain,
                peak(&e.clip)
            );
        }
        assert!(s.cue(Cue::Notify).is_some());
        let m = s.music.as_ref().expect("music");
        let (ch, rate) = (m.intro.channels, m.intro.rate);
        println!("music: peak {}, {ch} ch at {rate}", peak(&m.intro));
        assert!(m.intro.duration() > 1.0);
        assert!(!m.loops.is_empty() && m.loops.iter().all(|l| l.duration() > 1.0));
        assert_eq!(s.fade, Duration::from_millis(5500));
    }
}
