//! Reader for Halo 2 PC (Vista) `.map` cache files.
//!
//! A cache file is laid out as:
//! - a 0x800-byte header (`head` ... `foot`)
//! - string id and tag-name tables (offsets given by the header)
//! - the meta area: a small index header, the tag group table, the tag table,
//!   then the tag data itself. Tag data is addressed with 32-bit "memory
//!   addresses"; `address - meta_mask + meta_offset` gives the file offset.
//!
//! Layout follows the Halo 2 Vista definitions used by the Assembly editor.

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub const HEADER_SIZE: usize = 0x800;
const HEAD_MAGIC: u32 = u32::from_be_bytes(*b"head");
const FOOT_MAGIC: u32 = u32::from_be_bytes(*b"foot");
const TAGS_MAGIC: u32 = u32::from_be_bytes(*b"tags");
const META_HEADER_SIZE: u64 = 0x20;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a Halo 2 map file: {0}")]
    BadMagic(&'static str),
    #[error("unsupported cache version {0} (expected 8)")]
    Version(i32),
    #[error("corrupt map: {0}")]
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A four-character tag group code such as `scnr`, `sbsp` or `bitm`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupTag(pub u32);

impl GroupTag {
    pub const NONE: GroupTag = GroupTag(u32::MAX);

    pub fn parse(s: &str) -> Option<GroupTag> {
        let b: [u8; 4] = s.as_bytes().try_into().ok()?;
        Some(GroupTag(u32::from_be_bytes(b)))
    }

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

impl fmt::Display for GroupTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            return f.write_str("none");
        }
        let s: String = self.0.to_be_bytes().iter().map(|&c| c as char).collect();
        f.write_str(&s)
    }
}

impl fmt::Debug for GroupTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GroupTag({self})")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapType {
    Campaign,
    Multiplayer,
    MainMenu,
    Shared,
    SharedCampaign,
    Unknown(i32),
}

impl From<i32> for MapType {
    fn from(v: i32) -> Self {
        match v {
            0 => MapType::Campaign,
            1 => MapType::Multiplayer,
            2 => MapType::MainMenu,
            3 => MapType::Shared,
            4 => MapType::SharedCampaign,
            other => MapType::Unknown(other),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub version: i32,
    pub file_size: u32,
    pub meta_offset: u32,
    pub tag_table_size: u32,
    pub tag_data_size: u32,
    pub meta_size: u32,
    pub meta_mask: u32,
    pub build: String,
    pub map_type: MapType,
    pub string_count: u32,
    pub string_table_size: u32,
    pub string_index_offset: u32,
    pub string_table_offset: u32,
    pub internal_name: String,
    pub scenario_name: String,
    pub file_count: u32,
    pub file_table_offset: u32,
    pub file_table_size: u32,
    pub file_index_offset: u32,
}

impl Header {
    pub fn parse(b: &[u8]) -> Result<Header> {
        if b.len() < HEADER_SIZE {
            return Err(Error::BadMagic("file shorter than header"));
        }
        if be_magic(b, 0) != HEAD_MAGIC {
            return Err(Error::BadMagic("missing 'head'"));
        }
        if be_magic(b, 0x7FC) != FOOT_MAGIC {
            return Err(Error::BadMagic("missing 'foot'"));
        }
        let version = i32_at(b, 0x4);
        if version != 8 {
            return Err(Error::Version(version));
        }
        Ok(Header {
            version,
            file_size: u32_at(b, 0x8),
            meta_offset: u32_at(b, 0x10),
            tag_table_size: u32_at(b, 0x14),
            tag_data_size: u32_at(b, 0x18),
            meta_size: u32_at(b, 0x1C),
            meta_mask: u32_at(b, 0x20),
            build: cstr(&b[0x12C..0x14C]),
            map_type: MapType::from(i32_at(b, 0x14C)),
            string_count: u32_at(b, 0x170),
            string_table_size: u32_at(b, 0x174),
            string_index_offset: u32_at(b, 0x178),
            string_table_offset: u32_at(b, 0x17C),
            internal_name: cstr(&b[0x1A4..0x1C8]),
            scenario_name: cstr(&b[0x1C8..0x2CC]),
            file_count: u32_at(b, 0x2CC),
            file_table_offset: u32_at(b, 0x2D0),
            file_table_size: u32_at(b, 0x2D4),
            file_index_offset: u32_at(b, 0x2D8),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagGroup {
    pub tag: GroupTag,
    pub parent: GroupTag,
    pub grandparent: GroupTag,
}

/// Datum index: salt in the high 16 bits, table index in the low 16 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DatumIndex(pub u32);

impl DatumIndex {
    pub const NONE: DatumIndex = DatumIndex(u32::MAX);
    pub fn index(self) -> u16 {
        self.0 as u16
    }
    pub fn salt(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub group: GroupTag,
    pub datum: DatumIndex,
    /// Memory address of the tag's data; 0 or 0xFFFFFFFF when the tag has no data in this map.
    pub address: u32,
    pub size: u32,
    pub name: String,
}

impl Tag {
    pub fn has_data(&self) -> bool {
        self.address != 0 && self.address != u32::MAX && !self.group.is_none()
    }
}

pub struct CacheFile<R> {
    reader: R,
    pub header: Header,
    pub groups: Vec<TagGroup>,
    pub tags: Vec<Tag>,
    pub scenario: DatumIndex,
    pub globals: DatumIndex,
    strings: Vec<String>,
}

impl CacheFile<BufReader<File>> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_reader(BufReader::new(File::open(path)?))
    }
}

impl<R: Read + Seek> CacheFile<R> {
    pub fn from_reader(mut reader: R) -> Result<Self> {
        let mut hb = vec![0u8; HEADER_SIZE];
        reader.seek(SeekFrom::Start(0))?;
        reader.read_exact(&mut hb)?;
        let header = Header::parse(&hb)?;

        let strings = read_string_table(
            &mut reader,
            header.string_index_offset,
            header.string_table_offset,
            header.string_table_size,
            header.string_count,
        )?;
        let names = read_string_table(
            &mut reader,
            header.file_index_offset,
            header.file_table_offset,
            header.file_table_size,
            header.file_count,
        )?;

        let meta = header.meta_offset as u64;
        let mut mh = [0u8; META_HEADER_SIZE as usize];
        reader.seek(SeekFrom::Start(meta))?;
        reader.read_exact(&mut mh)?;
        if be_magic(&mh, 0x1C) != TAGS_MAGIC {
            return Err(Error::BadMagic("missing 'tags' in meta header"));
        }
        let group_table = u32_at(&mh, 0x0) as u64;
        let group_count = u32_at(&mh, 0x4) as usize;
        let tag_table = u32_at(&mh, 0x8) as u64;
        let scenario = DatumIndex(u32_at(&mh, 0xC));
        let globals = DatumIndex(u32_at(&mh, 0x10));
        let tag_count = u32_at(&mh, 0x18) as usize;

        if group_count > 0x1000 || tag_count > 0x10000 {
            return Err(Error::Corrupt(format!(
                "implausible counts: {group_count} groups, {tag_count} tags"
            )));
        }

        let gb = read_at(&mut reader, meta + group_table, group_count * 0xC)?;
        let groups = gb
            .chunks_exact(0xC)
            .map(|c| TagGroup {
                tag: GroupTag(u32_at(c, 0)),
                parent: GroupTag(u32_at(c, 4)),
                grandparent: GroupTag(u32_at(c, 8)),
            })
            .collect();

        let tb = read_at(&mut reader, meta + tag_table, tag_count * 0x10)?;
        let tags = tb
            .chunks_exact(0x10)
            .enumerate()
            .map(|(i, c)| Tag {
                group: GroupTag(u32_at(c, 0)),
                datum: DatumIndex(u32_at(c, 4)),
                address: u32_at(c, 8),
                size: u32_at(c, 0xC),
                name: names.get(i).cloned().unwrap_or_default(),
            })
            .collect();

        Ok(CacheFile {
            reader,
            header,
            groups,
            tags,
            scenario,
            globals,
            strings,
        })
    }

    /// Convert a meta memory address into a file offset.
    pub fn pointer_to_offset(&self, address: u32) -> Option<u64> {
        let rel = address.checked_sub(self.header.meta_mask)?;
        let off = self.header.meta_offset as u64 + rel as u64;
        let end = self.header.meta_offset as u64 + self.header.meta_size as u64;
        (off < end).then_some(off)
    }

    pub fn tag(&self, datum: DatumIndex) -> Option<&Tag> {
        self.tags
            .get(datum.index() as usize)
            .filter(|t| t.datum == datum)
    }

    pub fn find_tag(&self, group: GroupTag, name: &str) -> Option<&Tag> {
        self.tags
            .iter()
            .find(|t| t.group == group && t.name == name)
    }

    /// Read a tag's raw meta bytes.
    pub fn read_tag_data(&mut self, tag: &Tag) -> Result<Vec<u8>> {
        if !tag.has_data() {
            return Ok(Vec::new());
        }
        let off = self.pointer_to_offset(tag.address).ok_or_else(|| {
            Error::Corrupt(format!(
                "tag {} address {:#x} outside meta",
                tag.name, tag.address
            ))
        })?;
        read_at(&mut self.reader, off, tag.size as usize)
    }

    /// Read `len` bytes at a meta memory address (e.g. the target of a tag block pointer).
    pub fn read_pointer(&mut self, address: u32, len: usize) -> Result<Vec<u8>> {
        let off = self
            .pointer_to_offset(address)
            .ok_or_else(|| Error::Corrupt(format!("address {address:#x} outside meta")))?;
        read_at(&mut self.reader, off, len)
    }

    /// Resolve a string id (low 24 bits index, high 8 bits length) to its text.
    pub fn string_id(&self, id: u32) -> Option<&str> {
        if id == 0 {
            return Some("");
        }
        self.strings
            .get((id & 0x00FF_FFFF) as usize)
            .map(String::as_str)
    }

    pub fn strings(&self) -> &[String] {
        &self.strings
    }
}

fn read_string_table<R: Read + Seek>(
    r: &mut R,
    index_offset: u32,
    data_offset: u32,
    data_size: u32,
    count: u32,
) -> Result<Vec<String>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > 0x100000 || data_size > 0x4000000 {
        return Err(Error::Corrupt("implausible string table size".into()));
    }
    let index = read_at(r, index_offset as u64, count as usize * 4)?;
    let data = read_at(r, data_offset as u64, data_size as usize)?;
    Ok(index
        .chunks_exact(4)
        .map(|c| {
            let start = i32_at(c, 0);
            if start < 0 || start as usize >= data.len() {
                String::new()
            } else {
                cstr(&data[start as usize..])
            }
        })
        .collect())
}

fn read_at<R: Read + Seek>(r: &mut R, offset: u64, len: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    r.seek(SeekFrom::Start(offset))?;
    r.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

pub fn i32_at(b: &[u8], o: usize) -> i32 {
    u32_at(b, o) as i32
}

/// Magic values are stored little-endian, so `head` appears on disk as `daeh`.
fn be_magic(b: &[u8], o: usize) -> u32 {
    u32_at(b, o)
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

#[cfg(test)]
mod tests;
