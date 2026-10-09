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

## In progress (branches, being merged into `main`)

- `controller-wip`: everything a controller-only player needs. On-screen
  keyboard for gamertags, presses that close a menu no longer act in the game,
  vibration from the game's tags, Automatic Look Centering, Dual Wield
  Inversion, Use Default Settings, reconnect dialog, menu auto-repeat, guest
  limits, Warthog e-brake, and a fake-pad hook (`H2_PAD_SCRIPT`) for testing
  without a real pad. Values are being checked against the decompilation.
  Next: finish the fake-pad play sessions, review, fix, merge, ship.
- `menu-preview`: Halo 2's main menu from `mainmenu.map` (UI tags, Halo 2's
  fonts from `maps\fonts`), the intro movie (Windows Media Foundation), the
  start screen and the flythrough, Halo 2 dialogs, scoreboard and carnage
  report. Finished and checked; being merged with `main` and the feel branches.
- `feel-weapons`, `feel-combat`, `feel-bots`: shotgun shell reloads, view
  kick, BXR, lift fixes, better bots. Reviewed and rechecked. They set the
  network `PROTOCOL` to 27, so the online server must be updated when they land.

## Next steps

1. Merge the branches above into `main`, run the gate, build the Windows exe
   and give it to the owner.
2. Update the h2live server on the Proxmox host to the new protocol (steps in
   `deploy/proxmox/` and `docs/ONLINE.md`).
3. Apply the confirmed findings from the decompilation research
   (`docs/notes/`) for movement, damage, game rules, menus and online play.
4. Owner's own steps when he wants friends online: router port forward and an
   address reservation for the server.

## Where things are

- Code: https://github.com/fortminorpark2020/halo2-rs (private).
- Design notes: `docs/notes/` (controller plan, main menu plan, decompilation findings).
- Online server: `crates/h2live`, `deploy/proxmox/`, `docs/ONLINE.md`.
- The owner runs builds from `C:\Games\Halo 2 Project Cartographer\halo2-rs`.
