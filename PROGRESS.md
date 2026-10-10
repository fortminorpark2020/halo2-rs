# Progress and handoff

Last updated 2026-10-10 05:30 UTC. Any assistant that works on the project
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
- Milestone 2 (two launchers in one match through the relay) is being
  tested on the owner's PC by a Remote Control session in `C:\h2work\launch`.
  What is known (static reads of halo2.dll 1.3528 there, in our own words;
  details in `docs/notes/launcher/`): the engine sends only through host
  slot 41 and receives through slot 43; port 1000 carries game links, 1002
  out-of-band traffic, and first contact must come on 1002. Options flag
  0x08 makes the launch job host a session; with it clear the engine
  searches instead and sends an 18-byte search to its own id on 1002 once
  a second. As of b602c19 the session file gives 0x08 (and 0x40, meaning
  unknown) to the host only, and the launcher forwards those self-addressed
  1002 sends to every launcher in the match. Next: see whether the host
  answers the search and the guest joins.
- Design rule kept so far: no patches, hooks, byte writes or calls by RVA
  into halo2.dll; read-only diagnostics (`--diag`, `--watch`) only. If the
  only way forward needs more, ask the owner first.
- h2live launcher support (milestone 3) is on `live-launcher` (3a330e1,
  WIP, unreviewed): `LIVE_PROTOCOL` 1 with a versioned LOGIN, launcher
  playlists, a relay room per match, LAUNCHER_MATCH, JOINED and
  LAUNCHER_RESULT. It may change once the engine's join flow is known.
- The owner's PC clone has 3 unpushed controller commits (5301293 on
  `controller-wip`); from there, `git push origin
  5301293:refs/heads/pc-controller-review-fixes` saves them.

## Where things are

- Code: https://github.com/fortminorpark2020/halo2-rs (private; keep it private).
- Pivot memo and research: `docs/notes/pivot/`.
- Design notes for the old engine: `docs/notes/` and `docs/notes/decomp/`.
- Online server: `crates/h2live`, `deploy/proxmox/`, `docs/ONLINE.md`.
