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
  using the speeds, jump velocity and size from the game's own globals and biped tags
  (running keeps to the ground up steps and over the tops of ramps, follows it
  down slopes and drops of up to about 0.06 world units, and falls off anything
  deeper; it won't squeeze under anything lower than the body),
  vehicles (wheels with suspension, hovering, flight, seats, turrets, splatters
  and wrecks),
  weapons (fire rate, bursts, spread, magazines, reloads, zoom, autoaim) from the weapon tags,
  with Halo 2's damage table (sniper rounds twice as hard on shields, plasma
  1.5 times on shields but a third on bodies, explosions half on shields),
  the maps' power-ups (the overshield charges shields to three times and
  drains back over a minute; active camouflage all but hides a player for 45
  seconds, less so while they fire or get hurt) and ammo packs for power
  weapons, timed from their tags, the maps' teleporters,
  the rules of every Halo 2 game type: Slayer, Capture the Flag, King of the
  Hill, Oddball, Juggernaut, Territories and Assault, with team versions of
  Slayer, King and Oddball (shields, health, grenades, pickups, respawns,
  friendly fire and betrayals, multi-kill and spree medals, the lead, flags,
  balls and bombs taken, dropped, returned, captured, armed and defused, hills
  that move, territories taken), falling damage and the map's kill zones from
  the game's own tags, and bots that play each objective, escort carriers and
  defend, finding their way over a walking graph sampled from each level's
  floors (taking teleporters where they're the shorter way), and in Slayer take the vehicles they come across (driving, running
  people over, manning turrets and a teammate's Warthog gun). Bots go for
  power weapons and power-ups lying nearby, dual wield, switch to the gun
  that suits the fight, and only spot a camouflaged player close up or when
  they fire.
- `h2net`: LAN games: hosting, joining, and finding games on the local network.
- `h2live`: online play, like Halo 2 on Xbox Live: Bungie's levels 1 to 50
  (hidden XP per ranked playlist, won and lost against each opponent), Halo
  2's launch playlists and its later Team Snipers and Team Hardcore
  (replaceable by a `playlists.txt`), and matchmaking (level ranges that
  widen, even teams, map choice and host choice).
- `h2launch`: the launcher (Windows only, work in progress) that starts MCC's
  own classic Halo 2 engine from your Master Chief Collection install,
  without running MCC. Double-click it to open its lobby: pick a gamertag,
  form a party, search a playlist, play the match on the real engine, and
  see the carnage report with your new level afterwards. Custom Game (under
  the playlists) lets the party leader pick any game type and map for the
  party, unranked; it works alone too. Y on the playlists opens the party:
  its leader can hand the lead to another member, remove someone, or make
  the party invite only, and anyone can leave it there. The carnage report
  lists each player's score, kills, assists and deaths, as Halo 2 on Xbox
  did. RB opens your friends list: send friend requests by gamertag (Y),
  accept or decline the ones you get, see what your friends are doing,
  invite them or join their party (if you can't, it says why: invite only,
  full, in a match), and remove them. LB opens a service
  record (yours from the playlists, anyone else's from the players list,
  the party, the friends list or the carnage report): their highest level,
  and their games, wins, kills, deaths and K/D in each ranked playlist. The
  levels the launcher shows count the launcher's playlists only, and the
  party panel and the party screen show each member's level in the playlist
  you have selected. On
  a keyboard, Q and E (or Page Up and Page Down) are LB and RB. Levels show
  as Halo 2's own level icons, read from your MCC install (or from a Halo 2
  Vista install if MCC's can't be read); without either they are numbers. Friends and service records need the matching
  server: an older launcher signing in to it is told UPDATE YOUR LAUNCHER.
  Its text is in Halo 2's own fonts, read from your MCC install.
  Settings (the row after Custom Game, or X on a controller at the sign-in
  screen) holds Halo 2's controller settings, in the order of its
  CONTROLLER screen: Thumbstick Layout (Default, Southpaw, Legacy, Legacy
  Southpaw), Button Layout (Halo 2's Default, Southpaw, Boxer and Green
  Thumb, plus Halo 3's Bumper Jumper, with jump on LB and melee on RB, and
  MCC's Recon), Look Sensitivity (1 to 10, 3 by default), Look Inversion
  (the thumbstick's), Automatic Look Centering and Controller Vibration,
  then Mouse Sensitivity and Mouse Inversion, and Restore Defaults. It
  lists what every button does in the layout you pick, and the keyboard
  keys. Your choice is saved and used in every match. On a keyboard and
  mouse: W A S D to move, the mouse to look, left button to fire, right
  button to zoom, Space to jump, Left Ctrl to crouch, G to throw a
  grenade, F for the flashlight, Q to melee, R to reload, E for the action
  and Tab to switch weapons. Those are Halo 2's own keys on the PC, as
  others have read them from the game's code; they haven't been tried in
  this launcher yet. MCC's other keys (the right button for the left gun,
  1 to switch weapons, 2 to swap grenades, 4 for the flashlight, C to
  dual wield) are given to the game too, but may do nothing. When a match
  starts the game's window should come to the front by itself (not yet
  checked on a PC); if it doesn't, click it. While the game is on, the
  lobby window ignores the controller and the keys that could leave the
  game (Esc, Enter, Backspace, letters), so leaving takes a click. In the
  lobby, Enter or Space is A and Esc or Backspace is B.
  `--offline` starts
  an offline Slayer match on Lockout instead (with the controls saved in
  Settings, or flags such as `--layout bumper_jumper`). It reads the game's
  `halo2.dll` and never changes any MCC file. It is not part of the game
  above. See `docs/notes/launcher/README.md`.
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
Oddball, Team Oddball, Juggernaut, Territories or Assault), the map (every multiplayer map in the maps and dlc
folders, with Halo 2's own picture and description from `mainmenu.map`), score to win and number of bots, then Start Game. Game Options holds
Halo 2's variant settings: the weapons on the map, primary and secondary
starting weapons, starting grenades, shields, motion sensor, vehicles, respawn
time, friendly fire and a time limit (none, or 5 to 30 minutes), plus the SWAT,
Rockets, Snipers, Swords and Shotguns variants and those of Team Hardcore
(battle rifle starts) and Team Snipers, both with no motion sensor. The
motion sensor shows teammates (yellow) and enemies (red) within 25 metres who
move faster than a crouch-walk or fire; with it off, the HUD has no tracker.
The HUD is drawn at Halo 2's own size: its bitmaps are made for a 1280x960
screen, so at 1080p they are drawn at 1.125 times their size (the motion
tracker about a sixth of the screen's height). The remake's own HUD text (the
score and clock, kill feed, prompts, respawn countdown and names) goes at the
same scale, but never smaller than one pixel to each pixel of its 5x7 font
(8 pixel lines), with a dark shadow, so it stays readable in a 720p window,
in splitscreen views and over snow or sky.
In team games, T (or X on a controller) puts you on the red or blue team; bots
fill the smaller team (counting the people on other PCs in the lobby).
System Link lists games other PCs on the network are
hosting.
Menus work with the arrow keys or WASD, Enter and Esc, the mouse, or a
controller (d-pad or stick, A, B).

In a game, click in the window to look around with the mouse, WASD to move,
Space to jump, Ctrl or C to crouch, hold Tab for the scoreboard, Esc for the
pause menu (resume, end the game, quit). The game stops while it's up, its
sounds (engines, the shield alarm, rockets in flight) holding until you resume,
unless someone on another PC plays in it: then only you stop (as in Halo 2's
System Link). ` (backquote) switches between walking
and flying (flying: Space / C up / down, Shift fast). When someone reaches the
score to win, the game stops and the carnage report shows everyone's kills and
deaths; Continue goes back to the lobby. With a time limit, a clock above the
score counts down, and when it reaches 0:00 the game stops too: the best score
wins, and a tie for the best is a draw.

Player Profile (main menu): type your gamertag, choose Spartan or Elite and
your primary and secondary armour colours from Halo 2's 18, and build your
emblem (one of Halo 2's 64 pictures in two colours over one of its 32
backgrounds), with your model turning beside the menu. Your emblem shows in
the lobby and on the scoreboard, Elites wear it on their back, and a team's
Capture the Flag flag carries its first player's emblem. Its CONTROLLER
screen has Halo 2's controller settings: THUMBSTICK LAYOUT and BUTTON LAYOUT
(see Controllers below, with a list of what each button does), LOOK
SENSITIVITY (a controller's look stick) from 1 to 10, 3 by default as in
Halo 2, and LOOK INVERSION, which turns looking up and down around (the
mouse's too). MOUSE SENSITIVITY, also 1 to 10, is player one's alone.
Splitscreen guests have their own controller settings: CONTROLLER SETTINGS on
a player's pause menu changes theirs (player one's too) mid-game. It's saved
in `%APPDATA%\halo2-rs\profile.txt` (the controller settings as
`look_sensitivity=`, `mouse_sensitivity=`, `invert_look=yes`/`no`,
`button_layout=` (`default`, `southpaw`, `boxer`, `green_thumb`,
`bumper_jumper` or `recon`) and `thumbstick_layout=` (`default`,
`southpaw`, `legacy` or `legacy_southpaw`), the guests' with `guest1_` to
`guest3_` in front; a profile from before has the defaults) and sent to the
other PCs in a System Link game (the controller settings stay on your PC).
In team games armour takes the
team's colour. Splitscreen guests play the same model in other colours; bots
each have their own look, about a third of them Elites. Spartans and Elites
run, walk, strafe and crouch with the game's own animations played at the pace
they move, keep their stride changing direction, go into the airborne pose
only once really off the ground, and land soft or hard by how fast they came
down (from the biped tag). Elites hold weapons
with their own first person animations and make their own sounds getting
into vehicles.

Names: you play under your gamertag (at first your Windows user name; set
`H2_NAME` to override it), splitscreen guests as NAME(1), NAME(2) and so on,
and bots under callsigns.
Names show in the lobby, scoreboard, kill feed and announcements, over
teammates in sight, and over whoever is under your crosshair, which turns red
on an enemy and green on a teammate within your weapon's autoaim as in Halo 2
(the colours of the game's HUD shaders). `H2_LIST_WEAPONS=1` lists each
weapon's autoaim, aim assist and HUD pieces as a map loads. `H2_PADS=20`
checks the controllers without the maps or a window: it lists those found
(and how it reads them, XInput on Windows) with each player's layouts,
buzzes each for half a second, prints every button, trigger and stick for
20 seconds (each button with what it does in the layout of the player that
controller's number would be, and in Halo 2's default), and quits.
`H2_PADS=log` plays as usual and prints controllers coming and going, which
window holds each, the window coming to the front or going behind, and each
button pressed in a game with what it did.

Capture the Flag: walk onto the enemy flag and press E (on a controller, the
button the prompt names: X in Halo 2's own layouts) to take it, then carry it
to your own flag's stand while yours is home to score; Q (Y) drops it.
Carriers can't pick up weapons. A dropped flag goes home after 30 seconds.
Arrows over the flags (and over home while carrying) show where to go, and the
announcer calls every take, drop, return and capture. Long falls hurt
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
up (hold to swap weapons), 1-9 pick one directly. B adds a bot. The view is
Halo 2's: 70 degrees across a 4:3 screen (the biped tag's field of view;
wider screens see more to the sides, up to 100 degrees), with the crosshair
below the middle of the screen where the game's player control puts it, so
you aim at what's under it while the menus stay centred. All 15 multiplayer weapons are
loaded with their real first person models, HUD, crosshairs, scopes and firing
stats, held by Master Chief's arms with the game's own first person animations
(ready, idle, fire, reload, melee). Guns whose tags give no rate of fire
(the Magnum, Carbine, Shotgun, sniper rifles, Rocket Launcher and others) fire
as fast as their recovery time allows: the Magnum every 0.1 s, the Sniper
Rifle every 0.5 s. Holding the trigger keeps the Battle Rifle firing bursts,
and the Magnum, Carbine, Shotgun and sniper rifles shots, as fast as each
recovers. That is our reading of the barrel flag "don't clear fire bit after
recovering", which those weapons have and the Rocket Launcher, Brute Shot and
Fuel Rod (which need a fresh pull) don't. A pull late in the Carbine's recovery
is held over until it can fire. Spread grows over a burst or sustained fire as
the tags give it; the Sniper Rifle and Beam Rifle have none zoomed in (their
barrels' "use error when unzoomed" flag). Each weapon's autoaim angle and
range from its tags steer bullets fired at or close to an enemy towards the
middle of them (zoomed, it reaches farther through a narrower cone; the Sniper
Rifle and Beam Rifle, whose tags say aim assists work only zoomed, get none
unzoomed), and the crosshair turns red while an enemy is in reach. Rounds that
fly (plasma, needles, rockets, grenades) aren't steered. Getting hurt knocks
you out of zoom. The HUD shows each piece in the states its tag gives it, so the Sniper Rifle scope reads 5x or 10x by zoom level. Bullets hit at once; plasma bolts, needles,
rockets, Brute Shot grenades and Fuel Rod shots fly at their tag speeds (rockets
speed up, Brute Shot rounds arc), needles and the Fuel Rod home in, rockets lock
on to enemy vehicles, and explosive rounds go off in a blast that hurts and
throws everyone in reach, the shooter included. Needles stick in whoever they
hit and pop a moment later; seven in one target at once set off a supercombine
that kills. Other Spartans bend at the waist to aim up and down.
`H2_START_WEAPONS=needler,smg` changes what everyone spawns with, for testing.

Dual wielding: holding a one-handed gun (SMG, Magnum, Plasma Pistol, Plasma
Rifle, Needler), stand on another and hold Q (Y on a controller) to take it in
your left hand. Right mouse or G (the left trigger) fires it, left mouse the
right gun; R reloads both. Tapping Q drops the left gun and switches to the one
on your back. Each gun uses Halo 2's dual wield spread and damage and its dual
first person animations, with the left hand drawn as the right's mirror image,
and both ammo counters show. No grenades while dual wielding. In testing,
pressing the number of the one-handed gun in hand gives a second one, and
`H2_WEAPON=<n> H2_DUAL=1` starts with two.

Vehicles: each map's own Warthogs (chaingun and gauss), Ghosts, Banshees,
Scorpions, Wraiths and turrets, where the map places them (Zanzibar,
Ascension, Coagulation, Containment and Waterworks have them). Walk up
to one and hold E (on a controller, the button the prompt names: X in Halo 2's
own layouts) to drive it, man its gun or ride along, and hold it again to get
out; an overturned vehicle can be flipped back over the same way. The view
follows the vehicle from where Halo 2's own camera
tracks put it (higher and closer as you look down) and it steers toward where
you look; W/S (the left stick) drive, and G (the left trigger) boosts a Ghost
or Banshee. Scorpions and Wraiths drive like tanks, turning on the spot toward
the view while you drive, with the turret aimed at the crosshair: left mouse
fires the Scorpion's cannon or lobs the Wraith's mortar, and G (the left
trigger) fires the Scorpion's machine gun. F (B on a controller) drops the
Banshee's fuel rod bomb. Every vehicle gun fires at what the crosshair is on.
Space (A) loops the Banshee, or with A/D (the stick to one side) barrel rolls
it aside; rockets locked on lose it. Holding E by an enemy driver or gunner
boards their seat and throws them out; boarding a Scorpion or Wraith takes a
second and kills the driver through the hatch. Master Chief's own sounds play
getting in, getting out and boarding. Left mouse (RT) in a Warthog's driver
seat sounds its horn. Badly damaged vehicles smoke, then burn before they
blow up (`H2_VEHICLE_HEALTH=<fraction>` starts them damaged, for testing).
Their speeds, seats, guns, hover pads, wheels and hulls come from the vehicle,
model and physics tags; wheels turn and ride their suspension, turrets aim,
and riders sit in Master Chief's seat animations. Vehicles run over and kill
anyone in the way at speed, take damage (explosions hurt them most) and blow up
with their riders, and come back where they started once wrecked or left
behind. Engines sound faster the faster they go. In Slayer games, bots get into
vehicles they pass: they drive where they're going (and at enemies to run them
over), gun from turrets and behind a teammate driving a Warthog, back up when
stuck and get out when there's nothing to do, board enemies sitting in slow
vehicles close by, and loop away from rockets in a Banshee; in objective games
they stay on foot to play the objective. `H2_DRIVE="<vehicle> <forward>
<right> <look degrees> <seconds>"` drives one without a window for testing,
and `H2_LIST_VEHICLES=1` lists them as a map loads.

Controllers use Halo 2's DEFAULT layouts to start with: left stick move,
right stick look, RT fire, LT grenade (the left gun dual wielding), A jump, B
melee, X reload / hold to pick up, Y switch weapon / hold to dual wield, LB
the flashlight (the Arbiter's camouflage), RB swap grenades, click the sticks
to crouch and zoom, Start pauses, hold Back for the scoreboard. Each player
picks a BUTTON LAYOUT and a THUMBSTICK LAYOUT on the profile's CONTROLLER
screen (or CONTROLLER SETTINGS on their pause menu), which lists what each
button and stick does in the one picked:

- SOUTHPAW: the triggers swapped.
- BOXER: LT melees (fires the left gun dual wielding) and B throws grenades.
  Dual wielding there's no melee, as Halo 2 Vista's Boxer screen has it (the
  original Xbox's moves the left gun to B instead).
- GREEN THUMB: click the right stick to melee; B zooms.
- BUMPER JUMPER, Halo 3's, which Halo 2 never had: LB jumps, RB melees, B
  reloads / hold to pick up, A swaps grenades, X the flashlight.
- RECON, the Master Chief Collection's default: RB reloads / hold to pick
  up, X or up on the d-pad the flashlight, left or right on the d-pad swaps
  grenades.
- Thumbsticks: SOUTHPAW swaps the sticks; LEGACY moves forward and back and
  turns with the left stick, and looks up and down and strafes with the
  right; LEGACY SOUTHPAW is Legacy swapped.

Prompts name the button the player's layout uses (HOLD B TO DRIVE on Bumper
Jumper). The menus keep the d-pad or left stick, A to choose, B to go back
and X to change team whatever the layout. The look stick turns as the
game's player control tag sets it: its look curve
and turn rates (120 degrees a second across and 60 up and down at the default
sensitivity, slower zoomed), speeding up to two and a half times that over
0.8 s while the stick is pushed nearly all the way. Controllers also get
Halo 2's aim assist, from each weapon's magnetism angle and range (narrower
and farther zoomed, none for the sniper rifles unzoomed): the stick turns
slower with the crosshair on an enemy (friction), and while either stick moves
the view follows an enemy it's on (adhesion). The mouse gets none, and nor
does a Warthog's or Spectre's driver, who has no gun. A on a new controller
takes over player one (the keyboard and mouse still work for them too); in
the lobby, Start on another controller adds a splitscreen player (up to
four) and Back or B takes them out again. Start on a new
controller during a game drops them straight in. As in Halo 2, two players
split the screen top and bottom, three give the first player the top half
and the others a quarter each, and four take a quarter each; each view uses
the HUD tags' own half or quarter screen layout (quarter views have no ammo
meter). A player who pauses gets the pause menu in their own view, and only
their controller (or the keyboard, for Esc) works it. If a controller is
unplugged or its battery runs out, its player stands still until it comes
back or A on another controller takes over (a guest before player one,
who plays on at the keyboard). Start on a new controller also takes over a
guest whose controller went, and otherwise drops a new player in. A
guest's pause menu goes with their controller: player one gets it while the
game stands still, and over a System Link game that plays on it closes.

LAN: every lobby and game is open to other PCs on the same network (allow
h2viewer through Windows Firewall when asked); System Link lists them and joins
the one you pick (a game from a different version of h2viewer shows as
ANOTHER VERSION: update both PCs). Joined PCs wait in the host's lobby, seeing its choices and
everyone in it (T or X picks your team there; until then the host picks it
and the lobby shows you in grey), and follow the host into each game it starts (loading the map
if needed; the game holds its start for them) and back to the lobby after it.
A PC without the lobby's map can wait there, warned that it doesn't have it,
but can't follow the host into a game on it.
The host runs the game and its bots (fewer bots when more people join); if the
host leaves, the joined PCs go back to System Link. A PC that leaves a game in
progress leaves its Spartan to a bot, and takes it back, score and all, if it
joins again (so does a member back in their party's custom game online).
Two h2viewer windows on one PC can play System Link together. On Windows the
game reads controllers through XInput (Xbox Series X|S, Xbox One and Xbox 360
controllers, wired or wireless; other controllers through DS4Windows, or Steam
Input once h2viewer is added to Steam as a non-Steam game, since Steam running
in the background isn't enough), which reaches every window whichever is in
front: a controller with a player in a window (after A or Start there) stays
that window's, even while you're in the other one, until its player leaves the
game (Back in the lobby, or quit), and one without works whichever window is
in front. The keyboard and mouse only reach the window in front, so two people
at one PC should play splitscreen or take a controller each; two windows are
also for trying System Link on your own (set `H2_AUTOPILOT=1` for one of them
and a bot plays it).

Sound: weapons, reloads, grenades, footsteps, landings, shield recharge, the
low shield alarm and rockets in flight play from the game's own sound files, placed left or right and
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
the score and kills by weapon (`H2_SEED=<n>` varies the run, `H2_SIM_WHERE=1` also prints where
every bot is, what it carries and what it is doing every 20 seconds).
`H2_TIME_LIMIT=<seconds>` gives games that time limit. `H2_TEST_LEVELS=1`
gives every player a level, so the lobby and the scoreboard's LEVEL column
show Halo 2's rank icons (read from `mainmenu.map`), which otherwise show only
for players whose level is known.

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
h2tool hud    lockout.map battle_rifle [dir]  # HUD widgets in each screen layout
h2tool jmad   lockout.map fp_battle_rifle  # animation graph: skeleton and animations
                                        # (and how fast each moves; H2_INHERIT=1 adds the graph's parent's)
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

### h2live

`h2live.exe` is the online service: accounts, parties, matchmaking and the
relay that carries online games. It reads no map files. Double-click it on
the PC that hosts it (allow it through Windows Firewall when asked): it
listens on port 47050 (TCP for signing in, and UDP for the relay that
carries the launcher's games), keeps accounts in `h2live-data` (back that
folder up), asks the router to open the port (UPnP), and says how players
reach it: `READY: ws://<address>:47050`, `FORWARD TCP AND UDP 47050 TO
<this PC>` when the router needs port forwards set by hand, or `CGNAT: USE
THE HOSTED OPTION` when the internet provider makes that impossible. Ctrl+C
stops it. An `h2live.txt` next to it can set `port=`, `data=` and `relay=`
(the relay's UDP port, or `off`). On a host online, `PORT`, `H2LIVE_DATA`,
`H2LIVE_RELAY` and `H2LIVE_SECRET` (what signs players' stat cards) set the
same; `/health` answers health checks, and `/` says how many players are
online. `H2LIVE_UPNP=0` leaves the router alone. The relay only lets in
the players of matches h2live sets up. `h2relay.exe` runs a relay on its
own for tests on one PC or a home network (`h2relay [port]`); it lets
anyone in, so don't forward its port.

Games sign in to h2live on the same PC if it's running, or else the server a
`server=` line in `profile.txt` names, or else the one built into the game;
`H2_LIVE=<address>` overrides them all. [docs/ONLINE.md](docs/ONLINE.md) has
the steps for running the server online (on Render's free plan, from
`render.yaml` and `Dockerfile.live`) or on a PC at home. Online, RECENT
PLAYERS lists the last 50 people you played a match or custom game with,
newest first, with what, where and when, to invite or join if they're
online; each PC keeps its own list in `recent-players.txt` beside
`identity.key`.

For testing a service, `H2_LIVE_BOT=<playlist key>` runs h2viewer with no
window or sound as a player who signs in (to the service `H2_LIVE` names),
searches that playlist and plays each match it finds with a bot, over and
over. Each takes little CPU, so many can run on one PC (give each its own
`H2_NAME`, `H2_PROFILE` and `H2_IDENTITY`). As each map goes in, the game
prints the memory it has and the most it has had (`memory: ...`), so a long
session's log shows whether it keeps growing.

Online, a PC that joins moves its own Spartans on foot (splitscreen guests
too) the moment their controls say so, and the host's word on where they
are puts them right a round trip later: eased over if it's close, at once
after a teleporter, a death or a respawn. The host moves them once for
each tick of controls that PC sent, however late they come, so the two
agree. In a vehicle they're shown as the host has them, as are firing and
everyone else. If the host goes quiet for half a second, they stand where
they are until it answers. On a LAN, joined PCs show the host's game
alone, as before.

To try a slow connection, `H2_NET_LAG=<ms>` adds that much to the round
trip between a PC and the host it joins (half each way), `H2_NET_JITTER=<ms>`
up to that much more at random, and `H2_NET_LOSS=<percent>` loses that share
of messages (each comes a round trip later, holding up those behind it, as
over TCP); only a PC that joins uses them. `H2_NET_PROBE=1` makes player
one on a joined PC stand, run, jump, crouch and strafe while turning, over
and over, and print how long each start took to show and where the view is
every frame.

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
