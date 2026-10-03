# halo2-rs

A from-scratch Rust recreation of Halo 2, loading the original game's assets
(`.map` cache files) at runtime from an existing Halo 2 PC / Project Cartographer
install. No game files are included in this repository.

## Crates

- `blam-cache`: reads Halo 2 PC (Vista) `.map` files: header, string ids, tag
  names, tag groups, tag data, shared-map lookups, level collision and render
  geometry, shaders and bitmaps (DXT and uncompressed formats decoded to RGBA),
  render models with nodes, markers and skinning weights, animation graphs
  (keyframe codecs decoded), weapons, projectiles, damage and HUD widgets.
- `h2tool`: command-line inspector.
- `h2sim`: game simulation (no rendering): level collision, Spartan movement
  using the speeds, jump velocity and size from the game's own globals and biped tags,
  and weapons (fire rate, bursts, spread, magazines, reloads, zoom) from the weapon tags.
- `h2viewer`: walk or fly around a level with its real geometry and textures (simple lighting; lightmaps not yet).

### h2viewer

Double-click `h2viewer.exe` to open Lockout from `C:\Games\Halo 2 Project Cartographer\maps`
(or drag any `.map` file onto it). You start on foot at a player spawn.
Click in the window to look around with the mouse, WASD to move, Space to jump,
Ctrl or C to crouch, Tab to switch between walking and flying (flying: Space / C up / down,
Shift fast), Esc to release the mouse, Esc again to quit.

Weapons: left mouse fires, right mouse or Z zooms, R reloads, F melees, Q or the
mouse wheel switches weapon, 1-9 pick one directly. All 15 multiplayer weapons are
loaded with their real first person models, HUD, crosshairs, scopes and firing
stats, held by Master Chief's arms with the game's own first person animations
(ready, idle, fire, reload, melee).

```
h2tool scan  "C:\Games\Halo 2 Project Cartographer\maps"
h2tool info  lockout.map
h2tool tags  lockout.map sbsp
h2tool check lockout.map
h2tool obj    lockout.map lockout.obj   # export level collision geometry
h2tool render lockout.map lockout.png   # software-rendered preview, no GPU needed
h2tool level  lockout.map [texdir]      # render geometry, shaders, textures (optionally dumped as PNGs)
h2tool sim    lockout.map               # drop a Spartan at every spawn and walk (collision sanity check)
h2tool model  lockout.map battle_rifle  # render model nodes and markers
h2tool weapon lockout.map battle_rifle  # firing stats
h2tool hud    lockout.map battle_rifle [dir]  # HUD widgets
h2tool jmad   lockout.map fp_battle_rifle  # animation graph: skeleton and animations
h2tool jmadscan lockout.map             # decode every animation (reports failures)
h2tool shader lockout.map fp_arms       # shader template and bitmaps
```

Multiplayer maps only store the tags unique to them; the rest live in
`shared.map` (and `single_player_shared.map` for campaign).

## Roadmap

1. Map file reader (done)
2. Fly-camera level viewer (textured geometry done; lightmaps, scenery and skybox next)
3. Spartan movement, collision, weapons with first person animations (done)
4. Splitscreen and LAN multiplayer
5. Matchmaking, 1-50 ranks, parties
6. Campaign and AI

## Building

Windows builds are produced by GitHub Actions (`build` workflow artifacts).
Locally: `cargo build --release`.
