# Progress and handoff

Last updated 2026-10-12 (menu 2.5: the flythrough behind the start
screen and main menu; before it 2.4, the menu scene as data in the new
`crates/h2scene`, and its review fixes, rounds 1 and 2; P3 passed). Any assistant that works on the
project should update this file before it stops, so the next one can pick
up. Read AGENTS.md first for the rules.

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

## Resume here (2026-10-11 02:55 UTC)

The owner's MVP (2026-10-10 21:56): matchmaking with the 1-50 levels,
custom games, a main menu like the original Xbox Halo 2's, and a build
friends can install to matchmake together. No campaign. Controller only,
Default and Bumper Jumper layouts only (keyboard, mouse and the other
layouts after the MVP). Every task should serve the MVP.

Branches, each a draft PR on the one before (all pushed, CI green):
`launcher-lobby` (#4), `launcher-friends` (#5, live protocol 3, what the
Proxmox server runs), `launcher-controls` (#6, the controller fix, head
320dd85), `launcher-menu` (#7, the menus, with phase 1 since 2026-10-11 02:50).
`launcher-menu-phase1-wip` is an older snapshot of phase 1 and can go.

Done 2026-10-11: menu phase 1 (the lobby opens on the start screen, then a
main menu with ONLINE and SETTINGS; B goes back; no sign-in until ONLINE
is picked) and the lobby window's redraw fix (326a5bf: 30 frames a
second instead of non-stop; on the owner's PC 99% of a core before, 10% on
the menus and 4% during a game after, engine still 60 fps). Both are tested
on the owner's PC through h2.mohnjorrow.com and are in the desktop build
(`C:\Games\Halo 2 Live`, shortcut "Halo 2 Live", which stays on
192.168.8.102) and the friends' zip (`C:\Games\H2-Live-friends.zip`,
launcher only, defaults to h2.mohnjorrow.com). Test accounts FRIENDTEST and
MENUTEST stay on the server. Not yet seen: the owner pressing the pad in the
real window.

Next, in order:
1. Menu phase 2, widened (`docs/notes/launcher/menu.md`, "Phase 2
   (widened, 2026-10-11)": 16 sub-phases, Cartographer's files first) on
   `launcher-menu2`, stacked on `launcher-menu`. 2.1 is done
   (2026-10-11): one finder for the Halo 2 Vista (Cartographer) folder,
   which the menus, fonts and rank icons now try before MCC
   (`H2LOBBY_VISTA`, `vista =` in lobby.txt); every screen's frame time
   in `lobby.log`; and the flows test (`crates/h2launch/tests/flows.rs`),
   real headless lobbies against an in-process h2live in the gate, which
   every later sub-phase keeps passing. On the owner's PC (check P1):
   `lobby.log` should name the Cartographer folder for menus, fonts and
   rank icons, MCC's with `H2LOBBY_VISTA=off`, and give a ms line for
   each screen kept on show for about a second (30 frames at that window
   size; a short one such as signing in may have none). 2.2 is done
   (2026-10-11): Halo 2's menu music and menu sounds from Cartographer's
   `mainmenu.map` (`lobby/sound.rs`, `lobby/audio.rs` on cpal; menu.md
   "What Phase 2.2 built"): music on the start screen and main menu,
   faded on the playlists, party, custom game and match screens (Halo 2's
   rule; the settings keep it; at most one start or stop every 6 s, each
   start from the beginning, each stop over its track's 5 s fade-out);
   cursor, select, back, error, advance, transport and message sounds
   from `App::take_cues`, at their tags' gains; silent and paused while
   an engine runs; silent without Cartographer (`H2LOBBY_SOUND=off`,
   `menu_sound = off`). Review round 1 fixed the music's timing, the
   gains, the start screen's transport sound, friend requests and
   presses that only ask the server sounding, and a re-sent invitation
   sounding again. Review round 2: a screen or popup opening sounds
   select then advance, as Halo 2 does (from the decompilation; the
   pregame lobby and carnage report advance as they open, a custom game
   asked for is select alone), and a music fade still running when an
   engine starts no longer plays on after the match (every voice fades,
   then stops, before the pause). Review round 3: the music is never
   dropped to make room for menu sounds (a held Esc could leave the menus
   silent), only A sounds select before a screen's advance (X, Y and
   the bumpers opening one give advance alone, as Halo 2's Y menu
   does), and the playlists after signing in and the failure screen
   advance as they open. On the owner's PC (check P2): music on
   start and main from Cartographer's files, fading on the playlists;
   the sounds with the pad and the keyboard; silence in a match and the
   music again on the main menu after; with `H2LOBBY_VISTA=off`, silence
   and no error. **Before the next friends' zip** (the first with 2.2):
   `h2launch.exe` now contains `crates/wma` (ported from FFmpeg, LGPL 2.1
   or later), so the zip's READ ME must name it and the zip must carry
   `crates/wma/LICENSE` and the crate's source or a written offer of it
   (and of h2launch's for relinking); menu.md "What Phase 2.2 built",
   LGPL. P2 passed on the owner's PC; its follow-ups (menu.md "After
   P2"): the log names the audio device and each music and engine change,
   and the menu sounds and music now take their sound class's gain (-4 dB)
   with the mix no longer scaled by 0.55 (1.2 dB louder: the tags' level;
   the music's first seconds are quiet in the file itself). 2.3 is done
   (2026-10-11; menu.md "What Phase 2.3 built"): with a window on
   Windows the lobby opens on Halo 2's intro movie from Cartographer's
   `movie` folder (`lobby/intro.rs`, Media Foundation, ported from
   `menu-preview`), with its soundtrack at its own level (gain 1, as
   Halo 2 plays its movies); any button (the triggers too) or a click
   skips it (not a direction), and at its end, a skip, a 5 s stall or a
   failure the start screen comes in with its music.
   `H2LOBBY_INTRO=off` or `intro = off` in lobby.txt skips it; headless
   and Linux never play it. `App::take_cues` became `take_sound`
   (`SoundOut`: cues, the soundtrack and its stop), and the window draws
   every 16 ms while it plays. Review fixes, round 1 (2026-10-12):
   Media Foundation's DLLs are loaded when the intro plays instead of
   linked (linked, an N edition without the Media Feature Pack couldn't
   start the launcher at all), and the gate checks the exe imports
   neither; the soundtrack at gain 1 rather than the music's 0.32; the
   triggers skip it; the intro's timer counts only draws of a new
   picture. P3 passed on the owner's PC (2026-10-12, on fcfb9e1; menu.md
   "After P3"), driven by keys and clicks posted to a 1600x900 window,
   so no pad button (A among them) was tried and the Windows N check
   was skipped: the intro played with its soundtrack (18.0 s at gain 1,
   louder than the menu music); Enter, Esc, Space, letters, Backspace
   and a click skipped it and the arrows and Tab didn't; Enter or Esc
   held gave one skip and no Quit popup; played out it logged 540
   pictures, 1 late (2 in another run); the intro took 7.9-8.2 ms a
   frame on the CPU at 1600x900 (the 1080p figure is still to measure,
   with P4) and the start screen 2.1 ms; each `intro: none` case opened
   on the start screen. It found that the windowed lobby started from
   its own console exited with code 101 in about 1 launch in 7 (a
   `println!` panicking as it raced `FreeConsole`); 18279ad on
   `launcher-menu` fixed it (0 failures in 30 launches after), merged
   here as 81b2e07. Review fixes, round 2
   (2026-10-12): a key held down repeats in the lobby only as a
   direction or typing (`App::key_repeated`), so Enter held to skip the
   intro no longer goes on to the main menu and signs in; the README
   says the launcher opens on the intro; menu.md now has the full P3
   list ("On the owner's PC (P3)"). 2.4 is done (2026-10-12; menu.md
   "What Phase 2.4 built"): a new crate, `crates/h2scene`, reads
   Cartographer's `mainmenu.map` scene (the level, sky, fog, scenery and
   the flythrough's camera path, ported from `menu-preview`'s h2viewer)
   into plain data on any thread with no GPU; `blam-cache` gained the
   sky's fog (`model::sky_atmosphere`) and the menu's shader reading
   (`shader::read_menu_shader`). Nothing visible changes. In the
   container: 38,738 level triangles (45,919 with the scenery and sky),
   39 textures, 40 materials, 12 of 18 camera points flown through in a
   2,990-tick loop, read in about 110 ms (1.9 s from a cold disk). MCC's
   format-13 maps are refused ("no geometry reader yet"). On the owner's
   PC (with P4): `cargo run --release -p h2scene --example scene_info --
   "C:\Games\Halo 2 Project Cartographer\maps\mainmenu.map"` should
   print the same counts. Review fixes, round 1 (2026-10-12): the
   camera points (and placed scenery) now turn as Halo 2's `camera_set`
   turns them, the roll last about the world's x (the port banked the
   camera up to 45 degrees and pointed it at the sky; now at most 13
   over the loop), and the camera follows Halo 2's fifth-order curve,
   which keeps its acceleration through each cut; a damaged map's far
   vertex or looping script can no longer take the process down; the
   real-map test pins every count exactly. Review fixes, round 2
   (2026-10-12): a lightmap page that can't be decoded is read and
   counted once, not once for each section that uses it; a `camera_set`
   to no point (-1) is skipped, as Halo 2 skips it, where it flew to the
   first point; the old roll notes in `docs/notes/mainmenu-plan.md` are
   marked superseded. 2.5 is done (2026-10-12; menu.md "What Phase 2.5
   built"): with Cartographer's files the darkened navy flythrough plays
   behind the start screen and main menu, drawn by `h2scene`'s own wgpu
   renderer offscreen on a thread of its own (`render.rs`,
   `backdrop.rs`), read back and copied under the menus; it fades in
   over 0.25 s, sleeps (no GPU device at all) on every other screen and
   from the moment a match is made, and any failure leaves the still
   background with one log line. The veil (grey, multiply by a navy, add
   a little) is our estimate, fitted on lavapipe to the owner's
   Cartographer screenshot's mean colour (within 1 level); its numbers
   and the departure from the brief (a post pass that greys first) are
   in menu.md. `background = still` in `lobby.txt` or
   `H2LOBBY_SCENE=off` switches it off; `H2LOBBY_SCENE=on` allows it
   headless and on a software GPU, `H2LOBBY_VEIL` and
   `H2LOBBY_SCENE_FPS=60` are for checks. Still missing from the look:
   the gravity-lift beam, particles, engine glow, bloom; the 70-degree
   field of view is unverified. On the owner's PC: menu.md's P5 list
   (the flythrough plays, at most about 9 ms a frame on the UI thread
   at 1080p, nothing on the GPU during a match, the still background
   with `H2LOBBY_VISTA=off`, `H2LOBBY_SCENE=off` or the forced failure
   `WGPU_BACKEND=metal`). Next: 2.6, Halo 2's shell on every lobby
   screen.
2. Friends: the owner sends their public IPs; add each to the allowlist
   (below) and play a two-PC match with a friend through
   h2.mohnjorrow.com, the first test of the relay from outside.
3. A real game with kills, to confirm the kills offsets and set
   `COUNTS_SEEN` (`crates/h2launch/src/results.rs`).
4. Merge the PR stack into `main`.

The server for friends: h2.mohnjorrow.com is an A record to the owner's
public IP in Cloudflare, DNS only (the proxy would break UDP). The router
forwards TCP and UDP 47050 to 192.168.8.102. Container 102 drops port
47050 except from an allowlist: nftables, `/etc/h2live-allow.nft`, table
`inet h2live_allow`, set `allow_v4` (localhost, 192.168.8.0/24 and the
owner's public IP), included from `/etc/nftables.conf` (backups end in
`.pre-h2live-allow`). To add a friend, add the address to the set and run
`nft -f /etc/h2live-allow.nft`; to undo it all, `nft delete table inet
h2live_allow` and remove the include line. Server changes go through the
owner's PC (Remote Control) and need the owner's go-ahead typed in the
thread, naming the action.

On the owner's PC only (it can't push): the parked kills and streaks test
player is on `hunt-wip` (6822f71) in `C:\h2work\hunt`, notes in
`HUNT-NOTES.md`; kills, deaths and streaks are confirmed there, betrayals,
assists and the team offset aren't. The PC's own log of the day is
`reports\2026-10-10_mcc-pivot_pc-work-log.md` in the Cartographer folder.

After the MVP: clans, keyboard and mouse, the other layouts, and the
owner's idea (2026-10-11) of Halo 2 Anniversary weapon and Spartan models
in classic multiplayer. There's no switch for that: MCC's remastered look
is campaign only and the Anniversary multiplayer is a Halo 4-based engine,
so it means converting those models into classic tags on each player's PC
(official mod tools), keeping the classic skeletons and collision. First
step, when the owner says so: one weapon.

## In progress

- In-game GAME MENU (Start or Esc in a match, as Halo 2's screen 0xC3), on
  `launcher-gamemenu` off `launcher-menu` f0b718b. Stage 1 done
  (2026-10-11): the menu's logic, pad edges and input gate in
  `crates/h2launch/src/gamemenu/` (plain Rust, Linux-tested; no Windows,
  wire or lobby changes yet), and its opaque panel with the players column
  in `crates/h2ui/src/gamemenu.rs` (`cargo run --release -p h2ui --example
  gamemenu_png -- <out dir> [<h2_fonts dir>]` draws every page). Stage 2
  done (2026-10-11): the lobby and the engine's copy exchange the menu's
  events (`H2EVENT left game|end`, `H2EVENT controls` with the eight
  `lobby.txt` pairs; `PROTOCOL` and `LIVE_PROTOCOL` unchanged); the lobby
  saves the controls through `App::set_control`, lands a player who left
  on ONLINE (a custom game's leader on Custom) with the lobby in front,
  and passes `--game-kind`; the stand-in engine runs the real menu from
  `H2LOBBY_FAKE_MENU` (`crates/h2launch/tests/game_menu.rs`). New engine
  flags `--no-game-menu`, `--reload-message`, `--end-game-message` are
  parsed. Stage 3 done in code (2026-10-11, cross-compiled and
  clippy-clean, not yet run on the PC): the driver on the window thread
  (`win/menu.rs`), the panel copied into back buffer 0 in `end_frame`
  (`win/overlay.rs`), the input gate for player 0 (`win/input.rs`), live
  controls, and the profile and gamepad mapping served as generations
  (`win/host.rs`); what it does and the settings table are in
  `docs/notes/launcher/menu.md`, phase 4. Review fixes, round 1
  (2026-10-11): the engine's profile and mapping generations are built
  when the menu closes (not per step; each kind has its own cap of 64);
  a stick held into the menu no longer scrolls it; a pad's first Start
  opens it; `--live` takes the game kind from the server's match; runs
  without the lobby save only the settings changed in the menu; turning
  vibration on in a game started with it off says NEXT GAME. Review
  fixes, round 2 (2026-10-11): a trigger resting above 30 is let go near
  its rest; the menu's safety rules moved into `gamemenu::watch` with
  Linux tests, PLEASE WAIT survives an overlay failure, a failed panel
  publish turns the overlay off, and `H2LAUNCH_OVERLAY_FAIL=stall|copy`
  fail at run time for the new check 17; the engine says `H2EVENT menu
  off` and the lobby drops its hint; Use Default Settings is said
  explicitly (`Effect::Defaults`); the lobby saves only what the menu
  changed from the engine's own controls. Review fixes, round 3
  (2026-10-11): the players column's team colours follow Halo 2's tag
  order (red, blue, yellow, green, purple, orange, brown, pink) in the
  lobby roster's shades; Use Default Settings reaches the lobby as
  `H2EVENT controls defaults ...`, so a lobby match saves every one too;
  `H2LOBBY_ENGINE_ARGS` is read once into the lobby's `Config`, and the
  engine's start and start failure have tests; `--reload-message` and
  `--end-game-message` are documented as tried offline first, not
  refused online. PC checks 1 to 15 ran on the owner's PC on 2026-10-11
  (672bf4a; results in menu.md section 9.5): the menu draws, holds and
  releases input, applies the live settings, survives resizing and the
  game's end, and costs about 2 us a frame. The engine doesn't read a new
  profile or mapping by itself (check 7), but engine message 5 makes it
  (check 8); no engine message ends the game for everyone (check 15).
  Follow-ups from those checks (2026-10-11): the reload message 5 is the
  default (`--reload-message 0` or `off` turns it off), so Button Layout,
  Look Sensitivity and Auto Look Centering take effect when the menu
  closes and aren't tagged NEXT GAME; the controls go to the lobby once a
  visit to CONTROLLER SETTINGS (leaving it or the menu, 1 s after the
  last change, or before a quit; `gamemenu::save`), not once a step; a
  guest who left from the menu is told "You left the game." even when
  the server's verdict with no reason came first; End Game passes the
  host's quit-time result on, and the lobby sends it as LAUNCHER_RESULT
  with `finished` false, then LEFT_MATCH, so h2live ends the match with
  no HOST LEFT wording and the host gets the carnage report too (h2live
  unchanged; a new server test covers it); before the server's GO (a
  guest still joining) it goes as LEFT_MATCH alone and the host gets "You
  ended the game." instead (`Told::ended_before_go`). Next: check 14 again on the
  PC with this build (both lobbies on the carnage report), and checks 16
  and 17.
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
  start screen and main menu as draw lists on the CPU. On the owner's PC
  (2026-10-11) MCC's mainmenu.map read in full (every screen the same as
  Vista's, all 16 pictures, the fonts from `halo2\h2_fonts`), and the
  pictures came out right: 5.3 ms a frame for the main menu at 1080p on
  the CPU, the background 20 ms once (results in menu.md 9.2). Phase 1 is
  built there too (2026-10-11): the lobby opens on Halo 2's start screen,
  then the main menu (ONLINE, SETTINGS), drawn by `h2ui` from MCC's
  mainmenu.map, and signs in only when ONLINE is picked ("What Phase 1
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
