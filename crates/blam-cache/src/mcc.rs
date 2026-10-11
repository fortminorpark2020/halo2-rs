//! MCC's Halo 2 maps (cache format 13), read only as far as the launcher
//! needs: find a tag by name and group, read its meta and the blocks nested
//! in it, resolve tag references and string ids, and decode a bitmap whose
//! pixels are in the `textures.dat` beside the maps. Nothing here is shared
//! with the Vista reader in `lib.rs`, whose behaviour is unchanged. The
//! format, in our own words, is in `docs/notes/launcher/mcc-maps.md`.
//!
//! A format-13 map is a 0x380-byte header, kept as it is, then everything
//! else as zlib streams that each inflate to 256 KiB. The header followed by
//! the inflated chunks is the map's "image", and every offset the header
//! gives (but the chunk table's) is an offset in that image. The tag index
//! is Halo 2 Vista's, with addresses counted from the index's start instead
//! of memory addresses.
//!
//! Some of what is read here is a prediction from Halo 2 Vista's layouts
//! that no real file has confirmed yet: the string-id table's header fields,
//! the English string table, bitmap sequences, the bitmap entry's fields
//! other than size, format, pointer and stored size, `textures.dat` records
//! of more than one stream, padded rows, and pointers with a top bit set.
//! Each says so where it is read, and stays unverified until the console
//! probe (`examples/mcc_ui_probe.rs`) has run on the owner's PC.
//!
//! Every size read from a file is checked against the file and against a
//! limit before anything is allocated for it, so a damaged or hostile file
//! gives an error, never a huge allocation or a panic. Chunks are inflated
//! when a read first needs them and a few are kept.

use crate::bitmap::{self, Format, Image, Sequence};
use crate::{cstr, u32_at, DatumIndex, Error, GroupTag, Result};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// MCC's Halo 2 cache version (Halo 2 Vista's is 8).
pub const VERSION: i32 = 13;
/// The header, stored as it is at the start of the file.
pub const HEADER_SIZE: usize = 0x380;
/// One image entry in a bitmap tag (Halo 2 Vista's are 116 bytes).
pub const BITMAP_ENTRY_SIZE: usize = 168;
/// A tag reference in a tag's meta: group, then datum (as in Vista).
pub const TAG_REF_SIZE: usize = 8;

const HEAD_MAGIC: u32 = u32::from_be_bytes(*b"head");
const FOOT_MAGIC: u32 = u32::from_be_bytes(*b"foot");
const TAGS_MAGIC: u32 = u32::from_be_bytes(*b"tags");
const BITM: GroupTag = GroupTag(u32::from_be_bytes(*b"bitm"));
const MATG: GroupTag = GroupTag(u32::from_be_bytes(*b"matg"));
/// The tag index's own header.
const INDEX_HEADER_SIZE: usize = 0x20;
/// One tag in the index: group, datum, address, size.
const TAG_ENTRY_SIZE: usize = 16;
/// A chunk table entry: compressed size, then where it is in the file.
const CHUNK_ENTRY_SIZE: usize = 8;
/// In a bitm tag's meta: the bitmaps block's count, then its address.
const BITM_BITMAPS: usize = 0x44;
/// A one-stream `textures.dat` record's own header: the stream count (1)
/// and the stream's size (negative when it is stored raw).
const RECORD_HEADER_SIZE: u64 = 8;
/// The top two bits of a pixel pointer. In Halo 2 Vista they say which file
/// holds the pixels; every rank icon has them clear and is in textures.dat.
/// A pointer with either set is read with them masked off, as other
/// readers of MCC's maps do (unverified for Halo 2 until the probe runs).
const POINTER_LOCATION_BITS: u32 = 0xC000_0000;
/// In matg's meta, the English string table's count, size, index offset
/// and text offset (Halo 2 Vista's place, predicted to stay).
const MATG_ENGLISH: usize = 0x190;
/// A language table offset with this bit is in the shared map (Vista).
const LANGUAGE_SHARED: u32 = 0x8000_0000;

/// Limits on sizes read from the file. MCC's maps use 0x40000-byte chunks;
/// anything from 16 bytes to 16 MiB is taken so tests can use small ones.
const MIN_CHUNK: u32 = 16;
const MAX_CHUNK: u32 = 16 << 20;
/// Tags: a datum's index is 16 bits.
const MAX_TAGS: u32 = 0x1_0000;
/// The tag names together (estimate: a large map's are a few MiB).
const MAX_NAMES: u32 = 64 << 20;
/// String ids and English strings (estimate, as the Vista reader allows).
const MAX_STRINGS: u32 = 0x10_0000;
/// The most read from the image at once (a tag's meta, a block).
const MAX_READ: usize = 64 << 20;
/// Elements in one nested block (as the Vista reader allows).
const MAX_BLOCK: u32 = 0x10_0000;
/// Images in one bitmap tag.
const MAX_BITMAPS: u32 = 0xFFFF;
/// A bitmap's side, in pixels (estimate: Halo 2's largest are 2048).
const MAX_SIDE: u16 = 8192;
/// Zlib streams in one textures.dat record (estimate: a 4096 square image
/// in 256 KiB pieces is 256).
const MAX_STREAMS: u32 = 1024;
/// Row alignments tried for padded rows, after none (estimates: the
/// alignments Direct3D and its tools commonly use).
const ROW_ALIGNMENTS: [usize; 5] = [16, 32, 64, 128, 256];
/// Inflated chunks kept for later reads.
const CACHE_CHUNKS: usize = 8;

/// The header fields this reader uses. Offsets are in the image (the
/// header followed by the inflated chunks) unless said otherwise.
#[derive(Debug, Clone)]
pub struct Header {
    pub version: i32,
    /// The image's size: the header plus every chunk inflated.
    pub image_size: u32,
    /// Where the tag index starts.
    pub index_offset: u32,
    /// The index and the meta after it.
    pub index_size: u32,
    pub name_count: u32,
    pub names_offset: u32,
    pub names_size: u32,
    /// A u32 per tag: where its name starts in the names buffer.
    pub name_table_offset: u32,
    /// The string ids, in the same shape as the tag names just before
    /// them: how many, where their text starts and its size, and where the
    /// table of a u32 per string (its text's place) starts. A prediction,
    /// unverified until the probe runs on the owner's PC.
    pub string_count: u32,
    pub strings_offset: u32,
    pub strings_size: u32,
    pub string_index_offset: u32,
    pub map_name: String,
    pub scenario_path: String,
    /// Where the meta starts, counted from the index's start.
    pub meta_start: u32,
    pub meta_size: u32,
    /// The words at 0x2E4 and 0x2E8, thought to place the "locale globals"
    /// (the string tables); 0xFFFFFFFF when unused. Unverified.
    pub locale: [u32; 2],
    /// The inflated size of every chunk but the last.
    pub chunk_size: u32,
    /// In the file as stored, not the image.
    pub chunk_table_offset: u32,
    pub chunk_count: u32,
}

impl Header {
    /// The header from the file's first bytes. A Halo 2 Vista map says
    /// "version 8" here rather than failing on `foot`.
    pub fn parse(b: &[u8]) -> Result<Header> {
        if b.len() < 8 {
            return Err(Error::BadMagic("file shorter than a header"));
        }
        if u32_at(b, 0) != HEAD_MAGIC {
            return Err(Error::BadMagic("missing 'head'"));
        }
        let version = crate::i32_at(b, 4);
        if version != VERSION {
            return Err(Error::WrongVersion {
                found: version,
                expected: VERSION,
            });
        }
        if b.len() < HEADER_SIZE {
            return Err(Error::BadMagic("file shorter than MCC's header"));
        }
        if u32_at(b, HEADER_SIZE - 4) != FOOT_MAGIC {
            return Err(Error::BadMagic("missing 'foot' at 0x37C"));
        }
        Ok(Header {
            version,
            image_size: u32_at(b, 0x08),
            index_offset: u32_at(b, 0x10),
            index_size: u32_at(b, 0x14),
            name_count: u32_at(b, 0x20),
            names_offset: u32_at(b, 0x24),
            names_size: u32_at(b, 0x28),
            name_table_offset: u32_at(b, 0x2C),
            string_count: u32_at(b, 0x30),
            strings_offset: u32_at(b, 0x34),
            strings_size: u32_at(b, 0x38),
            string_index_offset: u32_at(b, 0x3C),
            // The two strings' lengths are estimates: each runs to the next
            // field we know of.
            map_name: cstr(&b[0xB0..0xD0]),
            scenario_path: cstr(&b[0xD0..0x1D0]),
            meta_start: u32_at(b, 0x2D4),
            meta_size: u32_at(b, 0x2D8),
            locale: [u32_at(b, 0x2E4), u32_at(b, 0x2E8)],
            chunk_size: u32_at(b, 0x308),
            chunk_table_offset: u32_at(b, 0x310),
            chunk_count: u32_at(b, 0x314),
        })
    }
}

/// The tag index's header fields kept.
#[derive(Debug, Clone, Copy)]
pub struct Index {
    pub scenario: DatumIndex,
    pub globals: DatumIndex,
    pub checksum: u32,
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub group: GroupTag,
    pub datum: DatumIndex,
    /// Where the tag's meta starts, counted from the tag index's start; 0
    /// or 0xFFFFFFFF when it has none in this map.
    pub address: u32,
    pub size: u32,
    pub name: String,
}

impl Tag {
    pub fn has_data(&self) -> bool {
        self.address != 0 && self.address != u32::MAX && self.size != 0
    }

    /// Whether `len` bytes from `address` (counted from the index's start,
    /// as nested blocks are) lie inside this tag's meta.
    pub fn holds(&self, address: u32, len: usize) -> bool {
        let (start, end) = (
            u64::from(self.address),
            u64::from(self.address) + u64::from(self.size),
        );
        self.has_data() && u64::from(address) >= start && u64::from(address) + len as u64 <= end
    }
}

/// The string ids' names, by index. Halo 2 packs a string id as the name's
/// length in the top 8 bits and its index in the low 24, as Vista does;
/// index numbers may differ from Vista's, so names are matched, not
/// numbers.
#[derive(Debug, Clone, Default)]
pub struct StringIds {
    pub names: Vec<String>,
    /// Where each name starts in the text.
    pub offsets: Vec<u32>,
    /// The text's size.
    pub size: u32,
}

impl StringIds {
    /// The name of string id `id`; 0 is the empty name.
    pub fn get(&self, id: u32) -> Option<&str> {
        if id == 0 {
            return Some("");
        }
        self.names
            .get((id & 0x00FF_FFFF) as usize)
            .map(String::as_str)
    }

    /// The id of the string named `name`, packed as Halo 2 packs them.
    pub fn find(&self, name: &str) -> Option<u32> {
        let index = self.names.iter().position(|n| n == name)?;
        Some(pack_string_id(index, name))
    }

    /// Where the text of the string that ends last ends, past its NUL.
    pub fn end(&self) -> usize {
        self.offsets
            .iter()
            .zip(&self.names)
            .map(|(&o, n)| o as usize + n.len() + 1)
            .max()
            .unwrap_or(0)
    }
}

/// A string id from its index and name: the length in the top 8 bits.
pub fn pack_string_id(index: usize, name: &str) -> u32 {
    ((name.len().min(0xFF) as u32) << 24) | (index as u32 & 0x00FF_FFFF)
}

/// A language's strings, each with its string id, and where the table was
/// found (for the probe's log).
#[derive(Debug, Clone, Default)]
pub struct Language {
    pub source: String,
    pub strings: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Copy)]
struct Chunk {
    compressed: u32,
    offset: u32,
}

/// An open format-13 map.
pub struct Map<R> {
    reader: R,
    file_len: u64,
    raw_header: Vec<u8>,
    pub header: Header,
    chunks: Vec<Chunk>,
    /// Inflated chunks by number, the most recently used last.
    cache: Vec<(usize, Vec<u8>)>,
    pub index: Index,
    pub tags: Vec<Tag>,
    /// Read on first use (`read_string_ids`), since its layout is a
    /// prediction and the rank icons don't need it.
    string_ids: Option<StringIds>,
}

impl Map<BufReader<File>> {
    pub fn open(path: &Path) -> Result<Self> {
        Map::from_reader(BufReader::new(File::open(path)?))
    }
}

impl<R: Read + Seek> Map<R> {
    /// Reads the header, the chunk table, the tag index and the names.
    pub fn from_reader(mut reader: R) -> Result<Self> {
        let file_len = reader.seek(SeekFrom::End(0))?;
        let mut raw_header = vec![0u8; HEADER_SIZE.min(file_len as usize)];
        reader.seek(SeekFrom::Start(0))?;
        reader.read_exact(&mut raw_header)?;
        let header = Header::parse(&raw_header)?;
        let chunks = chunk_table(&mut reader, file_len, &header)?;
        let mut map = Map {
            reader,
            file_len,
            raw_header,
            header,
            chunks,
            cache: Vec::new(),
            index: Index {
                scenario: DatumIndex::NONE,
                globals: DatumIndex::NONE,
                checksum: 0,
            },
            tags: Vec::new(),
            string_ids: None,
        };
        map.read_index()?;
        Ok(map)
    }

    fn read_index(&mut self) -> Result<()> {
        let at = u64::from(self.header.index_offset);
        let ih = self.read_image(at, INDEX_HEADER_SIZE)?;
        if u32_at(&ih, 0x1C) != TAGS_MAGIC {
            return Err(Error::BadMagic("missing 'tags' in the tag index"));
        }
        self.index = Index {
            scenario: DatumIndex(u32_at(&ih, 0x0C)),
            globals: DatumIndex(u32_at(&ih, 0x10)),
            checksum: u32_at(&ih, 0x14),
        };
        let count = u32_at(&ih, 0x18);
        if count > MAX_TAGS {
            return Err(Error::Corrupt(format!("{count} tags is too many")));
        }
        let table = self.read_image(
            at + u64::from(u32_at(&ih, 0x08)),
            count as usize * TAG_ENTRY_SIZE,
        )?;
        let names = self.read_names()?;
        self.tags = table
            .as_chunks::<TAG_ENTRY_SIZE>()
            .0
            .iter()
            .enumerate()
            .map(|(i, e)| Tag {
                group: GroupTag(u32_at(e, 0)),
                datum: DatumIndex(u32_at(e, 4)),
                address: u32_at(e, 8),
                size: u32_at(e, 12),
                name: names.get(i).cloned().unwrap_or_default(),
            })
            .collect();
        Ok(())
    }

    /// Every tag's name, in tag order.
    fn read_names(&mut self) -> Result<Vec<String>> {
        let h = &self.header;
        let (count, size) = (h.name_count, h.names_size);
        let (buffer_at, table_at) = (h.names_offset, h.name_table_offset);
        if count > MAX_TAGS {
            return Err(Error::Corrupt(format!("{count} tag names is too many")));
        }
        if size > MAX_NAMES {
            return Err(Error::Corrupt(format!(
                "tag names take {size} bytes, more than {MAX_NAMES}"
            )));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let buffer = self.read_image(u64::from(buffer_at), size as usize)?;
        let table = self.read_image(u64::from(table_at), count as usize * 4)?;
        table
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let o = u32_at(o, 0) as usize;
                buffer
                    .get(o..)
                    .map(cstr)
                    .ok_or_else(|| Error::Corrupt(format!("tag {i}'s name is past the names")))
            })
            .collect()
    }

    /// `len` bytes of the image from `offset`, inflating chunks as needed.
    pub fn read_image(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        if len > MAX_READ {
            return Err(Error::Corrupt(format!(
                "a read of {len} bytes is more than {MAX_READ}"
            )));
        }
        let end = offset + len as u64;
        let size = u64::from(self.header.image_size);
        if end > size {
            return Err(Error::Corrupt(format!(
                "bytes {offset:#x}..{end:#x} are past the end of the map's {size:#x}"
            )));
        }
        let mut out = Vec::with_capacity(len);
        let mut pos = offset;
        while pos < end {
            if pos < HEADER_SIZE as u64 {
                let stop = end.min(HEADER_SIZE as u64);
                out.extend_from_slice(&self.raw_header[pos as usize..stop as usize]);
                pos = stop;
                continue;
            }
            let chunk_size = u64::from(self.header.chunk_size);
            let n = ((pos - HEADER_SIZE as u64) / chunk_size) as usize;
            let start = HEADER_SIZE as u64 + n as u64 * chunk_size;
            let slot = self.chunk(n)?;
            let data = &self.cache[slot].1;
            let from = (pos - start) as usize;
            let stop = data.len().min(from + (end - pos) as usize);
            out.extend_from_slice(&data[from..stop]);
            pos = start + stop as u64;
        }
        Ok(out)
    }

    /// Chunk `n` inflated, as a place in the cache.
    fn chunk(&mut self, n: usize) -> Result<usize> {
        if let Some(i) = self.cache.iter().position(|(c, _)| *c == n) {
            let hit = self.cache.remove(i);
            self.cache.push(hit);
            return Ok(self.cache.len() - 1);
        }
        let c = *self
            .chunks
            .get(n)
            .ok_or_else(|| Error::Corrupt(format!("no chunk {n}")))?;
        let chunk_size = u64::from(self.header.chunk_size);
        let body = u64::from(self.header.image_size) - HEADER_SIZE as u64;
        let want = chunk_size.min(body - n as u64 * chunk_size);
        let data = inflate(
            &mut self.reader,
            u64::from(c.offset),
            u64::from(c.compressed),
            want as usize,
        )
        .map_err(|e| Error::Corrupt(format!("chunk {n}: {e}")))?;
        if data.len() as u64 != want {
            return Err(Error::Corrupt(format!(
                "chunk {n} inflates to {} bytes, not {want}",
                data.len()
            )));
        }
        if self.cache.len() >= CACHE_CHUNKS {
            self.cache.remove(0);
        }
        self.cache.push((n, data));
        Ok(self.cache.len() - 1)
    }

    /// The file's length as stored.
    pub fn file_len(&self) -> u64 {
        self.file_len
    }

    /// The u32 at `offset` in the 0x380-byte header, for printing fields
    /// whose meaning isn't known yet.
    pub fn header_word(&self, offset: usize) -> Option<u32> {
        self.raw_header
            .get(offset..offset.checked_add(4)?)
            .map(|b| u32_at(b, 0))
    }

    /// The tag of `group` named `name` (letter case ignored, as Windows
    /// paths are).
    pub fn find_tag(&self, group: GroupTag, name: &str) -> Option<&Tag> {
        self.tags
            .iter()
            .find(|t| t.group == group && t.name.eq_ignore_ascii_case(name))
    }

    /// The tag a datum names: the one at its index when the salt matches
    /// (as in Vista), else any with that datum.
    pub fn tag(&self, datum: DatumIndex) -> Option<&Tag> {
        if datum == DatumIndex::NONE {
            return None;
        }
        self.tags
            .get(usize::from(datum.index()))
            .filter(|t| t.datum == datum)
            .or_else(|| self.tags.iter().find(|t| t.datum == datum))
    }

    /// The tag a tag reference in `meta` at `at` names, if its group
    /// matches the tag's.
    pub fn resolve(&self, meta: &[u8], at: usize) -> Option<&Tag> {
        let (group, datum) = tag_ref(meta, at)?;
        self.tag(datum).filter(|t| t.group == group)
    }

    /// A tag's meta bytes; empty when it has none in this map.
    pub fn tag_meta(&mut self, tag: &Tag) -> Result<Vec<u8>> {
        if !tag.has_data() {
            return Ok(Vec::new());
        }
        let at = u64::from(self.header.index_offset) + u64::from(tag.address);
        self.read_image(at, tag.size as usize)
            .map_err(|e| Error::Corrupt(format!("tag {}: {e}", tag.name)))
    }

    /// The elements of a block nested in a tag's meta, `size` bytes each,
    /// one after another. Its header is at `at` in `parent`: the count, then
    /// the address, counted from the tag index's start as tags' addresses
    /// are (seen for bitm's bitmaps block; predicted for the rest).
    pub fn block(&mut self, parent: &[u8], at: usize, size: usize) -> Result<Vec<u8>> {
        let (count, address) = block_header(parent, at)?;
        if count == 0 || size == 0 {
            return Ok(Vec::new());
        }
        let len = count as usize * size;
        let end = u64::from(address) + len as u64;
        if end > u64::from(self.header.index_size) {
            return Err(Error::Corrupt(format!(
                "a block of {count} at {address:#x} runs past the index and meta ({:#x})",
                self.header.index_size
            )));
        }
        self.read_image(
            u64::from(self.header.index_offset) + u64::from(address),
            len,
        )
    }

    /// Reads the string-id table, if it isn't read yet. Its place in the
    /// header is predicted (`Header::string_count`), so every offset is
    /// checked: one past the text says the prediction is wrong.
    pub fn read_string_ids(&mut self) -> Result<&StringIds> {
        if self.string_ids.is_none() {
            let ids = self.load_string_ids()?;
            self.string_ids = Some(ids);
        }
        Ok(self.string_ids.as_ref().expect("just read"))
    }

    fn load_string_ids(&mut self) -> Result<StringIds> {
        let h = &self.header;
        let (count, size) = (h.string_count, h.strings_size);
        let (text_at, index_at) = (h.strings_offset, h.string_index_offset);
        if count == 0 || count > MAX_STRINGS {
            return Err(Error::Corrupt(format!(
                "the header says {count} string ids (read at 0x30, a predicted place)"
            )));
        }
        if size == 0 || size > MAX_NAMES {
            return Err(Error::Corrupt(format!(
                "the header says the string ids take {size} bytes (read at 0x38, a predicted place)"
            )));
        }
        let text = self
            .read_image(u64::from(text_at), size as usize)
            .map_err(|e| Error::Corrupt(format!("string-id text: {e}")))?;
        let index = self
            .read_image(u64::from(index_at), count as usize * 4)
            .map_err(|e| Error::Corrupt(format!("string-id index: {e}")))?;
        let mut ids = StringIds {
            names: Vec::with_capacity(count as usize),
            offsets: Vec::with_capacity(count as usize),
            size,
        };
        for (i, o) in index.as_chunks::<4>().0.iter().enumerate() {
            let o = u32_at(o, 0);
            let Some(rest) = text.get(o as usize..).filter(|r| !r.is_empty()) else {
                return Err(Error::Corrupt(format!(
                    "string id {i} starts at {o:#x}, past the text's {size:#x} bytes"
                )));
            };
            ids.names.push(cstr(rest));
            ids.offsets.push(o);
        }
        Ok(ids)
    }

    /// The string-id table, once `read_string_ids` has read it.
    pub fn string_ids(&self) -> Option<&StringIds> {
        self.string_ids.as_ref()
    }

    /// A string id's name, once `read_string_ids` has read the table.
    pub fn string_id(&self, id: u32) -> Option<&str> {
        self.string_ids.as_ref()?.get(id)
    }

    /// The English strings, each with its string id. Where the table is
    /// is a prediction: the header's locale words (0x2E4) are tried as the
    /// place of Vista's 16-byte language entry (count, size, index offset,
    /// text offset), then matg's own at 0x190 as in Vista; both with their
    /// offsets in the image, then counted from the tag index. The first
    /// that lies inside the image and whose index entries (string id,
    /// offset) each land on the start of a string is taken.
    pub fn language_table(&mut self) -> Result<Language> {
        let mut tried = Vec::new();
        let mut candidates = Vec::new();
        let locale = self.header.locale[0];
        if locale != 0 && locale != u32::MAX {
            if let Ok(b) = self.read_image(u64::from(locale), 16) {
                candidates.push((format!("header 0x2E4 ({locale:#x})"), b));
            } else {
                tried.push(format!("header 0x2E4 ({locale:#x}): past the image"));
            }
        }
        let matg = self
            .tags
            .iter()
            .find(|t| t.group == MATG && t.has_data())
            .cloned();
        if let Some(matg) = matg {
            match self.tag_meta(&matg) {
                Ok(m) if m.len() >= MATG_ENGLISH + 16 => candidates.push((
                    format!("matg +{MATG_ENGLISH:#x}"),
                    m[MATG_ENGLISH..MATG_ENGLISH + 16].to_vec(),
                )),
                Ok(m) => tried.push(format!("matg: {} bytes of meta, too few", m.len())),
                Err(e) => tried.push(format!("matg: {e}")),
            }
        } else {
            tried.push("no matg with meta in this map".into());
        }
        let index_base = u64::from(self.header.index_offset);
        for (source, entry) in candidates {
            for (base, from) in [(0, "image"), (index_base, "index")] {
                match self.language_at(&entry, base) {
                    Ok(strings) => {
                        return Ok(Language {
                            source: format!("{source}, offsets in the {from}"),
                            strings,
                        })
                    }
                    Err(e) => tried.push(format!("{source}, offsets in the {from}: {e}")),
                }
            }
        }
        Err(Error::Corrupt(format!(
            "no English string table found ({})",
            tried.join("; ")
        )))
    }

    /// The strings of a 16-byte language entry whose offsets count from
    /// `base` in the image.
    fn language_at(&mut self, entry: &[u8], base: u64) -> Result<Vec<(u32, String)>> {
        let (count, size) = (u32_at(entry, 0), u32_at(entry, 4));
        let (index_at, text_at) = (u32_at(entry, 8), u32_at(entry, 12));
        if count == 0 || count > MAX_STRINGS || size == 0 || size > MAX_NAMES {
            return Err(Error::Corrupt(format!("{count} strings in {size} bytes")));
        }
        if (index_at | text_at) & LANGUAGE_SHARED != 0 {
            return Err(Error::Corrupt("the table is in shared.map".into()));
        }
        let index = self.read_image(base + u64::from(index_at), count as usize * 8)?;
        let text = self.read_image(base + u64::from(text_at), size as usize)?;
        index
            .as_chunks::<8>()
            .0
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let at = u32_at(e, 4) as usize;
                let starts = at < text.len() && (at == 0 || text[at - 1] == 0);
                if !starts {
                    return Err(Error::Corrupt(format!(
                        "string {i}'s offset {at:#x} isn't the start of a string"
                    )));
                }
                let rest = &text[at..];
                let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
                Ok((
                    u32_at(e, 0),
                    String::from_utf8_lossy(&rest[..end]).into_owned(),
                ))
            })
            .collect()
    }

    /// A bitm tag's image entries as stored, 168 bytes each.
    pub fn bitmap_block(&mut self, tag: &Tag) -> Result<Vec<u8>> {
        let meta = self.bitm_meta(tag)?;
        let count = u32_at(&meta, BITM_BITMAPS);
        if count > MAX_BITMAPS {
            return Err(Error::Corrupt(format!(
                "bitmap {} says it has {count} images",
                tag.name
            )));
        }
        self.block(&meta, BITM_BITMAPS, BITMAP_ENTRY_SIZE)
            .map_err(|e| Error::Corrupt(format!("bitmap {}: {e}", tag.name)))
    }

    /// A bitm tag's image entries, in order.
    pub fn bitmaps(&mut self, tag: &Tag) -> Result<Vec<BitmapEntry>> {
        Ok(self
            .bitmap_block(tag)?
            .as_chunks::<BITMAP_ENTRY_SIZE>()
            .0
            .iter()
            .map(|e| BitmapEntry::parse(e))
            .collect())
    }

    /// A bitm tag's sequences, with their sprites (Vista's layout: the
    /// block at 0x3C, 0x3C bytes each; predicted to stay).
    pub fn sequences(&mut self, tag: &Tag) -> Result<Vec<Sequence>> {
        let meta = self.bitm_meta(tag)?;
        let at = |e: Error| Error::Corrupt(format!("bitmap {}'s sequences: {e}", tag.name));
        let raw = self
            .block(&meta, bitmap::BITM_SEQUENCES, bitmap::SEQUENCE_SIZE)
            .map_err(at)?;
        let mut out = Vec::new();
        for s in raw.as_chunks::<{ bitmap::SEQUENCE_SIZE }>().0 {
            let sprites = self
                .block(s, bitmap::SEQUENCE_SPRITES, bitmap::SPRITE_SIZE)
                .map_err(at)?;
            out.push(bitmap::parse_sequence(s, &sprites));
        }
        Ok(out)
    }

    /// A bitm tag's meta, checked to be a bitm long enough for its blocks.
    fn bitm_meta(&mut self, tag: &Tag) -> Result<Vec<u8>> {
        if tag.group != BITM {
            return Err(Error::Corrupt(format!(
                "tag {} is a {}, not a bitm",
                tag.name, tag.group
            )));
        }
        let meta = self.tag_meta(tag)?;
        if meta.len() < BITM_BITMAPS + 8 {
            return Err(Error::Corrupt(format!(
                "bitmap {} has {} bytes of meta, too few",
                tag.name,
                meta.len()
            )));
        }
        Ok(meta)
    }
}

/// A tag reference's group and datum at `at` in `meta`, if it fits.
pub fn tag_ref(meta: &[u8], at: usize) -> Option<(GroupTag, DatumIndex)> {
    let b = meta.get(at..at.checked_add(TAG_REF_SIZE)?)?;
    Some((GroupTag(u32_at(b, 0)), DatumIndex(u32_at(b, 4))))
}

/// A nested block's header at `at` in `parent`: its count and address.
pub fn block_header(parent: &[u8], at: usize) -> Result<(u32, u32)> {
    let b = parent
        .get(at..at.saturating_add(8))
        .filter(|b| b.len() == 8)
        .ok_or_else(|| {
            Error::Corrupt(format!(
                "a block header at {at:#x} is past its parent's {:#x} bytes",
                parent.len()
            ))
        })?;
    let (count, address) = (u32_at(b, 0), u32_at(b, 4));
    if count > MAX_BLOCK {
        return Err(Error::Corrupt(format!(
            "a block at {address:#x} says it has {count} elements"
        )));
    }
    Ok((count, address))
}

/// Reads and checks the chunk table against the header and the file.
fn chunk_table<R: Read + Seek>(r: &mut R, file_len: u64, h: &Header) -> Result<Vec<Chunk>> {
    if (h.image_size as usize) < HEADER_SIZE {
        return Err(Error::Corrupt(format!(
            "the map says it is {} bytes, less than its header",
            h.image_size
        )));
    }
    if !(MIN_CHUNK..=MAX_CHUNK).contains(&h.chunk_size) {
        return Err(Error::Corrupt(format!(
            "chunk size {:#x} isn't between {MIN_CHUNK:#x} and {MAX_CHUNK:#x}",
            h.chunk_size
        )));
    }
    let body = u64::from(h.image_size) - HEADER_SIZE as u64;
    let want = body.div_ceil(u64::from(h.chunk_size));
    if u64::from(h.chunk_count) != want {
        return Err(Error::Corrupt(format!(
            "{} chunks for {body} bytes in chunks of {:#x}; expected {want}",
            h.chunk_count, h.chunk_size
        )));
    }
    let table_len = u64::from(h.chunk_count) * CHUNK_ENTRY_SIZE as u64;
    let table_end = u64::from(h.chunk_table_offset) + table_len;
    if table_end > file_len {
        return Err(Error::Corrupt(format!(
            "the chunk table ends at {table_end:#x}, past the file's end ({file_len:#x}): truncated?"
        )));
    }
    let mut table = vec![0u8; table_len as usize];
    r.seek(SeekFrom::Start(u64::from(h.chunk_table_offset)))?;
    r.read_exact(&mut table)?;
    // zlib grows data that doesn't compress by a few bytes per 16 KiB;
    // half as much again is far more than that.
    let most = u64::from(h.chunk_size) * 3 / 2 + 1024;
    table
        .as_chunks::<CHUNK_ENTRY_SIZE>()
        .0
        .iter()
        .enumerate()
        .map(|(n, e)| {
            let compressed = crate::i32_at(e, 0);
            let offset = u32_at(e, 4);
            if compressed <= 0 || compressed as u64 > most {
                return Err(Error::Corrupt(format!(
                    "chunk {n} says it is {compressed} bytes compressed"
                )));
            }
            let end = u64::from(offset) + compressed as u64;
            if end > file_len {
                return Err(Error::Corrupt(format!(
                    "chunk {n} ends at {end:#x}, past the file's end ({file_len:#x}): truncated?"
                )));
            }
            Ok(Chunk {
                compressed: compressed as u32,
                offset,
            })
        })
        .collect()
}

/// Inflates the zlib stream of `compressed` bytes at `offset`, keeping at
/// most `most + 1` bytes (one more than wanted, so a stream that is too
/// long shows).
fn inflate<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    compressed: u64,
    most: usize,
) -> std::io::Result<Vec<u8>> {
    r.seek(SeekFrom::Start(offset))?;
    // zlib inflates at most about 1032 times, so a short stream that claims
    // a big result doesn't get that much reserved for it up front.
    let likely = usize::try_from(compressed)
        .unwrap_or(usize::MAX)
        .saturating_mul(1032);
    let mut out = Vec::with_capacity(most.min(likely));
    flate2::read::ZlibDecoder::new(r.take(compressed))
        .take(most as u64 + 1)
        .read_to_end(&mut out)?;
    Ok(out)
}

/// One stream of a textures.dat record: inflated, or copied if it is
/// stored raw, keeping at most `most + 1` bytes either way.
fn read_stream<R: Read + Seek>(r: &mut R, s: &Stream, most: usize) -> std::io::Result<Vec<u8>> {
    if !s.raw {
        return inflate(r, s.offset, s.size.into(), most);
    }
    r.seek(SeekFrom::Start(s.offset))?;
    let keep = u64::from(s.size).min(most as u64 + 1);
    let mut out = Vec::with_capacity(keep as usize);
    r.take(keep).read_to_end(&mut out)?;
    Ok(out)
}

/// One image of a bitm tag: the fields read from its 168 bytes. Width,
/// height, format, pointer and stored size are seen on the rank icons; the
/// rest are at Halo 2 Vista's places, predicted to stay, and unverified
/// until the probe runs on the owner's PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapEntry {
    pub width: u16,
    pub height: u16,
    /// A 3D texture's depth (0x8).
    pub depth: u8,
    /// 2D, 3D, cube map or white, in Vista's numbering (0xA).
    pub kind: u16,
    pub format: Format,
    /// Vista's flags word (0xE): power of two, compressed, palettized and
    /// so on.
    pub flags: u16,
    /// The registration point (0x10).
    pub registration: [i16; 2],
    /// Mip levels below the top one (0x14).
    pub mip_count: u16,
    /// Where its pixels start in textures.dat: the first of Vista's six
    /// level-of-detail offsets.
    pub pointer: u32,
    /// The record there: its header and its zlib streams (the first of the
    /// six level-of-detail sizes).
    pub stored_size: u32,
    /// The other five level-of-detail offsets and sizes (0x3C and 0x54).
    pub lod_pointers: [u32; 5],
    pub lod_sizes: [u32; 5],
}

impl Default for BitmapEntry {
    fn default() -> Self {
        BitmapEntry {
            width: 0,
            height: 0,
            depth: 0,
            kind: 0,
            format: Format::A8R8G8B8,
            flags: 0,
            registration: [0; 2],
            mip_count: 0,
            pointer: 0,
            stored_size: 0,
            lod_pointers: [0; 5],
            lod_sizes: [0; 5],
        }
    }
}

impl BitmapEntry {
    /// Where the pixel pointer and the stored size are in an entry (the
    /// first of six level-of-detail offsets, and of six sizes).
    pub const POINTER_AT: usize = 0x38;
    pub const STORED_SIZE_AT: usize = 0x50;

    /// From an entry's 168 bytes.
    pub fn parse(e: &[u8]) -> BitmapEntry {
        let u16_at = |o: usize| u16::from_le_bytes([e[o], e[o + 1]]);
        let words = |at: usize| std::array::from_fn(|k| u32_at(e, at + 4 * k));
        BitmapEntry {
            width: u16_at(4),
            height: u16_at(6),
            depth: e[8],
            kind: u16_at(0xA),
            format: Format::from(crate::i16_at(e, 0x0C)),
            flags: u16_at(0xE),
            registration: [crate::i16_at(e, 0x10), crate::i16_at(e, 0x12)],
            mip_count: u16_at(0x14),
            pointer: u32_at(e, Self::POINTER_AT),
            stored_size: u32_at(e, Self::STORED_SIZE_AT),
            lod_pointers: words(Self::POINTER_AT + 4),
            lod_sizes: words(Self::STORED_SIZE_AT + 4),
        }
    }

    /// The entry as stored (the inverse of `parse`; other bytes are 0).
    pub fn to_bytes(&self) -> [u8; BITMAP_ENTRY_SIZE] {
        let mut b = [0u8; BITMAP_ENTRY_SIZE];
        let mut put = |o: usize, v: &[u8]| b[o..o + v.len()].copy_from_slice(v);
        put(4, &self.width.to_le_bytes());
        put(6, &self.height.to_le_bytes());
        put(8, &[self.depth]);
        put(0xA, &self.kind.to_le_bytes());
        put(0xC, &self.format.number().to_le_bytes());
        put(0xE, &self.flags.to_le_bytes());
        put(0x10, &self.registration[0].to_le_bytes());
        put(0x12, &self.registration[1].to_le_bytes());
        put(0x14, &self.mip_count.to_le_bytes());
        put(Self::POINTER_AT, &self.pointer.to_le_bytes());
        put(Self::STORED_SIZE_AT, &self.stored_size.to_le_bytes());
        for k in 0..5 {
            put(
                Self::POINTER_AT + 4 + 4 * k,
                &self.lod_pointers[k].to_le_bytes(),
            );
            put(
                Self::STORED_SIZE_AT + 4 + 4 * k,
                &self.lod_sizes[k].to_le_bytes(),
            );
        }
        b
    }
}

/// How a textures.dat record lays out its streams. A record of one stream
/// is its count (1), its size, then the stream, as the rank icons show.
/// Other readers of MCC's maps lay out any record as `SizesFirst`, each
/// stream zlib, or stored raw when its size is written negative; that is
/// tried first. Records of more than one stream aren't seen yet, so the
/// two other shapes that agree with the one-stream case are tried after
/// it, and the first whose sizes add up to the bitmap's stored size and
/// whose zlib streams each start with a zlib header is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordLayout {
    /// The count (1), the stream's size, the stream.
    Single,
    /// The count, every stream's size, then the streams.
    SizesFirst,
    /// The count, then each stream's size just before it.
    Interleaved,
    /// The count, the streams' total size, every stream's size, then the
    /// streams.
    TotalThenSizes,
}

/// Where a textures.dat record's streams are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Where it starts in textures.dat: the pointer, top bits masked off.
    pub offset: u64,
    /// The pointer's top two bits (0 for every rank icon).
    pub location: u32,
    pub layout: RecordLayout,
    pub streams: Vec<Stream>,
}

/// One stream of a textures.dat record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    /// Where it starts in textures.dat.
    pub offset: u64,
    /// Its size in bytes.
    pub size: u32,
    /// Stored as is rather than zlib (its size is written negative).
    pub raw: bool,
}

impl Stream {
    /// The stream at `offset` whose size is written as `word`.
    fn new(offset: u64, word: u32) -> Stream {
        let signed = word as i32;
        Stream {
            offset,
            size: signed.unsigned_abs(),
            raw: signed < 0,
        }
    }
}

/// An image decoded, with how its record was read (for the probe).
#[derive(Debug, Clone)]
pub struct Decoded {
    pub image: Image,
    pub record: Record,
    /// The streams inflated (or copied, if raw) together, in bytes.
    pub inflated: usize,
    /// Bytes from one stored row's start to the next's, and the bytes of
    /// pixels in a row (DXT counts rows of 4 by 4 blocks).
    pub row_pitch: usize,
    pub row_bytes: usize,
}

/// MCC's `textures.dat`, which holds the maps' bitmap pixels.
pub struct Textures<R> {
    reader: R,
    len: u64,
}

impl Textures<BufReader<File>> {
    pub fn open(path: &Path) -> Result<Self> {
        Textures::from_reader(BufReader::new(File::open(path)?))
    }
}

impl<R: Read + Seek> Textures<R> {
    pub fn from_reader(mut reader: R) -> Result<Self> {
        let len = reader.seek(SeekFrom::End(0))?;
        Ok(Textures { reader, len })
    }

    /// The file's length.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn words(&mut self, at: u64, count: usize) -> Result<Vec<u32>> {
        let mut b = vec![0u8; count * 4];
        self.reader.seek(SeekFrom::Start(at))?;
        self.reader.read_exact(&mut b)?;
        Ok(b.as_chunks::<4>().0.iter().map(|w| u32_at(w, 0)).collect())
    }

    /// Whether a zlib header starts at `at`.
    fn zlib_starts(&mut self, at: u64) -> bool {
        let mut b = [0u8; 2];
        let read = self
            .reader
            .seek(SeekFrom::Start(at))
            .and_then(|_| self.reader.read_exact(&mut b));
        read.is_ok()
            && b[0] & 0x0F == 8
            && b[0] >> 4 <= 7
            && (u16::from(b[0]) * 256 + u16::from(b[1])) % 31 == 0
    }

    /// Where the record of an image's pixels is and how its streams lie,
    /// checked against the bitmap's stored size.
    pub fn record(&mut self, e: &BitmapEntry) -> Result<Record> {
        let location = e.pointer >> 30;
        let at = u64::from(e.pointer & !POINTER_LOCATION_BITS);
        if at + RECORD_HEADER_SIZE > self.len {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x} are past textures.dat's end ({:#x})",
                self.len
            )));
        }
        let count = self.words(at, 1)?[0];
        if count == 0 || count > MAX_STREAMS {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x} are in {count} chunks, more than {MAX_STREAMS} or none"
            )));
        }
        let stored = u64::from(e.stored_size);
        if count == 1 {
            let stream = Stream::new(at + RECORD_HEADER_SIZE, self.words(at + 4, 1)?[0]);
            let size = RECORD_HEADER_SIZE + u64::from(stream.size);
            if stream.size == 0 || at + size > self.len {
                return Err(Error::Corrupt(format!(
                    "pixels at {at:#x}: {} bytes stored, past textures.dat's end ({:#x})",
                    stream.size, self.len
                )));
            }
            if stored != size {
                return Err(Error::Corrupt(format!(
                    "pixels at {at:#x}: the bitmap says {stored} bytes, textures.dat {size}"
                )));
            }
            return Ok(Record {
                offset: at,
                location,
                layout: RecordLayout::Single,
                streams: vec![stream],
            });
        }
        let n = count as usize;
        let mut tried = Vec::new();
        for layout in [
            RecordLayout::SizesFirst,
            RecordLayout::Interleaved,
            RecordLayout::TotalThenSizes,
        ] {
            match self.streams(at, n, layout) {
                Ok((streams, size)) if size == stored => {
                    if let Some(k) =
                        (0..n).find(|&k| !streams[k].raw && !self.zlib_starts(streams[k].offset))
                    {
                        tried.push(format!("{layout:?}: stream {k} isn't zlib"));
                        continue;
                    }
                    return Ok(Record {
                        offset: at,
                        location,
                        layout,
                        streams,
                    });
                }
                Ok((_, size)) => tried.push(format!("{layout:?}: {size} bytes")),
                Err(e) => tried.push(format!("{layout:?}: {e}")),
            }
        }
        Err(Error::Corrupt(format!(
            "pixels at {at:#x} are in {count} chunks, in no layout this reader knows \
             (the bitmap says {stored} bytes; {})",
            tried.join(", ")
        )))
    }

    /// A record's streams if it is laid out as `layout`, and the record's
    /// size that way.
    fn streams(&mut self, at: u64, n: usize, layout: RecordLayout) -> Result<(Vec<Stream>, u64)> {
        let past = |what: &str| Error::Corrupt(format!("{what} past textures.dat's end"));
        let sizes_at = |extra: u64| at + 4 + extra;
        let mut streams = Vec::with_capacity(n);
        let end = match layout {
            RecordLayout::Single | RecordLayout::SizesFirst | RecordLayout::TotalThenSizes => {
                let extra = u64::from(layout == RecordLayout::TotalThenSizes) * 4;
                let table = sizes_at(extra);
                if table + 4 * n as u64 > self.len {
                    return Err(past("sizes"));
                }
                let mut pos = table + 4 * n as u64;
                for word in self.words(table, n)? {
                    let s = Stream::new(pos, word);
                    streams.push(s);
                    pos += u64::from(s.size);
                }
                if layout == RecordLayout::TotalThenSizes {
                    let total = self.words(at + 4, 1)?[0];
                    let sum: u64 = streams.iter().map(|s| u64::from(s.size)).sum();
                    if sum != u64::from(total) {
                        return Err(Error::Corrupt(format!("sizes add to {sum}, not {total}")));
                    }
                }
                pos
            }
            RecordLayout::Interleaved => {
                let mut pos = at + 4;
                for _ in 0..n {
                    if pos + 4 > self.len {
                        return Err(past("a size"));
                    }
                    let s = Stream::new(pos + 4, self.words(pos, 1)?[0]);
                    streams.push(s);
                    pos += 4 + u64::from(s.size);
                }
                pos
            }
        };
        if streams.iter().any(|s| s.size == 0) {
            return Err(Error::Corrupt("an empty stream".into()));
        }
        if end > self.len {
            return Err(past("streams"));
        }
        Ok((streams, end - at))
    }

    /// One image's top level, decoded to RGBA8.
    pub fn read(&mut self, e: &BitmapEntry) -> Result<Image> {
        self.decode(e).map(|d| d.image)
    }

    /// One image's top level, decoded to RGBA8, with how its record was
    /// read. Rows are taken as packed unless the inflated size matches a
    /// whole chain of mip levels with rows padded to a common alignment
    /// (a guess at how MCC may store them, unverified).
    pub fn decode(&mut self, e: &BitmapEntry) -> Result<Decoded> {
        let (w, h) = (e.width, e.height);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(Error::Corrupt(format!("a bitmap {w} by {h}")));
        }
        // The decoded image is 4 bytes a pixel whatever the format, and no
        // stored level is bigger, so this bounds every buffer below.
        let rgba = usize::from(w) * usize::from(h) * 4;
        if rgba > MAX_READ {
            return Err(Error::Corrupt(format!(
                "a bitmap {w} by {h} is {rgba} bytes decoded, more than {MAX_READ}"
            )));
        }
        let (w, h) = (usize::from(w), usize::from(h));
        // Vista's decoders draw a P8 bump map as one flat colour, a stand-in
        // that mustn't pass for decoded pixels here.
        let (row_bytes, rows) = Some(e.format)
            .filter(|&f| f != Format::P8Bump)
            .and_then(|f| f.rows(w, h))
            .ok_or_else(|| Error::Corrupt(format!("unsupported bitmap format {:?}", e.format)))?;
        let need = row_bytes * rows;
        let record = self.record(e)?;
        let at = record.offset;
        // Room for the most padded chain tried, and a little more so a
        // stream longer than every candidate shows.
        let most = chain(e.format, w, h, ROW_ALIGNMENTS[ROW_ALIGNMENTS.len() - 1])
            .last()
            .copied()
            .unwrap_or(need)
            .saturating_add(0x1000)
            .min(MAX_READ);
        let mut pixels = Vec::new();
        for (k, s) in record.streams.iter().enumerate() {
            let left = (most - pixels.len().min(most)).max(1);
            let part = read_stream(&mut self.reader, s, left)
                .map_err(|err| Error::Corrupt(format!("pixels at {at:#x}, stream {k}: {err}")))?;
            pixels.extend(part);
            if pixels.len() > most {
                break;
            }
        }
        let inflated = pixels.len();
        if inflated < need {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x}: {inflated} of {need} bytes"
            )));
        }
        let pitch = row_pitch(e.format, w, h, inflated).unwrap_or(row_bytes);
        let packed = if pitch == row_bytes {
            pixels.truncate(need);
            pixels
        } else {
            (0..rows)
                .flat_map(|r| &pixels[r * pitch..r * pitch + row_bytes])
                .copied()
                .collect()
        };
        Ok(Decoded {
            image: Image {
                width: w as u32,
                height: h as u32,
                rgba: bitmap::decode(e.format, w, h, &packed),
            },
            record,
            inflated,
            row_pitch: pitch,
            row_bytes,
        })
    }
}

/// The sizes of a whole chain of mip levels from `w` by `h` down to 1 by
/// 1, each level's rows padded to `align` bytes, added up level by level.
fn chain(format: Format, w: usize, h: usize, align: usize) -> Vec<usize> {
    let mut sizes = Vec::new();
    let (mut lw, mut lh, mut total) = (w, h, 0usize);
    while let Some((row, rows)) = format.rows(lw, lh) {
        total += row.next_multiple_of(align) * rows;
        sizes.push(total);
        if lw == 1 && lh == 1 {
            break;
        }
        (lw, lh) = ((lw / 2).max(1), (lh / 2).max(1));
    }
    sizes
}

/// The row pitch that explains `inflated` bytes: packed rows if it is the
/// top level or a whole packed chain of levels, else the first alignment
/// whose padded chain it is; None when nothing matches.
fn row_pitch(format: Format, w: usize, h: usize, inflated: usize) -> Option<usize> {
    let (row, _) = format.rows(w, h)?;
    if chain(format, w, h, 1).contains(&inflated) {
        return Some(row);
    }
    ROW_ALIGNMENTS
        .iter()
        .filter(|&&a| row.next_multiple_of(a) != row)
        .find(|&&a| chain(format, w, h, a).contains(&inflated))
        .map(|&a| row.next_multiple_of(a))
}

/// Builds synthetic format-13 maps and textures.dat files, for tests here
/// and in the crates that use this reader. Nothing in them comes from a
/// real file.
#[cfg(any(test, feature = "synthetic"))]
pub mod synthetic {
    use super::*;
    use std::io::Write;

    fn put(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn zlib(data: &[u8]) -> Vec<u8> {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(data).unwrap();
        z.finish().unwrap()
    }

    /// One image: its size and its pixels as stored (B, G, R, A).
    #[derive(Clone)]
    pub struct Picture {
        pub width: u16,
        pub height: u16,
        pub bgra: Vec<u8>,
    }

    /// A textures.dat being built.
    #[derive(Default)]
    pub struct TexturesDat {
        pub bytes: Vec<u8>,
    }

    impl TexturesDat {
        /// Adds a record for `p` (A8R8G8B8) after a few bytes of padding,
        /// and returns the entry a bitmap tag gives for it.
        pub fn push(&mut self, p: &Picture) -> BitmapEntry {
            self.push_record(
                p.width,
                p.height,
                Format::A8R8G8B8,
                &p.bgra,
                1,
                RecordLayout::Single,
            )
        }

        /// Adds a record of `data` (pixels as stored, in `format`) cut into
        /// `streams` zlib streams laid out as `layout`, after a few bytes
        /// of padding, and returns the entry a bitmap tag gives for it.
        pub fn push_record(
            &mut self,
            width: u16,
            height: u16,
            format: Format,
            data: &[u8],
            streams: usize,
            layout: RecordLayout,
        ) -> BitmapEntry {
            let raw = vec![false; streams];
            self.push_mixed(width, height, format, data, &raw, layout)
        }

        /// As `push_record`, with one stream for each of `raw`: stored as
        /// is, with its size written negative, where it is true, else zlib.
        pub fn push_mixed(
            &mut self,
            width: u16,
            height: u16,
            format: Format,
            data: &[u8],
            raw: &[bool],
            layout: RecordLayout,
        ) -> BitmapEntry {
            let streams = raw.len();
            assert!(streams >= 1 && (streams == 1) == (layout == RecordLayout::Single));
            self.bytes.extend([0xEE; 5]);
            let pointer = self.bytes.len() as u32;
            let piece = data.len().div_ceil(streams).max(1);
            let mut pieces: Vec<&[u8]> = data.chunks(piece).collect();
            pieces.resize(streams, &[]);
            let parts: Vec<Vec<u8>> = pieces
                .iter()
                .zip(raw)
                .map(|(p, &raw)| if raw { p.to_vec() } else { zlib(p) })
                .collect();
            let size = |k: usize| {
                let n = parts[k].len() as i32;
                (if raw[k] { -n } else { n }).to_le_bytes()
            };
            self.bytes.extend((streams as u32).to_le_bytes());
            match layout {
                RecordLayout::Single | RecordLayout::SizesFirst => {
                    (0..streams).for_each(|k| self.bytes.extend(size(k)));
                    parts.iter().for_each(|p| self.bytes.extend(p));
                }
                RecordLayout::TotalThenSizes => {
                    let total: usize = parts.iter().map(Vec::len).sum();
                    self.bytes.extend((total as u32).to_le_bytes());
                    (0..streams).for_each(|k| self.bytes.extend(size(k)));
                    parts.iter().for_each(|p| self.bytes.extend(p));
                }
                RecordLayout::Interleaved => {
                    for (k, p) in parts.iter().enumerate() {
                        self.bytes.extend(size(k));
                        self.bytes.extend(p);
                    }
                }
            }
            BitmapEntry {
                width,
                height,
                depth: 1,
                format,
                pointer,
                stored_size: self.bytes.len() as u32 - pointer,
                ..BitmapEntry::default()
            }
        }
    }

    /// A tag's meta (or an element of one of its blocks) to be laid out:
    /// its bytes, the blocks nested in it, and the tag references and
    /// string ids in it, by name, filled in when the map is built.
    #[derive(Clone, Debug, Default)]
    pub struct Struct {
        pub bytes: Vec<u8>,
        /// A block's header place and its elements (all the same size).
        pub blocks: Vec<(usize, Vec<Struct>)>,
        /// A tag reference's place, the group and the tag's name ("" is
        /// none).
        pub refs: Vec<(usize, GroupTag, String)>,
        /// A string id's place and its name.
        pub ids: Vec<(usize, String)>,
    }

    impl Struct {
        pub fn new(size: usize) -> Struct {
            Struct {
                bytes: vec![0; size],
                ..Struct::default()
            }
        }

        pub fn u8(mut self, o: usize, v: u8) -> Struct {
            self.bytes[o] = v;
            self
        }

        pub fn i16(mut self, o: usize, v: i16) -> Struct {
            self.bytes[o..o + 2].copy_from_slice(&v.to_le_bytes());
            self
        }

        pub fn u32(mut self, o: usize, v: u32) -> Struct {
            put(&mut self.bytes, o, v);
            self
        }

        pub fn f32(self, o: usize, v: f32) -> Struct {
            self.u32(o, v.to_bits())
        }

        /// Text (ASCII, as names are stored) from `o`.
        pub fn text(mut self, o: usize, s: &str) -> Struct {
            self.bytes[o..o + s.len()].copy_from_slice(s.as_bytes());
            self
        }

        pub fn block(mut self, o: usize, elements: Vec<Struct>) -> Struct {
            self.blocks.push((o, elements));
            self
        }

        pub fn tag_ref(mut self, o: usize, group: &str, name: &str) -> Struct {
            let group = GroupTag::parse(group).expect("four letters");
            self.refs.push((o, group, name.to_string()));
            self
        }

        pub fn string_id(mut self, o: usize, name: &str) -> Struct {
            self.ids.push((o, name.to_string()));
            self
        }

        /// Every string id name in it and its blocks, in a fixed order.
        fn names(&self, out: &mut Vec<String>) {
            for (_, name) in &self.ids {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            for (_, elements) in &self.blocks {
                elements.iter().for_each(|e| e.names(out));
            }
        }
    }

    /// The id of `name` in a string-id table built by `MapBuilder`.
    pub fn string_id(table: &[String], name: &str) -> u32 {
        match table.iter().position(|n| n == name) {
            Some(0) | None => 0,
            Some(i) => pack_string_id(i, name),
        }
    }

    pub enum Meta {
        /// Bytes as they are.
        Raw(Vec<u8>),
        /// A bitm tag: 0x80 bytes with the bitmaps block at 0x44 pointing
        /// just after them, then the entries.
        Bitmaps(Vec<BitmapEntry>),
        /// A bitm tag with sequences (the block at 0x3C, laid out before
        /// the entries) as well as images.
        Bitmap {
            entries: Vec<BitmapEntry>,
            sequences: Vec<Sequence>,
        },
        /// Meta with nested blocks, tag references and string ids.
        Struct(Struct),
    }

    impl Meta {
        /// The meta as a `Struct` (raw bytes have no blocks).
        pub fn to_struct(&self) -> Struct {
            match self {
                Meta::Raw(b) => Struct {
                    bytes: b.clone(),
                    ..Struct::default()
                },
                Meta::Bitmaps(entries) => bitm(entries, &[]),
                Meta::Bitmap { entries, sequences } => bitm(entries, sequences),
                Meta::Struct(s) => s.clone(),
            }
        }
    }

    /// A bitm tag's meta: 0x80 bytes, the sequences with their sprites,
    /// then the image entries.
    fn bitm(entries: &[BitmapEntry], sequences: &[Sequence]) -> Struct {
        let sequences = sequences
            .iter()
            .map(|s| {
                let sprites = s
                    .sprites
                    .iter()
                    .map(|p| {
                        Struct::new(bitmap::SPRITE_SIZE)
                            .i16(0, p.bitmap)
                            .f32(0x8, p.left)
                            .f32(0xC, p.right)
                            .f32(0x10, p.top)
                            .f32(0x14, p.bottom)
                            .f32(0x18, p.registration[0])
                            .f32(0x1C, p.registration[1])
                    })
                    .collect();
                Struct::new(bitmap::SEQUENCE_SIZE)
                    .text(0, &s.name)
                    .i16(0x20, s.first_bitmap)
                    .i16(0x22, s.bitmap_count)
                    .block(bitmap::SEQUENCE_SPRITES, sprites)
            })
            .collect();
        let entries = entries
            .iter()
            .map(|e| Struct {
                bytes: e.to_bytes().to_vec(),
                ..Struct::default()
            })
            .collect();
        Struct::new(0x80)
            .block(bitmap::BITM_SEQUENCES, sequences)
            .block(BITM_BITMAPS, entries)
    }

    /// A format-13 map being built: header, names, index, metas, the
    /// string ids and the English strings, then the chunks and the chunk
    /// table.
    pub struct MapBuilder {
        pub chunk_size: u32,
        /// Bytes of zeros before the first tag's meta, to move it across a
        /// chunk boundary.
        pub pad: usize,
        pub tags: Vec<(GroupTag, String, Meta)>,
        /// The English strings: each one's string id name and its words.
        /// Their table's place goes in the first matg's meta at 0x190 when
        /// it is long enough, as in Vista.
        pub language: Vec<(String, String)>,
    }

    impl MapBuilder {
        pub fn new(chunk_size: u32) -> MapBuilder {
            MapBuilder {
                chunk_size,
                pad: 0,
                tags: Vec::new(),
                language: Vec::new(),
            }
        }

        pub fn tag(mut self, group: &str, name: &str, meta: Meta) -> MapBuilder {
            let group = GroupTag::parse(group).expect("four letters");
            self.tags.push((group, name.to_string(), meta));
            self
        }

        /// The datum of tag `i`.
        pub fn datum(i: usize) -> u32 {
            0xE000_0000 + i as u32
        }

        /// The datum of the tag of `group` named `name`, or none.
        pub fn datum_of(&self, group: GroupTag, name: &str) -> u32 {
            self.tags
                .iter()
                .position(|(g, n, _)| *g == group && n == name)
                .map_or(u32::MAX, MapBuilder::datum)
        }

        /// The string-id table: the empty name, then every name the metas
        /// and the English strings use, in order.
        pub fn string_ids(&self) -> Vec<String> {
            let mut names = vec![String::new()];
            for (_, _, meta) in &self.tags {
                meta.to_struct().names(&mut names);
            }
            for (name, _) in &self.language {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            names
        }

        /// The English strings as (string id, words).
        pub fn language_table(&self) -> Vec<(u32, String)> {
            let table = self.string_ids();
            self.language
                .iter()
                .map(|(name, words)| (string_id(&table, name), words.clone()))
                .collect()
        }

        /// Copies `s` to `out` at `at`, fills in its references and ids,
        /// and lays its blocks out after everything in `out`; `base` is
        /// `out[0]`'s address, counted from the tag index.
        fn lay_out(&self, s: &Struct, at: usize, base: usize, out: &mut Vec<u8>, ids: &[String]) {
            out[at..at + s.bytes.len()].copy_from_slice(&s.bytes);
            for (o, group, name) in &s.refs {
                let datum = self.datum_of(*group, name);
                let group = if datum == u32::MAX { u32::MAX } else { group.0 };
                put(out, at + o, group);
                put(out, at + o + 4, datum);
            }
            for (o, name) in &s.ids {
                put(out, at + o, string_id(ids, name));
            }
            for (o, elements) in &s.blocks {
                let size = elements.first().map_or(0, |e| e.bytes.len());
                let start = out.len();
                out.resize(start + size * elements.len(), 0);
                put(out, at + o, elements.len() as u32);
                let address = if elements.is_empty() { 0 } else { base + start };
                put(out, at + o + 4, address as u32);
                for (k, e) in elements.iter().enumerate() {
                    assert_eq!(e.bytes.len(), size, "a block's elements differ in size");
                    self.lay_out(e, start + k * size, base, out, ids);
                }
            }
        }

        /// The image (header and inflated body), before compressing.
        pub fn image(&self) -> Vec<u8> {
            let mut b = vec![0u8; HEADER_SIZE];
            put(&mut b, 0, HEAD_MAGIC);
            put(&mut b, 4, VERSION as u32);
            put(&mut b, HEADER_SIZE - 4, FOOT_MAGIC);
            put(&mut b, 0x2E4, u32::MAX);
            put(&mut b, 0x2E8, u32::MAX);
            b[0xB0..0xB8].copy_from_slice(b"mainmenu");
            let scenario = br"scenarios\ui\mainmenu\mainmenu";
            b[0xD0..0xD0 + scenario.len()].copy_from_slice(scenario);
            // Names: the buffer, then the table.
            let mut names = Vec::new();
            let mut starts = Vec::new();
            for (_, name, _) in &self.tags {
                starts.push(names.len() as u32);
                names.extend(name.as_bytes());
                names.push(0);
            }
            put(&mut b, 0x20, self.tags.len() as u32);
            let v = b.len() as u32;
            put(&mut b, 0x24, v);
            put(&mut b, 0x28, names.len() as u32);
            b.extend(&names);
            let v = b.len() as u32;
            put(&mut b, 0x2C, v);
            for s in starts {
                b.extend(s.to_le_bytes());
            }
            // The index: its header, one group, the tags, then the metas.
            while !b.len().is_multiple_of(16) {
                b.push(0);
            }
            let index = b.len();
            put(&mut b, 0x10, index as u32);
            let groups = INDEX_HEADER_SIZE;
            let tags = groups + 12;
            let metas = tags + self.tags.len() * TAG_ENTRY_SIZE + self.pad;
            b.resize(index + metas, 0);
            put(&mut b, index, groups as u32);
            put(&mut b, index + 4, 1);
            put(&mut b, index + 8, tags as u32);
            put(&mut b, index + 0xC, 0xE000_0000);
            put(&mut b, index + 0x10, u32::MAX);
            put(&mut b, index + 0x14, 0x1234_5678);
            put(&mut b, index + 0x18, self.tags.len() as u32);
            put(&mut b, index + 0x1C, TAGS_MAGIC);
            put(&mut b, index + groups, BITM.0);
            put(&mut b, index + groups + 4, u32::MAX);
            put(&mut b, index + groups + 8, u32::MAX);
            let ids = self.string_ids();
            let mut matg_meta = None;
            for (i, (group, _, meta)) in self.tags.iter().enumerate() {
                let address = b.len() - index;
                let s = meta.to_struct();
                let mut bytes = vec![0u8; s.bytes.len()];
                self.lay_out(&s, 0, address, &mut bytes, &ids);
                if *group == MATG && matg_meta.is_none() && bytes.len() >= MATG_ENGLISH + 16 {
                    matg_meta = Some(b.len());
                }
                let row = index + tags + i * TAG_ENTRY_SIZE;
                put(&mut b, row, group.0);
                put(&mut b, row + 4, MapBuilder::datum(i));
                put(&mut b, row + 8, address as u32);
                put(&mut b, row + 12, bytes.len() as u32);
                b.extend(bytes);
            }
            let v = (b.len() - index) as u32;
            put(&mut b, 0x14, v);
            put(&mut b, 0x2D4, metas as u32);
            let v = (b.len() - index - metas) as u32;
            put(&mut b, 0x2D8, v);
            // The string ids: the text, then a u32 per string.
            let mut text = Vec::new();
            let mut offsets = Vec::new();
            for name in &ids {
                offsets.push(text.len() as u32);
                text.extend(name.as_bytes());
                text.push(0);
            }
            put(&mut b, 0x30, ids.len() as u32);
            let v = b.len() as u32;
            put(&mut b, 0x34, v);
            put(&mut b, 0x38, text.len() as u32);
            b.extend(&text);
            let v = b.len() as u32;
            put(&mut b, 0x3C, v);
            offsets.iter().for_each(|o| b.extend(o.to_le_bytes()));
            // The English strings: an index of (string id, offset), then
            // the text; their place in matg's meta.
            if let (Some(matg), false) = (matg_meta, self.language.is_empty()) {
                let index_at = b.len();
                let mut words = Vec::new();
                for (id, s) in self.language_table() {
                    b.extend(id.to_le_bytes());
                    b.extend((words.len() as u32).to_le_bytes());
                    words.extend(s.as_bytes());
                    words.push(0);
                }
                let text_at = b.len();
                b.extend(&words);
                let entry = matg + MATG_ENGLISH;
                put(&mut b, entry, self.language.len() as u32);
                put(&mut b, entry + 4, words.len() as u32);
                put(&mut b, entry + 8, index_at as u32);
                put(&mut b, entry + 12, text_at as u32);
            }
            let v = b.len() as u32;
            put(&mut b, 0x08, v);
            b
        }

        /// The map as stored: the header, the chunks compressed one after
        /// another, then the chunk table.
        pub fn build(&self) -> Vec<u8> {
            compress(&self.image(), self.chunk_size)
        }
    }

    /// An image as a format-13 file: the header as it is, the rest in
    /// zlib chunks of `chunk_size`, then the chunk table.
    pub fn compress(image: &[u8], chunk_size: u32) -> Vec<u8> {
        let mut out = image[..HEADER_SIZE].to_vec();
        let mut table = Vec::new();
        for c in image[HEADER_SIZE..].chunks(chunk_size as usize) {
            let z = zlib(c);
            table.extend((z.len() as i32).to_le_bytes());
            table.extend((out.len() as u32).to_le_bytes());
            out.extend(z);
        }
        put(&mut out, 0x308, chunk_size);
        let v = out.len() as u32;
        put(&mut out, 0x310, v);
        put(&mut out, 0x314, (table.len() / CHUNK_ENTRY_SIZE) as u32);
        out.extend(table);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::synthetic::*;
    use super::*;
    use std::io::Cursor;

    fn put(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn open(bytes: Vec<u8>) -> Result<Map<Cursor<Vec<u8>>>> {
        Map::from_reader(Cursor::new(bytes))
    }

    fn corrupt(r: Result<Map<Cursor<Vec<u8>>>>) -> String {
        match r {
            Err(Error::Corrupt(s)) => s,
            Err(e) => panic!("not Corrupt: {e}"),
            Ok(_) => panic!("opened"),
        }
    }

    /// Bytes that say where they are: byte i is i's low byte, xor `salt`.
    fn pattern(len: usize, salt: u8) -> Vec<u8> {
        (0..len).map(|i| i as u8 ^ salt).collect()
    }

    /// Picture n: B, G, R, A = 3, set, n, 0 in the left column and 255
    /// elsewhere.
    fn picture(w: u16, h: u16, set: u8, n: u8) -> Picture {
        let mut bgra = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                bgra.extend([3, set, n, if x == 0 { 0 } else { 255 }]);
            }
        }
        Picture {
            width: w,
            height: h,
            bgra,
        }
    }

    /// A map with a raw tag, a bitm of `count` icons, and the matching
    /// textures.dat, in 64-byte chunks.
    fn icons_map(count: u8) -> (Vec<u8>, Vec<u8>) {
        let mut dat = TexturesDat::default();
        let entries: Vec<BitmapEntry> = (0..count)
            .map(|n| dat.push(&picture(7, 5, 1, n + 1)))
            .collect();
        let map = MapBuilder::new(64)
            .tag("matg", r"globals\globals", Meta::Raw(pattern(300, 0x5A)))
            .tag(
                "bitm",
                r"ui\global_bitmaps\rank_icons",
                Meta::Bitmaps(entries),
            )
            .build();
        (map, dat.bytes)
    }

    #[test]
    fn reads_header_names_and_tags() {
        let (bytes, _) = icons_map(3);
        let map = open(bytes).unwrap();
        assert_eq!(map.header.version, 13);
        assert_eq!(map.header.chunk_size, 64);
        assert_eq!(map.header.map_name, "mainmenu");
        assert!(map.header.chunk_count > 10, "{}", map.header.chunk_count);
        assert_eq!(map.index.scenario, DatumIndex(0xE000_0000));
        assert_eq!(map.index.checksum, 0x1234_5678);
        let names: Vec<(String, &str)> = map
            .tags
            .iter()
            .map(|t| (t.group.to_string(), t.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("matg".to_string(), r"globals\globals"),
                ("bitm".to_string(), r"ui\global_bitmaps\rank_icons"),
            ]
        );
        assert_eq!(map.tags[1].datum, DatumIndex(0xE000_0001));
        let bitm = GroupTag::parse("bitm").unwrap();
        assert!(map
            .find_tag(bitm, r"UI\Global_Bitmaps\Rank_Icons")
            .is_some());
        assert!(map.find_tag(bitm, r"globals\globals").is_none());
    }

    #[test]
    fn meta_across_chunk_boundaries_matches_the_image() {
        // Several chunk sizes and paddings, so the raw tag starts and ends
        // at different places in its chunks, across many of them.
        for chunk in [16u32, 64, 100, 0x40000] {
            for pad in [0usize, 1, 37] {
                let mut b = MapBuilder::new(chunk).tag("matg", "g", Meta::Raw(pattern(1000, 7)));
                b.pad = pad;
                // The image as read has the stored header (with the chunk
                // table's place filled in).
                let stored = b.build();
                let mut image = b.image();
                image[..HEADER_SIZE].copy_from_slice(&stored[..HEADER_SIZE]);
                let mut map = open(stored).unwrap();
                let tag = map.tags[0].clone();
                let meta = map.tag_meta(&tag).unwrap();
                assert_eq!(meta, pattern(1000, 7), "chunk {chunk} pad {pad}");
                // The whole image, and pieces of it, read back the same.
                let all = map.read_image(0, image.len()).unwrap();
                assert!(all == image, "chunk {chunk} pad {pad}");
                for (at, len) in [(0, 1), (0x37F, 2), (0x380, 0), (0x3A0, 200)] {
                    let len = len.min(image.len() - at);
                    let piece = map.read_image(at as u64, len).unwrap();
                    assert!(piece == image[at..at + len], "{at:#x} chunk {chunk}");
                }
            }
        }
    }

    #[test]
    fn reads_bitmaps_and_pixels_from_textures_dat() {
        let (bytes, dat) = icons_map(50);
        let mut map = open(bytes).unwrap();
        let bitm = GroupTag::parse("bitm").unwrap();
        let tag = map
            .find_tag(bitm, r"ui\global_bitmaps\rank_icons")
            .unwrap()
            .clone();
        let entries = map.bitmaps(&tag).unwrap();
        assert_eq!(entries.len(), 50);
        assert_eq!(
            (entries[0].width, entries[0].height, entries[0].format),
            (7, 5, Format::A8R8G8B8)
        );
        let mut textures = Textures::from_reader(Cursor::new(dat)).unwrap();
        for (n, e) in entries.iter().enumerate() {
            let im = textures.read(e).unwrap();
            assert_eq!((im.width, im.height), (7, 5));
            assert_eq!(im.rgba.len(), 7 * 5 * 4);
            // R, G, B, A from B, G, R, A.
            let level = n as u8 + 1;
            assert_eq!(im.rgba[..8], [level, 1, 3, 0, level, 1, 3, 255]);
        }
        // A tag that isn't a bitm is refused.
        let matg = map.tags[0].clone();
        assert!(map.bitmaps(&matg).is_err());
    }

    #[test]
    fn refuses_other_versions_and_magic() {
        let (good, _) = icons_map(1);
        let mut b = good.clone();
        put(&mut b, 4, 8);
        assert!(matches!(
            open(b),
            Err(Error::WrongVersion {
                found: 8,
                expected: 13
            })
        ));
        let mut b = good.clone();
        b[0] = b'x';
        assert!(matches!(open(b), Err(Error::BadMagic(_))));
        let mut b = good.clone();
        put(&mut b, HEADER_SIZE - 4, 0);
        assert!(matches!(open(b), Err(Error::BadMagic(_))));
        // Shorter than the header, and shorter than the version word.
        assert!(matches!(
            open(good[..0x200].to_vec()),
            Err(Error::BadMagic(_))
        ));
        assert!(matches!(open(b"daeh".to_vec()), Err(Error::BadMagic(_))));
        assert!(open(Vec::new()).is_err());
    }

    #[test]
    fn refuses_bad_sizes_without_allocating() {
        let (good, _) = icons_map(2);
        let h = Header::parse(&good).unwrap();
        // Truncated: the chunk table is the file's last bytes.
        let e = corrupt(open(good[..good.len() - 1].to_vec()));
        assert!(e.contains("truncated"), "{e}");
        // Cut in the chunks, with the table moved to stay in the file.
        let mut b = good.clone();
        let table = good[h.chunk_table_offset as usize..].to_vec();
        b.truncate(0x400);
        let v = b.len() as u32;
        put(&mut b, 0x310, v);
        b.extend(&table);
        assert!(corrupt(open(b)).contains("chunk"));
        // An absurd image size, chunk size, chunk count.
        for (field, value) in [
            (0x08, u32::MAX),
            (0x08, 0x10),
            (0x308, 0),
            (0x308, u32::MAX),
            (0x314, u32::MAX),
            (0x314, h.chunk_count - 1),
        ] {
            let mut b = good.clone();
            put(&mut b, field, value);
            let e = corrupt(open(b));
            assert!(!e.is_empty(), "{field:#x}");
        }
        // A chunk's compressed size: negative, huge, past the end.
        let first = h.chunk_table_offset as usize;
        for value in [u32::MAX, 0, 0x7FFF_FFFF] {
            let mut b = good.clone();
            put(&mut b, first, value);
            assert!(corrupt(open(b)).contains("chunk 0"));
        }
        // A chunk that is not zlib, or inflates to the wrong length.
        let mut b = good.clone();
        let at = u32_at(&good, first + 4) as usize;
        b[at] ^= 0xFF;
        assert!(corrupt(open(b)).contains("chunk 0"));
        // One byte less in the header's size: the last chunk is then too
        // long, which shows when it is read.
        let mut b = good.clone();
        put(&mut b, 0x08, h.image_size - 1);
        let e = match open(b) {
            Ok(mut map) => map.read_image(0, h.image_size as usize - 1).unwrap_err(),
            Err(e) => e,
        };
        assert!(e.to_string().contains("chunk"), "{e}");
    }

    #[test]
    fn refuses_bad_index_and_names() {
        // Edits to the image, recompressed.
        let base = MapBuilder::new(64).tag("matg", "g", Meta::Raw(pattern(40, 1)));
        let image = base.image();
        let index = u32_at(&image, 0x10) as usize;
        let edit = |o: usize, v: u32| {
            let mut i = image.clone();
            put(&mut i, o, v);
            open(compress(&i, 64))
        };
        assert!(matches!(edit(index + 0x1C, 0), Err(Error::BadMagic(_))));
        assert!(corrupt(edit(index + 0x18, u32::MAX)).contains("too many"));
        assert!(corrupt(edit(index + 0x18, 0xFFFF)).contains("past the end"));
        assert!(corrupt(edit(index + 8, 0x7FFF_FFF0)).contains("past the end"));
        assert!(corrupt(edit(0x10, u32::MAX)).contains("past the end"));
        assert!(corrupt(edit(0x20, u32::MAX)).contains("too many"));
        assert!(corrupt(edit(0x28, u32::MAX)).contains("tag names"));
        assert!(corrupt(edit(0x24, u32::MAX - 2)).contains("past the end"));
        let table = u32_at(&image, 0x2C) as usize;
        assert!(corrupt(edit(table, 0x1000)).contains("past the names"));
        // A tag whose meta is past the end is refused when it is read.
        let tags = index + u32_at(&image, index + 8) as usize;
        let mut map = edit(tags + 8, 0x7FFF_0000).unwrap();
        let tag = map.tags[0].clone();
        assert!(map
            .tag_meta(&tag)
            .unwrap_err()
            .to_string()
            .contains("tag g"));
        let mut map = edit(tags + 12, u32::MAX - 1).unwrap();
        let tag = map.tags[0].clone();
        assert!(map.tag_meta(&tag).is_err());
        // No meta in this map: nothing to read.
        let mut map = edit(tags + 8, 0).unwrap();
        let tag = map.tags[0].clone();
        assert!(map.tag_meta(&tag).unwrap().is_empty());
    }

    #[test]
    fn refuses_bad_bitmaps() {
        let (bytes, dat) = icons_map(2);
        let mut map = open(bytes).unwrap();
        let tag = map.tags[1].clone();
        let good = map.bitmaps(&tag).unwrap()[1];
        let mut textures = Textures::from_reader(Cursor::new(dat.clone())).unwrap();
        assert!(textures.read(&good).is_ok());
        let bad = |e: BitmapEntry, t: &mut Textures<Cursor<Vec<u8>>>| match t.read(&e) {
            Err(Error::Corrupt(s)) => s,
            other => panic!("{other:?}"),
        };
        for (w, h) in [(0, 5), (7, 0), (u16::MAX, u16::MAX), (8193, 1)] {
            let e = BitmapEntry {
                width: w,
                height: h,
                ..good
            };
            assert!(bad(e, &mut textures).contains("bitmap"));
        }
        let e = BitmapEntry {
            format: Format::Other(99),
            ..good
        };
        assert!(bad(e, &mut textures).contains("unsupported"));
        // Sides within the limit but too big decoded (256 MiB, or 64 MiB
        // and a byte as DXT1): refused before the record is read, so
        // nothing that size is reserved. 4096 square is just allowed.
        for (w, h, format) in [
            (8192, 8192, Format::A8R8G8B8),
            (8192, 8192, Format::Dxt1),
            (4097, 4096, Format::A8R8G8B8),
        ] {
            let e = BitmapEntry {
                width: w,
                height: h,
                format,
                ..good
            };
            assert!(bad(e, &mut textures).contains("more than"), "{w} {h}");
        }
        let e = BitmapEntry {
            width: 4096,
            height: 4096,
            ..good
        };
        assert!(bad(e, &mut textures).contains("of 67108864 bytes"));
        for pointer in [
            0x8000_0000,
            0x4000_0010,
            dat.len() as u32 - 4,
            u32::MAX >> 2,
        ] {
            let e = BitmapEntry { pointer, ..good };
            assert!(bad(e, &mut textures).contains("pixel"), "{pointer:#x}");
        }
        let e = BitmapEntry {
            stored_size: good.stored_size + 1,
            ..good
        };
        assert!(bad(e, &mut textures).contains("says"));
        // Bigger than its pixels: the stream ends short.
        let e = BitmapEntry { width: 8, ..good };
        assert!(bad(e, &mut textures).contains("of 160 bytes"));
        // The record's own header: two chunks; a size past the end.
        let p = good.pointer as usize;
        let mut d = dat.clone();
        put(&mut d, p, 2);
        let mut t = Textures::from_reader(Cursor::new(d)).unwrap();
        assert!(bad(good, &mut t).contains("2 chunks"));
        let mut d = dat.clone();
        put(&mut d, p + 4, i32::MAX as u32);
        let mut t = Textures::from_reader(Cursor::new(d)).unwrap();
        assert!(bad(good, &mut t).contains("past"));
        // As big a raw stream; a raw stream of 1 byte, which the bitmap's
        // stored size disagrees with.
        for (size, says) in [(i32::MIN as u32, "past"), (u32::MAX, "says")] {
            let mut d = dat.clone();
            put(&mut d, p + 4, size);
            let mut t = Textures::from_reader(Cursor::new(d)).unwrap();
            assert!(bad(good, &mut t).contains(says), "{size:#x}");
        }
        // Not zlib.
        let mut d = dat.clone();
        d[p + 8] ^= 0xFF;
        let mut t = Textures::from_reader(Cursor::new(d)).unwrap();
        assert!(bad(good, &mut t).contains("pixels"));
        // A bitm whose meta is too short, or counts too many images.
        let short = Tag {
            size: 0x40,
            ..tag.clone()
        };
        assert!(map.bitmaps(&short).is_err());
        let mut image = MapBuilder::new(64)
            .tag("bitm", "b", Meta::Bitmaps(vec![good]))
            .image();
        let index = u32_at(&image, 0x10) as usize;
        let tags = index + u32_at(&image, index + 8) as usize;
        let meta = index + u32_at(&image, tags + 8) as usize;
        put(&mut image, meta + BITM_BITMAPS, u32::MAX);
        let mut map = open(compress(&image, 64)).unwrap();
        let tag = map.tags[0].clone();
        assert!(map
            .bitmaps(&tag)
            .unwrap_err()
            .to_string()
            .contains("images"));
        put(&mut image, meta + BITM_BITMAPS, 0x1000);
        let mut map = open(compress(&image, 64)).unwrap();
        assert!(map.bitmaps(&tag).unwrap_err().to_string().contains("past"));
        put(&mut image, meta + BITM_BITMAPS, 0);
        let mut map = open(compress(&image, 64)).unwrap();
        assert!(map.bitmaps(&tag).unwrap().is_empty());
    }

    #[test]
    fn the_vista_header_still_says_version_8() {
        // A Vista map's first bytes give the version error, not a missing
        // 'foot' at MCC's place.
        let mut b = vec![0u8; 0x800];
        put(&mut b, 0, HEAD_MAGIC);
        put(&mut b, 4, 8);
        put(&mut b, 0x7FC, FOOT_MAGIC);
        let e = open(b).err().unwrap();
        assert_eq!(e.to_string(), "unsupported cache version 8 (expected 13)");
    }

    /// A map with a tag of nested blocks, references and string ids.
    fn nested_map(chunk: u32) -> MapBuilder {
        let leaf = |n: u32| Struct::new(8).u32(0, n).string_id(4, &format!("leaf_{n}"));
        let branch = |n: u32| {
            Struct::new(20)
                .u32(0, n)
                .block(4, (0..n).map(|k| leaf(n * 10 + k)).collect())
                .tag_ref(12, "bitm", "b")
        };
        let root = Struct::new(0x30)
            .tag_ref(0, "matg", r"globals\globals")
            .block(8, vec![branch(1), branch(2), branch(3)])
            .string_id(0x10, "root")
            .block(0x14, Vec::new())
            .tag_ref(0x1C, "bitm", "missing");
        MapBuilder::new(chunk)
            .tag("matg", r"globals\globals", Meta::Raw(pattern(40, 3)))
            .tag("bitm", "b", Meta::Bitmaps(Vec::new()))
            .tag("wgit", "root", Meta::Struct(root))
    }

    #[test]
    fn reads_nested_blocks_references_and_string_ids() {
        for chunk in [16u32, 64, 0x40000] {
            let mut map = open(nested_map(chunk).build()).unwrap();
            let tag = map.tags[2].clone();
            let meta = map.tag_meta(&tag).unwrap();
            assert!(meta.len() > 0x30, "the tag's own bytes, then its blocks");
            // References: a matg and a bitm by datum, and none.
            assert_eq!(map.resolve(&meta, 0).unwrap().name, r"globals\globals");
            assert_eq!(
                tag_ref(&meta, 0x1C),
                Some((GroupTag::NONE, DatumIndex::NONE))
            );
            assert!(map.resolve(&meta, 0x1C).is_none());
            assert!(tag_ref(&meta, meta.len() - 4).is_none(), "past the meta");
            assert_eq!(map.tag(DatumIndex(0xE000_0001)).unwrap().name, "b");
            assert!(map.tag(DatumIndex(0xE001_0001)).is_none(), "salt differs");
            assert!(map.tag(DatumIndex::NONE).is_none());
            // Blocks two deep, inside the tag's meta as Vista keeps them.
            let branches = map.block(&meta, 8, 20).unwrap();
            assert_eq!(branches.len(), 60);
            let (count, address) = block_header(&meta, 8).unwrap();
            assert_eq!(count, 3);
            assert!(tag.holds(address, 60), "blocks inside the tag's meta");
            assert!(!tag.holds(address, meta.len()));
            assert!(map.block(&meta, 0x14, 16).unwrap().is_empty());
            let mut leaves = Vec::new();
            for b in branches.as_chunks::<20>().0 {
                assert_eq!(map.resolve(b, 12).unwrap().name, "b");
                for l in map.block(b, 4, 8).unwrap().as_chunks::<8>().0 {
                    leaves.push((u32_at(l, 0), u32_at(l, 4)));
                }
            }
            assert_eq!(
                leaves.iter().map(|l| l.0).collect::<Vec<_>>(),
                [10, 20, 21, 30, 31, 32]
            );
            // String ids: none before the table is read, then by name.
            assert!(map.string_id(leaves[0].1).is_none());
            let ids = map.read_string_ids().unwrap();
            assert_eq!(ids.names[0], "");
            assert_eq!(ids.end(), ids.size as usize);
            let root = u32_at(&meta, 0x10);
            assert_eq!(ids.find("root"), Some(root));
            assert_eq!(root >> 24, 4, "the length in the top 8 bits");
            assert_eq!(map.string_id(root), Some("root"));
            assert_eq!(map.string_id(leaves[5].1), Some("leaf_32"));
            assert_eq!(map.string_id(0), Some(""));
            assert!(map.string_id(0x00FF_FFFF).is_none());
        }
    }

    #[test]
    fn refuses_bad_blocks_and_string_ids() {
        let mut map = open(nested_map(64).build()).unwrap();
        let tag = map.tags[2].clone();
        let meta = map.tag_meta(&tag).unwrap();
        let edit = |o: usize, v: u32| {
            let mut m = meta.clone();
            put(&mut m, o, v);
            m
        };
        let e = map.block(&edit(8, 0x10_0001), 8, 20).unwrap_err();
        assert!(e.to_string().contains("elements"), "{e}");
        let e = map.block(&edit(12, 0x7FFF_0000), 8, 20).unwrap_err();
        assert!(e.to_string().contains("past the index"), "{e}");
        let e = map.block(&meta, meta.len() - 4, 16).unwrap_err();
        assert!(e.to_string().contains("parent"), "{e}");
        assert!(block_header(&meta, usize::MAX).is_err());
        // The string-id table's header fields, edited.
        let image = nested_map(64).image();
        let bad = |o: usize, v: u32| {
            let mut i = image.clone();
            put(&mut i, o, v);
            let mut map = open(compress(&i, 64)).unwrap();
            map.read_string_ids().unwrap_err().to_string()
        };
        assert!(bad(0x30, 0).contains("0x30"));
        assert!(bad(0x30, u32::MAX).contains("0x30"));
        assert!(bad(0x38, u32::MAX).contains("0x38"));
        assert!(bad(0x34, u32::MAX - 8).contains("text"));
        assert!(bad(0x3C, u32::MAX - 8).contains("index"));
        // The first string's offset past the text.
        let index = u32_at(&image, 0x3C) as usize;
        assert!(bad(index, 0x1000).contains("past the text"));
    }

    #[test]
    fn reads_sequences_and_sprites() {
        let sequences = vec![
            Sequence {
                name: "frames".into(),
                first_bitmap: 1,
                bitmap_count: 2,
                sprites: Vec::new(),
            },
            Sequence {
                name: "buttons".into(),
                first_bitmap: 0,
                bitmap_count: 1,
                sprites: vec![
                    bitmap::Sprite {
                        bitmap: 0,
                        left: 0.0,
                        right: 0.5,
                        top: 0.25,
                        bottom: 1.0,
                        registration: [0.5, 0.5],
                    },
                    bitmap::Sprite {
                        bitmap: 2,
                        left: 0.5,
                        right: 1.0,
                        top: 0.0,
                        bottom: 0.25,
                        registration: [0.0, 1.0],
                    },
                ],
            },
        ];
        let mut dat = TexturesDat::default();
        let entries: Vec<_> = (0..3).map(|n| dat.push(&picture(8, 4, 2, n))).collect();
        let map = MapBuilder::new(64)
            .tag(
                "bitm",
                r"ui\shared\buttons",
                Meta::Bitmap {
                    entries,
                    sequences: sequences.clone(),
                },
            )
            .build();
        let mut map = open(map).unwrap();
        let tag = map.tags[0].clone();
        assert_eq!(map.sequences(&tag).unwrap(), sequences);
        assert_eq!(map.bitmaps(&tag).unwrap().len(), 3);
        assert_eq!(sequences[1].sprites[0].pixels(8, 4), [0, 1, 4, 3]);
        // A tag with no sequences has none; a matg is refused.
        let (bytes, _) = icons_map(2);
        let mut map = open(bytes).unwrap();
        let (matg, bitm) = (map.tags[0].clone(), map.tags[1].clone());
        assert!(map.sequences(&bitm).unwrap().is_empty());
        assert!(map.sequences(&matg).is_err());
    }

    #[test]
    fn reads_records_in_several_streams() {
        let p = picture(9, 7, 4, 5);
        for layout in [
            RecordLayout::SizesFirst,
            RecordLayout::Interleaved,
            RecordLayout::TotalThenSizes,
        ] {
            for streams in [2usize, 3, 7, 300] {
                let mut dat = TexturesDat::default();
                dat.push(&picture(2, 2, 0, 0));
                let e = dat.push_record(9, 7, Format::A8R8G8B8, &p.bgra, streams, layout);
                let mut t = Textures::from_reader(Cursor::new(dat.bytes.clone())).unwrap();
                let d = t.decode(&e).unwrap();
                assert_eq!(d.record.layout, layout, "{streams}");
                assert_eq!(d.record.streams.len(), streams);
                assert_eq!(d.inflated, 9 * 7 * 4);
                assert_eq!(d.image.rgba[..8], [5, 4, 3, 0, 5, 4, 3, 255]);
                assert_eq!(d.image.rgba.len(), 9 * 7 * 4);
                // A stored size that matches no layout is refused.
                let off = BitmapEntry {
                    stored_size: e.stored_size + 1,
                    ..e
                };
                let err = t.decode(&off).unwrap_err().to_string();
                assert!(err.contains("no layout"), "{err}");
            }
        }
        // A stream that isn't zlib, in a record whose sizes add up.
        let mut dat = TexturesDat::default();
        let e = dat.push_record(9, 7, Format::A8R8G8B8, &p.bgra, 2, RecordLayout::SizesFirst);
        let first = e.pointer as usize + 12;
        dat.bytes[first] = 0;
        let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
        let err = t.decode(&e).unwrap_err().to_string();
        assert!(err.contains("isn't zlib"), "{err}");
    }

    #[test]
    fn reads_streams_stored_raw() {
        let p = picture(9, 7, 4, 5);
        // One raw stream, whose size is written negative.
        let mut dat = TexturesDat::default();
        let e = dat.push_mixed(
            9,
            7,
            Format::A8R8G8B8,
            &p.bgra,
            &[true],
            RecordLayout::Single,
        );
        assert_eq!(
            dat.bytes[e.pointer as usize + 4..][..4],
            (-(9 * 7 * 4i32)).to_le_bytes()
        );
        let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
        let d = t.decode(&e).unwrap();
        assert!(d.record.streams[0].raw);
        assert_eq!(d.record.streams[0].size, 9 * 7 * 4);
        assert_eq!(d.image.rgba[..8], [5, 4, 3, 0, 5, 4, 3, 255]);
        // Raw and zlib streams mixed, in each layout.
        let raw = [false, true, true, false, true];
        for layout in [
            RecordLayout::SizesFirst,
            RecordLayout::Interleaved,
            RecordLayout::TotalThenSizes,
        ] {
            let mut dat = TexturesDat::default();
            let e = dat.push_mixed(9, 7, Format::A8R8G8B8, &p.bgra, &raw, layout);
            let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
            let d = t.decode(&e).unwrap();
            assert_eq!(d.record.layout, layout);
            let kinds: Vec<bool> = d.record.streams.iter().map(|s| s.raw).collect();
            assert_eq!(kinds, raw);
            assert_eq!(d.inflated, 9 * 7 * 4);
            assert_eq!(d.image.rgba[..8], [5, 4, 3, 0, 5, 4, 3, 255]);
            assert_eq!(d.image.rgba[9 * 7 * 4 - 4..], [5, 4, 3, 255]);
        }
        // A raw stream that runs past the file's end is refused.
        let mut dat = TexturesDat::default();
        let e = dat.push_mixed(
            9,
            7,
            Format::A8R8G8B8,
            &p.bgra,
            &[true],
            RecordLayout::Single,
        );
        dat.bytes.truncate(dat.bytes.len() - 1);
        let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
        let err = t.decode(&e).unwrap_err().to_string();
        assert!(err.contains("past textures.dat's end"), "{err}");
    }

    #[test]
    fn reads_padded_rows_mip_chains_and_other_formats() {
        // 9 by 3, A8R8G8B8: 36-byte rows padded to 48.
        let p = picture(9, 3, 6, 7);
        let mut padded = Vec::new();
        for row in p.bgra.chunks(36) {
            padded.extend(row);
            padded.extend([0xAB; 12]);
        }
        // The rest of a padded chain: 4x1, 2x1, 1x1, each row 16 bytes.
        padded.extend([0xCD; 48]);
        let mut dat = TexturesDat::default();
        let e = dat.push_record(9, 3, Format::A8R8G8B8, &padded, 1, RecordLayout::Single);
        let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
        let d = t.decode(&e).unwrap();
        assert_eq!((d.row_pitch, d.row_bytes), (48, 36));
        assert_eq!(d.image.rgba[..8], [7, 6, 3, 0, 7, 6, 3, 255]);
        assert_eq!(d.image.rgba[36..44], [7, 6, 3, 0, 7, 6, 3, 255], "row 1");
        // A packed chain of levels: the top level is the start.
        let mut chain = p.bgra.clone();
        chain.extend([0x11; (4 + 2 + 1) * 4]);
        let mut dat = TexturesDat::default();
        let e = dat.push_record(9, 3, Format::A8R8G8B8, &chain, 1, RecordLayout::Single);
        let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
        let d = t.decode(&e).unwrap();
        assert_eq!(d.row_pitch, 36);
        assert_eq!(d.image.rgba[..8], [7, 6, 3, 0, 7, 6, 3, 255]);
        // DXT1 (one solid red block), A8 and A4R4G4B4.
        let cases: [(Format, Vec<u8>, [u8; 4]); 3] = [
            (
                Format::Dxt1,
                vec![0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0],
                [255, 0, 0, 255],
            ),
            (Format::A8, vec![0x40; 16], [255, 255, 255, 0x40]),
            (Format::A4R4G4B4, [0x0F, 0xF0].repeat(16), [0, 0, 255, 255]),
        ];
        for (format, data, pixel) in cases {
            let mut dat = TexturesDat::default();
            let e = dat.push_record(4, 4, format, &data, 1, RecordLayout::Single);
            let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
            let im = t.read(&e).unwrap();
            assert!(
                im.rgba.as_chunks::<4>().0.iter().all(|c| *c == pixel),
                "{format:?}"
            );
        }
        // P8 bump has no real decoder (Vista's draws one flat colour).
        let mut dat = TexturesDat::default();
        let e = dat.push_record(4, 4, Format::P8Bump, &[7; 16], 1, RecordLayout::Single);
        let mut t = Textures::from_reader(Cursor::new(dat.bytes)).unwrap();
        let err = t.read(&e).unwrap_err().to_string();
        assert!(err.contains("unsupported"), "{err}");
    }

    #[test]
    fn reads_pointers_with_top_bits_masked_off() {
        let (bytes, dat) = icons_map(2);
        let mut map = open(bytes).unwrap();
        let tag = map.tags[1].clone();
        let good = map.bitmaps(&tag).unwrap()[1];
        let mut t = Textures::from_reader(Cursor::new(dat)).unwrap();
        for bits in [1u32, 2, 3] {
            let e = BitmapEntry {
                pointer: good.pointer | bits << 30,
                ..good
            };
            let d = t.decode(&e).unwrap();
            assert_eq!(d.record.location, bits);
            assert_eq!(d.record.offset, u64::from(good.pointer));
            assert_eq!(d.image.rgba[..4], [2, 1, 3, 0]);
        }
    }

    #[test]
    fn entries_write_and_parse_the_same() {
        let e = BitmapEntry {
            width: 300,
            height: 2,
            depth: 1,
            kind: 3,
            format: Format::Dxt5,
            flags: 0x81,
            registration: [-4, 9],
            mip_count: 6,
            pointer: 0x4000_1234,
            stored_size: 99,
            lod_pointers: [1, 2, 3, 4, 5],
            lod_sizes: [6, 7, 8, 9, 10],
        };
        assert_eq!(BitmapEntry::parse(&e.to_bytes()), e);
    }

    #[test]
    fn reads_the_english_strings_through_matg() {
        let mut b = MapBuilder::new(64)
            .tag("matg", r"globals\globals", Meta::Raw(vec![0; 0x200]))
            .tag(
                "unic",
                "u",
                Meta::Struct(Struct::new(0x50).string_id(0, "start")),
            );
        b.language = vec![
            ("start".into(), "PRESS START".into()),
            ("live".into(), "XBOX LIVE".into()),
            ("start".into(), "AGAIN".into()),
        ];
        let image = b.image();
        let mut map = open(compress(&image, 64)).unwrap();
        let lang = map.language_table().unwrap();
        assert!(lang.source.contains("matg"), "{}", lang.source);
        map.read_string_ids().unwrap();
        let named: Vec<(&str, &str)> = lang
            .strings
            .iter()
            .map(|(id, s)| (map.string_id(*id).unwrap(), s.as_str()))
            .collect();
        assert_eq!(
            named,
            [
                ("start", "PRESS START"),
                ("live", "XBOX LIVE"),
                ("start", "AGAIN")
            ]
        );
        assert_eq!(lang.strings, b.language_table());
        // An offset in the middle of a string, or none at all, is refused.
        let matg = u32_at(&image, 0x10) as usize + map.tags[0].address as usize;
        let entry = matg + MATG_ENGLISH;
        let index_at = u32_at(&image, entry + 8) as usize;
        let mut i = image.clone();
        put(&mut i, index_at + 4, 3);
        let mut map = open(compress(&i, 64)).unwrap();
        let e = map.language_table().unwrap_err().to_string();
        assert!(e.contains("isn't the start"), "{e}");
        let mut i = image.clone();
        put(&mut i, entry, 0);
        let mut map = open(compress(&i, 64)).unwrap();
        assert!(map.language_table().is_err());
        // No matg at all.
        let mut map = open(nested_map(64).build()).unwrap();
        let e = map.language_table().unwrap_err().to_string();
        assert!(e.contains("matg"), "{e}");
    }
}
