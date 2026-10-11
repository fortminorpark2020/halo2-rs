# Guide for AI assistants and contributors

Read this before changing anything. README.md describes what the game does;
this file says how the project is run.

## What this is

Since 2026-10-09 the main goal is a **standalone launcher** (`crates/h2launch`,
being built) that runs the classic Halo 2 engine (`halo2\halo2.dll`) and maps
from the owner's own Steam copy of Halo: The Master Chief Collection (MCC),
with our own server (`crates/h2live`) supplying the original Xbox Halo 2
matchmaking experience: parties, playlists, the 1-50 levels and carnage
report. See PROGRESS.md for the milestones and `docs/notes/pivot/` for why.

Before that, the project was a from-scratch Rust remake of **Halo 2
multiplayer** (no campaign) reading Halo 2 Vista / Project Cartographer `.map`
files. That code is still here and is the fallback if the launcher can't play
online. The owner has no game-dev experience, plays on Windows with keyboard
and mouse or an Xbox controller, and just runs the .exe.

## Hard rules

- **Never commit game files**: no `.map` files, fonts from `maps\fonts`,
  movies, extracted bitmaps, sounds, tag dumps or screenshots. Everything is
  read at runtime from the user's install. `.gitignore` blocks `*.map`.
- **Tests must not need game files.** CI has none. Build synthetic byte blocks
  in tests (see `crates/blam-cache/src/tests.rs`). Tests that need the real
  maps are `#[ignore]` and read the folder from `H2_MAPS`.
- **Values**: read them from the game's tags where they exist. When a value is
  an estimate, say so in a comment.
- **Decompilation reference**: https://github.com/kirklandsig/halo2-decompiled
  (CC0, a matching decompilation of the Xbox retail Halo 2, no leaked code) may
  be used to check behaviour and numbers. Re-implement in Rust; never paste
  its code. It is the Xbox build, while the maps are Halo 2 Vista.
- **MCC files and code**: never commit or upload anything from MCC
  (halo2.dll, maps, variants, fonts, shaders) or bytes, dumps or disassembly
  taken from them. Offsets, sizes and layouts written as facts in our own words
  are fine. HaloX and libmcc (no licence) and Blam Creation Suite and
  Cartographer (GPL) may be read as references only; never paste their code.
- **Keep the repository private and the project free.** MCC's licence forbids
  third-party matchmaking and using its code outside MCC
  (`docs/notes/pivot/legal.md`). No donations, no "Halo" in a public name.
- The launcher can only be run on the owner's PC (CI and containers have no
  MCC). Keep it compiling on Linux with a stub `main` so the workspace gate
  still passes there.
- `crates/wma` is ported from FFmpeg and is LGPL 2.1 or later; keep it a
  separate crate.

## Layout

- `crates/blam-cache`: `.map` reader (tags, geometry, bitmaps, sounds, UI tags, fonts).
- `crates/h2tool`: command-line inspector (`h2tool <map> ...`) for tags.
- `crates/h2sim`: game simulation with no rendering: movement, collision,
  weapons, damage, vehicles, every game type, bots and their navigation.
- `crates/h2net`: LAN play and the wire protocol. `PROTOCOL` in
  `crates/h2net/src/lib.rs` must be bumped whenever the wire format changes,
  and the game and the online server must use the same number.
- `crates/h2live`: the online matchmaking server (levels 1-50, playlists,
  parties). Deployed to a Linux container; see `deploy/proxmox/` and `docs/ONLINE.md`.
- `crates/h2relay`: the UDP relay that carries halo2.dll's packets between
  launchers (client for h2launch, server run inside h2live on UDP 47050, and
  a standalone `h2relay` program). Its wire format has its own
  `RELAY_PROTOCOL` (in `src/frame.rs`), separate from h2net's `PROTOCOL`:
  bump it whenever the relay's frames change. h2live's relay admits only
  rooms it issues (`RelayHandle::issue`, with a `member_key` per player);
  the standalone one has open rooms and is for tests on a LAN only.
- `crates/h2viewer`: the from-scratch game (wgpu + winit): menus, HUD,
  rendering, input (keyboard, mouse, XInput controllers through gilrs),
  splitscreen, LAN and online clients.
- `crates/h2launch` (new, Windows only): the launcher that hosts MCC's
  halo2.dll. Design notes in `docs/notes/launcher/`.
- `crates/h2ui`: the launcher's Halo 2 menus as draw lists (layout,
  animations, text, pictures, CPU backend). Plan in
  `docs/notes/launcher/menu.md`.
- `docs/notes/`: design notes and research behind larger features (controller
  layouts, the main menu, the decompilation findings). Paths there that start
  with `$SP` or `scratchpad` refer to a scratch folder that no longer exists.

## Build and test

Linux needs `libudev-dev` and `libasound2-dev`. Before every commit:

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
```

Windows exe: `cargo build --release -p h2viewer -p h2live` on Windows, or
cross-compile from Linux with
`cargo build --release --target x86_64-pc-windows-gnu -p h2viewer -p h2live`
(mingw-w64). CI (`.github/workflows/build.yml`) runs clippy and the tests on
Ubuntu and uploads the Windows exes as the `halo2-rs-windows` artifact.

## Running

`h2viewer.exe` finds `lockout.map` in `C:\Games\Halo 2 Project Cartographer\maps`,
the standard Halo 2 Vista folders, or `.\maps`; or pass a map path as the first
argument. Useful environment variables for testing are listed in README.md
(for example `H2_MUTE=1`, `H2_PROFILE=<file>`, `H2_BOTS`, `H2_SIM`, `H2_PADS`,
`H2_NET_LAG`). Headless rendering on Linux works with Xvfb and Mesa's lavapipe:
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json WGPU_BACKEND=vulkan`.

## Working alongside others

Several assistants may work on this repository. To avoid clobbering each other:

- Branch from `main`, keep changes focused, and open a pull request. Never
  force-push or rewrite `main`.
- Check open branches and pull requests first. The from-scratch engine's
  branches (`controller-wip` with draft PR #1, `menu-preview`, `feel-weapons`,
  `feel-combat`, `feel-bots`) are parked until the launcher's go/no-go; don't
  merge them without the owner asking. Launcher work goes on its own branches
  off `main`.
- Update README.md in plain words for anything a player would notice.
