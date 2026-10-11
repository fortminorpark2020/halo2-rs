# Progress and handoff

Last updated 2026-10-11 00:50 UTC. Any assistant that works on the project
should update this file before it stops, so the next one can pick up. Read
AGENTS.md first for the rules.

## Goal (changed 2026-10-09 23:12)

The owner's words: "a standalone launcher that uses MCC halo 2 assets and
systems where applicable to re-create the halo 2 matchmaking experience from
the original halo 2 xbox version. That is my biggest end goal."

So the project is now a launcher, in the spirit of Project Reclaimer for
Halo 3: our Rust program (`crates/h2launch`, being built) loads the classic
Halo 2 engine (`halo2\halo2.dll`) and maps from the owner's own Steam copy of
Halo: The Master Chief Collection (MCC) and runs matches on that real engine.
Our own server (`crates/h2live`) supplies what MCC and Cartographer lack:
Halo 2 Xbox-style parties, playlists, the 1-50 levels and level ranges, and the
post-game carnage report. The reasoning, options and legal caveats are in
`docs/notes/pivot/pivot-memo.md`.

Milestones:
1. From our launcher, start an offline Slayer match on Lockout in halo2.dll.
2. Go/no-go: two launchers on two PCs in one online match, with the game
   traffic carried through our relay. If this can't work, the fallback is to
   teach our own Rust engine (below) to read MCC's Halo 2 maps (format v13).
3. Matchmaking from h2live: parties, playlists, levels, match results to XP.
4. Halo 2-style lobby, rank screens and carnage report.

## Decisions the owner made

- 2026-10-09 23:12: the MCC launcher above is the main path. The earlier
  from-scratch Rust remake is reused where it helps (h2live, protocol, menus).
- **No campaign.** Don't propose campaign work.
- Game files stay on the owner's PC and are never committed or uploaded. That
  now includes everything from MCC: halo2.dll, maps, variants, fonts, and any
  bytes or disassembly taken from them.
- The matchmaking server (h2live) runs on the owner's Proxmox home server
  (container 102, 192.168.8.102, TCP 47050). He confirmed this again on
  2026-10-09.
- Controllers: Halo 2's button and thumbstick layouts, plus Bumper Jumper and
  Recon (not in the original, added on request).

## The owner's PC (where all launcher testing happens)

- MCC on Steam: `C:\Program Files (x86)\Steam\steamapps\common\Halo The Master Chief Collection`,
  build 2025.08.16.178512.1. Halo 2 installed 2026-10-09: `halo2\halo2.dll`
  1.3528.0.0 (SHA-256 DE65B4F4...171946, PE timestamp 0x68A0F0F2), maps in
  `halo2\h2_maps_win64_dx11` (cache format 13, includes mainmenu.map and
  shared.map), fonts in `halo2\h2_fonts`, variants in `halo2\game_variants`
  and `halo2\hopper_game_variants`.
- Rust 1.99 (MSVC target) and Visual Studio Build Tools 2022 are installed, so
  the launcher is built there with `cargo build --release -p h2launch` from the
  clone at `C:\Halo 2 Rust Project`. That clone had an unpushed local commit
  5301293 "Controllers: review fixes" on `controller-wip` (2026-10-09 19:33 UTC).
- Containers and CI can't hold MCC files, so the engine can only be run there.

## Done (on `main`)

1. Map reader for Halo 2 Vista maps (`blam-cache`, `h2tool`).
2. A from-scratch Halo 2 multiplayer game (`h2viewer`, `h2sim`): levels,
   Spartans and Elites, weapons, vehicles, every game type, splitscreen, System
   Link, bots, sound, announcer, profiles, XInput controllers.
3. Online: the h2live matchmaking server (levels 1-50, playlists, parties, host
   choice, relay, lag prediction), live on the owner's Proxmox server at
   `PROTOCOL` 26.
4. Pivot research (`docs/notes/pivot/`).

## Parked branches (all pushed; stopped 2026-10-09 ~19:00 UTC)

These belong to the from-scratch engine. They are paused until the launcher's
go/no-go; if it fails they are the fallback, and their state is exact:

- `controller-wip` (draft PR #1, based on `main` fcec0f4): gate passes on
  a447980. Left: fake-pad play sessions on Xvfb, README controller section,
  full review, merge.
- `menu-preview` (896a83d): Halo 2 main menu, intro movie, flythrough, game
  flow (dialogs, scoreboard, carnage report; `PROTOCOL` 27). Its lobby,
  carnage report and menu logic may move into the launcher.
- `feel-weapons`, `feel-combat`, `feel-bots`: reviewed feel fixes, unmerged.
  Merge order and expected conflicts were: feel-weapons, feel-combat,
  feel-bots, then controller-wip, into menu-preview (keep one
  `camera_field_of_view`, the one that falls back to 70 degrees).
- Hold the Proxmox server's update to `PROTOCOL` 27 until the launcher's
  client protocol is settled.

## In progress

- Milestone 1 is done (2026-10-10 02:50 UTC): `h2launch` started MCC's
  halo2.dll on the owner's PC and played offline Slayer on Lockout (HUD,
  60 fps, mouse and controller). Code is on `h2launch-m1`, then
  `h2launch-m2`, in draft PR #2 (it also carries `relay`: the UDP relay
  h2live runs on UDP 47050 for the engine's packets).
- Milestone 2's engine side is done (2026-10-10, owner's PC, launchers on
  one PC through a local h2relay; h2launch-m2 0385192): two and three
  launchers play one match; a timed match ends by itself on every PC;
  leaving works, and a new launcher for the same machine joins a match in
  progress. What it took and the engine facts (own words) are in
  `docs/notes/launcher/README.md`: the host alone gets options flag 0x08
  (the others search and join, system-link style), the launcher forwards
  the guest's self-addressed search on port 1002 to every launcher, and
  each machine writes its own id at 0x58. Left for the go/no-go: two
  separate PCs, which needs a relay reachable from outside (h2live on the
  Proxmox server, below).
- Design rule kept so far: no patches, hooks, byte writes or calls by RVA
  into halo2.dll; read-only diagnostics (`--diag`, `--watch`) only. If the
  only way forward needs more, ask the owner first.
- Milestone 3 is on `launcher-live` (h2launch-m2 plus the merged
  `live-launcher` WIP): h2live speaks `LIVE_PROTOCOL` 1 to launchers (2 on
  `launcher-lobby`, which adds custom games),
  has launcher playlists 10-17, issues a relay room and key per match and
  sends LAUNCHER_MATCH; `h2launch --live <server>` signs in, searches,
  writes the session from that match, and tells the server HOSTING,
  JOINED, LAUNCHER_RESULT and LEFT_MATCH. A local h2live on the owner's PC
  (`H2LIVE_BIND=127.0.0.1`) matched two launchers and they played
  (2026-10-10 06:00). LAUNCHER_RESULT carries each player's team, standing,
  score and deaths from the engine's results block (host slot 6, 0x5D138
  bytes; layout in `crates/h2launch/src/results.rs`). Next: kills,
  assists and betrayals in that block (needs a game with real kills).
  The launcher playlists play MCC's own matchmaking variants, chosen from
  the `h2launch --variants` list; Team Snipers (opposite teams) and a
  three-player Rumble Pit worked (2026-10-10 06:30).
- The Proxmox server (container 102) runs h2live from `launcher-friends`
  af23128 since 2026-10-10 20:30 (`PROTOCOL` 26 for the Rust game,
  `LIVE_PROTOCOL` 3 for launchers: friends, service records, kills). The
  rollback copies are `/opt/h2live/h2live.prev` (`launcher-lobby` 214ee4b,
  protocol 2) and `/var/lib/h2live.v2-backup` (the data before: the new
  server writes longer account lines the old one can't read, so a rollback
  restores both). The relay is on UDP 47050. On the owner's PC, two lobbies
  made friends, partied from the friends list, and played a custom Team
  Slayer on Lockout (unranked) and a Head to Head match (counted) through
  it (`--live 192.168.8.102`). Players outside his network need
  TCP and UDP 47050 forwarded to 192.168.8.102 in his router (his step;
  UPnP gets no answer). The binary goes over as a static musl build on a
  temporary orphan branch `deploy-h2live`, which can be deleted (the
  container proxy refuses branch deletes). The owner's PC safety check
  may block the scp/ssh install until he approves it in his own words.
- Milestone 4 (the lobby) is on `launcher-lobby`, based on
  `launcher-live`: `h2launch` with no flags opens a window of its own
  (winit, softbuffer, a system font through ab_glyph, gilrs pads) that signs
  in to h2live, shows the playlists, the party, players online and
  invitations, searches, shows the pregame lobby, runs each match's engine
  as a child (`--session <file> --events`, which prints `H2EVENT` lines),
  tells the server what the engine did, and shows the carnage report.
  `--offline` is now how to start an offline match. Design, keys and the
  headless test harness: `docs/notes/launcher/lobby.md`. Tested on Linux
  with a local h2live and two headless lobbies on the stand-in engine
  (`--fake-engine`), and on the owner's PC (07:45) with two lobbies
  playing a real match on halo2.dll through the Proxmox h2live: counted,
  carnage report on both, engines closed by themselves. Custom games
  (LAUNCHER_CUSTOM, `LIVE_PROTOCOL` 2, so the Proxmox server needs the
  new h2live): the party leader picks one of the launcher playlists'
  variants (`names::CUSTOM_GAMES`) and a map, the server makes an unranked
  match for the party with the leader hosting; tested on Linux with two
  lobbies, and on the owner's PC with the real engine against a local
  h2live (a party game and a solo game, 07:45), then through the Proxmox
  server once it was updated (17:30). A party
  screen (Y on the playlists: make leader, remove, invite only, leave;
  no protocol change) was tested on Linux with three lobbies. The lobby
  now keeps each game's results block in `%LOCALAPPDATA%\h2launch\results`
  on the owner's PC (never upload them), so the first real games with
  kills give the kills offset (Slayer: kills = score + suicides). The
  lobby's text is drawn in Halo 2's own fonts from MCC's `halo2\h2_fonts`
  (same format as Vista's; `blam_cache::font`, taken from `menu-preview`
  unchanged; scaled by capital height, digits from conduit; checked on
  the owner's PC 08:30).
- `launcher-friends` (draft PR #5, on `launcher-lobby`; design in
  `docs/notes/launcher/live-v3.md`) adds `LIVE_PROTOCOL` 3: kills,
  assists and deaths on the carnage report, a service record screen (LB),
  a friends list like Xbox Live 1.0's (RB: requests, accept, decline,
  remove, status, invite, join), and Halo 2's rank icons next to every
  level, read from MCC's own `mainmenu.map` and `textures.dat` (format 13,
  `blam_cache::mcc`, `docs/notes/launcher/mcc-maps.md`) or a Halo 2 Vista
  `mainmenu.map`, else numbers. Kills, assists and betrayals are read at
  offsets inferred from their neighbours (`results.rs`); until a real game
  with kills confirms them (`COUNTS_SEEN`), kills are sent only from
  Slayer games that bear them out, assists as 0, and the carnage report
  shows "-". Tested on Linux with three headless lobbies and on the
  owner's PC with the real engine (2026-10-10 20:00 local server, 20:30
  through Proxmox). Next: a real game with kills (to set `COUNTS_SEEN`),
  a play session by the owner, two PCs, then clans.
- `launcher-controls` (on `launcher-friends` 2427b8a, reviewed twice,
  gate passing) answers the owner's 21:45 report that no controller or
  keyboard input worked in game, and his ask for layouts like Bumper
  Jumper. The gamepad mapping (host slot 116) was all zero, every action
  on LT; it is now the button layout picked in the lobby's new Settings
  screen (CONTROLLER: Halo 2's Default, Southpaw, Boxer, Green Thumb,
  plus Bumper Jumper and Recon; thumbstick layouts Default, Southpaw,
  Legacy, Legacy Southpaw done by the launcher; look sensitivity,
  inversion, auto centering, vibration, mouse sensitivity and inversion),
  saved in `lobby.txt` and passed to every engine as flags (`--layout`
  and the rest; `crates/h2launch/src/controls.rs`). The profile's
  keyboard table (0x42C) is filled with halo2.dll's own keys (as HaloX
  read them) plus MCC's; whether the engine reads it, and whether keys
  reach it at all (focus, AttachThreadInput), is the next PC check. The
  checklist is "Controls" in `docs/notes/launcher/README.md`. Tested on the
  owner's PC with the real engine (2026-10-10 23:45): the pad works in
  matches with Default and Bumper Jumper, the lobby's choice carries into
  a lobby match, and his desktop build has it. The owner scoped the MVP to
  the controller with those two layouts (keyboard and mouse later).
- The original Xbox menus for the launcher are on `launcher-menu` (plan:
  `docs/notes/launcher/menu.md`). Phase 0 is built there: `blam_cache::ui`
  reads the UI tags of MCC's and Vista's `mainmenu.map` (`Menus::open`),
  `blam_cache::mcc` reads the format-13 pieces they need
  (`docs/notes/launcher/mcc-maps.md`), and the new `crates/h2ui` draws the
  start screen and main menu as draw lists on the CPU. Phase 1 is built
  there too (2026-10-11): the lobby opens on Halo 2's start screen, then
  the main menu (XBOX LIVE, SETTINGS), drawn by `h2ui` from MCC's
  mainmenu.map, and signs in only when XBOX LIVE is picked ("What Phase 1
  built" in menu.md). Next: the owner runs it at 1080p and reads the
  `menus:` frame-time lines in `lobby.log`; over 16 ms a frame moves the
  D3D11 backend into phase 2.
- The owner's PC clone has 3 unpushed controller commits (5301293 on
  `controller-wip`); from there, `git push origin
  5301293:refs/heads/pc-controller-review-fixes` saves them.

## Where things are

- Code: https://github.com/fortminorpark2020/halo2-rs (private; keep it private).
- Pivot memo and research: `docs/notes/pivot/`.
- Design notes for the old engine: `docs/notes/` and `docs/notes/decomp/`.
- Online server: `crates/h2live`, `deploy/proxmox/`, `docs/ONLINE.md`.
