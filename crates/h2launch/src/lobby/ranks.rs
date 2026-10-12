//! Halo 2's level icons: the 50 rank icons Xbox Live showed beside a
//! player's level, big and small. The design is in
//! `docs/notes/launcher/live-v3.md` (section 4).
//!
//! They are two bitmap tags in `mainmenu.map`, 50 images each in level
//! order (image index = level - 1). MCC's own mainmenu.map (cache format
//! 13, its pixels in the textures.dat beside it) is tried first, through
//! `blam_cache::mcc`; then a Halo 2 Vista one (format 8), read as the old
//! game's emblems are. Both hold the same icons. Without either the lobby
//! draws level numbers. Nothing here draws or keeps lobby state, and no
//! image is shipped: they are read from the player's own files each time.

use blam_cache::bitmap::{self, Image};
use blam_cache::{mcc, GroupTag, MapSet};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The environment variable that names the map (a file, or a folder holding
/// mainmenu.map; MCC's or Halo 2 Vista's, told apart by its version), or
/// `off`.
pub const ENV: &str = "H2LOBBY_RANKS";
/// Levels, and so icons in each tag.
pub const LEVELS: u8 = 50;
pub const BIG_TAG: &str = r"ui\global_bitmaps\rank_icons";
pub const SMALL_TAG: &str = r"ui\global_bitmaps\rank_icons_sm";
/// Where Halo 2 Vista keeps its maps, as the old h2viewer looked.
pub const MAP_DIRS: [&str; 3] = [
    r"C:\Games\Halo 2 Project Cartographer\maps",
    r"C:\Program Files (x86)\Microsoft Games\Halo 2\maps",
    r"C:\Program Files\Microsoft Games\Halo 2\maps",
];
/// The map the icons are in.
const MAP_NAME: &str = "mainmenu.map";
/// Beside MCC's maps: their bitmaps' pixels.
const TEXTURES_NAME: &str = "textures.dat";
/// MCC's Halo 2 maps, inside the MCC folder.
pub const MCC_MAPS: &str = r"halo2\h2_maps_win64_dx11";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// rank_icons_sm, 17 by 17: lists.
    Small,
    /// rank_icons, 28 by 26: the carnage report, pregame, service record.
    Big,
}

/// One level's icon: RGBA8 with straight (not premultiplied) alpha, the top
/// row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Both sets of icons, and the map they came from.
#[derive(Clone, Debug)]
pub struct RankIcons {
    big: Vec<Icon>,
    small: Vec<Icon>,
    from: PathBuf,
    mcc: bool,
}

impl RankIcons {
    /// From decoded images in level order: exactly LEVELS of each, every
    /// one at least 1 by 1 with `width * height * 4` bytes. Sizes other
    /// than 28 by 26 and 17 by 17 are taken (a mod may change them).
    pub fn new(big: Vec<Image>, small: Vec<Image>, from: PathBuf) -> Result<RankIcons, String> {
        Ok(RankIcons {
            big: icons(big, "rank_icons")?,
            small: icons(small, "rank_icons_sm")?,
            from,
            mcc: false,
        })
    }

    /// The icon for `level` (1 to 50); None for 0 or above 50.
    pub fn icon(&self, level: u8, size: Size) -> Option<&Icon> {
        let set = match size {
            Size::Small => &self.small,
            Size::Big => &self.big,
        };
        set.get(usize::from(level).checked_sub(1)?)
    }

    /// The map they were read from.
    pub fn from(&self) -> &Path {
        &self.from
    }

    /// Whether that map is MCC's (cache format 13) rather than Halo 2
    /// Vista's.
    pub fn from_mcc(&self) -> bool {
        self.mcc
    }
}

/// One tag's images as icons, checked.
fn icons(images: Vec<Image>, tag: &str) -> Result<Vec<Icon>, String> {
    if images.len() != usize::from(LEVELS) {
        return Err(format!("{tag} has {} images, not {LEVELS}", images.len()));
    }
    images
        .into_iter()
        .enumerate()
        .map(|(i, im)| {
            let level = i + 1;
            if im.width == 0 || im.height == 0 {
                return Err(format!(
                    "{tag}: level {level}'s image is {} by {}",
                    im.width, im.height
                ));
            }
            let want = im.width as usize * im.height as usize * 4;
            if im.rgba.len() != want {
                return Err(format!(
                    "{tag}: level {level}'s image is {} by {} but has {} bytes, not {want}",
                    im.width,
                    im.height,
                    im.rgba.len()
                ));
            }
            Ok(Icon {
                width: im.width,
                height: im.height,
                rgba: im.rgba,
            })
        })
        .collect()
}

/// The maps to try, in order, or None when `env` (the variable the caller
/// read: `H2LOBBY_RANKS` for the rank icons, `H2LOBBY_MENU` for the start
/// screen and main menu) is `off`. With `env` set: only that (a folder
/// means its mainmenu.map). Without: MCC's mainmenu.map (`mcc_maps`, its
/// `halo2\h2_maps_win64_dx11`), each of MAP_DIRS' mainmenu.map, then
/// `maps\mainmenu.map` beside the launcher (`exe_dir`). An empty `env`
/// counts as unset.
pub fn candidates(
    env: Option<&OsStr>,
    mcc_maps: Option<&Path>,
    exe_dir: Option<&Path>,
) -> Option<Vec<PathBuf>> {
    match env.filter(|v| !v.is_empty()) {
        Some(v) if v == "off" => None,
        Some(v) => {
            let path = PathBuf::from(v);
            if path.is_dir() {
                Some(vec![path.join(MAP_NAME)])
            } else {
                Some(vec![path])
            }
        }
        None => {
            let mut out: Vec<PathBuf> = mcc_maps.map(|d| d.join(MAP_NAME)).into_iter().collect();
            out.extend(MAP_DIRS.iter().map(|d| Path::new(d).join(MAP_NAME)));
            if let Some(dir) = exe_dir {
                out.push(dir.join("maps").join(MAP_NAME));
            }
            Some(out)
        }
    }
}

/// The candidates that are files, in order.
pub fn existing(candidates: &[PathBuf]) -> Vec<PathBuf> {
    candidates.iter().filter(|p| p.is_file()).cloned().collect()
}

/// Both tags' 50 images from a mainmenu.map, MCC's or Halo 2 Vista's (told
/// apart by the cache version in its first bytes). An error names the map
/// and why: not a map, a version neither reader takes, a tag missing, an
/// image that won't decode, MCC's textures.dat missing.
pub fn read(map: &Path) -> Result<RankIcons, String> {
    let at = |why: String| format!("{}: {why}", map.display());
    // The magic and the version word first: the two formats' headers part
    // ways after them (Vista's `foot` is at 0x7FC, MCC's at 0x37C).
    let mut start = [0u8; 8];
    std::fs::File::open(map)
        .and_then(|mut f| f.read_exact(&mut start))
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => at("not a Halo 2 map (too short)".into()),
            _ => at(e.to_string()),
        })?;
    let Some(version) = blam_cache::cache_version(&start) else {
        return Err(at("not a Halo 2 map (no 'head')".into()));
    };
    if version == mcc::VERSION {
        return read_mcc(map).map_err(|e| at(format!("MCC's map (cache format 13): {e}")));
    }
    let mut set = MapSet::open(map).map_err(|e| at(e.to_string()))?;
    let big = images(&mut set, BIG_TAG).map_err(at)?;
    let small = images(&mut set, SMALL_TAG).map_err(at)?;
    RankIcons::new(big, small, map.to_path_buf()).map_err(at)
}

/// From MCC's format-13 mainmenu.map, its pixels in the textures.dat in
/// the same folder.
fn read_mcc(map: &Path) -> Result<RankIcons, String> {
    let dat = map.with_file_name(TEXTURES_NAME);
    let mut tags = mcc::Map::open(map).map_err(|e| e.to_string())?;
    let mut textures = mcc::Textures::open(&dat).map_err(|e| format!("{}: {e}", dat.display()))?;
    let big = mcc_images(&mut tags, &mut textures, BIG_TAG)?;
    let small = mcc_images(&mut tags, &mut textures, SMALL_TAG)?;
    let mut icons = RankIcons::new(big, small, map.to_path_buf())?;
    icons.mcc = true;
    Ok(icons)
}

/// A bitmap tag's first LEVELS images from an MCC map, decoded.
fn mcc_images<R, T>(
    tags: &mut mcc::Map<R>,
    textures: &mut mcc::Textures<T>,
    name: &str,
) -> Result<Vec<Image>, String>
where
    R: std::io::Read + std::io::Seek,
    T: std::io::Read + std::io::Seek,
{
    let group = GroupTag::parse("bitm").expect("four letters");
    let tag = tags
        .find_tag(group, name)
        .ok_or_else(|| format!("no bitmap {name} in it"))?
        .clone();
    let entries = tags.bitmaps(&tag).map_err(|e| format!("{name}: {e}"))?;
    (0..usize::from(LEVELS))
        .map(|i| {
            let level = i + 1;
            let entry = entries
                .get(i)
                .ok_or_else(|| format!("{name}, level {level}: the tag has no such image"))?;
            textures
                .read(entry)
                .map_err(|e| format!("{name}, level {level}: {e}"))
        })
        .collect()
}

/// A bitmap tag's first LEVELS images, decoded.
fn images(set: &mut MapSet, name: &str) -> Result<Vec<Image>, String> {
    let group = GroupTag::parse("bitm").expect("four letters");
    let tag = set
        .map
        .find_tag(group, name)
        .ok_or_else(|| format!("no bitmap {name} in it"))?
        .datum;
    (0..usize::from(LEVELS))
        .map(|i| {
            bitmap::read_bitmap_at(set, tag, i).map_err(|e| format!("{name}, level {}: {e}", i + 1))
        })
        .collect()
}

/// The first of `candidates` that reads, saying in the log which was used
/// and why any before it weren't. None means levels are drawn as numbers.
pub fn first_readable(candidates: &[PathBuf], log: &dyn Fn(&str)) -> Option<RankIcons> {
    let found = existing(candidates);
    if found.is_empty() {
        log("lobby: no mainmenu.map found (MCC's or Halo 2 Vista's); levels show as numbers");
        return None;
    }
    for map in &found {
        match read(map) {
            Ok(icons) => {
                let kind = if icons.from_mcc() {
                    "MCC's, cache format 13"
                } else {
                    "Halo 2 Vista's"
                };
                log(&format!(
                    "lobby: rank icons from {} ({kind})",
                    map.display()
                ));
                return Some(icons);
            }
            Err(e) => log(&format!("lobby: rank icons: {e}")),
        }
    }
    log("lobby: no rank icons could be read; levels show as numbers");
    None
}

/// What the lobby calls at start: `candidates` from the environment, MCC's
/// maps folder (`mcc_maps`, when MCC was found) and the running launcher's
/// folder, then `first_readable`. None means levels are drawn as numbers.
pub fn load(mcc_maps: Option<&Path>, log: &dyn Fn(&str)) -> Option<RankIcons> {
    let env = std::env::var_os(ENV);
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    let Some(list) = candidates(env.as_deref(), mcc_maps, exe_dir) else {
        log("lobby: rank icons off (H2LOBBY_RANKS=off); levels show as numbers");
        return None;
    };
    first_readable(&list, log)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of our own for one test, empty.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h2ranks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An image whose pixels all say which one it is.
    fn image(w: u32, h: u32, n: u8) -> Image {
        Image {
            width: w,
            height: h,
            rgba: [n, 0, 0, 255].repeat(w as usize * h as usize),
        }
    }

    fn set(count: u8, w: u32, h: u32) -> Vec<Image> {
        (0..count).map(|n| image(w, h, n)).collect()
    }

    #[test]
    fn new_takes_fifty_of_each_and_refuses_others() {
        let from = PathBuf::from("mainmenu.map");
        assert!(RankIcons::new(set(50, 28, 26), set(50, 17, 17), from.clone()).is_ok());
        // Other sizes are taken.
        assert!(RankIcons::new(set(50, 32, 32), set(50, 8, 8), from.clone()).is_ok());
        for (big, small) in [(49, 50), (51, 50), (50, 49), (50, 51), (0, 0)] {
            let r = RankIcons::new(set(big, 28, 26), set(small, 17, 17), from.clone());
            assert!(r.is_err(), "{big} and {small} images were taken");
        }
        let mut empty = set(50, 28, 26);
        empty[7] = Image {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
        let e = RankIcons::new(empty, set(50, 17, 17), from.clone()).unwrap_err();
        assert!(e.contains("level 8"), "{e}");
        let mut short = set(50, 17, 17);
        short[49].rgba.pop();
        let e = RankIcons::new(set(50, 28, 26), short, from.clone()).unwrap_err();
        assert!(e.contains("rank_icons_sm") && e.contains("level 50"), "{e}");
        let mut long = set(50, 28, 26);
        long[0].rgba.extend([0; 4]);
        assert!(RankIcons::new(long, set(50, 17, 17), from).is_err());
    }

    #[test]
    fn icon_is_level_minus_one_in_the_right_set() {
        let from = PathBuf::from(r"C:\maps\mainmenu.map");
        let icons = RankIcons::new(set(50, 28, 26), set(50, 17, 17), from.clone()).unwrap();
        assert_eq!(icons.from(), from.as_path());
        let first = icons.icon(1, Size::Big).unwrap();
        assert_eq!((first.width, first.height, first.rgba[0]), (28, 26, 0));
        assert_eq!(first.rgba.len(), 28 * 26 * 4);
        let last = icons.icon(50, Size::Big).unwrap();
        assert_eq!(last.rgba[0], 49);
        let small = icons.icon(50, Size::Small).unwrap();
        assert_eq!((small.width, small.height, small.rgba[0]), (17, 17, 49));
        assert_eq!(icons.icon(25, Size::Small).unwrap().rgba[0], 24);
        for size in [Size::Small, Size::Big] {
            assert!(icons.icon(0, size).is_none());
            assert!(icons.icon(51, size).is_none());
            assert!(icons.icon(255, size).is_none());
        }
    }

    #[test]
    fn candidates_off_file_folder_and_unset() {
        let mcc_dir = Path::new(r"C:\MCC\halo2\h2_maps_win64_dx11");
        assert_eq!(
            candidates(Some(OsStr::new("off")), Some(mcc_dir), None),
            None
        );
        let dir = folder("candidates");
        let file = dir.join("other.map");
        std::fs::write(&file, b"x").unwrap();
        // A file is taken as it is, existing or not; nothing else is tried.
        let exe = Path::new("/launcher");
        assert_eq!(
            candidates(Some(file.as_os_str()), Some(mcc_dir), Some(exe)),
            Some(vec![file.clone()])
        );
        let gone = dir.join("gone.map");
        assert_eq!(
            candidates(Some(gone.as_os_str()), None, None),
            Some(vec![gone])
        );
        // A folder means its mainmenu.map.
        assert_eq!(
            candidates(Some(dir.as_os_str()), Some(mcc_dir), Some(exe)),
            Some(vec![dir.join("mainmenu.map")])
        );
        // Unset, or empty: MCC's maps, the Vista folders, then beside the
        // launcher.
        let want: Vec<PathBuf> = vec![
            mcc_dir.join("mainmenu.map"),
            Path::new(r"C:\Games\Halo 2 Project Cartographer\maps").join("mainmenu.map"),
            Path::new(r"C:\Program Files (x86)\Microsoft Games\Halo 2\maps").join("mainmenu.map"),
            Path::new(r"C:\Program Files\Microsoft Games\Halo 2\maps").join("mainmenu.map"),
            exe.join("maps").join("mainmenu.map"),
        ];
        assert_eq!(
            candidates(None, Some(mcc_dir), Some(exe)),
            Some(want.clone())
        );
        assert_eq!(
            candidates(Some(OsStr::new("")), Some(mcc_dir), Some(exe)),
            Some(want.clone())
        );
        // No MCC found, no launcher folder: the Vista folders alone.
        assert_eq!(candidates(None, None, None), Some(want[1..4].to_vec()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_keeps_the_files_in_order() {
        let dir = folder("find");
        let [a, b, c] = ["a", "b", "c"].map(|n| dir.join(n).join("mainmenu.map"));
        for p in [&b, &c] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        // A folder named like the map isn't a file.
        let folder_named = dir.join("d").join("mainmenu.map");
        std::fs::create_dir_all(&folder_named).unwrap();
        assert_eq!(
            existing(&[a.clone(), b.clone(), c.clone()]),
            [b.clone(), c.clone()]
        );
        assert_eq!(existing(&[c.clone(), b.clone()]), [c, b]);
        assert!(existing(&[folder_named.clone(), a.clone()]).is_empty());
        assert!(existing(&[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn put(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A word as blam-cache reads it (`head`, `foot`, `tags`, `bitm`).
    fn magic(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    #[test]
    fn read_refuses_what_isnt_a_map_without_panicking() {
        let dir = folder("refuse");
        let missing = dir.join("mainmenu.map");
        let e = read(&missing).unwrap_err();
        assert!(e.contains("mainmenu.map"), "{e}");

        let zeros = dir.join("zeros.map");
        std::fs::write(&zeros, vec![0u8; 0x1000]).unwrap();
        let e = read(&zeros).unwrap_err();
        assert!(e.contains("not a Halo 2 map"), "{e}");

        let tiny = dir.join("tiny.map");
        std::fs::write(&tiny, b"daeh").unwrap();
        let e = read(&tiny).unwrap_err();
        assert!(e.contains("not a Halo 2 map"), "{e}");

        // Version 13 goes to MCC's reader, which wants `foot` at 0x37C,
        // not where Vista's is.
        let mut b = vec![0u8; 0x800];
        put(&mut b, 0, magic(b"head"));
        put(&mut b, 4, 13);
        put(&mut b, 0x7FC, magic(b"foot"));
        let with_foot = dir.join("mcc-foot.map");
        std::fs::write(&with_foot, &b).unwrap();
        let e = read(&with_foot).unwrap_err();
        assert!(
            e.contains("cache format 13") && e.contains("'foot'") && e.contains("mcc-foot.map"),
            "{e}"
        );

        // MCC's header, but nothing after it.
        let mut b = vec![0u8; 0x1000];
        put(&mut b, 0, magic(b"head"));
        put(&mut b, 4, 13);
        put(&mut b, 0x37C, magic(b"foot"));
        let no_body = dir.join("mcc-nobody.map");
        std::fs::write(&no_body, &b).unwrap();
        let e = read(&no_body).unwrap_err();
        assert!(e.contains("cache format 13"), "{e}");

        // Another version is left to blam-cache to refuse.
        put(&mut b, 4, 9);
        put(&mut b, 0x7FC, magic(b"foot"));
        let other = dir.join("nine.map");
        std::fs::write(&other, &b).unwrap();
        let e = read(&other).unwrap_err();
        assert!(e.contains("version 9"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `data` as a zlib stream of stored (uncompressed) blocks.
    fn zlib_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let chunks: Vec<&[u8]> = data.chunks(0xFFFF).collect();
        for (i, c) in chunks.iter().enumerate() {
            out.push(u8::from(i + 1 == chunks.len()));
            let len = c.len() as u16;
            out.extend(len.to_le_bytes());
            out.extend((!len).to_le_bytes());
            out.extend_from_slice(c);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        out.extend(((b << 16) | a).to_be_bytes());
        out
    }

    /// A Halo 2 Vista mainmenu.map holding the two icon tags, `count`
    /// images each, A8R8G8B8 (stored B, G, R, A). Level n's pixels are
    /// red n, green 1 for the big set or 2 for the small, blue 3, alpha
    /// 0 in the left column and 255 elsewhere. `big_name` lets a test
    /// leave the big tag out.
    fn vista_map(count: usize, big_name: &str) -> Vec<u8> {
        const MASK: u32 = 0x8000_0000;
        const META: usize = 0x1000;
        const ENTRY: usize = 116;
        let tags = [(big_name, 28usize, 26usize, 1u8), (SMALL_TAG, 17, 17, 2)];
        // Tag data: 0x80 bytes each, then each tag's image entries.
        let data_at = |t: usize| 0x100 + t * 0x80;
        let entries_at = |t: usize| 0x200 + t * count * ENTRY;
        let meta_size = entries_at(2);
        let mut b = vec![0u8; META + meta_size];
        put(&mut b, 0, magic(b"head"));
        put(&mut b, 4, 8);
        put(&mut b, 0x7FC, magic(b"foot"));
        put(&mut b, 0x14C, 2); // main menu
        b[0x1A4..0x1A4 + 8].copy_from_slice(b"mainmenu");
        // Tag names: index at 0x800, text at 0x810.
        let names = format!("{big_name}\0{SMALL_TAG}\0");
        put(&mut b, 0x2CC, 2);
        put(&mut b, 0x2D0, 0x810);
        put(&mut b, 0x2D4, names.len() as u32);
        put(&mut b, 0x2D8, 0x800);
        put(&mut b, 0x804, big_name.len() as u32 + 1);
        b[0x810..0x810 + names.len()].copy_from_slice(names.as_bytes());
        // The meta area.
        put(&mut b, 0x10, META as u32);
        put(&mut b, 0x14, 0x100);
        put(&mut b, 0x1C, meta_size as u32);
        put(&mut b, 0x20, MASK);
        put(&mut b, META, 0x20); // groups
        put(&mut b, META + 4, 1);
        put(&mut b, META + 8, 0x2C); // tags
        put(&mut b, META + 0xC, u32::MAX);
        put(&mut b, META + 0x10, u32::MAX);
        put(&mut b, META + 0x18, 2);
        put(&mut b, META + 0x1C, magic(b"tags"));
        put(&mut b, META + 0x20, magic(b"bitm"));
        put(&mut b, META + 0x24, u32::MAX);
        put(&mut b, META + 0x28, u32::MAX);
        let mut pixels = Vec::new();
        for (t, &(_, w, h, set)) in tags.iter().enumerate() {
            let row = META + 0x2C + t * 0x10;
            put(&mut b, row, magic(b"bitm"));
            put(&mut b, row + 4, 0xE000_0000 + t as u32);
            put(&mut b, row + 8, MASK + data_at(t) as u32);
            put(&mut b, row + 0xC, 0x80);
            // The bitmaps block at 68: count, address.
            put(&mut b, META + data_at(t) + 68, count as u32);
            put(&mut b, META + data_at(t) + 72, MASK + entries_at(t) as u32);
            for n in 0..count {
                let e = META + entries_at(t) + n * ENTRY;
                b[e + 4..e + 6].copy_from_slice(&(w as i16).to_le_bytes());
                b[e + 6..e + 8].copy_from_slice(&(h as i16).to_le_bytes());
                b[e + 12..e + 14].copy_from_slice(&11i16.to_le_bytes());
                let mut raw = Vec::with_capacity(w * h * 4);
                for _ in 0..h {
                    for x in 0..w {
                        raw.extend([3, set, n as u8 + 1, if x == 0 { 0 } else { 255 }]);
                    }
                }
                let z = zlib_stored(&raw);
                // A pointer's top two bits 0: in this map, at its offset.
                pixels.push((e, z));
            }
        }
        for (e, z) in pixels {
            let at = b.len();
            put(&mut b, e + 28, at as u32);
            put(&mut b, e + 52, z.len() as u32);
            b.extend(z);
        }
        b
    }

    #[test]
    fn read_decodes_both_tags_from_a_vista_map() {
        let dir = folder("vista");
        let map = dir.join("mainmenu.map");
        std::fs::write(&map, vista_map(50, BIG_TAG)).unwrap();
        let icons = read(&map).unwrap();
        assert_eq!(icons.from(), map.as_path());
        for level in [1u8, 2, 37, 50] {
            let big = icons.icon(level, Size::Big).unwrap();
            assert_eq!((big.width, big.height), (28, 26));
            // R, G, B, A from the file's B, G, R, A; see-through on the left.
            assert_eq!(big.rgba[..8], [level, 1, 3, 0, level, 1, 3, 255]);
            let small = icons.icon(level, Size::Small).unwrap();
            assert_eq!((small.width, small.height), (17, 17));
            assert_eq!(small.rgba.len(), 17 * 17 * 4);
            assert_eq!(small.rgba[4..8], [level, 2, 3, 255]);
        }
        // The same file found by its folder, as H2LOBBY_RANKS may name it.
        let list = candidates(Some(dir.as_os_str()), None, None).unwrap();
        assert_eq!(existing(&list), std::slice::from_ref(&map));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_names_a_missing_tag_or_image() {
        let dir = folder("missing");
        let map = dir.join("mainmenu.map");
        std::fs::write(&map, vista_map(50, r"ui\global_bitmaps\emblems")).unwrap();
        let e = read(&map).unwrap_err();
        assert!(e.contains(BIG_TAG) && e.contains("mainmenu.map"), "{e}");
        std::fs::write(&map, vista_map(49, BIG_TAG)).unwrap();
        let e = read(&map).unwrap_err();
        assert!(e.contains("level 50"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// MCC's mainmenu.map and textures.dat (made up) in `dir`, holding the
    /// two icon tags with `count` images each, in 4 KiB chunks so the tags
    /// cross chunk boundaries. Pixels as in `vista_map`.
    fn mcc_files(dir: &Path, count: u8, big_name: &str) {
        use blam_cache::mcc::synthetic::{MapBuilder, Meta, Picture, TexturesDat};
        let mut dat = TexturesDat::default();
        let mut tag = |w: u16, h: u16, set: u8| -> Meta {
            let entries = (0..count)
                .map(|n| {
                    let mut bgra = Vec::new();
                    for _ in 0..h {
                        for x in 0..w {
                            bgra.extend([3, set, n + 1, if x == 0 { 0 } else { 255 }]);
                        }
                    }
                    dat.push(&Picture {
                        width: w,
                        height: h,
                        bgra,
                    })
                })
                .collect();
            Meta::Bitmaps(entries)
        };
        let big = tag(28, 26, 1);
        let small = tag(17, 17, 2);
        let map = MapBuilder::new(0x1000)
            .tag("matg", r"globals\globals", Meta::Raw(vec![7; 0x900]))
            .tag("bitm", big_name, big)
            .tag("bitm", SMALL_TAG, small)
            .build();
        std::fs::write(dir.join("mainmenu.map"), map).unwrap();
        std::fs::write(dir.join("textures.dat"), dat.bytes).unwrap();
    }

    #[test]
    fn read_decodes_both_tags_from_an_mcc_map() {
        let dir = folder("mcc");
        mcc_files(&dir, 50, BIG_TAG);
        let map = dir.join("mainmenu.map");
        let icons = read(&map).unwrap();
        assert!(icons.from_mcc());
        assert_eq!(icons.from(), map.as_path());
        for level in [1u8, 2, 37, 50] {
            let big = icons.icon(level, Size::Big).unwrap();
            assert_eq!((big.width, big.height), (28, 26));
            assert_eq!(big.rgba[..8], [level, 1, 3, 0, level, 1, 3, 255]);
            let small = icons.icon(level, Size::Small).unwrap();
            assert_eq!((small.width, small.height), (17, 17));
            assert_eq!(small.rgba[4..8], [level, 2, 3, 255]);
        }
        // A Vista map's icons say they aren't MCC's.
        std::fs::write(&map, vista_map(50, BIG_TAG)).unwrap();
        assert!(!read(&map).unwrap().from_mcc());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_names_what_an_mcc_map_lacks() {
        let dir = folder("mcc-missing");
        let map = dir.join("mainmenu.map");
        mcc_files(&dir, 50, r"ui\global_bitmaps\emblems");
        let e = read(&map).unwrap_err();
        assert!(e.contains(BIG_TAG) && e.contains("cache format 13"), "{e}");
        mcc_files(&dir, 49, BIG_TAG);
        let e = read(&map).unwrap_err();
        assert!(e.contains("level 50"), "{e}");
        mcc_files(&dir, 50, BIG_TAG);
        std::fs::remove_file(dir.join("textures.dat")).unwrap();
        let e = read(&map).unwrap_err();
        assert!(e.contains("textures.dat"), "{e}");
        // A textures.dat cut short.
        mcc_files(&dir, 50, BIG_TAG);
        let dat = dir.join("textures.dat");
        let bytes = std::fs::read(&dat).unwrap();
        std::fs::write(&dat, &bytes[..bytes.len() / 2]).unwrap();
        let e = read(&map).unwrap_err();
        assert!(e.contains("level"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_readable_falls_back_to_vista_and_says_so() {
        let dir = folder("first");
        let (mcc_dir, vista_dir) = (dir.join("mcc"), dir.join("vista"));
        for d in [&mcc_dir, &vista_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        mcc_files(&mcc_dir, 50, BIG_TAG);
        std::fs::write(vista_dir.join("mainmenu.map"), vista_map(50, BIG_TAG)).unwrap();
        let list = vec![
            dir.join("gone").join("mainmenu.map"),
            mcc_dir.join("mainmenu.map"),
            vista_dir.join("mainmenu.map"),
        ];
        let lines = std::cell::RefCell::new(Vec::<String>::new());
        let log = |l: &str| lines.borrow_mut().push(l.to_string());
        // MCC's first when it reads.
        let icons = first_readable(&list, &log).unwrap();
        assert!(icons.from_mcc());
        assert_eq!(lines.borrow().len(), 1);
        assert!(lines.borrow()[0].contains("MCC's"), "{:?}", lines.borrow());
        // MCC's without textures.dat: Vista's, and the log says why.
        std::fs::remove_file(mcc_dir.join("textures.dat")).unwrap();
        lines.borrow_mut().clear();
        let icons = first_readable(&list, &log).unwrap();
        assert!(!icons.from_mcc());
        assert_eq!(icons.from(), list[2].as_path());
        let got = lines.borrow().clone();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].contains("textures.dat"), "{got:?}");
        assert!(got[1].contains("Halo 2 Vista's"), "{got:?}");
        // Neither reads; none there.
        std::fs::write(vista_dir.join("mainmenu.map"), b"daeh").unwrap();
        lines.borrow_mut().clear();
        assert!(first_readable(&list, &log).is_none());
        assert!(lines.borrow().last().unwrap().contains("numbers"));
        lines.borrow_mut().clear();
        assert!(first_readable(&list[..1], &log).is_none());
        assert!(lines.borrow()[0].contains("no mainmenu.map found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads the real icons from MCC's maps folder, named by `H2_MCC_MAPS`
    /// (its `halo2\h2_maps_win64_dx11`, with textures.dat).
    #[test]
    #[ignore]
    fn reads_real_icons_from_mcc_maps() {
        let dir = std::env::var("H2_MCC_MAPS").expect("H2_MCC_MAPS names MCC's maps folder");
        let icons = read(&Path::new(&dir).join("mainmenu.map")).unwrap();
        assert!(icons.from_mcc());
        for level in 1..=LEVELS {
            let big = icons.icon(level, Size::Big).unwrap();
            assert_eq!((big.width, big.height), (28, 26), "level {level}");
            let small = icons.icon(level, Size::Small).unwrap();
            assert_eq!((small.width, small.height), (17, 17), "level {level}");
        }
    }

    /// Reads the real icons from `H2_MAPS` (a Halo 2 Vista maps folder).
    #[test]
    #[ignore]
    fn reads_real_icons_from_h2_maps() {
        let dir = std::env::var("H2_MAPS").expect("H2_MAPS names the maps folder");
        let icons = read(&Path::new(&dir).join("mainmenu.map")).unwrap();
        for level in 1..=LEVELS {
            let big = icons.icon(level, Size::Big).unwrap();
            assert_eq!((big.width, big.height), (28, 26), "level {level}");
            let small = icons.icon(level, Size::Small).unwrap();
            assert_eq!((small.width, small.height), (17, 17), "level {level}");
        }
    }
}
