//! MCC's Halo 2 maps (cache format 13), read only as far as the launcher
//! needs: find a tag by name and group, read its meta, and decode a bitmap
//! whose pixels are in the `textures.dat` beside the maps. Nothing here is
//! shared with the Vista reader in `lib.rs`, whose behaviour is unchanged.
//! The format, in our own words, is in `docs/notes/launcher/mcc-maps.md`.
//!
//! A format-13 map is a 0x380-byte header, kept as it is, then everything
//! else as zlib streams that each inflate to 256 KiB. The header followed by
//! the inflated chunks is the map's "image", and every offset the header
//! gives (but the chunk table's) is an offset in that image. The tag index
//! is Halo 2 Vista's, with addresses counted from the index's start instead
//! of memory addresses.
//!
//! Every size read from a file is checked against the file and against a
//! limit before anything is allocated for it, so a damaged or hostile file
//! gives an error, never a huge allocation or a panic. Chunks are inflated
//! when a read first needs them and a few are kept.

use crate::bitmap::{self, Format, Image};
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

const HEAD_MAGIC: u32 = u32::from_be_bytes(*b"head");
const FOOT_MAGIC: u32 = u32::from_be_bytes(*b"foot");
const TAGS_MAGIC: u32 = u32::from_be_bytes(*b"tags");
const BITM: GroupTag = GroupTag(u32::from_be_bytes(*b"bitm"));
/// The tag index's own header.
const INDEX_HEADER_SIZE: usize = 0x20;
/// One tag in the index: group, datum, address, size.
const TAG_ENTRY_SIZE: usize = 16;
/// A chunk table entry: compressed size, then where it is in the file.
const CHUNK_ENTRY_SIZE: usize = 8;
/// In a bitm tag's meta: the bitmaps block's count, then its address.
const BITM_BITMAPS: usize = 0x44;
/// A `textures.dat` record's own header: chunk count, compressed size.
const RECORD_HEADER_SIZE: u64 = 8;
/// Top bits of a pixel pointer that would say it lives elsewhere (as the
/// top two bits do in Halo 2 Vista). An inference: every rank icon has
/// them clear and is in textures.dat; other places aren't read.
const POINTER_LOCATION_BITS: u32 = 0xC000_0000;

/// Limits on sizes read from the file. MCC's maps use 0x40000-byte chunks;
/// anything from 16 bytes to 16 MiB is taken so tests can use small ones.
const MIN_CHUNK: u32 = 16;
const MAX_CHUNK: u32 = 16 << 20;
/// Tags: a datum's index is 16 bits.
const MAX_TAGS: u32 = 0x1_0000;
/// The tag names together (estimate: a large map's are a few MiB).
const MAX_NAMES: u32 = 64 << 20;
/// The most read from the image at once (a tag's meta, a block).
const MAX_READ: usize = 64 << 20;
/// Images in one bitmap tag.
const MAX_BITMAPS: u32 = 0xFFFF;
/// A bitmap's side, in pixels (estimate: Halo 2's largest are 2048).
const MAX_SIDE: u16 = 8192;
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
    pub map_name: String,
    pub scenario_path: String,
    /// Where the meta starts, counted from the index's start.
    pub meta_start: u32,
    pub meta_size: u32,
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
            // The two strings' lengths are estimates: each runs to the next
            // field we know of.
            map_name: cstr(&b[0xB0..0xD0]),
            scenario_path: cstr(&b[0xD0..0x1D0]),
            meta_start: u32_at(b, 0x2D4),
            meta_size: u32_at(b, 0x2D8),
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

    /// The tag of `group` named `name` (letter case ignored, as Windows
    /// paths are).
    pub fn find_tag(&self, group: GroupTag, name: &str) -> Option<&Tag> {
        self.tags
            .iter()
            .find(|t| t.group == group && t.name.eq_ignore_ascii_case(name))
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

    /// A bitm tag's image entries, in order.
    pub fn bitmaps(&mut self, tag: &Tag) -> Result<Vec<BitmapEntry>> {
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
        let count = u32_at(&meta, BITM_BITMAPS);
        if count == 0 {
            return Ok(Vec::new());
        }
        if count > MAX_BITMAPS {
            return Err(Error::Corrupt(format!(
                "bitmap {} says it has {count} images",
                tag.name
            )));
        }
        // The block's address is counted from the index's start, as tags'
        // are (an inference that the rank icons bear out).
        let at = u64::from(self.header.index_offset) + u64::from(u32_at(&meta, BITM_BITMAPS + 4));
        let block = self
            .read_image(at, count as usize * BITMAP_ENTRY_SIZE)
            .map_err(|e| Error::Corrupt(format!("bitmap {}: {e}", tag.name)))?;
        Ok(block
            .as_chunks::<BITMAP_ENTRY_SIZE>()
            .0
            .iter()
            .map(|e| BitmapEntry::parse(e))
            .collect())
    }
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
    let mut out = Vec::with_capacity(most);
    flate2::read::ZlibDecoder::new(r.take(compressed))
        .take(most as u64 + 1)
        .read_to_end(&mut out)?;
    Ok(out)
}

/// One image of a bitm tag: the fields read from its 168 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapEntry {
    pub width: u16,
    pub height: u16,
    pub format: Format,
    /// Where its pixels start in textures.dat.
    pub pointer: u32,
    /// The record there: its 8-byte header and the zlib stream.
    pub stored_size: u32,
}

impl BitmapEntry {
    /// From an entry's 168 bytes.
    pub fn parse(e: &[u8]) -> BitmapEntry {
        BitmapEntry {
            width: u16::from_le_bytes([e[4], e[5]]),
            height: u16::from_le_bytes([e[6], e[7]]),
            format: Format::from(crate::i16_at(e, 0x0C)),
            pointer: u32_at(e, 0x38),
            stored_size: u32_at(e, 0x50),
        }
    }
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

    /// One image's top level, decoded to RGBA8. Its record is one zlib
    /// stream; records split in more than one chunk, and pixels kept
    /// anywhere but textures.dat, aren't read.
    pub fn read(&mut self, e: &BitmapEntry) -> Result<Image> {
        let (w, h) = (e.width, e.height);
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(Error::Corrupt(format!("a bitmap {w} by {h}")));
        }
        let need = e
            .format
            .top_level_size(w.into(), h.into())
            .ok_or_else(|| Error::Corrupt(format!("unsupported bitmap format {:?}", e.format)))?;
        if e.pointer & POINTER_LOCATION_BITS != 0 {
            return Err(Error::Corrupt(format!(
                "pixel pointer {:#x} isn't in textures.dat (only that is read)",
                e.pointer
            )));
        }
        let at = u64::from(e.pointer);
        if at + RECORD_HEADER_SIZE > self.len {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x} are past textures.dat's end ({:#x})",
                self.len
            )));
        }
        let mut head = [0u8; RECORD_HEADER_SIZE as usize];
        self.reader.seek(SeekFrom::Start(at))?;
        self.reader.read_exact(&mut head)?;
        let (chunks, compressed) = (u32_at(&head, 0), u32_at(&head, 4));
        if chunks != 1 {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x} are in {chunks} chunks; only one is read"
            )));
        }
        let end = at + RECORD_HEADER_SIZE + u64::from(compressed);
        if compressed == 0 || end > self.len {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x}: {compressed} bytes compressed, past textures.dat's end \
                 ({:#x})",
                self.len
            )));
        }
        if u64::from(e.stored_size) != RECORD_HEADER_SIZE + u64::from(compressed) {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x}: the bitmap says {} bytes, textures.dat {}",
                e.stored_size,
                RECORD_HEADER_SIZE + u64::from(compressed)
            )));
        }
        let mut pixels = inflate(
            &mut self.reader,
            at + RECORD_HEADER_SIZE,
            compressed.into(),
            need,
        )
        .map_err(|err| Error::Corrupt(format!("pixels at {at:#x}: {err}")))?;
        // Uncompressed formats inflate to exactly the top level; others may
        // have smaller levels after it, which aren't needed.
        if pixels.len() < need {
            return Err(Error::Corrupt(format!(
                "pixels at {at:#x}: {} of {need} bytes",
                pixels.len()
            )));
        }
        pixels.truncate(need);
        Ok(Image {
            width: w.into(),
            height: h.into(),
            rgba: bitmap::decode(e.format, w.into(), h.into(), &pixels),
        })
    }
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
            self.bytes.extend([0xEE; 5]);
            let pointer = self.bytes.len() as u32;
            let z = zlib(&p.bgra);
            self.bytes.extend(1u32.to_le_bytes());
            self.bytes.extend((z.len() as u32).to_le_bytes());
            self.bytes.extend(&z);
            BitmapEntry {
                width: p.width,
                height: p.height,
                format: Format::A8R8G8B8,
                pointer,
                stored_size: z.len() as u32 + 8,
            }
        }
    }

    pub enum Meta {
        /// Bytes as they are.
        Raw(Vec<u8>),
        /// A bitm tag: 0x80 bytes with the bitmaps block at 0x44 pointing
        /// just after them, then the entries.
        Bitmaps(Vec<BitmapEntry>),
    }

    /// A format-13 map being built: header, names, index, metas, then the
    /// chunks and the chunk table.
    pub struct MapBuilder {
        pub chunk_size: u32,
        /// Bytes of zeros before the first tag's meta, to move it across a
        /// chunk boundary.
        pub pad: usize,
        pub tags: Vec<(GroupTag, String, Meta)>,
    }

    impl MapBuilder {
        pub fn new(chunk_size: u32) -> MapBuilder {
            MapBuilder {
                chunk_size,
                pad: 0,
                tags: Vec::new(),
            }
        }

        pub fn tag(mut self, group: &str, name: &str, meta: Meta) -> MapBuilder {
            let group = GroupTag::parse(group).expect("four letters");
            self.tags.push((group, name.to_string(), meta));
            self
        }

        /// The image (header and inflated body), before compressing.
        pub fn image(&self) -> Vec<u8> {
            let mut b = vec![0u8; HEADER_SIZE];
            put(&mut b, 0, HEAD_MAGIC);
            put(&mut b, 4, VERSION as u32);
            put(&mut b, HEADER_SIZE - 4, FOOT_MAGIC);
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
            for (i, (group, _, meta)) in self.tags.iter().enumerate() {
                let address = b.len() - index;
                let bytes = match meta {
                    Meta::Raw(m) => m.clone(),
                    Meta::Bitmaps(list) => {
                        let mut m = vec![0u8; 0x80];
                        put(&mut m, BITM_BITMAPS, list.len() as u32);
                        put(&mut m, BITM_BITMAPS + 4, address as u32 + 0x80);
                        for e in list {
                            let mut x = vec![0u8; BITMAP_ENTRY_SIZE];
                            x[4..6].copy_from_slice(&e.width.to_le_bytes());
                            x[6..8].copy_from_slice(&e.height.to_le_bytes());
                            let format: i16 = match e.format {
                                Format::A8R8G8B8 => 11,
                                Format::Dxt1 => 14,
                                Format::Other(n) => n,
                                other => panic!("no code for {other:?} here"),
                            };
                            x[0x0C..0x0E].copy_from_slice(&format.to_le_bytes());
                            put(&mut x, 0x38, e.pointer);
                            put(&mut x, 0x50, e.stored_size);
                            m.extend(x);
                        }
                        m
                    }
                };
                let row = index + tags + i * TAG_ENTRY_SIZE;
                put(&mut b, row, group.0);
                put(&mut b, row + 4, 0xE000_0000 + i as u32);
                put(&mut b, row + 8, address as u32);
                put(&mut b, row + 12, bytes.len() as u32);
                b.extend(bytes);
            }
            let v = (b.len() - index) as u32;
            put(&mut b, 0x14, v);
            put(&mut b, 0x2D4, metas as u32);
            let v = (b.len() - index - metas) as u32;
            put(&mut b, 0x2D8, v);
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
        put(&mut d, p + 4, u32::MAX);
        let mut t = Textures::from_reader(Cursor::new(d)).unwrap();
        assert!(bad(good, &mut t).contains("past"));
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
}
