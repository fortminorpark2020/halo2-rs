# Progress and handoff

Last updated 2026-10-09. Any assistant that works on the project should
update this file before it stops, so the next one can pick up. Read
AGENTS.md first for the rules.

## Goal

Halo 2 multiplayer on PC with the original Xbox features, including
matchmaking and the 1-50 ranks, built in Rust and reading the game's own
files from the owner's Project Cartographer install
(`C:\Games\Halo 2 Project Cartographer`). The aim is to feel exactly like
Halo 2.

## Decisions the owner made

- Re-create in Rust rather than mod the PC game, so it is easy to change and extend.
- **No campaign.** Campaign work stopped on 2026-10-04 and Campaign was taken
  off the main menu. Don't propose campaign work.
- Game files stay on the owner's PC and are never committed or uploaded.
- Controllers: Halo 2's button and thumbstick layouts, plus Bumper Jumper and
  Recon (not in the original, added on request).
- Re-create Halo 2's original main menu, its intro movie and the live 3D
  flythrough behind the menus.
- The online server (h2live) runs on the owner's Proxmox home server; it may
  move to Oracle Cloud later.
- Current priority (2026-10-09): **controller support working fully**, then
  the main menu, then the paused "Halo 2 feel" fixes.

## Done (on `main`)

1. Map reader for every tag the game needs (`blam-cache`, `h2tool`).
2. Levels with lightmaps, scenery and skies.
3. Spartan movement, collision, weapons with first-person animations, Halo 2's
   damage table, power-ups, falling damage, kill zones.
4. Splitscreen for up to four, System Link (LAN) lobby and play, every game
   type (Slayer, CTF, King of the Hill, Oddball, Juggernaut, Territories,
   Assault and team versions), vehicles, dual wield, Elites, profiles,
   emblems, game options, all 23 multiplayer maps, bots, 3D sound and the
   announcer.
5. Online: the h2live matchmaking server (levels 1-50, playlists, parties,
   host choice, lag prediction). It runs on the owner's Proxmox server; friends
   outside his home network need a router port forward (TCP 47050) that he
   still has to set up.
6. Play-test fixes from 2026-10-07/08 (BR feel, floating bots, HUD size,
   splitscreen and LAN problems), Halo 2's 70 degree view, lowered crosshair,
   aim assist, and XInput controllers with Halo 2's layouts (`fcec0f4`).

## In progress (branches on GitHub; stopped cleanly on 2026-10-09 ~19:00 UTC)

All work is pushed. Nothing is left only on a local machine.

### `controller-wip` (top priority; based on `main` fcec0f4)

Done: Halo 2's controller settings (Controller Vibration driven by the
weapons' and damage's jpt! player-response tags, Automatic Look Centering,
Dual Wield Inversion, Use Default Settings), an on-screen keyboard for
gamertags, presses that close a menu or claim a pad no longer act in the game
(hold-off mask), a reconnect dialog when a pad drops, menu auto-repeat,
guests can't quit the program, guest settings follow the pad, Boxer melee
while dual wielding, Warthog e-brake, reload vs pick-up hold, swap hold time
from the tags, and Halo 2's dead zone, square sticks, trigger hysteresis,
look acceleration, seat look rates and pitch limits from the decompilation
(`docs/notes/decomp/controller.md`). A fake pad for testing without hardware:
`H2_PAD_SCRIPT` (see README).

Left to do:
1. Play whole sessions with the fake pad on Xvfb (menus, Slayer on Lockout,
   a vehicle map, splitscreen join, Bumper Jumper) and fix what fails.
2. Finish README's controller section (partly written in the last commit).
3. Review the whole diff (`git diff fcec0f4..controller-wip`) for keyboard
   and mouse regressions, splitscreen, LAN, rumble stopping, profile.txt
   compatibility; fix; gate; merge into `main`; ship a Windows build.

### `menu-preview` (Halo 2 main menu; 896a83d)

Contains: the finished main menu (1919492: intro movie, start screen,
flythrough, menus from Halo 2's UI tags and fonts, QUIT double-click safety,
controller legends), the game-flow branch (Halo 2 dialogs, scoreboard,
carnage report; sets `PROTOCOL` 27), and a merge of `main` fcec0f4
(controllers). Left to do: check that the CONTROLLER and CONTROLLER SETTINGS
screens are drawn in the Halo 2 menu look, run the gate, then merge in order:
`feel-weapons`, `feel-combat`, `feel-bots` (all reviewed and rechecked; expect
conflicts in camera.rs, local.rs, physics.rs (keep one
`camera_field_of_view`, the one that falls back to 70 degrees) and README),
then `controller-wip` once it is done. Review, gate, then merge to `main`.

### `feel-weapons`, `feel-combat`, `feel-bots`

Shotgun shell-by-shell reload with queued shot, view kick clamp, BXR (melee
recovery called off by reload or switch), lift phantoms (Relic's watchtower
and Ascension's pad left out), bot perception, aim and navigation. Reviewed,
fixed and rechecked; waiting to be merged as above.

### Decompilation research (`docs/notes/decomp/`)

Verified findings for controller, movement and combat, damage, and game
engines. Menus/HUD and online are first-pass notes, not yet verified
(`*-unverified.md`). Apply them after the merges above.

## Next steps

1. Finish and ship `controller-wip` (see above).
2. Merge `menu-preview` + the three feel branches + controllers into `main`,
   gate, ship a Windows build.
3. Update the h2live server on the Proxmox host to `PROTOCOL` 27 (steps in
   `deploy/proxmox/` and `docs/ONLINE.md`); game and server must match.
4. Apply the confirmed decompilation findings, area by area.
5. Owner's own steps when he wants friends online: router port forward and an
   address reservation for the server.

## Where things are

- Code: https://github.com/fortminorpark2020/halo2-rs (private).
- Design notes: `docs/notes/` (controller plan, main menu plan) and `docs/notes/decomp/` (decompilation findings).
- Online server: `crates/h2live`, `deploy/proxmox/`, `docs/ONLINE.md`.
- The owner runs builds from `C:\Games\Halo 2 Project Cartographer\halo2-rs`.
