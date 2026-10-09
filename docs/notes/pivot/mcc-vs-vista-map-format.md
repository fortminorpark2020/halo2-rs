# Halo 2 MCC (classic) maps vs Halo 2 Vista maps, and what it takes for blam-cache to read them

Angle: file formats only. Researched 2026-10-09. Tool sources were cloned read-only into
`scratchpad/pivot/repos/` (assembly, assembly-hist, reclaimer, h2taglayouts, c20). No MCC map file
was available to test against, so everything below comes from tool source code and docs, not from
opening a real MCC file. Each claim says how sure it is.

## 1. Short answer

- MCC's classic Halo 2 maps are the **Halo 2 Vista format with changes**. They are not a different
  engine family (c20 says H2A "is derived from the Halo 2 Vista port").
  https://c20.reclaimers.net/h2/
- Most tag layouts that our reader uses are **byte-identical** to Vista: weapon, projectile,
  scenario, sbsp top level, collision BSP, biped/vehicle top level, globals player tables, hlmt,
  shader, HUD. (Assembly MCC vs Vista plugin diff, below.)
- What is different:
  1. **The container.** The header version is 10 (May 2020 to Oct 2021) or **13 (Season 8, Oct 2021
     onward)**. The header is a different size with fields at new offsets, and the file after the
     header is usually **zlib-compressed in 0x40000-byte chunks**.
  2. **Bitmaps.** Pixel data moves out of the maps into one **`textures.dat`**. The bitmap data
     element grows from 0x74 to 0xA8 bytes, and the pixel pointer and size fields move.
  3. **Sound.** This is a real rework. `snd!` grows from 0x14 to 0x1C bytes, and the codec moves into
     a new "codecs" block in `ugh!`. Permutation chunks become per-language, and a new **Opus**
     codec is added (alongside Xbox ADPCM and WMA).
  4. **64-bit pointer growth.** MCC is a 64-bit build, so runtime-pointer fields in some structs
     grew from 4 to 8 bytes. This hits **Havok physics shapes** (phmo, coll, and the bipd/vehi
     physics blocks), geometry section structs (0x44 to 0x48), and vertex/pixel shaders.
  5. **Remastered hooks.** Small added fields reference H2A remastered sounds (matg, lsnd, sncl),
     plus "metagame" bytes in units. Classic multiplayer does not need these.
- **Halo 2 Anniversary multiplayer maps are a different engine**: groundhog, third-generation and
  Halo 4-derived. They are useless to our reader and not "original Halo 2" anyway.
- **Estimate.** Teaching blam-cache to read MCC classic multiplayer maps as well as Vista is about
  **1,000-1,500 lines over 5-10 working days for an assistant, plus a slow test loop on John's PC**.
  It is not a rewrite. About 80-90% of the ~400 hard-coded tag offsets carry over. The risky part
  is sound (Opus, language chunks).

## 2. Container: header, version, compression

| | Halo 2 Vista | MCC H2 (pre-Season 8) | MCC H2 (Season 8+) |
|---|---|---|---|
| `head` version | 8 | **10** | **13** |
| Header size | 0x800 | 0x1000 | **0x380** |
| Build string | "11081.07.04.30.0934.main" etc. | empty | empty |
| meta offset / size / mask | 0x10 / 0x1C / 0x20 | 0xC / 0x18 / 0x1C | 0x10 / 0x14 / **0x2D0** |
| string table count/offset | 0x170 / 0x17C | 0x168 / 0x174 | 0x30 / 0x34 |
| file (tag-name) table | 0x2CC.. | 0x2E8.. | 0x20.. |
| scenario name | 0x1C8 | 0x1E4 | 0xD0 |
| compression | none | chunk table at 0x1000, data from 0x3000 | header fields 0x308 (chunk size), 0x30C (data offset), 0x310 (table offset), 0x314 (count); flags at 0x1C, bit 0 = compressed |

Sources:
- Header offsets: Assembly `H2V_Layouts_Core.xml`, `H2MCC_Layouts_Core.xml` and
  `H2MCC_LayoutsU1_Core.xml`.
  https://github.com/XboxChaos/Assembly/tree/master/src/Blamite/Formats/Halo2MCC
  https://github.com/XboxChaos/Assembly/blob/master/src/Blamite/Formats/Halo2Vista/Layouts/H2V_Layouts_Core.xml
- Version numbers: Assembly `Engines.xml` (Halo 2 MCC version="10", "Halo 2 MCC Update 1"
  version="13", Vista version="8").
  https://github.com/XboxChaos/Assembly/blob/master/src/Blamite/Formats/Engines.xml
- Reclaimer agrees: it detects version 10 as MccHalo2 and 13 as MccHalo2U1, and gives the U1 header a
  fixed size of 896 (0x380). Reclaimer.Blam/Blam/Common/CacheArgs.cs and
  Halo2/Config/CacheFile.config.cs, https://github.com/Gravemind2401/Reclaimer
- Reclaimer refuses version 10 outright (`CacheFactory.cs`: `cacheType == CacheType.MccHalo2 =>
  throw Exceptions.UnknownMapFile`). Current installs are version 13. Confidence high.
- Version 13 arrived with Assembly commit b98c41b, 2021-10-15, "Support for Season 8 MCC Update".
  MCC Season 8 began on 13 Oct 2021:
  https://windowscentral.com/halo-mcc-season-8-begins-october-13-xbox-and-pc

Compression (Assembly `SecondGenSaberZLib.cs` and Reclaimer `MccCompressedCacheStream`):
- The header is stored raw.
- A chunk table follows, holding (int32 compressed size, int32 file offset) pairs. Each chunk is a
  zlib stream (2-byte header plus deflate) that inflates to 0x40000 bytes (the last chunk is shorter).
- A negative size means the chunk is stored raw (tools use this for "faux compression").
- Inflating all the chunks after the header gives an ordinary uncompressed cache file.
- After decompressing, Assembly clears flags bit 0 so the game will still run the file.
https://github.com/XboxChaos/Assembly/blob/master/src/Blamite/Compression/SecondGenSaberZLib.cs

The rest of the container is **the same as Vista**: meta header (0x20, `tags` magic), tag group table
(0xC per entry), tag table (0x10 per entry), string-id and tag-name tables, and the
`address - mask + meta offset` addressing. Same Assembly layouts; Reclaimer's TagAddressTranslator
uses the Vista path for every version from Halo2Vista on. Confidence high.

Unverified: where the `foot` magic sits in the 0x380 header. Our reader checks for it at 0x7FC.

The H2EK's own `build-cache-file` takes flags `compress | resource_sharing | mp_tag_sharing |
multilingual_sounds | remastered_support` and outputs to `h2_maps_win64_dx11`. So compression and
shared-map tags are optional per map.
https://c20.reclaimers.net/h2/tools/h2-ek/h2-tool/

## 3. Files on disk

- Maps live in `...\Halo The Master Chief Collection\halo2\h2_maps_win64_dx11\`.
  https://nexusmods.com/halothemasterchiefcollection/mods/2407
- `mainmenu.map`, `shared.map` and `single_player_shared.map` sit there too.
  https://archive.vg-resource.com/thread-38349.html
- Multiplayer maps still keep shared tags in `shared.map`. Reclaimer's comment reads: "H2V/H2MCC
  multiplayer maps appear to reserve first 10k/17k tag slots for local tags. and the rest are
  placeholders representing tags found in shared.map." So our `MapSet` model (map + shared +
  single_player_shared + mainmenu, 2-bit location in the pointer) still fits. Confidence medium-high.
- **`textures.dat`**: every MCC bitmap's pixel data is read from `textures.dat` in that folder,
  whatever the pointer's location bits say. Workshop maps may keep bitmaps locally.
  Reclaimer `Halo2/DataPointer.cs`.
- The fonts folder, movies and the Microsoft Store / Game Pass install location were not
  researched. **Unknown.** The `menu-preview` branch depends on Vista's `maps\fonts` and Vista's
  `mainmenu.map` UI tags. Whether MCC's H2 `mainmenu.map` still has the classic menu UI tags is
  **unknown** (MCC draws its menus in Unreal).
- MCC classic multiplayer has all original, map-pack and Vista maps. That includes **Desolation and
  Tombstone, which Vista never had**, and District/Uplift, which were Vista-only.
  https://www.halopedia.org/Halo:_The_Master_Chief_Collection
  https://www.halopedia.org/Halo_2_(Windows_Vista)

## 4. Bitmaps

- The `bitm` top level and the Sequences/Sprites blocks are unchanged (0x3C / 0x20). Our
  `read_sequences` works as is.
- The **Bitmaps data element grows from 0x74 to 0xA8**. Added fields include a native mipmap info
  block, native size, tile mode and several 64-bit runtime pointers. The largest-level pointer moves
  **0x1C to 0x38** and its size moves **0x34 to 0x50**. Width, height, type, format and flags stay
  at 0x4 to 0xE.
  - Reclaimer `BitmapTag.cs`: `[FixedSize(116, MaxVersion=MccHalo2)] [FixedSize(168,
    MinVersion=MccHalo2)]`, with Lod0Pointer at 28 / 56 and Lod0Size at 52 / 80.
  - Assembly `Halo2MCC/bitm.xml` agrees.
- Data format in `textures.dat`, at the pointer:
  1. int32 segment count.
  2. int32[count] segment sizes.
  3. The segments back to back. Each is zlib, or raw if its size is negative.

  Vista instead uses one zlib stream inside the map. (Reclaimer `DataPointer.cs`)
- The pixel format enum is unchanged (A8, Y8, AY8, A8Y8, R5G6B5, A1R5G5B5, A4R4G4B4, X8R8G8B8,
  A8R8G8B8, DXT1/3/5, P8-bump, ...), so our DXT and decode code is reusable. There is no BC7.
  (Assembly `Halo2MCC/bitm.xml`)
- MCC adds flag bits "WDP Compression" and "OG Xbox Mipmap Selection". Their effect on retail data
  is unknown.
- Texture resolution versus Vista: **unknown**. c20 says MCC H2 is derived from the Vista port,
  which suggests the same base textures, but this is unconfirmed.

## 5. Geometry and collision

- sbsp top level is the same (Clusters at 0x9C/0xB0, Materials at 0xA4/0x20, instanced geometry at
  0x138/0xC8 and 0x140/0x58, collision BSP at 0x14/0x40). Our `render.rs` and `geometry.rs`
  constants are right for MCC.
- The nested geometry "section" struct grows from **0x44 to 0x48**: its `index_buffer` runtime
  pointer is now 8 bytes. This is the Ptr in `global_geometry_section_struct`, in Halo2TagLayouts
  `common/global_geometry_definitions.xml`, and the Assembly sbsp/ltmp plugin diff shows the same.
- Reclaimer reads MCC geometry with the Vista code except for one change: **the node-map resource
  type is 104 instead of 100** (`Halo2Common.cs`: `if (CacheType >= MccHalo2) nodeMapType = 104;`).
  Index count stays at raw offset 40, and the index (32) and vertex-buffer (56) resource types are
  unchanged. Our `RES_NODE_MAP = 100` would need a per-version value, or skinned models break.
  Confidence medium (one tool's implementation).
- ltmp (lightmap) groups and buckets are unchanged at the levels our `lightmap.rs` reads.
- The collision BSP (the 3D/2D nodes, planes, surfaces and edges that our player collision uses)
  is unchanged. The sbsp "BSP Physics" (Havok MOPP) block grew from 0x74 to 0xA0, but we do not
  read it.

## 6. Physics (Havok): changed

phmo block element sizes, Vista to MCC (Assembly plugins; Halo2TagLayouts `physics_defintions.xml`
has the Ptr fields that cause the growth):

| Block | Vista | MCC |
|---|---|---|
| Rigid bodies | 0x90 | 0xA0 |
| Spheres | 0x80 | 0xA0 |
| Multi spheres | 0xB0 | 0xC0 |
| Pills | 0x50 | 0x60 |
| Boxes | 0x90 | 0xB0 |
| Triangles | 0x60 | 0x70 |
| Polyhedra | 0x100 | 0x120 |
| Lists | 0x38 | 0x68 |
| List shapes | 0x8 | 0x10 |
| MOPPs | 0x14 | 0x28 |
| Phantoms | 0x20 | 0x40 |

The fields inside each shape shift too. Our `vehicle.rs` `read_physics_model` (boxes, polyhedra,
pills, spheres, rigid bodies, lists) needs a second offset table. Other growth:

- vehi Havok vehicle physics block: 0xF0 to 0x130.
- bipd/crea physics struct block: 0x80 to 0xA0.
- coll: 0x74 to 0xA0.
- phys: base 0x74 to 0x80.

Our reader only uses bipd/vehi top-level fields, which are unchanged.

## 7. Sound: biggest change, highest risk

Vista: `snd!` is 0x14 bytes, with sample rate, encoding and compression at 0x3..0x5.
MCC: `snd!` is **0x1C bytes**:
- +0x3 is a **codec index** into the new `ugh!` "codecs" block. That block has 3-byte elements:
  rate, encoding, compression.
- +0x4 is a remastered-sound reference index.
- +0x14..0x1B add an inner silence distance, reflection and low-pass indices.

`ugh!` is 0x58 bytes on Vista and **0x78 on MCC**. Every block offset moves:

| Block | Vista | MCC |
|---|---|---|
| Codecs (new) | | 0x00 |
| Playback | 0x00 | 0x08 |
| Pitch ranges | 0x20 | 0x28 |
| Permutations | 0x28 | 0x30 |
| Reflections (new) | | 0x40 |
| Low-pass (new) | | 0x48 |
| Remastered sounds (new) | | 0x50 |
| Chunks | 0x40 | 0x60 (now one block per language) |
| Extra info | 0x50 | 0x70 (element 0x2C to 0x14C) |

- **Permutations** no longer hold sample size or first chunk / chunk count directly. They hold a
  "localized chunks info" block (up to 9 languages) with sample size, first chunk and chunk count
  per language.
- **Chunks** grow from 0xC to 0x10 bytes (file offset, sizes, runtime index).

Sources: Assembly `Halo2MCC/snd!.xml` and `ugh!.xml` ("Work started mapping h2 sound changes",
commit 181b7f9, 2020-04-24); Halo2TagLayouts `cache_file_sound.xml`, `sound_cache_file_gestalt.xml`
and `common/sound_defintions.xml`.

The sound compression enum adds **"opus" (value 5)**, and H2EK tool import commands take
`uncompressed | adpcm | opus`, including `reimport-sounds-to-opus`.
https://c20.reclaimers.net/h2/tools/h2-ek/h2-tool/

- **Unknown:** whether retail classic multiplayer maps use Opus, Xbox ADPCM, or both. Telling them
  apart needs a real file.
- **Unknown:** whether chunk file offsets point into the map or a shared file.
- H2 MCC sound appears to stay **inside the .map files, not FMOD banks**. A mod that restores
  classic campaign audio does it by rebuilding the H2 `.map` files with the official mod tools
  (https://nexusmods.com/halothemasterchiefcollection/mods/2407). Other MCC games such as Reach
  have `fmod\pc` folders; no Halo 2 equivalent turned up. Confidence medium.

Tool support: Reclaimer's `Halo2/SoundTag.cs` only knows the Vista sound layout and only supports
WMA and Xbox ADPCM, so **no open tool I checked decodes MCC H2 sounds**. We would be first.

Opus in Rust:
- libopus bindings (C) work.
- Pure-Rust decoders exist (`ruopus`, `opus-rs`, `mousiki`), but their maturity is unverified.
  https://docs.rs/crate/ruopus/latest  https://lib.rs/crates/opus-rs

## 8. Tags that changed and tags that did not

Method: a structural diff of Assembly's `Plugins/Halo2` (Vista) against `Plugins/Halo2MCC`. Only 35
groups have MCC overrides; the other 63 groups fall back to the Vista plugins
(`<fallbackPlugins>Halo2</fallbackPlugins>`). As a cross-check, Halo2TagLayouts (dumped from the
H2EK) marks the 64-bit Ptr fields, and they appear only in physics, physics_model, collision geometry,
global geometry, bitmap, vehicle, sbsp, vertex_shader, pixel_shader and shader_pass.
https://github.com/num0005/Halo2TagLayouts (no license file: use it as a reference, not code).

- **Same layout:** weap, proj, mulg, hlmt, shad, eqip, garb, scen, bloc, mach, ctrl, lifi, sily,
  ssce, nhdt, snde.
- **Same in practice:**
  - scnr: one formerly null block at 0x288 is now a 0x24-byte block; spawns, BSP block, netgame
    flags and so on are unchanged.
  - matg: block at 0xC0 grew 0x24 to 0x2C for remastered sound; 0xD8 is filled. Our 0xF0, 0x130 and
    0x140 tables are unchanged.
  - bipd, vehi, crea: 0x196 split into two "metagame" bytes; only the physics blocks grew.
- **Changed:** bitm, snd!, ugh!, lsnd (+8 for a remastered tagref), sncl, phmo, phys, coll, char,
  spas, vrtx, sbsp/ltmp (nested only).
- **No MCC override, so assumed the same (unverified):** mode, jmad, effe, jpt!, unic, wgit, skin,
  sky, itmc, vehc.

| blam-cache module | MCC status |
|---|---|
| lib.rs (header, tag table, spawns, BSPs, skies) | header: 2 new layouts; rest same |
| mapset.rs | new folder; textures.dat; same pointer scheme |
| bitmap.rs | element 0xA8; new pointer and size offsets; textures.dat segment format |
| render.rs | node-map resource 100 to 104; else same |
| lightmap.rs, geometry.rs, scenario.rs, weapon.rs, hud.rs, text.rs, shader.rs | same |
| model.rs | probably same (mode has no override) |
| physics.rs | same |
| vehicle.rs | phmo shape offsets all change |
| animation.rs | probably same (unverified) |
| sound.rs | rewrite the gestalt parsing, add an Opus decoder |
| ai/orders/pathfinding/script (campaign) | out of scope (no campaign) |

## 9. Halo 2 Anniversary multiplayer (groundhog): skip

Assembly's "Halo 2 Anniversary MCC" engine is `<generation>third</generation>`, plugins `Halo2AMCC`
with `<fallbackPlugins>Halo4</fallbackPlugins>`, and `<module>groundhog</module>`. From Update 5 it
uses string hashes. Reclaimer reads it with its Halo 4 code (`MccHalo2X`). Halopedia calls it "a
separate multiplayer engine described as a combination of the multiplayer of the classic Halo 2 and
that of the more recent Halo titles."
https://www.halopedia.org/Halo_2:_Anniversary

Supporting it would mean a whole new third/fourth-generation reader, and it is not original Halo 2.
Recommendation: ignore H2A multiplayer.

## 10. Tools that read MCC H2 maps

- **Assembly** (GPL-3): reads and edits versions 10 and 13, decompresses and recompresses, and has
  MCC plugins. https://github.com/XboxChaos/Assembly
- **Reclaimer** (GPL-3): opens version 13 only (version 10 throws). It extracts bitmaps (via
  textures.dat) and geometry, but not MCC sounds. https://github.com/Gravemind2401/Reclaimer
- **H2EK** (official, 2021): needs Halo 2 Anniversary on Steam. It ships "All tags used in H2C" as
  source tags (tags.zip and data.zip), so tag extraction is not needed.
  https://c20.reclaimers.net/h2/tools/h2-ek/
  - Its `halo2_tag_test.exe` standalone build "doesn't include network functionality".
    https://c20.reclaimers.net/h2/tools/h2-ek/h2-standalone-build/
  - The source tags use the BLM! tag-file format with tbfd field-set headers. That is a different
    reader again, but it is self-describing (element sizes stored).
    https://c20.reclaimers.net/h2/engine/tag-files/
- **Halo2TagLayouts** (num0005): MCC tag layouts dumped from the H2EK. They are machine-readable
  and could generate both Vista and MCC offset tables (each Ptr is 4 bytes in tags and 8 in MCC
  cache files; `pd64`/`pd32` pads).

Licence note: our crate is MIT. Do not paste GPL code from Assembly or Reclaimer. Re-implement from
the documented layouts, as AGENTS.md already requires for the decompilation.

## 11. Other format-adjacent findings

- c20 lists **hard-coded multiplayer tag patches that MCC applies at load time**, so the tag values
  are not what MCC players get. They cover:
  - BR error angle set to 0.1
  - melee damage
  - frag/plasma radius, damage, arming and timers
  - magnum 5.5, SMG 4.625, brute shot
  - plasma rifle dual-wield scale 0.7
  - AP turret radius

  https://c20.reclaimers.net/h2/

  Our sim reads values from tags, so on MCC maps (and maybe Vista) we must apply the same patches.
  Whether the Xbox exe applied the same ones (via title update) should be checked in the
  decompilation. **Unknown.**

## 12. Suggested plan for blam-cache (if the pivot happens)

1. Add `enum Flavor { Vista, Xbox, MccV10, MccV13 }`. Parse each header layout into the existing
   `Header` struct. Keep `version != 8` only for Vista and Xbox.
2. Add a `ChunkedZlib` reader that implements `Read + Seek`. It inflates 0x40000-byte chunks on
   demand with a small cache. `CacheFile<R: Read + Seek>` is already generic, so it plugs in.
   `MapSet`'s `Map = CacheFile<BufReader<File>>` becomes an enum or a boxed reader. Alternative: load
   each decompressed map fully into memory; multiplayer maps are tens to low hundreds of MB.
   About 300 lines plus synthetic tests.
3. MCC folder discovery (Steam path) next to the Vista discovery; open `textures.dat`.
4. Bitmaps: per-flavour element size and offsets; the textures.dat segment reader (about 100 lines).
5. Geometry: per-flavour node-map resource type (100 or 104).
6. phmo: a second offset table for MCC shapes. It could be generated from Halo2TagLayouts.
7. Sound: an MCC gestalt reader (codecs block, per-language chunks, English as language 0), plus
   Opus decoding (libopus binding or a pure-Rust crate) next to the existing ADPCM and WMA.
8. Tests: synthetic MCC headers and compressed chunks in `tests.rs`, as AGENTS.md requires. Real
   files go through `#[ignore]` tests reading `H2_MCC_MAPS`.
9. Verification needs John to run `h2tool` probes on his MCC install, because game files may not be
   uploaded. That makes a slow loop, so build a self-check command, for example
   `h2tool --mcc-selftest`, that prints summaries.

Rough size: 1,000-1,500 new or changed lines in blam-cache. h2sim and h2viewer barely change, since
they consume decoded structs. The biggest unknown is sound.

## 13. Unknowns (need a real MCC file or further research)

- Whether retail classic multiplayer maps are all compressed (flags bit 0), and where `foot` is in
  the version 13 header.
- Opus versus ADPCM in retail classic multiplayer sounds; the Opus packet framing; whether sound
  chunk offsets point into the map or `shared.map`.
- Texture resolution versus Vista; what "WDP Compression" and "Tile Mode" mean for retail bitmaps.
- Whether mode and jmad really are unchanged (Assembly has no MCC override; Halo2TagLayouts has no
  Ptr in them, which is consistent).
- Whether MCC's H2 mainmenu.map still has the classic menu UI tags and bitmaps, and where the fonts
  are.
- Store / Game Pass install path and file permissions.
- Whether future MCC patches change the format again (it changed in Oct 2021).
