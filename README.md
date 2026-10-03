# halo2-rs

A from-scratch Rust recreation of Halo 2, loading the original game's assets
(`.map` cache files) at runtime from an existing Halo 2 PC / Project Cartographer
install. No game files are included in this repository.

## Crates

- `blam-cache`: reads Halo 2 PC (Vista) `.map` files: header, string ids, tag
  names, tag groups, tag data, shared-map lookups, level collision and render
  geometry, shaders and bitmaps (DXT and uncompressed formats decoded to RGBA),
  render models with nodes, markers and skinning weights, animation graphs
  (keyframe codecs decoded), weapons, projectiles, damage, HUD widgets, and
  sounds (Xbox ADPCM and WMA).
- `h2tool`: command-line inspector.
- `h2sim`: game simulation (no rendering): level collision, Spartan movement
  using the speeds, jump velocity and size from the game's own globals and biped tags,
  weapons (fire rate, bursts, spread, magazines, reloads, zoom) from the weapon tags,
  Slayer rules (shields, health, grenades, pickups, respawns, multi-kill and
  spree medals, the lead), bots and their walking graph.
- `h2net`: LAN games: hosting, joining, and finding games on the local network.
- `wma`: Windows Media Audio 2 decoder for the announcer's lines, ported from
  FFmpeg (and so LGPL 2.1 or later, unlike the rest of the repository).
- `h2viewer`: the game: Halo 2 style menus and lobby, Halo 2's maps with their
  lighting, Spartans, weapons, HUD and sounds, bots, splitscreen for up to four
  with controllers, and LAN play.

### h2viewer

Double-click `h2viewer.exe` to open Halo 2's menus over Lockout from
`C:\Games\Halo 2 Project Cartographer\maps` (or drag any `.map` file onto it),
with Halo 2's main menu music. Multiplayer opens the lobby: pick the map
(every multiplayer map in the maps folder), score to win and number of bots,
then Start Game. System Link lists games other PCs on the network are hosting.
Menus work with the arrow keys or WASD, Enter and Esc, the mouse, or a
controller (d-pad or stick, A, B).

In a game, click in the window to look around with the mouse, WASD to move,
Space to jump, Ctrl or C to crouch, hold Tab for the scoreboard, Esc for the
pause menu (resume, end the game, quit). ` (backquote) switches between walking
and flying (flying: Space / C up / down, Shift fast). When someone reaches the
score to win, the game stops and the carnage report shows everyone's kills and
deaths; Continue goes back to the lobby.

Weapons: left mouse fires, right mouse or Z zooms, R reloads, F melees, Q or the
mouse wheel switches weapon, G throws a grenade, X switches grenade type, E picks
up (hold to swap weapons), 1-9 pick one directly. B adds a bot. All 15 multiplayer weapons are
loaded with their real first person models, HUD, crosshairs, scopes and firing
stats, held by Master Chief's arms with the game's own first person animations
(ready, idle, fire, reload, melee).

Controllers use Halo 2's layout (left stick move, right stick look, RT fire,
LT grenade, A jump, B melee, X reload / hold to pick up, Y switch weapon, click
sticks to crouch and zoom, Start pauses, hold Back for the scoreboard). A on a
new controller takes over player one; in the lobby, Start on another
controller adds a splitscreen player (up to four) and Back takes them out
again. Start on a new controller during a game drops them straight in.

LAN: every game is open to other PCs on the same network (allow h2viewer through
Windows Firewall when asked); System Link lists them and joins the one you
pick, loading its map first if it's another one. The host runs the game and its
bots; if the host leaves, the joined PCs go back to System Link.

Sound: weapons, reloads, grenades, footsteps, landings, shield recharge and the
low shield alarm play from the game's own sound files, placed left or right and
fading with distance (each splitscreen player hears through their own view).
The announcer calls the game type, multi-kills (Double Kill to Killimanjaro),
killing sprees, taking, losing and tying the lead, suicides and game over, as
Halo 2's multiplayer globals pair them. The menus have Halo 2's own music and
menu sounds.
`H2_MUTE=1` turns sound off; `H2_AUDIO_WAV=out.wav` records the mix to a file
instead of the speakers.

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
h2tool sound  lockout.map frag_expl out.wav  # decode a sound to WAV
h2tool soundscan lockout.map            # decode every sound (reports failures)
h2tool events lockout.map               # announcer events from the multiplayer globals
h2tool refs   lockout.map snd!:double_kill  # which tags refer to a tag
```

Multiplayer maps only store the tags unique to them; the rest live in
`shared.map` (and `single_player_shared.map` for campaign).

## Roadmap

1. Map file reader (done)
2. Fly-camera level viewer (done, with lightmaps, scenery and skies)
3. Spartan movement, collision, weapons with first person animations (done)
4. Splitscreen and LAN multiplayer (done), menus and lobby (done), more game
   types (next)
5. Matchmaking, 1-50 ranks, parties
6. Campaign and AI

## Building

Windows builds are produced by GitHub Actions (`build` workflow artifacts).
Locally: `cargo build --release`.
