//! Where MCC might be installed, in the order we try: `--mcc`, the
//! environment, Steam's library list (libraries that hold MCC's app id
//! first), Steam itself, then the usual folders HaloX tries. The Windows
//! side keeps the first candidate that has `halo2\halo2.dll`.

use crate::vdf::{Library, MCC_APP_ID};

/// MCC's folder inside a Steam library.
pub const MCC_FOLDER: &str = r"steamapps\common\Halo The Master Chief Collection";

/// Library folders tried when Steam's own list gives nothing (HaloX's
/// list, plus nothing else).
pub const FALLBACK_LIBRARIES: [&str; 7] = [
    r"C:\Program Files (x86)\Steam",
    r"D:\SteamLibrary",
    r"D:\Steam",
    r"E:\SteamLibrary",
    r"E:\Steam",
    r"F:\SteamLibrary",
    r"G:\SteamLibrary",
];

/// Environment variables naming the MCC folder.
pub const ENV_VARS: [&str; 2] = ["H2LAUNCH_MCC_ROOT", "HALOX_MCC_ROOT"];

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub path: String,
    /// Where it came from, for the log.
    pub source: String,
}

/// `a\b` without doubling the backslash.
pub fn join(a: &str, b: &str) -> String {
    let a = a.trim_end_matches(['\\', '/']);
    let b = b.trim_start_matches(['\\', '/']);
    if a.is_empty() {
        b.to_string()
    } else {
        format!("{a}\\{b}")
    }
}

/// Accepts the MCC folder, its `halo2` folder or halo2.dll itself, with
/// or without quotes and trailing slashes.
pub fn normalize_root(p: &str) -> String {
    let mut s = p.trim().trim_matches('"').replace('/', "\\");
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    for tail in [r"\halo2\halo2.dll", r"\halo2"] {
        if s.len() > tail.len() && s.to_ascii_lowercase().ends_with(tail) {
            s.truncate(s.len() - tail.len());
            break;
        }
    }
    s
}

pub fn candidates(
    cli: Option<&str>,
    env: &[(String, String)],
    steam_roots: &[String],
    libraries: &[Library],
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut add = |path: String, source: String| {
        if path.is_empty() {
            return;
        }
        let path = normalize_root(&path);
        if !out.iter().any(|c| c.path.eq_ignore_ascii_case(&path)) {
            out.push(Candidate { path, source });
        }
    };
    if let Some(p) = cli {
        add(p.to_string(), "--mcc".into());
    }
    for (name, value) in env {
        add(value.clone(), format!("environment {name}"));
    }
    let (with_mcc, others): (Vec<&Library>, Vec<&Library>) = libraries
        .iter()
        .partition(|l| l.apps.iter().any(|a| a == MCC_APP_ID));
    for l in with_mcc {
        add(
            join(&l.path, MCC_FOLDER),
            format!("Steam library (lists app {MCC_APP_ID})"),
        );
    }
    for l in others {
        add(join(&l.path, MCC_FOLDER), "Steam library".into());
    }
    for s in steam_roots {
        add(join(s, MCC_FOLDER), "Steam folder".into());
    }
    for f in FALLBACK_LIBRARIES {
        add(join(f, MCC_FOLDER), "usual folder".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_and_normalizes() {
        assert_eq!(join(r"C:\a\", r"\b"), r"C:\a\b");
        assert_eq!(join("", "b"), "b");
        assert_eq!(normalize_root(r#""C:\MCC\""#), r"C:\MCC");
        assert_eq!(normalize_root(r"C:\MCC\halo2"), r"C:\MCC");
        assert_eq!(normalize_root(r"C:\MCC\HALO2\halo2.dll"), r"C:\MCC");
        assert_eq!(normalize_root("C:/MCC/halo2/"), r"C:\MCC");
        assert_eq!(normalize_root(r"C:\"), r"C:\");
    }

    #[test]
    fn order() {
        let libs = vec![
            Library {
                path: r"D:\SteamLibrary".into(),
                apps: vec!["440".into()],
            },
            Library {
                path: r"C:\Program Files (x86)\Steam".into(),
                apps: vec![MCC_APP_ID.into()],
            },
        ];
        let env = vec![("HALOX_MCC_ROOT".to_string(), r"X:\mcc".to_string())];
        let c = candidates(
            Some(r"Y:\mcc\halo2"),
            &env,
            &[r"C:\Program Files (x86)\Steam".into()],
            &libs,
        );
        let paths: Vec<&str> = c.iter().map(|c| c.path.as_str()).collect();
        let mcc = |p: &str| join(p, MCC_FOLDER);
        assert_eq!(paths[0], r"Y:\mcc");
        assert_eq!(paths[1], r"X:\mcc");
        // The library that lists MCC's app id comes before the other one.
        assert_eq!(paths[2], mcc(r"C:\Program Files (x86)\Steam"));
        assert_eq!(paths[3], mcc(r"D:\SteamLibrary"));
        // No duplicates from the Steam folder and the fallback list.
        assert_eq!(
            paths
                .iter()
                .filter(|p| p.eq_ignore_ascii_case(&mcc(r"C:\Program Files (x86)\Steam")))
                .count(),
            1
        );
        assert!(paths.contains(&mcc(r"G:\SteamLibrary").as_str()));
        assert_eq!(c[0].source, "--mcc");
        assert_eq!(
            c.last().unwrap().path,
            mcc(r"G:\SteamLibrary"),
            "fallbacks come last"
        );
    }
}
