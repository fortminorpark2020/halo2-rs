//! Where the lobby's Halo 2 files come from (menu.md, "Phase 2
//! (widened)"). Halo 2 Vista, as Project Cartographer installs it, comes
//! first: its `maps\mainmenu.map` (menu art and layouts, rank icons, UI
//! sounds, music and the menu scene), its `maps\fonts` and its `movie`
//! folder (the intro). MCC's own files come second (art, layouts, fonts
//! and rank icons; no music, scene or intro), and the built-in look last.
//!
//! One finder (`find`) decides on the Vista folder once, when the lobby
//! opens: `H2LOBBY_VISTA`, else `vista =` in `lobby.txt`, else the first
//! of the folders Halo 2 Vista is usually in (`ranks::MAP_DIRS`) that
//! holds a mainmenu.map. `H2LOBBY_VISTA=off` shows what friends without
//! Cartographer get. Nothing here reads the files themselves.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The variable that names the Cartographer folder (or its `maps`
/// folder), or `off` to use none.
pub const ENV: &str = "H2LOBBY_VISTA";
/// The map that holds the menus.
const MAIN_MENU: &str = "mainmenu.map";
/// The maps folder inside the Cartographer folder.
const MAPS: &str = "maps";
/// The fonts folder inside the maps folder, and the file that lists them.
const FONTS: &str = "fonts";
const FONT_TABLE: &str = "font_table.txt";
/// The movies folder inside the Cartographer folder.
const MOVIES: &str = "movie";

/// A Halo 2 Vista install (Project Cartographer's): its folder and the
/// maps folder inside it, which holds mainmenu.map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vista {
    pub root: PathBuf,
    pub maps: PathBuf,
}

impl Vista {
    /// The Vista install at `dir`, its folder or its maps folder, if
    /// either holds mainmenu.map (the folder's `maps` first).
    pub fn at(dir: &Path) -> Option<Vista> {
        let inside = dir.join(MAPS);
        let maps = if inside.join(MAIN_MENU).is_file() {
            inside
        } else if dir.join(MAIN_MENU).is_file() {
            dir.to_path_buf()
        } else {
            return None;
        };
        let root = match maps.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => maps.clone(),
        };
        Some(Vista { root, maps })
    }

    /// Its mainmenu.map.
    pub fn mainmenu(&self) -> PathBuf {
        self.maps.join(MAIN_MENU)
    }

    /// Its fonts folder, when that holds a `font_table.txt` (the same
    /// format as MCC's `halo2\h2_fonts`, read by `blam_cache::font`).
    pub fn fonts(&self) -> Option<PathBuf> {
        let dir = self.maps.join(FONTS);
        dir.join(FONT_TABLE).is_file().then_some(dir)
    }

    /// Its movies folder (the intro is `intro_60.wmv` there).
    pub fn movies(&self) -> PathBuf {
        self.root.join(MOVIES)
    }
}

/// The maps folders Halo 2 Vista is usually in (`ranks::MAP_DIRS`), in
/// the order they are tried.
pub fn usual_dirs() -> Vec<&'static Path> {
    super::ranks::MAP_DIRS.iter().map(Path::new).collect()
}

/// What a folder setting says: `off`, a folder, or nothing (unset or
/// empty).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named<'a> {
    Unset,
    Off,
    Folder(&'a Path),
}

impl<'a> Named<'a> {
    pub fn read(value: Option<&'a OsStr>) -> Named<'a> {
        match value.filter(|v| !v.is_empty()) {
            None => Named::Unset,
            Some(v) if v == "off" => Named::Off,
            Some(v) => Named::Folder(Path::new(v)),
        }
    }
}

/// Whether Halo 2 Vista's files are switched off: `H2LOBBY_VISTA=off`, or
/// (with the variable unset) `vista = off` in `lobby.txt`. Then nothing
/// from a Vista folder is used, not even where `ranks::MAP_DIRS` would
/// find one.
pub fn off(env: Option<&OsStr>, saved: Option<&Path>) -> bool {
    match Named::read(env) {
        Named::Off => true,
        Named::Folder(_) => false,
        Named::Unset => Named::read(saved.map(Path::as_os_str)) == Named::Off,
    }
}

/// The Halo 2 Vista install to use, in this order:
///
/// 1. `env` (`H2LOBBY_VISTA`): the Cartographer folder, or its maps
///    folder; `off` gives None. Empty counts as unset.
/// 2. `saved` (`vista =` in `lobby.txt`), in the same forms.
/// 3. The first of `dirs` (`ranks::MAP_DIRS`, maps folders) that holds a
///    mainmenu.map.
///
/// A folder named in 1 or 2 that holds no mainmenu.map (nor its `maps`
/// folder) is passed over for the next; `named_but_missing` says which,
/// for the log.
pub fn find(env: Option<&OsStr>, saved: Option<&Path>, dirs: &[&Path]) -> Option<Vista> {
    if off(env, saved) {
        return None;
    }
    let named = [Named::read(env), Named::read(saved.map(Path::as_os_str))];
    for n in named {
        if let Named::Folder(dir) = n {
            if let Some(v) = Vista::at(dir) {
                return Some(v);
            }
        }
    }
    dirs.iter().find_map(|d| Vista::at(d))
}

/// The folders `env` and `saved` name that `find` passed over because
/// neither they nor their `maps` folder hold a mainmenu.map, with the
/// setting that named each: for the log.
pub fn named_but_missing<'a>(
    env: Option<&'a OsStr>,
    saved: Option<&'a Path>,
) -> Vec<(&'static str, &'a Path)> {
    if off(env, saved) {
        return Vec::new();
    }
    let named = [
        (ENV, Named::read(env)),
        ("lobby.txt's vista", Named::read(saved.map(Path::as_os_str))),
    ];
    named
        .into_iter()
        .filter_map(|(what, n)| match n {
            Named::Folder(dir) if Vista::at(dir).is_none() => Some((what, dir)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of our own for one test, empty.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h2sources-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A Cartographer folder at `root`: `maps\mainmenu.map` (not a real
    /// map; nothing here reads it).
    fn cartographer(root: &Path) -> PathBuf {
        let maps = root.join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::write(maps.join("mainmenu.map"), b"head").unwrap();
        maps
    }

    #[test]
    fn the_variable_names_the_folder_or_its_maps() {
        let dir = folder("env");
        let root = dir.join("Halo 2 Project Cartographer");
        let maps = cartographer(&root);
        let v = Vista {
            root: root.clone(),
            maps: maps.clone(),
        };
        assert_eq!(find(Some(root.as_os_str()), None, &[]).as_ref(), Some(&v));
        assert_eq!(find(Some(maps.as_os_str()), None, &[]).as_ref(), Some(&v));
        assert_eq!(v.mainmenu(), maps.join("mainmenu.map"));
        assert_eq!(v.movies(), root.join("movie"));
        // Over lobby.txt's folder and the usual ones.
        let other = dir.join("other");
        let other_maps = cartographer(&other);
        let found = find(Some(root.as_os_str()), Some(&other), &[&other_maps]).unwrap();
        assert_eq!(found.root, root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn off_gives_none_and_empty_counts_as_unset() {
        let dir = folder("off");
        let maps = cartographer(&dir.join("pc"));
        let dirs = [maps.as_path()];
        // Off: nothing, not even the usual folders or lobby.txt's.
        assert_eq!(find(Some(OsStr::new("off")), None, &dirs), None);
        assert_eq!(find(Some(OsStr::new("off")), Some(&maps), &dirs), None);
        assert!(off(Some(OsStr::new("off")), None));
        // lobby.txt's off, with the variable unset.
        assert_eq!(find(None, Some(Path::new("off")), &dirs), None);
        assert!(off(Some(OsStr::new("")), Some(Path::new("off"))));
        // The variable naming a folder wins over lobby.txt's off.
        assert!(!off(Some(maps.as_os_str()), Some(Path::new("off"))));
        assert_eq!(
            find(Some(maps.as_os_str()), Some(Path::new("off")), &[])
                .unwrap()
                .maps,
            maps
        );
        // Empty: as if unset, so the usual folders.
        assert_eq!(find(Some(OsStr::new("")), None, &dirs).unwrap().maps, maps);
        assert!(!off(Some(OsStr::new("")), None));
        assert!(!off(None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lobby_txt_names_a_folder_when_the_variable_doesnt() {
        let dir = folder("saved");
        let saved = dir.join("D Games").join("Cartographer");
        let maps = cartographer(&saved);
        let usual = cartographer(&dir.join("usual"));
        let found = find(None, Some(&saved), &[&usual]).unwrap();
        assert_eq!((found.root, found.maps), (saved.clone(), maps.clone()));
        // Its maps folder works too.
        assert_eq!(find(None, Some(&maps), &[&usual]).unwrap().maps, maps);
        // A folder without mainmenu.map is passed over, and said.
        let gone = dir.join("gone");
        assert_eq!(find(None, Some(&gone), &[&usual]).unwrap().maps, usual);
        assert_eq!(
            find(Some(gone.as_os_str()), Some(&saved), &[&usual])
                .unwrap()
                .maps,
            maps
        );
        assert_eq!(
            named_but_missing(Some(gone.as_os_str()), Some(&saved)),
            [(ENV, gone.as_path())]
        );
        assert_eq!(
            named_but_missing(None, Some(&gone)),
            [("lobby.txt's vista", gone.as_path())]
        );
        assert!(named_but_missing(None, Some(&saved)).is_empty());
        assert!(named_but_missing(Some(OsStr::new("off")), Some(&gone)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_usual_folder_with_mainmenu_is_taken() {
        let dir = folder("dirs");
        let [a, b, c] = ["a", "b", "c"].map(|n| dir.join(n).join("maps"));
        std::fs::create_dir_all(&a).unwrap();
        for d in [&b, &c] {
            cartographer(d.parent().unwrap());
        }
        // a has no mainmenu.map; b comes before c.
        let found = find(None, None, &[&a, &b, &c]).unwrap();
        assert_eq!((found.root, found.maps), (dir.join("b"), b.clone()));
        assert_eq!(find(None, None, &[&a, &c, &b]).unwrap().maps, c);
        assert_eq!(find(None, None, &[&a]), None);
        assert_eq!(find(None, None, &[]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fonts_need_their_table() {
        let dir = folder("fonts");
        let maps = cartographer(&dir);
        let v = Vista::at(&dir).unwrap();
        assert_eq!(v.fonts(), None);
        // A fonts folder without font_table.txt is no use.
        std::fs::create_dir_all(maps.join("fonts")).unwrap();
        std::fs::write(maps.join("fonts").join("handel_gothic_13"), b"x").unwrap();
        assert_eq!(v.fonts(), None);
        std::fs::write(maps.join("fonts").join("font_table.txt"), b"x\n").unwrap();
        assert_eq!(v.fonts(), Some(maps.join("fonts")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
