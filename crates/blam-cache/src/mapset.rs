//! A map together with the shared maps it depends on.
//!
//! Halo 2 PC maps only store the tags unique to them; every map has the same
//! tag table, and tags without data locally live in `shared.map` (and
//! `single_player_shared.map` for campaign). Raw resources (geometry, bitmaps)
//! are addressed by 32-bit pointers whose top two bits pick the file.

use crate::{CacheFile, DatumIndex, Error, MapType, Result, Tag};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub type Map = CacheFile<BufReader<File>>;

/// Which file a raw resource pointer refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataLocation {
    Local,
    MainMenu,
    Shared,
    SinglePlayerShared,
}

impl DataLocation {
    pub fn of(pointer: u32) -> DataLocation {
        match pointer >> 30 {
            0 => DataLocation::Local,
            1 => DataLocation::MainMenu,
            2 => DataLocation::Shared,
            _ => DataLocation::SinglePlayerShared,
        }
    }
}

pub fn pointer_offset(pointer: u32) -> u64 {
    (pointer & 0x3FFF_FFFF) as u64
}

pub struct MapSet {
    pub map: Map,
    pub shared: Option<Map>,
    pub sp_shared: Option<Map>,
    pub main_menu: Option<Map>,
    dir: PathBuf,
}

/// Index into a `MapSet`: which file a tag's data was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Map,
    Shared,
    SpShared,
}

impl MapSet {
    /// Open `path` and the shared maps next to it, or in the `maps` folder
    /// beside its own (add-on maps sit in a `dlc` folder there). Missing
    /// shared maps are tolerated.
    pub fn open(path: impl AsRef<Path>) -> Result<MapSet> {
        let path = path.as_ref();
        let own = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let beside = own.parent().map(|p| p.join("maps"));
        let dir = match beside {
            Some(b) if !own.join("shared.map").exists() && b.join("shared.map").exists() => b,
            _ => own,
        };
        let map = CacheFile::open(path)?;
        let same = |name: &str| {
            path.file_name()
                .is_some_and(|f| f.eq_ignore_ascii_case(name))
        };
        let open_opt = |name: &str| -> Option<Map> {
            if same(name) {
                return None;
            }
            CacheFile::open(dir.join(name)).ok()
        };
        let campaign = matches!(map.header.map_type, MapType::Campaign);
        Ok(MapSet {
            shared: open_opt("shared.map"),
            sp_shared: if campaign {
                open_opt("single_player_shared.map")
            } else {
                None
            },
            main_menu: None,
            map,
            dir,
        })
    }

    pub fn get(&mut self, source: Source) -> &mut Map {
        match source {
            Source::Map => &mut self.map,
            Source::Shared => self.shared.as_mut().expect("located in shared"),
            Source::SpShared => self.sp_shared.as_mut().expect("located in sp shared"),
        }
    }

    /// Find the file that stores `datum`'s data.
    pub fn locate(&self, datum: DatumIndex) -> Option<(Source, Tag)> {
        let candidates = [
            (Source::Map, Some(&self.map)),
            (Source::SpShared, self.sp_shared.as_ref()),
            (Source::Shared, self.shared.as_ref()),
        ];
        for (src, m) in candidates {
            if let Some(t) = m.and_then(|m| m.tag(datum)).filter(|t| t.has_data()) {
                return Some((src, t.clone()));
            }
        }
        None
    }

    /// Read a tag's data from whichever file stores it.
    pub fn tag_data(&mut self, datum: DatumIndex) -> Result<(Source, Tag, Vec<u8>)> {
        let (src, tag) = self.locate(datum).ok_or_else(|| {
            Error::Corrupt(format!("tag {:08x} not stored in any loaded map", datum.0))
        })?;
        let data = self.get(src).read_tag_data(&tag)?;
        Ok((src, tag, data))
    }

    /// The map file a raw resource pointer refers to. `owner` is the file the
    /// referencing tag came from ("local" means that file).
    pub fn resource_file(&mut self, owner: Source, pointer: u32) -> Result<&mut Map> {
        let missing =
            |n: &str| Error::Corrupt(format!("resource lives in {n}, which isn't loaded"));
        match DataLocation::of(pointer) {
            DataLocation::Local => Ok(self.get(owner)),
            DataLocation::Shared => self.shared.as_mut().ok_or_else(|| missing("shared.map")),
            DataLocation::SinglePlayerShared => {
                if self.sp_shared.is_none() {
                    self.sp_shared =
                        CacheFile::open(self.dir.join("single_player_shared.map")).ok();
                }
                self.sp_shared
                    .as_mut()
                    .ok_or_else(|| missing("single_player_shared.map"))
            }
            DataLocation::MainMenu => {
                if self.main_menu.is_none() {
                    self.main_menu = CacheFile::open(self.dir.join("mainmenu.map")).ok();
                }
                self.main_menu
                    .as_mut()
                    .ok_or_else(|| missing("mainmenu.map"))
            }
        }
    }

    /// Read an uncompressed raw resource.
    pub fn read_resource(&mut self, owner: Source, pointer: u32, size: usize) -> Result<Vec<u8>> {
        self.resource_file(owner, pointer)?
            .read_raw(pointer_offset(pointer), size)
    }
}
