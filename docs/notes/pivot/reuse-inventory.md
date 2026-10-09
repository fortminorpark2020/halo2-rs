# What halo2-rs can reuse under each pivot

Angle: an inventory of the existing code (main 98fe84a plus the five open branches), sorted by
pivot:

- A: keep the Rust engine, but read MCC's Halo 2 maps.
- B: a launcher that runs MCC's real Halo 2 engine (`halo2.dll`) on its own, with our online layer.
- C: a mod or plugin inside MCC.
- D: the original Xbox Halo 2 under an emulator, plus a revival service.

Researched 2026-10-09. Line counts are `wc -l` on the repo, with test lines split out by a script
(files named `tests.rs`, `tests/`, `testing.rs`, and inline `#[cfg(test)] mod` blocks). Facts about
outside projects carry a URL. Where a fact comes from a sibling note in this folder, I say so and
give that note's source. "Unknown" means nobody has checked it.

## 0. Baseline: what exists

Rust on `main`: 81,446 lines, of which about 18,263 are tests.

| Crate | Total | Tests | What it is |
|---|---|---|---|
| blam-cache | 8,200 | 473 | Vista `.map` reader. Campaign-only modules: ai 646, orders 448, script 308 (1,402) |
| h2sim | 18,351 | 4,272 | Simulation, game types, bots. Campaign-only: bot/actor 613, script 1,328, game/campaign 397 (2,338) |
| h2net | 7,916 | 3,525 | LAN game sync, transport (TCP/WebSocket), and the online control protocol `live.rs` |
| h2live | 10,730 | 5,340 | Online service: accounts, parties, matchmaker, levels 1-50, playlists, relay, stat cards |
| h2viewer | 32,134 | 4,529 | The game: wgpu renderer, menus, HUD, input, audio, LAN and online clients. Campaign 5,561 |
| h2tool | 1,918 | 0 | Command-line map inspector |
| wma | 2,197 | 124 | WMA2 decoder (LGPL) |

h2viewer by area (lines): rendering and world 7,063 (gpu, scene, scene/vehicles, body, rig, effects,
probe, vehicles, objective); HUD 527 (hud, font); menus and front end 6,584 (menu 4,211, flow 1,055,
options, profile, emblem, rank, mapinfo); input and camera 1,979; audio 1,354; online client 3,074
(online.rs, online/custom, matches, recent, netprobe); LAN 748; local player 2,104; app shell 3,140
(main, memory); campaign 5,561 (dead since the owner dropped campaign). About 1,200 lines of
menu.rs on main are the online screens and their tests (live lobby, players, recent players,
playlists, matchmaking, pregame).

Branches not on main, measured with `git diff --shortstat main...origin/<b>`:

- `controller-wip`: +3,876/-374. input.rs +1,317, rumble.rs 390 new, local +613, menu +608.
- `menu-preview`: +9,274/-889. blam-cache ui.rs 1,040 and font.rs 385; h2viewer menu.rs +3,539,
  menuart 986, messages 970, intro 524, flythrough 408, menuscene 119. Sets `PROTOCOL` 27.
- `feel-weapons`: +2,480. `feel-combat`: +1,754. `feel-bots`: +680. All three are mostly h2sim and
  h2viewer gameplay.

Non-code: `docs/notes/decomp/*` (1,907 lines), `controller-plan.md` 932, `mainmenu-plan.md` 464,
`docs/ONLINE.md` 110, the deploy files (`deploy/proxmox/install-h2live.sh` 45, `Dockerfile.live`
30, `render.yaml` 41), and CI (`.github/workflows/build.yml` 50: clippy, tests, the Windows exe,
and a Docker health check).

## 1. Summary matrix

Key: C = carries over directly; A = needs adapting; D = dropped. "Ref" means kept only as a
reference.

| Component (lines, code+tests) | A: Rust engine on MCC maps | B: launcher running halo2.dll | C: mod inside MCC | D: Xbox under emulator + revival |
|---|---|---|---|---|
| h2live server: server, party, matches, relay, store, card, main (≈3,700 code) | C | A: decouple from h2sim; datagram relay; variant and engine-build ids | A: no relay; matches become "join this MCC custom game" | Ref only: an Xbox client speaks Xbox Live protocols, not ours |
| h2live rules: levels.rs, matchmaker.rs, playlists.rs (≈1,500 code, ≈1,650 tests) | C | C, with playlist variants pointing at MCC variant files | C | levels: Ref (the original client computes XP itself). matchmaker: D (the client searches). playlists: A (served as hopper data) |
| h2live tests (5,340) | C | Mostly C. server/tests/matches.rs (1,861) plays games with h2sim: keep h2sim as a test fixture or script the results | Mostly C | D |
| h2live::client (420) | C | A: results come from the engine, not `h2sim::Game` | A: runs inside the DLL | D |
| h2net live.rs control protocol (1,309 + 414 tests) | C | C, plus new fields | C | D |
| h2net transport conn.rs + ws.rs (957) | C | C (control link). Game traffic needs datagrams | C | D |
| h2net LAN game sync: host, client, delta, predict, lobby, discovery (1,968) + h2sim game/sync.rs (1,269) + tests.rs (3,047) | C | D (the engine has its own netcode) | D | D |
| blam-cache (8,200) | A: about 1,000-1,500 lines for MCC headers, zlib chunks, textures.dat, Havok offsets, Opus sound | A, small part: map list, map pictures and text, rank icons and emblems for the launcher UI. Geometry, physics and animation not needed at runtime | D (maybe Ref) | D |
| h2tool (1,918) | C after blam-cache A | A: inspector and reverse-engineering aid for MCC maps | Ref | D |
| wma (2,197) | Maybe C (unknown whether retail MCC still uses WMA); add Opus | D | D | D |
| h2sim (18,351) | C (campaign 2,338 dead). Decide on MCC's hard-coded tag patches | D at runtime. Keep as a server test fixture and load-test bot | D | D |
| h2viewer rendering/world/HUD/local/audio (≈11,000) | C | D | D | D |
| h2viewer menus and flow (6,584 + menu-preview ≈7,500) | C, but menu-preview needs Vista fonts and mainmenu UI tags (MCC unknown) | A: logic and screens reused, renderer swapped (overlay on the engine's D3D11 swap chain) | A: overlay only | D |
| h2viewer online client screens (3,074 + ~1,200 in menu.rs) | C | A: same flow, new renderer | A | D |
| h2viewer input/controllers (1,473 + 1,317 branch, rumble 390) | C | A: pad reading, ownership, hot-plug and fake pad kept; layouts become profile mappings; Halo 2 stick maths left to the engine | D (MCC does input) | D (emulator does input) |
| h2viewer profile, emblem, rank (876) | C | A: fill MCC's player-profile struct; emblem callbacks need D3D11 textures | D/A | D |
| h2viewer LAN (748) | C | D, or A for the engine's system link | D | D |
| Campaign code (≈9,300 across crates) | D (owner dropped campaign) | D | D | D |
| decomp notes: controller, movement-combat, damage, game-engines (1,566) | C (Xbox values are the target) | Ref (the engine already behaves like this; helps find functions in halo2.dll) | Ref | Ref |
| decomp online-unverified.md (153) | C | C: XP tables, hoppers, search scoring, balancing | C | C: central |
| decomp ui-hud-unverified.md (188), mainmenu-plan, controller-plan | C | Ref (Halo 2 look for the launcher's own UI) | Ref | D |
| CI and deploy (Docker, Render, Proxmox) | C | Server parts C. The engine host can only be tested on Windows with MCC installed | Server parts C | D |

## 2. Pivot details

### A. Keep the Rust engine, read MCC's Halo 2 maps

Almost everything carries over. This continues the remake; it only swaps the data source.

- **blam-cache must learn MCC's container and a few tags.** All figures below are from the sibling
  note `mcc-vs-vista-map-format.md`, based on Assembly and Reclaimer sources:
  - Headers: version 10 and version 13 (Season 8). The version-13 header is 0x380 bytes. Our code
    accepts only version 8 (`crates/blam-cache/src/lib.rs` `Header::parse`).
  - Compression: zlib in 0x40000-byte chunks.
  - Bitmaps: pixel data moves to `textures.dat`, and the bitmap element grows from 0x74 to 0xA8.
  - Physics: Havok shapes grow because of 64-bit pointers.
  - Geometry: the node-map resource type changes from 100 to 104.
  - Sound: tags rebuilt, with a new Opus codec.
  - Sources: https://github.com/XboxChaos/Assembly (src/Blamite/Formats/Engines.xml, Halo2MCC
    layouts, SecondGenSaberZLib.cs) and https://github.com/Gravemind2401/Reclaimer.
  - The sibling note's estimate is 1,000-1,500 lines. The rest of blam-cache (scenario, weapon,
    HUD, shader, lightmap, geometry top level) has the same layout.
- **The generic reader helps.** `CacheFile<R: Read + Seek>` is already generic, so a chunk-inflating
  reader plugs in without touching the tag code (`crates/blam-cache/src/lib.rs`).
- **h2viewer needs small changes** for paths and names:
  - Map discovery is hard-coded to Vista and Cartographer folders
    (`crates/h2viewer/src/main.rs:105-107`); MCC uses `halo2\h2_maps_win64_dx11`.
  - `MAP_TITLES` in menu.rs needs MCC's extra maps, Desolation and Tombstone (sibling note, citing
    https://www.halopedia.org/Halo:_The_Master_Chief_Collection).
- **menu-preview may not port.** It reads Vista's `maps\fonts` font packages (blam-cache font.rs)
  and the UI tags in Vista's mainmenu.map. HaloX's source comment says MCC keeps text fonts as
  pre-rasterised glyph files in `halo2\h2_fonts\*`, and keeps only icon font packages in
  `maps\fonts` (https://github.com/SpringContingency/HaloX, src/text/font_cache.cpp lines 41-46;
  that repo has no licence, so reference only). Whether MCC's Halo 2 mainmenu.map still holds the
  classic UI tags is unknown.
- **Gameplay values.** c20 lists multiplayer tag values that MCC patches in at load time (BR error
  angle, melee, grenades, magnum, SMG and others), so MCC's tag values may not be what Xbox
  players had (https://c20.reclaimers.net/h2/). Our sim reads tag values, so each value has to be
  checked against the decompilation.
- **Cost.** The feel-matching work on gameplay is not reduced at all. That is the thing John wants
  to stop doing.

### B. Launcher that runs MCC's `halo2.dll` (the "Project Reclaimer for Halo 2" model)

**Precedent.**
- Project Reclaimer is a standalone Halo 3 launcher. It loads the game from the user's MCC install
  and ships no game files. Its developers say it has no matchmaking.
  - https://projectreclaimer.dev/ (cited in the sibling note `reclaimer-and-similar-projects.md`).
  - https://windowsforum.com/news/project-reclaimer-adds-64-player-halo-3-custom-games-on-windows.447125/
    (fetched).
- The sibling note says Reclaimer is written in Rust, per https://projectreclaimer.dev/download.html.

**What MCC's shell does for the engine.** This comes from libmcc's interface
(https://github.com/SpringContingency/libmcc, include/libmcc/game/game_manager.h; no licence,
reference only).
- The engine DLL calls the host for:
  - frames (`end_frame(IDXGISwapChain*)`);
  - `set_game_result`;
  - the player profile (`get_player_profile(XUID)`);
  - input (`get_input_state`, `get_input_state_gamepad`), and rumble (`set_input_state` with left
    and right motor speeds);
  - networking (`network_sendto_unreliable`, `network_sendto_reliable`, `network_recvfrom` with a
    network_id and port);
  - font glyph rasterisation;
  - emblem and skin drawing (`get_player_emblem` returns an `ID3D11ShaderResourceView`).
- The engine receives a launch struct listing peers and players. It does no matchmaking itself
  (sibling note `mcc-engine-hosting.md`).

**What carries over.**
- **h2live.** Almost all of it: accounts, parties, matchmaker, levels, playlists, relay, stat cards
  and agreement on results, plus its 5,340 test lines.
  - It touches h2sim only for `Look`/`Emblem`, the little-endian `Reader`/`Writer`, `GameType` and
    `clean_name` (`crates/h2live/src/{server,store,playlists}.rs`, `crates/h2net/src/live.rs`), and
    for `client::results(&h2sim::Game, …)` (`crates/h2live/src/client.rs:144`).
  - Moving those few hundred lines into a small shared protocol crate lets the server build
    without the game.
  - `server/tests/matches.rs` (1,861) plays real games with h2sim bots over the relay. It can keep
    h2sim as a dev-dependency, as a "fake engine".
- **Party and match flow.** The flow MATCH → HOST_MATCH → HOSTING → LINK → GO → RESULT →
  MATCH_OVER fits the engine's launch model: the launcher builds the peer list from MatchInfo and
  the LINKs. That fit is my inference.
- **Host choice.** h2live already picks the host by round-trip time (`matchmaker.rs`).

**What needs adapting.**
- **Relay.** It is opaque, but it carries kind+body messages over WebSocket/TCP
  (`crates/h2live/src/server/relay.rs`). The engine sends datagrams, some marked unreliable. TCP
  head-of-line blocking would hurt, so add a UDP or unreliable relay mode, keyed by the same
  tokens.
- **MatchInfo.** It carries our `GameType` enum, a preset name, score, time limit and bots. It
  needs a reference to an MCC game variant and map variant instead. HaloX reads
  `halo2\hopper_game_variants\*.bin`; per the sibling note, the same holds for
  `hopper_map_variants`.
- **Engine build.** LOGIN should also carry the engine build or hash, because Reclaimer pins each
  release to one engine version.
- **Results.** RESULT (account, team, place, score, kills, deaths, left) is engine-agnostic. The
  launcher must fill it from the engine. libmcc's `s_game_result` is an opaque 0x5D138-byte
  buffer, so this is reverse-engineering work.
- **Input** (input.rs, plus the controller-wip branch).
  - Kept: XInput through gilrs, pad ownership across windows, hot-plug handling, the hold-off mask
    and the `H2_PAD_SCRIPT` fake pad. They feed the host's gamepad callback.
  - The Halo 2 dead-zone, square-stick and look-acceleration maths from the decompilation is
    probably done by the engine itself, so it is dropped. This is an inference: the profile struct
    has look dead zones, look acceleration and button and stick presets (libmcc players.h).
  - Bumper Jumper and Recon become button mappings, or a remap applied before the state is handed
    over.
- **Rumble.** The XInput output carries over. The jpt! mixer is dropped, because the engine
  computes motor speeds.
- **Profile** (profile.rs) maps onto MCC's player-profile struct: colours, Elite, emblem,
  inversion, sensitivities.
- **Menus and the online screens.** Their logic and layout carry over, but the renderer must
  change.
  - The engine draws into a D3D11 swap chain that the host creates. wgpu cannot draw into a
    foreign D3D11 swap chain, so the pregame lobby and the carnage report (drawn over or between
    engine frames) need a small D3D11 overlay renderer.
  - The alternative is to draw menus in our own wgpu window only while the engine is not running.
  - menuart.rs (986, branch) produces quads, so it can be retargeted.
  - Project Reclaimer draws its own menus with OpenGL 3.0 (https://projectreclaimer.dev/play.html,
    cited in the sibling note).
- **Halo 2's own menus.** Opus patched halo2.dll's map table so it booted
  `scenarios\ui\mainmenu\mainmenu`, with per-build offsets for 11 MCC builds (1477-1955)
  (https://github.com/ChimpsAtSea/Blam-Creation-Suite/commit/797dbf473b1b97e3dcd38daa4df0237fd84c9638,
  GPL-3). Whether Halo 2's own Live and matchmaking screens still work inside halo2.dll is
  unknown. If they do, the engine could draw the original pregame lobby itself.

**What is dropped.**
- All of h2sim at runtime.
- h2net LAN game sync.
- In h2viewer: rendering, HUD, local player, audio, campaign and camera (about 18,000 lines).
- wma.
- Most of blam-cache at runtime. Keep it for h2tool and for the launcher's map list, pictures and
  rank icons, once it reads MCC maps.

**New work.**
- A Rust host for C++ vtables: about 60 or more callbacks, a launch struct of 0x2BF30 bytes, and
  per-build offsets.
- A network bridge and a way to extract results.
- An overlay UI.
- Testing needs Windows with MCC installed, so CI cannot run it.
- No public project has networked halo2.dll multiplayer outside MCC (sibling note).

### C. Mod or plugin inside MCC

- **EAC.** Mods need Easy Anti-Cheat off. With it off, "the public matchmaking features will be
  unavailable", while "you can modify game files and content, then play online through Custom
  Games"
  (https://support.halowaypoint.com/hc/en-us/articles/360037475251-How-to-Launch-Halo-The-Master-Chief-Collection-with-Easy-Anti-Cheat-EAC-Disabled,
  fetched).
- **So our matchmaking would sit beside MCC's custom games.**
  - The h2live server and client carry over as an outside matchmaker and rank service.
  - The match must then become "the host opens an MCC custom game with this variant, the others
    join it" through MCC's own UE4 shell and network layer. How to drive that from a mod is
    unknown.
  - Our relay is dropped, because MCC's custom games use their own networking.
- **UI and controls.** The UI is an overlay inside MCC's D3D11 frame. MCC's Unreal menus stay.
  Input and controllers are MCC's own, so ours is dropped.
- **Precedent.** AlphaRing, an MCC mod DLL, was tied to MCC versions and was archived in May 2026
  (https://github.com/WinterSquire/AlphaRing, cited in the sibling note).
- **What survives.** Only h2live, h2net::live and the transport, plus a slice of the menu logic.
  Everything else is dropped.

### D. Original Xbox Halo 2 under an emulator, plus a revival service

- **The original client matches itself.**
  - It searches sessions and scores them by ping, language, skill and player count.
  - It computes per-opponent XP and writes XP and levels to Live stats.
  - Source: the decompilation drafts in `docs/notes/decomp/online-unverified.md` sections 1 and 5
    (draft functions, unverified).
  - So a revival service is an **Xbox Live server reimplementation**: Kerberos-based auth with a
    Secure Gateway, matchmaking, statistics and arbitration (https://xboxdevwiki.net/Xbox_Live).
- **That already exists.** Insignia is closed source, "created via closed-source reverse
  engineering of the original Live server software", and works with xemu
  (https://en.wikipedia.org/wiki/Insignia_(Xbox)). Halo 2 has run on it since March 2024:
  - ranked and unranked playlists, parties, clans and custom games (https://insignia.live/halo2);
  - its leaderboard "is not ranked in-game" (same page).
- **xemu** needs BIOS and MCPX dumps from the user's own Xbox (https://xemu.app/docs/required-files/,
  cited in the sibling note).
- **Reuse is near zero.**
  - levels.rs and online-unverified.md remain useful as references and for validating levels.
  - Hopper and playlist data would need serving in the game's `network_configuration` and hopper
    binary layouts (16 × 0x614 records per the decomp notes).
  - Every Rust game crate, h2net, the relay and the matchmaker are dropped.
- **It also ignores John's request** to use the MCC files, and it duplicates Insignia.

## 3. What h2live needs before a non-Rust client can talk to it

The protocol as it is today, from the code:

- **Transport.** WebSocket, `ws://` or `wss://` (rustls), on one port (47050).
  - `/live` is the control link and `/link` is a relay leg.
  - Plain HTTP `GET /` returns a status page and `/health` returns `OK`
    (`crates/h2live/src/main.rs:45,255,295`).
  - Each WebSocket binary message is one byte of message kind followed by the body
    (`crates/h2net/src/conn.rs:259-264,487`).
- **Encoding.**
  - Integers, f32 and f64 are little-endian. A string is a u16 length followed by UTF-8 bytes. A
    bool is one byte.
  - `Look` is 8 bytes: elite, two armour colours, emblem foreground, emblem background, three
    emblem colours (`crates/h2sim/src/game/sync.rs:28-140`, `crates/h2sim/src/game.rs:144-200`).
  - Limits: 64 KiB per control message, 64-byte names, 8 KiB stat cards
    (`crates/h2net/src/live.rs:25-60`).
- **Sign-in.**
  1. LOGIN starts with the magic `H2LV` and the u32 `PROTOCOL`, followed by the Ed25519 public
     key, gamertag, look, stat card, maps (name and FNV-1a hash of the first 2 KiB plus the
     length) and guest count.
  2. The server answers with CHALLENGE (a 32-byte nonce and the message of the day).
  3. The client sends PROVE: an Ed25519 signature over `"h2live-login" || nonce || key`.
  4. Both sides then ping every second and give up after 15 seconds of silence
    (`crates/h2net/src/live.rs:22,468-500`).
  5. A version mismatch is refused with "UPDATE YOUR GAME" (`crates/h2live/src/server.rs:406`).
- **Message kinds.** 1-42 from client to server and 101-115 from server to client
  (`crates/h2net/src/live.rs` module `kind`).

To be practical for a C++ or C# engine host, an MCC overlay, or a web stats site, it needs:

1. **A separate version number for the control link.** `PROTOCOL` (`crates/h2net/src/lib.rs:37`)
   is shared with the LAN game sync, so every LAN or snapshot change (it is 26 on main and 27 on
   menu-preview) locks out every other client. Split them, and give the control link a
   version-and-capabilities handshake.
2. **A written spec plus golden byte vectors** for every message, generated from the Rust
   encoders. `live.rs` already has 414 lines of round-trip tests to build on. Alternatively, move
   the low-volume control link to a self-describing format (protobuf or JSON). Leave relay traffic
   binary.
3. **A protocol crate with no game in it.** Move `Look`, `Emblem`, `Reader`/`Writer`, `GameType`
   and `clean_name` out of h2sim, so h2live, the protocol and a C ABI (`cdylib`) build without the
   simulation. A cdylib wrapping h2live::client lets a C++ host reuse the Rust client rather than
   re-implement it. If the launcher is written in Rust, as Project Reclaimer is, nothing needs
   re-implementing.
4. **Identifiers a real engine can use.**
   - An engine build or hash in LOGIN.
   - Game and map variant references (file name and hash, or the blob) in PlaylistInfo and
     MatchInfo, instead of our preset names.
   - Account ids already fit XUID-style u64s.
5. **Datagram relay legs.** Either UDP with token auth, or WebSocket frames marked droppable, for
   engine traffic sent unreliably. Map relay ends to the engine's `network_id`. The current
   five-second QUIET rule is fine for an engine that sends constantly.
6. **Richer RESULT, as an optional extension.** Medals, per-weapon kills and assists for a
   Halo 2-style carnage report and web stats. Today RESULT has only team, place, score, kills,
   deaths and left.
7. **A read-only HTTP JSON API** (players, levels per playlist, recent games from `games.log`) for a
   Bungie.net-style stats page. Today there is only `/` and `/health`.

## 4. Unknowns

- Whether MCC's halo2.dll still contains working Halo 2 Live, pregame-lobby and matchmaking UI
  screens.
- The current halo2.dll host-interface layout, and how results come out of it (libmcc dates from
  Nov 2025; the Opus tables stop at build 1955).
- Whether retail MCC classic multiplayer sounds are Opus, ADPCM or WMA. This decides whether the
  wma crate survives pivot A.
- Whether MCC's Halo 2 mainmenu.map has the classic UI tags, and the format of the
  `halo2\h2_fonts` glyph files.
- Whether the engine applies Halo 2's own stick processing to the raw pad state. This is inferred,
  not checked.
- How a mod could create or join MCC custom games programmatically (pivot C).
- What the MCC EULA's "Competing Service" section means for a standalone launcher. The FAQ says
  "Anything contained within MCC is allowed to be used for modding efforts" and that selling is
  not allowed (https://www.halowaypoint.com/news/mccs-eula-the-faq, fetched). It does not address
  standalone launchers.
