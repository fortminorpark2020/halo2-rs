//! Command-line flags. Every flag takes `--flag value` or `--flag=value`.

use crate::options::RawWrite;
use crate::profile::PadMap;

pub const USAGE: &str = "\
h2launch: starts MCC's classic Halo 2 engine from your MCC install, without
MCC, for an offline Slayer match on Lockout.

Usage: h2launch [flags]

  --check                 Check the install and print what a launch would use;
                          does not start the engine.
  --mcc <folder>          MCC's folder (the one holding halo2\\halo2.dll), if it
                          is not found by itself.
  --map <name>            Map to play (lockout by default). Names as in the
                          map list: lockout, midship, zanzibar, ...
  --variant <file>        Game variant .bin. A bare name is looked up in
                          halo2\\hopper_game_variants (default 01_slayer.bin).
  --no-variant            Start without a game variant (testing only; by
                          default a variant that cannot be loaded stops the
                          launch).
  --name <gamertag>       Player name (default Player).
  --xuid <number>         Fixed player id instead of a random one.
  --windowed <W>x<H>      Window size (default 1280x720).
  --quit-after <seconds>  Ask the engine to quit after this long, then exit.
  --screenshot <s,s,...>  Save the picture as a PNG at these times (seconds
                          from start) in the log folder.
  --input-script <file>   Timed fake input for unattended tests.
  --groundhog             Also load groundhog.dll the way MCC does.
  --attach-input          Share the window's keyboard input with the game
                          thread (default on).
  --no-attach-input       Do not.
  --host-fonts            Tell the engine the host draws text (setting 6).
                          Fonts are not served yet, so the font calls still
                          answer no; they are logged.
  --pad-map <zero|h2>     Gamepad mapping handed to the engine (default zero).
  --set-option <o>=<t>:<v>   Write a value into the game options before the
                          start, e.g. 0x03=u8:0 (repeatable; for testing).
  --set-profile <o>=<t>:<v>  The same for the player profile.
  --no-watchdog           Do not end the run when the window thread stops
                          responding for 30 s (for a debugger session).
  --diag                  More detail in the log: dumps of the options, the
                          variant copy's effect, new calling threads, every
                          event-manager slot's first call, the input sent to
                          the engine each second, and two read-only engine
                          values about keyboard polling.
  --help                  This text.
  --version               The build.

The log is %LOCALAPPDATA%\\h2launch\\h2launch.log
(PowerShell: $env:LOCALAPPDATA\\h2launch\\h2launch.log).";

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub help: bool,
    pub version: bool,
    pub check: bool,
    pub mcc: Option<String>,
    pub map: Option<String>,
    pub variant: Option<String>,
    pub no_variant: bool,
    pub name: String,
    pub xuid: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub quit_after: Option<f64>,
    pub screenshots: Vec<f64>,
    pub input_script: Option<String>,
    pub groundhog: bool,
    pub attach_input: bool,
    pub host_fonts: bool,
    pub pad_map: PadMap,
    pub set_option: Vec<RawWrite>,
    pub set_profile: Vec<RawWrite>,
    pub diag: bool,
    pub watchdog: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            help: false,
            version: false,
            check: false,
            mcc: None,
            map: None,
            variant: None,
            no_variant: false,
            name: "Player".into(),
            xuid: None,
            width: 1280,
            height: 720,
            quit_after: None,
            screenshots: Vec::new(),
            input_script: None,
            groundhog: false,
            attach_input: true,
            host_fonts: false,
            pad_map: PadMap::Zero,
            set_option: Vec::new(),
            set_profile: Vec::new(),
            diag: false,
            watchdog: true,
        }
    }
}

fn seconds(flag: &str, v: &str) -> Result<f64, String> {
    match v.trim().parse::<f64>() {
        Ok(s) if s.is_finite() && s >= 0.0 => Ok(s),
        _ => Err(format!("{flag}: {v:?} is not a number of seconds")),
    }
}

fn size(v: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--windowed: {v:?} should look like 1280x720");
    let (w, h) = v.trim().split_once(['x', 'X']).ok_or_else(bad)?;
    let w: u32 = w.trim().parse().map_err(|_| bad())?;
    let h: u32 = h.trim().parse().map_err(|_| bad())?;
    if !(320..=16384).contains(&w) || !(200..=16384).contains(&h) {
        return Err(format!("--windowed: {w}x{h} is out of range"));
    }
    Ok((w, h))
}

/// Parses the arguments after the program name.
pub fn parse<I, S>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut a = Args::default();
    let list: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    let mut i = 0;
    while i < list.len() {
        let raw = &list[i];
        i += 1;
        let (flag, inline) = match raw.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (raw.clone(), None),
        };
        let switch = |a: &mut Args, set: fn(&mut Args)| -> Result<(), String> {
            if inline.is_some() {
                return Err(format!("{flag} takes no value"));
            }
            set(a);
            Ok(())
        };
        let mut value = || -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            let v = list
                .get(i)
                .ok_or_else(|| format!("{flag} needs a value"))?
                .clone();
            i += 1;
            Ok(v)
        };
        match flag.as_str() {
            "--help" | "-h" | "/?" => switch(&mut a, |a| a.help = true)?,
            "--version" => switch(&mut a, |a| a.version = true)?,
            "--check" => switch(&mut a, |a| a.check = true)?,
            "--groundhog" => switch(&mut a, |a| a.groundhog = true)?,
            "--attach-input" => switch(&mut a, |a| a.attach_input = true)?,
            "--no-attach-input" => switch(&mut a, |a| a.attach_input = false)?,
            "--host-fonts" => switch(&mut a, |a| a.host_fonts = true)?,
            "--diag" => switch(&mut a, |a| a.diag = true)?,
            "--no-variant" => switch(&mut a, |a| a.no_variant = true)?,
            "--no-watchdog" => switch(&mut a, |a| a.watchdog = false)?,
            "--mcc" => a.mcc = Some(value()?),
            "--map" => {
                let v = value()?;
                if crate::maps::find(&v).is_none() {
                    return Err(format!("--map: no map called {v:?}"));
                }
                a.map = Some(v);
            }
            "--variant" => a.variant = Some(value()?),
            "--name" => {
                let v = value()?;
                let n = v.chars().count();
                if n == 0 || n > 15 {
                    return Err(format!("--name: {v:?} should be 1 to 15 characters"));
                }
                a.name = v;
            }
            "--xuid" => {
                let v = value()?;
                let x = crate::util::parse_u64(&v)
                    .filter(|&x| x != 0)
                    .ok_or_else(|| format!("--xuid: {v:?} is not a non-zero number"))?;
                a.xuid = Some(x);
            }
            "--windowed" => (a.width, a.height) = size(&value()?)?,
            "--quit-after" => a.quit_after = Some(seconds("--quit-after", &value()?)?),
            "--screenshot" => {
                let v = value()?;
                for part in v.split(',').filter(|p| !p.trim().is_empty()) {
                    a.screenshots.push(seconds("--screenshot", part)?);
                }
                a.screenshots.sort_by(f64::total_cmp);
            }
            "--input-script" => a.input_script = Some(value()?),
            "--pad-map" => {
                let v = value()?;
                a.pad_map = match v.to_ascii_lowercase().as_str() {
                    "zero" => PadMap::Zero,
                    "h2" | "halo2" => PadMap::Halo2,
                    _ => return Err(format!("--pad-map: {v:?} should be zero or h2")),
                };
            }
            "--set-option" => {
                let w = RawWrite::parse(&value()?)?;
                if !w.fits(crate::options::SIZE) {
                    return Err(format!("--set-option: {} is past the end", w.text));
                }
                a.set_option.push(w);
            }
            "--set-profile" => {
                let w = RawWrite::parse(&value()?)?;
                if !w.fits(crate::profile::PROFILE_SIZE) {
                    return Err(format!("--set-profile: {} is past the end", w.text));
                }
                a.set_profile.push(w);
            }
            other => return Err(format!("unknown flag {other:?} (see --help)")),
        }
    }
    Ok(a)
}

/// Makes the paths the user gave absolute against the current folder,
/// because the launcher changes its working folder to the MCC folder
/// before it reads them: `--variant` when it names a path (a bare name is
/// looked up in the hopper folder) and `--input-script`. (`--mcc` is made
/// absolute where the MCC folder is chosen.)
pub fn absolutize_paths(a: &mut Args) {
    let abs = |p: &str| -> String {
        std::path::absolute(p)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string())
    };
    if let Some(v) = a.variant.as_deref() {
        if v.contains(['\\', '/']) {
            a.variant = Some(abs(v));
        }
    }
    if let Some(sc) = a.input_script.as_deref() {
        a.input_script = Some(abs(sc));
    }
}

/// The variant file to load: a bare name goes in the hopper variant
/// folder, `.bin` is added when missing, a path is used as given.
pub fn variant_path(mcc_root: &str, variant: Option<&str>) -> String {
    let v = variant.unwrap_or("01_slayer");
    let has_dir = v.contains(['\\', '/']);
    let file = if v.to_ascii_lowercase().ends_with(".bin") {
        v.to_string()
    } else {
        format!("{v}.bin")
    };
    if has_dir {
        file
    } else {
        crate::mccroot::join(mcc_root, &format!(r"halo2\hopper_game_variants\{file}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let a = parse(Vec::<String>::new()).unwrap();
        assert_eq!(a, Args::default());
        assert_eq!(a.name, "Player");
        assert!(a.attach_input);
        assert!(a.watchdog);
        assert_eq!((a.width, a.height), (1280, 720));
        assert_eq!(a.pad_map, PadMap::Zero);
    }

    #[test]
    fn every_flag() {
        let a = parse([
            "--check",
            "--mcc",
            r"D:\MCC",
            "--map=midship",
            "--variant",
            "02_team_slayer",
            "--name",
            "John",
            "--xuid",
            "0x1234",
            "--windowed",
            "1920x1080",
            "--quit-after",
            "120",
            "--screenshot",
            "90,30,60",
            "--input-script",
            "walk.txt",
            "--groundhog",
            "--no-attach-input",
            "--host-fonts",
            "--pad-map",
            "h2",
            "--set-option",
            "0x03=u8:0",
            "--set-profile=0x1B5=u8:5",
            "--diag",
            "--no-variant",
            "--no-watchdog",
        ])
        .unwrap();
        assert!(!a.watchdog);
        assert!(a.check && a.groundhog && a.host_fonts && a.diag && a.no_variant);
        assert!(!a.attach_input);
        assert_eq!(a.mcc.as_deref(), Some(r"D:\MCC"));
        assert_eq!(a.map.as_deref(), Some("midship"));
        assert_eq!(a.variant.as_deref(), Some("02_team_slayer"));
        assert_eq!(a.name, "John");
        assert_eq!(a.xuid, Some(0x1234));
        assert_eq!((a.width, a.height), (1920, 1080));
        assert_eq!(a.quit_after, Some(120.0));
        assert_eq!(a.screenshots, vec![30.0, 60.0, 90.0]);
        assert_eq!(a.input_script.as_deref(), Some("walk.txt"));
        assert_eq!(a.pad_map, PadMap::Halo2);
        assert_eq!(a.set_option[0].offset, 3);
        assert_eq!(a.set_option[0].bytes, vec![0]);
        assert_eq!(a.set_profile[0].offset, 0x1B5);
    }

    #[test]
    fn errors() {
        for bad in [
            vec!["--nope"],
            vec!["--map"],
            vec!["--map", "nowhere"],
            vec!["--windowed", "big"],
            vec!["--windowed", "10x10"],
            vec!["--quit-after", "-1"],
            vec!["--quit-after", "soon"],
            vec!["--screenshot", "1,x"],
            vec!["--xuid", "0"],
            vec!["--name", ""],
            vec!["--name", "a name far too long"],
            vec!["--pad-map", "mine"],
            vec!["--check=yes"],
            vec!["--set-option", "0x2BF30=u8:1"],
            vec!["--set-profile", "0xACB=u16:1"],
        ] {
            assert!(parse(&bad).is_err(), "{bad:?} should fail");
        }
        // The last of a repeated flag wins; switches can be undone.
        let a = parse(["--no-attach-input", "--attach-input"]).unwrap();
        assert!(a.attach_input);
    }

    #[test]
    fn variants() {
        let root = r"C:\MCC";
        assert_eq!(
            variant_path(root, None),
            r"C:\MCC\halo2\hopper_game_variants\01_slayer.bin"
        );
        assert_eq!(
            variant_path(root, Some("02_team_slayer.BIN")),
            r"C:\MCC\halo2\hopper_game_variants\02_team_slayer.BIN"
        );
        assert_eq!(variant_path(root, Some(r"D:\v\x.bin")), r"D:\v\x.bin");
        assert_eq!(variant_path(root, Some(r"v/x")), r"v/x.bin");
    }

    #[test]
    fn relative_paths_become_absolute() {
        let mut a = parse(["--variant", "v/x.bin", "--input-script", "walk.txt"]).unwrap();
        absolutize_paths(&mut a);
        let cwd = std::env::current_dir().unwrap();
        let v = a.variant.unwrap();
        assert!(std::path::Path::new(&v).is_absolute(), "{v}");
        assert!(v.starts_with(&*cwd.to_string_lossy()), "{v}");
        assert!(std::path::Path::new(a.input_script.as_deref().unwrap()).is_absolute());
        // A bare variant name stays a name (it is looked up in the hopper folder).
        let mut b = parse(["--variant", "02_team_slayer"]).unwrap();
        absolutize_paths(&mut b);
        assert_eq!(b.variant.as_deref(), Some("02_team_slayer"));
    }
}
