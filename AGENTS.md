# Guide for AI assistants and contributors

Read this before changing anything. README.md describes what the game does;
this file says how the project is run.

## What this is

A from-scratch Rust remake of **Halo 2 multiplayer** (no campaign) that reads
the original game's `.map` files at runtime from the owner's Halo 2 Vista /
Project Cartographer install. The owner has no game-dev experience, plays on
Windows with keyboard and mouse or an Xbox controller, and just runs the .exe.
The goal is to match Halo 2 (Xbox original) as closely as possible, with a few
requested extras (Bumper Jumper and Recon controller layouts).

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
- `crates/h2viewer`: the game itself (wgpu + winit): menus, HUD, rendering,
  input (keyboard, mouse, XInput controllers through gilrs), splitscreen, LAN
  and online clients.
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
- Check open branches and pull requests first. As of 2026-10-09 these were in
  progress: `controller-wip` (full controller support), `menu-preview` (Halo
  2's main menu, intro movie and menu art), `feel-weapons`, `feel-combat` and
  `feel-bots` (Halo 2 feel fixes). They are being merged into `main`; build
  on `main` after they land, or coordinate before touching the same files
  (`crates/h2viewer/src/menu.rs`, `input.rs`, `main.rs`, `local.rs` change a lot).
- Update README.md in plain words for anything a player would notice.
