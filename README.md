# halo2-rs

A from-scratch Rust recreation of Halo 2, loading the original game's assets
(`.map` cache files) at runtime from an existing Halo 2 PC / Project Cartographer
install. No game files are included in this repository.

## Crates

- `blam-cache`: reads Halo 2 PC (Vista) `.map` files: header, string ids, tag
  names, tag groups, tag data, shared-map lookups, level collision and render
  geometry, shaders and bitmaps (DXT and uncompressed formats decoded to RGBA).
- `h2tool`: command-line inspector.
- `h2viewer`: fly around a level with its real geometry and textures (simple lighting; lightmaps not yet).

### h2viewer

Double-click `h2viewer.exe` to open Lockout from `C:\Games\Halo 2 Project Cartographer\maps`
(or drag any `.map` file onto it). Click in the window to look around with the mouse,
WASD to fly, Space / C to go up / down, hold Shift to go fast, Esc to release the mouse,
Esc again to quit.

```
h2tool scan  "C:\Games\Halo 2 Project Cartographer\maps"
h2tool info  lockout.map
h2tool tags  lockout.map sbsp
h2tool check lockout.map
h2tool obj    lockout.map lockout.obj   # export level collision geometry
h2tool render lockout.map lockout.png   # software-rendered preview, no GPU needed
h2tool level  lockout.map [texdir]      # render geometry, shaders, textures (optionally dumped as PNGs)
```

Multiplayer maps only store the tags unique to them; the rest live in
`shared.map` (and `single_player_shared.map` for campaign).

## Roadmap

1. Map file reader (done)
2. Fly-camera level viewer (textured geometry done; lightmaps, scenery and skybox next)
3. Spartan movement, collision, weapons
4. Splitscreen and LAN multiplayer
5. Matchmaking, 1-50 ranks, parties
6. Campaign and AI

## Building

Windows builds are produced by GitHub Actions (`build` workflow artifacts).
Locally: `cargo build --release`.
