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

The reader does only that: find a tag by name and group, read its meta,
list a bitmap tag's images and decode one image's pixels. It doesn't read
geometry, sounds, string ids or anything else, and nothing in the old
from-scratch engine uses it.

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
| 0xB0 | the map's name (a C string; its room assumed to run to 0xD0) |
| 0xD0 | the scenario's path (a C string; its room assumed to be 0x100 bytes) |
| 0x2D4 | where the meta starts, counted from the index's start |
| 0x2D8 | the meta's size |
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

## Bitmap tags and textures.dat

- In a `bitm` tag's meta, the bitmaps block is still at 0x44: a count, then
  the block's address. The address is taken as counted from the index's
  start, as tags' addresses are; the rank icons bear this out.
- Each image entry is 168 bytes (Vista's are 116). Read from it: width
  (u16) at 4, height (u16) at 6, format (u16) at 0x0C (11 is A8R8G8B8, the
  same numbering as Vista), the pixel pointer (u32) at 0x38, and the stored
  size (u32) at 0x50.
- The pixels are in `textures.dat`, in the same folder as the maps. At the
  pointer: a u32 chunk count (1 for every rank icon), a u32 compressed
  size, then one zlib stream. The stored size is those 8 bytes plus the
  compressed size, which the reader checks as a sign it is at the right
  place. An A8R8G8B8 image inflates to exactly width * height * 4 bytes,
  in B, G, R, A order.
- Unknown: which bits or field say an image's pixels are in textures.dat
  rather than in the map or a shared file. Every rank icon's pointer has
  its top two bits clear and points into textures.dat, so the reader
  treats the pointer as an offset there and refuses pointers with either
  top bit set. Records in more than one chunk aren't read either.

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
at most 65,535 images in a bitmap tag; images at most 8192 pixels a side.
Chunks are inflated when a read first needs them, and the last eight are
kept.

## Tests

`blam_cache::mcc::synthetic` builds made-up format-13 maps (with small
chunks, so tags cross chunk boundaries) and textures.dat files; the tests
in `mcc.rs` and `ranks.rs` use them, and nothing in them comes from a real
file. Two ignored tests read the real icons on the owner's PC:
`H2_MCC_MAPS=<MCC>\halo2\h2_maps_win64_dx11 cargo test -p h2launch --release
reads_real_icons_from_mcc_maps -- --ignored`, and the Vista one with
`H2_MAPS`.
