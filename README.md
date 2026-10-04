# halo2-rs

A from-scratch Rust recreation of Halo 2, loading the original game's assets
(`.map` cache files) at runtime from an existing Halo 2 PC / Project Cartographer
install. No game files are included in this repository.

## Crates

- `blam-cache`: reads Halo 2 PC (Vista) `.map` files: header, string ids, tag
  names, tag groups, tag data, shared-map lookups, level collision and render
  geometry, shaders and bitmaps (DXT and uncompressed formats decoded to RGBA),
  render models with nodes, markers and skinning weights, animation graphs
  (keyframe codecs decoded), weapons, projectiles, damage, HUD widgets,
  vehicles (seats, speeds, hover and friction points, model variants, physics
  hulls), and sounds (Xbox ADPCM and WMA).
- `h2tool`: command-line inspector.
- `h2sim`: game simulation (no rendering): level collision, Spartan movement
  using the speeds, jump velocity and size from the game's own globals and biped tags,
  vehicles (wheels with suspension, hovering, flight, seats, turrets, splatters
  and wrecks),
  weapons (fire rate, bursts, spread, magazines, reloads, zoom) from the weapon tags,
  the rules of every Halo 2 game type: Slayer, Capture the Flag, King of the
  Hill, Oddball, Juggernaut, Territories and Assault, with team versions of
  Slayer, King and Oddball (shields, health, grenades, pickups, respawns,
  friendly fire and betrayals, multi-kill and spree medals, the lead, flags,
  balls and bombs taken, dropped, returned, captured, armed and defused, hills
  that move, territories taken), falling damage and the map's kill zones from
  the game's own tags, and bots that play each objective, escort carriers and
  defend, finding their way over a walking graph sampled from each level's
  floors, and in Slayer take the vehicles they come across (driving, running
  people over, manning turrets and a teammate's Warthog gun).
- `h2net`: LAN games: hosting, joining, and finding games on the local network.
- `wma`: Windows Media Audio 2 decoder for the announcer's lines, ported from
  FFmpeg (and so LGPL 2.1 or later, unlike the rest of the repository).
- `h2viewer`: the game: Halo 2 style menus and lobby, Halo 2's maps with their
  lighting, Spartans, weapons, HUD and sounds, bots, splitscreen for up to four
  with controllers, and LAN play.

### h2viewer

Double-click `h2viewer.exe` to open Halo 2's menus over Lockout from
`C:\Games\Halo 2 Project Cartographer\maps` (or drag any `.map` file onto it),
with Halo 2's main menu music. Multiplayer opens the lobby: pick the game
type (Slayer, Team Slayer, Capture the Flag, King of the Hill, Team King,
Oddball, Team Oddball, Juggernaut, Territories or Assault), the map (every multiplayer map in the maps
folder, with Halo 2's own picture and description from `mainmenu.map`), score to win and number of bots, then Start Game. In team games, T
(or X on a controller) puts you on the red or blue team; bots fill the smaller
team. System Link lists games other PCs on the network are hosting.
Menus work with the arrow keys or WASD, Enter and Esc, the mouse, or a
controller (d-pad or stick, A, B).

In a game, click in the window to look around with the mouse, WASD to move,
Space to jump, Ctrl or C to crouch, hold Tab for the scoreboard, Esc for the
pause menu (resume, end the game, quit). ` (backquote) switches between walking
and flying (flying: Space / C up / down, Shift fast). When someone reaches the
score to win, the game stops and the carnage report shows everyone's kills and
deaths; Continue goes back to the lobby.

Capture the Flag: walk onto the enemy flag and press E (X on a controller) to
take it, then carry it to your own flag's stand while yours is home to score; Q
(Y) drops it. Carriers can't pick up weapons. A dropped flag goes home after 30
seconds. Arrows over the flags (and over home while carrying) show where to go,
and the announcer calls every take, drop, return and capture. Long falls hurt
or kill, and so do the map's death pits.

The other game types use each map's own hills, ball spawns, territories and
bomb spots. King of the Hill: stand in the hill (its outline glows in the
holder's colour) to score a point a second; it moves every minute. Oddball:
take the ball (E) and hold on to it to score a point a second. Juggernaut: the
first kill makes you the Juggernaut, tough and fast; only the Juggernaut's
kills score, and killing the Juggernaut takes over. Territories: stand alone in
a territory for six seconds to take it; each one your team holds scores a point
a second. Assault: carry your bomb into the enemy base and stand there to arm
it; it goes off four seconds later unless a defender holds E on it to defuse
it. Timed scores read as minutes and seconds.

Weapons: left mouse fires, right mouse or Z zooms, R reloads, F melees, Q or the
mouse wheel switches weapon, G throws a grenade, X switches grenade type, E picks
up (hold to swap weapons), 1-9 pick one directly. B adds a bot. All 15 multiplayer weapons are
loaded with their real first person models, HUD, crosshairs, scopes and firing
stats, held by Master Chief's arms with the game's own first person animations
(ready, idle, fire, reload, melee).

Dual wielding: holding a one-handed gun (SMG, Magnum, Plasma Pistol, Plasma
Rifle, Needler), stand on another and hold Q (Y on a controller) to take it in
your left hand. Right mouse or G (the left trigger) fires it, left mouse the
right gun; R reloads both. Tapping Q drops the left gun and switches to the one
on your back. Each gun uses Halo 2's dual wield spread and damage and its dual
first person animations, with the left hand drawn as the right's mirror image,
and both ammo counters show. No grenades while dual wielding. In testing,
pressing the number of the one-handed gun in hand gives a second one, and
`H2_WEAPON=<n> H2_DUAL=1` starts with two.

Vehicles: each map's own Warthogs (chaingun and gauss), Ghosts, Banshees and
turrets, where the map places them (Zanzibar and Ascension have them). Walk up
to one and hold E (X on a controller) to drive it, man its gun or ride along,
and hold it again to get out; an overturned vehicle can be flipped back over
the same way. The view follows the vehicle and it steers toward where you look;
W/S (the left stick) drive, and G (the left trigger) boosts a Ghost or Banshee.
Their speeds, seats, guns, hover pads, wheels and hulls come from the vehicle,
model and physics tags; wheels turn and ride their suspension, turrets aim,
and riders sit in Master Chief's seat animations. Vehicles run over and kill
anyone in the way at speed, take damage (explosions hurt them most) and blow up
with their riders, and come back where they started once wrecked or left
behind. Engines sound faster the faster they go. In Slayer games, bots get into
vehicles they pass: they drive where they're going (and at enemies to run them
over), gun from turrets and behind a teammate driving a Warthog, back up when
stuck and get out when there's nothing to do; in objective games they stay on
foot to play the objective. `H2_DRIVE="<vehicle> <forward>
<right> <look degrees> <seconds>"` drives one without a window for testing,
and `H2_LIST_VEHICLES=1` lists them as a map loads.

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
killing sprees, taking, losing and tying the lead, suicides, betrayals and
game over, as
Halo 2's multiplayer globals pair them. The menus have Halo 2's own music and
menu sounds.
`H2_MUTE=1` turns sound off; `H2_AUDIO_WAV=out.wav` records the mix to a file
instead of the speakers. For testing, `H2_GAME=<type>` (`slayer`, `team`, `ctf`, `king`,
`teamking`, `oddball`, `teamoddball`, `juggernaut`, `territories` or `assault`)
and `H2_BOTS=<n>` start a game straight away, and `H2_SIM=<seconds>` plays the
bots against each other without a window, printing kills, objective events and
the score (`H2_SEED=<n>` varies the run, `H2_SIM_WHERE=1` also prints where
every bot is and what it is doing every 20 seconds).

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
h2tool vehicle zanzibar.map warthog     # seats, speeds, model variants and physics hull
h2tool hud    lockout.map battle_rifle [dir]  # HUD widgets
h2tool jmad   lockout.map fp_battle_rifle  # animation graph: skeleton and animations
h2tool jmadscan lockout.map             # decode every animation (reports failures)
h2tool shader lockout.map fp_arms       # shader template and bitmaps
h2tool sound  lockout.map frag_expl out.wav  # decode a sound to WAV
h2tool soundscan lockout.map            # decode every sound (reports failures)
h2tool events lockout.map               # announcer events from the multiplayer globals
h2tool refs   lockout.map snd!:double_kill  # which tags refer to a tag
h2tool netgame lockout.map              # game type points (flags, hills, territories) and spawns
h2tool sid    lockout.map 0x1234abcd    # look up a string id
```

Multiplayer maps only store the tags unique to them; the rest live in
`shared.map` (and `single_player_shared.map` for campaign).

## Roadmap

1. Map file reader (done)
2. Fly-camera level viewer (done, with lightmaps, scenery and skies)
3. Spartan movement, collision, weapons with first person animations (done)
4. Splitscreen and LAN multiplayer, menus and lobby, and every game type:
   Slayer, Capture the Flag, King of the Hill, Oddball, Juggernaut,
   Territories and Assault (done)
5. Matchmaking, 1-50 ranks, parties
6. Campaign and AI

## Building

Windows builds are produced by GitHub Actions (`build` workflow artifacts).
Locally: `cargo build --release`.
