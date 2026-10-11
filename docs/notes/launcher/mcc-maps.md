# MCC's Halo 2 maps (cache format 13)

What the launcher reads from MCC's own Halo 2 maps, and why. Written in
our own words from read-only looks at the owner's install (MCC build
2025.08.16, `halo2\h2_maps_win64_dx11`) on 2026-10-10; no bytes from those
files are in the repository. The reader is `crates/blam-cache/src/mcc.rs`
(`blam_cache::mcc`), and its only user so far is the lobby's rank icons
(`crates/h2launch/src/lobby/ranks.rs`).

## Why

The lobby draws Halo 2's 50 level icons. They are two bitmap tags in
`mainmenu.map`. Until now they could only come from a Halo 2 Vista install
(cache format 8, which the rest of blam-cache reads), which most players
with only MCC don't have. MCC's `mainmenu.map` holds the same two tags,
and once their pixels are turned from B, G, R, A into R, G, B, A they are
the same images as Vista's. So the lobby reads MCC's first and Vista's
second.

The reader started with only that: find a tag by name and group, read its
meta, list a bitmap tag's images and decode one image's pixels. For the
launcher's Halo 2 menus (`menu.md`) it now also reads the blocks nested in
a tag, tag references, the string ids, the English strings, bitmap
sequences and records of more than one stream, so `blam_cache::ui` reads
MCC's UI tags with the same parsers as Vista's. Several of those layouts
are predictions from Vista's, marked as such below; the console probe
`blam-cache/examples/mcc_ui_probe.rs` (checks A to H of `menu.md` 9.2)
checks them on the owner's PC. It doesn't read geometry or sounds, and
nothing in the old from-scratch engine uses it.

## The file

All numbers little-endian. Four-letter words (`head`, `foot`, `tags`,
group names such as `bitm`) are stored as Halo 2 Vista stores them: the
letters read as a big-endian word, written little-endian, so the file holds
them backwards.

- A header of 0x380 bytes, stored as it is: `head` at 0, the version (13)
  at 4, `foot` at 0x37C. (Vista's header is 0x800 bytes with `foot` at
  0x7FC, so the version word is checked before `foot`: a Vista map then
  says "version 8" instead of "missing foot".)
- Everything after the header is in zlib streams (normal zlib, 15-bit
  window), each the compression of 256 KiB (0x40000 bytes) of the map; the
  last one is shorter. A chunk table lists them, 8 bytes per chunk: the
  compressed size (signed 32-bit), then where the stream starts in the
  file.
- The header followed by every chunk inflated in order is the map's
  "image", exactly as long as the size the header gives. Every offset in
  the header except the chunk table's is an offset in that image.

Header fields read:

| Offset | Field |
| --- | --- |
| 0x08 | the image's size |
| 0x10 | where the tag index starts |
| 0x14 | the index and meta together, in bytes |
| 0x20 | how many tag names |
| 0x24 | where the names buffer starts |
| 0x28 | the names buffer's size |
| 0x2C | where the name table starts (a u32 per tag: its name's place in the buffer) |
| 0x30 | how many string ids (predicted) |
| 0x34 | where the string ids' text starts (predicted) |
| 0x38 | the string ids' text's size (predicted) |
| 0x3C | where the string-id table starts, a u32 per string: its text's place (predicted) |
| 0xB0 | the map's name (a C string; its room assumed to run to 0xD0) |
| 0xD0 | the scenario's path (a C string; its room assumed to be 0x100 bytes) |
| 0x2D4 | where the meta starts, counted from the index's start |
| 0x2D8 | the meta's size |
| 0x2E4, 0x2E8 | thought to place the "locale globals" (the string tables); 0xFFFFFFFF when unused (unverified) |
| 0x308 | the chunk size (0x40000) |
| 0x310 | where the chunk table is, in the file as stored |
| 0x314 | how many chunks |

## The tag index

Halo 2 Vista's layout, except that addresses are counted from the index's
start instead of being memory addresses.

- A 0x20-byte header: the groups' place (from the index's start), how
  many groups, the tags' place (from the index's start), the scenario's
  datum, the globals' datum, a checksum, how many tags, then `tags`.
- Groups: 12 bytes each (not used by the reader).
- Tags: 16 bytes each: group, datum, address, size. A tag's meta is at
  the index's start plus its address, `size` bytes. An address of 0 or
  0xFFFFFFFF means no meta in this map.
- Names come from the map itself (the names buffer and table above).

## Blocks, references and string ids

- A block nested in a tag's meta is 8 bytes, as in Vista: the count, then
  the address, counted from the tag index's start as tags' addresses are.
  Seen for bitm's bitmaps block; predicted for every other block. A block
  must lie inside the index and meta (header 0x14), and holds at most
  1,048,576 elements. Vista keeps a tag's blocks inside its meta; the probe
  counts any that aren't.
- A tag reference is 8 bytes: the group, then the datum (predicted). A
  datum is looked up at its index in the tag table when the salt matches,
  else by search.
- The string ids are predicted to be laid out as the tag names are, at
  0x30 to 0x3C in the header (table above). Halo 2 packs a string id as
  the name's length in the top 8 bits and its index in the low 24, as
  Vista does; index numbers may differ from Vista's, so names are matched,
  not numbers. The table is read on first use, and an offset past the text
  says the prediction is wrong.
- The English strings: Vista's 16-byte language entry (count, size, index
  offset, text offset; index entries are a string id and an offset) is
  looked for at the header's 0x2E4 when it isn't 0xFFFFFFFF, then in matg's
  meta at 0x190, with its offsets taken first as offsets in the image, then
  as counted from the tag index. The first that lies inside the image and
  whose every entry lands on the start of a string is taken. A unic tag's
  English strings are its range of that table (start and count at 0x10).

## Bitmap tags and textures.dat

- In a `bitm` tag's meta, the bitmaps block is still at 0x44: a count, then
  the block's address. The address is taken as counted from the index's
  start, as tags' addresses are; the rank icons bear this out.
- Each image entry is 168 bytes (Vista's are 116). Read from it: width
  (u16) at 4, height (u16) at 6, format (u16) at 0x0C (11 is A8R8G8B8, the
  same numbering as Vista), the pixel pointer (u32) at 0x38, and the stored
  size (u32) at 0x50. Predicted from Vista's entry and read too: depth (u8)
  at 8, type at 0xA, flags at 0xE, the registration point at 0x10, the mip
  count at 0x14, and the other five level-of-detail offsets and sizes after
  the first ones (0x3C and 0x54).
- The sequences block is predicted at 0x3C as in Vista, 0x3C bytes each
  (name, first image, image count, then a sprites block at 0x34 of 0x20
  bytes each: image, left, right, top, bottom, registration).
- The pixels are in `textures.dat`, in the same folder as the maps. At the
  pointer: a u32 chunk count (1 for every rank icon), a u32 compressed
  size, then one zlib stream. The stored size is those 8 bytes plus the
  compressed size, which the reader checks as a sign it is at the right
  place. An A8R8G8B8 image inflates to exactly width * height * 4 bytes,
  in B, G, R, A order.
- Other readers of MCC's maps (Reclaimer's `DataPointer`, see
  `docs/notes/pivot/mcc-vs-vista-map-format.md` section 4) give the
  general record: the count, every part's size as a signed 32-bit number,
  then the parts back to back, each zlib, or stored raw when its size is
  negative. The reader takes sizes that way in every layout below.
- Unknown: which bits or field say an image's pixels are in textures.dat
  rather than in the map or a shared file. Every rank icon's pointer has
  its top two bits clear and points into textures.dat. A pointer with
  either top bit set is read with them masked off, as other readers of
  MCC's maps do; the stored-size check catches a wrong place.
- Records of more than one stream aren't seen yet on the rank icons. The
  reader tries the documented layout first (every stream's size, then the
  streams), then two guesses that also agree with the one-stream case
  (each stream's size just before it; or the streams' total, every
  stream's size, then the streams), and takes the first whose sizes add
  up to the stored size and whose zlib streams each start with a zlib
  header. The streams inflated (raw ones copied) one after another are
  the pixels.
- Rows are taken as packed, unless the inflated size is exactly a chain
  of mip levels with rows padded to 16, 32, 64, 128 or 256 bytes (a guess;
  a packed chain wins a tie). The probe prints the inflated size, the row
  pitch taken, and the entry's words no field is known for, where the
  native mip information and tile mode are thought to be.
- Formats are decoded with the Vista decoders (`blam_cache::bitmap`),
  which take the same numbers: A8, Y8, AY8, A8Y8, R5G6B5, A1R5G5B5,
  A4R4G4B4, X8R8G8B8, A8R8G8B8, DXT1, DXT3 and DXT5. P8 bump (17), which
  the Vista reader only draws as one flat colour, P8 (18) and anything
  else is refused, and the probe lists the format numbers it meets.

## The rank icons

`mainmenu.map` holds:

- `ui\global_bitmaps\rank_icons`: 50 images, 28 by 26, A8R8G8B8;
- `ui\global_bitmaps\rank_icons_sm`: 50 images, 17 by 17, A8R8G8B8.

Image n is level n + 1, as in Vista.

## Limits the reader keeps

Every size read from a file is checked before anything is allocated for
it, and a file that breaks a check gives an error naming what was wrong:
the header's image size must hold at least the header; the chunk size must
be between 16 bytes and 16 MiB; the chunk count must match the image size
and chunk size; the chunk table and every chunk must lie inside the file,
each chunk's compressed size at most one and a half times the chunk size;
every chunk must inflate to exactly its share of the image; at most 65,536
tags and names and 64 MiB of names; one read of the image at most 64 MiB;
at most 65,535 images in a bitmap tag; images at most 8192 pixels a side
and 64 MiB decoded (4096 by 4096); at most 1,048,576 string ids, English
strings or block elements; at most 1,024 streams in a textures.dat record,
inflating to no more than the largest chain of mip levels tried. A short
pixel stream that claims a big image gets no more reserved for it than
zlib could inflate it to.
Chunks are inflated when a read first needs them, and the last eight are
kept.

## Tests

`blam_cache::mcc::synthetic` builds made-up format-13 maps (with small
chunks, so tags cross chunk boundaries) and textures.dat files: metas with
nested blocks, tag references and string ids by name (`Struct`), bitmaps
with sequences, a string-id table, English strings, and records of several
streams in each layout, zlib or raw. The tests in `mcc.rs`, `ui.rs` and
`ranks.rs` use them (`ui.rs` runs each parser on a Vista-style fake and on
a synthetic format-13 map and checks they agree), and nothing in them
comes from a real file.

The probe, from the clone on the owner's PC (prints only):

```
cargo run --release -p blam-cache --example mcc_ui_probe -- "<MCC>\halo2\h2_maps_win64_dx11\mainmenu.map" "<Vista>\maps\mainmenu.map"
```

and `h2tool ui <mainmenu.map>` prints either kind of map's menus with tags
by name, so the two outputs can be diffed.

Two ignored tests read the real icons on the owner's PC. In PowerShell,
from the clone:

```
$env:H2_MCC_MAPS = "<MCC>\halo2\h2_maps_win64_dx11"
cargo test -p h2launch --release reads_real_icons_from_mcc_maps -- --ignored
```

(in cmd, `set H2_MCC_MAPS=<MCC>\halo2\h2_maps_win64_dx11` first), and the
Vista one the same way with `H2_MAPS`.
