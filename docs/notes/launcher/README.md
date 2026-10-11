# h2launch

`h2launch.exe` starts MCC's classic Halo 2 engine (`halo2\halo2.dll`) from
your Master Chief Collection install, without MCC. With no flags it opens
the lobby (see below and `lobby.md`): sign in to h2live, form a party,
search a playlist, and each match runs on the engine in a second copy of
the launcher. `--offline` starts an offline multiplayer Slayer match on
Lockout with one local player instead. A gamepad works, in Halo 2's
button and thumbstick layouts (see "Controls"); keyboard and mouse
should. The engine is Windows only; on other systems only the lobby runs,
with a stand-in engine (so the Linux CI build and tests pass).

This is milestone 1: it reaches a running match and reports in detail what
happened, so the next step (running it on the owner's PC) can see what works
and what to fix. It never changes any MCC file and never patches or hooks the
game's code: it talks to the engine only through the DLL's three exports, the
objects they return, and a host object and event manager we provide, plus
documented Windows APIs.

## How it works, in short

1. Finds the MCC folder (the one that contains `halo2\halo2.dll`).
2. Opens a window and creates a Direct3D 11 device and swap chain.
3. Registers the DLL search folders, sets the working directory to the MCC
   root, and loads `halo2.dll` by absolute path.
4. Builds the packed game-options block for an offline Lockout Slayer match,
   loading the Slayer variant through the engine's own data-access object.
5. On a worker thread: `SetLibrarySettings`, `CreateGameEngine`,
   `initialize_graphics`, the two preloads, then `initialize_game`, which
   starts the engine's game thread.
6. The engine renders and presents; our window thread only pumps messages,
   serves the engine's callbacks (input, paths, the profile, rumble...), and
   handles resize and quit.

Everything it does is written to a log (see below), including every launch
step, the `halo2.dll` build it loaded, the first call of each engine
callback, a call-count summary every ten seconds, and a final `RESULT:` line.

## Running it on the owner's PC

In a terminal (PowerShell or cmd), from the repo clone:

```
cd "C:\Halo 2 Rust Project"
git fetch
git checkout h2launch-m1
cargo build --release -p h2launch
target\release\h2launch.exe --check
```

`--check` does not start the engine. It prints the MCC folder it found, the
`halo2` file list, `halo2.dll`'s version, SHA-256 and PE facts, its exports
and (delay-)imports, the map and variant a launch would use, and the display
adapters. Read it first to confirm the install looks right.

Then an unattended run that quits itself after two minutes and saves three
screenshots:

```
target\release\h2launch.exe --quit-after 120 --screenshot 30,60,90
```

Watch the window, or come back and read the log and the screenshots. To play
by hand instead, run it with `--offline` and close the window to quit (with
no flags at all it opens the lobby).

The log is `%LOCALAPPDATA%\h2launch\h2launch.log` in cmd, or
`$env:LOCALAPPDATA\h2launch\h2launch.log` in PowerShell (the previous run is
kept as `h2launch.prev.log`). Screenshots are PNGs in the same folder
(`h2launch-shot-*.png`); the previous run's are moved into `prev-shots` at
start, so every PNG in the folder is from the latest run. When started from a
terminal, the log is also printed there.

The last line of the log reads like

```
RESULT: in-game=yes frames=7012 crashed=none presents=7013 states=[1] game-thread-exited=yes step="running" reason="--quit-after 120"
```

- `in-game=yes` only when the engine reported the map loaded
  (`set_game_state(1)`); `maybe(...)` when it drew many frames and polled
  input without saying so (it also does that on a loading screen); `no`
  otherwise.
- `step` is the furthest launch step reached (for example
  `preload_common_begin` if the engine never came back from it).
- The exit code is 0 only for `in-game=yes` with no crash, 2 after a crash,
  and 1 otherwise.

If the engine stops drawing, the log lists the process's own windows (an
engine error dialog appears there with its text), and at the end a screen
grab of the window area is saved as `h2launch-shot-final-gdi.png`. A
watchdog ends the run with a `RESULT` line if the window thread stops
responding for 30 s, or 30 s after `--quit-after` if the run has not ended by
then (`--no-watchdog` turns both off, for a debugger session).

## Flags

```
--check                 Check the install and print what a launch would use;
                        does not start the engine.
--variants              List the settings of every matchmaking game variant
                        (halo2\hopper_game_variants), read through halo2.dll's
                        data access; does not start the engine.
--mcc <folder>          MCC's folder, if it is not found by itself.
--map <name>            Map to play (lockout by default): lockout, midship,
                        zanzibar, ...
--variant <file>        Game variant .bin. A bare name is looked up in
                        halo2\hopper_game_variants (default 01_slayer.bin).
                        A variant that cannot be loaded stops the launch.
--no-variant            Start without a game variant (testing only).
--name <gamertag>       Player name (default Player).
--xuid <number>         Fixed player id instead of a random one.
--windowed <W>x<H>      Window size (default 1280x720).
--quit-after <seconds>  Ask the engine to quit after this long, then exit.
--screenshot <s,s,...>  Save the picture as a PNG at these times (seconds
                        from start).
--input-script <file>   Timed fake input for unattended tests (see below).
--groundhog             Also load groundhog.dll the way MCC does.
--attach-input          Share the window's keyboard input with the game
                        thread (default on).
--no-attach-input       Do not (to compare).
--host-fonts            Tell the engine the host draws text (setting 6).
                        Fonts are not served yet, so the font calls still
                        answer no; they are logged.
--layout <name>         Button layout: default, southpaw, boxer,
                        green_thumb, bumper_jumper or recon (see "Controls").
--sticks <name>         Thumbstick layout: default, southpaw, legacy or
                        legacy_southpaw.
--look-sensitivity <n>  1 to 10 (Halo 2's default 3).
--invert-look, --no-invert-look      Look inversion.
--auto-center, --no-auto-center      Automatic look centering.
--vibration, --no-vibration          Controller vibration.
--mouse-sensitivity <f> 0.1 to 10 (default 1.6).
--invert-mouse, --no-invert-mouse    Mouse look inversion (apart from the
                        controller's, as MCC has it).
                        These eight default to what the lobby's Settings
                        saved in lobby.txt (else Halo 2's defaults); the
                        lobby passes all of them to every engine it starts.
--pad-map <layout|zero> Gamepad mapping handed to the engine: the button
                        layout's (default), or all zero, every action on
                        LT, which is what HaloX returns (for diagnosis; `h2`
                        still means layout).
--no-key-bindings       Leave the profile's keyboard and mouse table empty,
                        as before (for diagnosis).
--set-option <o>=<t>:<v>   Write a value into the game options before the
                        start, e.g. 0x03=u8:0 (repeatable; for testing).
--set-profile <o>=<t>:<v>  The same for the player profile.
--no-watchdog           Do not end the run when the window thread stops
                        responding for 30 s (for a debugger session).
--session <file>        Play a networked match (see "Networked match" below).
--me <index>            Which machine in the session this launcher is.
--relay-wait <seconds>  How long to wait for the relay before the engine
                        starts (default 15).
--live <server>         Sign in to h2live (host, host:port or ws:// URL),
                        search a playlist and play the match it makes (see
                        "Matchmaking through h2live" below).
--playlist <n>          The launcher playlist to search (default 11, Head
                        to Head).
--live-wait <seconds>   How long to search before giving up (default 600).
--recv-port <mode>      How the receive call's fourth argument is read: auto
                        (default), value, pointer or ignore.
--send-return <len|0|1> What the send calls return for a packet that went.
--instance <name>       Own log, screenshot and engine folder
                        (%LOCALAPPDATA%\h2launch\<name>) and window title, for
                        two launchers on one PC.
--pad <0-3|none|any>    Which controller is player 1's (default any: the
                        first connected, until another one is used, a
                        button pressed or a trigger or stick pushed far
                        from rest, while it has been idle for 2 s; a
                        reading that never changes doesn't count).
--slot-return <n>=<v>   Make host slot n's logging stub return v (testing).
--event-return <n>=<v>  The same for an event-manager slot.
--watch <name>=<path>:<type>  Log an engine value (read only) whenever it
                        changes, e.g. --watch state=0xE15048+0x90A8:i32
                        (see --help for the path syntax).
--diag                  More detail in the log: dumps of the options, the
                        variant copy's effect, new calling threads, every
                        event-manager slot's first call, the input sent each
                        second, and two read-only engine values about
                        keyboard polling (the poller's branch flag and its
                        key gate array; read, never written).
--help                  Full help.
--version               The build the exe was made from.
```

`H2LAUNCH_NET_HEAD=<n>` makes the `net:` lines show the first n bytes of
each packet (default 16, at most 4096). The log stays on the owner's PC.

`--set-option` and `--set-profile` take `<offset>=<type>:<value>`, where the
offset is decimal or `0x...` and the type is one of `u8 i8 u16 i16 u32 i32
u64 i64 f32 hex`. They exist so a value can be tried without a rebuild (for
example `--set-option 0x03=u8:0` to match HaloX's player-limit bytes).

## Networked match (milestone 2, being tested)

The engine hands every network packet to its host through host slots 41
(unreliable send), 42 (reliable send) and 43 (receive); it opens no sockets
of its own (seen on the owner's PC). With `--session` the launcher carries
those packets through the relay (`crates/h2relay`, also run inside h2live
on UDP 47050) to the other launchers in the match. Every launcher gets the
same session file and its own `--me`:

```
# Two launchers on one PC: run `h2relay 127.0.0.1:47051` first.
relay = 127.0.0.1:47051
room = 0x5EED000000000001
secure = 0x1234567890ABCDEF
host = 0
machine = 0x4832000000000001
machine = 0x4832000000000002
player = 0x0009000000000001 machine=0 team=0 name=Host
player = 0x0009000000000002 machine=1 team=1 name=Guest
```

```
h2launch --session match.txt --me 0 --instance host
h2launch --session match.txt --me 1 --instance guest --pad none
```

Optional lines: `threshold = <n>` (written to option byte 0x0B), `me =
<index>`, and `key = <32 hex>` (this launcher's member key for a room h2live
issued). The session is written into the game options after the variant
and before `--set-option`: flags 0x48 on the host and 0x00 elsewhere
(bit 0x08 makes the engine host a session; without it the engine searches
for one and joins it, the way system link does), the secure address at
0x50, this machine's own id at 0x58 (a guest gives it in its join-request,
so writing the host's id there gets the guest refused), the machines at
0x60, the players from 0xE8 (XUID, machine id, team, players on that
machine, machine index, controller), the machine count at 0x2F0 and this
machine's id again at 0x2F8. With that, two launchers on the owner's PC
played one match on Lockout through the relay (2026-10-10 05:30 UTC): the
guest's search reached the host, the host answered, the guest joined, and
both loaded the map and played with about 1 MB of game traffic each way. The log has every network call (`net:` lines) and a
summary with the relay's counters at the end.

Later runs there (2026-10-10) showed:

- A match ends by itself only if the variant gives it a time or score
  limit. The variant copy's time limit is at options 0x354 (seconds; 0 is
  none, which `01_slayer` gives) and its score to win at 0x350 (25), so
  `--set-option 0x354=i32:60` makes a one-minute match. At the end every PC
  gets host slot 6 (`set_game_result`, a 0x5D138-byte block), then about
  7 s of scoreboard, then `restart_game(0)`, and the launcher quits.
- A player who quits writes its own result through slot 6 first. In a
  two-player match the host then ends the match normally; with three, the
  match goes on, and a new launcher for the same machine joins it in
  progress.
- Three launchers play one match the same way as two.
- `--variants` loaded all 180 hopper variants. After the variant copy the
  options hold its name at 0x304 (UTF-16), game type at 0x344 (1 CTF,
  2 Slayer, 3 Oddball, 4 King, 7 Juggernaut, 8 Territories, 9 Assault),
  flags at 0x348 (bit 0 is teams on), rounds at 0x34C (0 means one), score
  to win at 0x350, time limit at 0x354 and the player limit at 0x378 (16,
  or 3 and 4 in 2-on-1 and 3-on-1). `H2LAUNCH_VARIANTS_RAW=1` also prints
  every other non-zero dword in 0x340..0x430. h2live's launcher playlists
  now name these variants (`crates/h2live/src/playlists.txt`).

## Matchmaking through h2live (milestone 3, being built)

With `--live <server>` the launcher signs in to h2live (its own key is kept
as `live-key.bin` in its log folder, so each `--instance` is its own
account), searches a launcher playlist (`--playlist`, default 11, Head to
Head) and waits for the match the server makes. The match names the map,
the variant, who hosts, the room on the relay beside the server and this
PC's key for it; the launcher writes the session from that and starts the
engine as with `--session`. While the engine runs it tells the server when
the host is up (HOSTING), when a joining PC has loaded the map (JOINED),
when the game ended (LAUNCHER_RESULT, with each player's team, standing,
score and deaths read from the engine's results block; see
`crates/h2launch/src/results.rs` for what is known of its layout), and that
it left (LEFT_MATCH) if it closes before the end. Kills aren't found in the
block yet, so they go as 0. Every launcher logs the block's players as
`result:` lines; `H2LAUNCH_RESULT_DUMP=<folder>` also keeps a copy of the
block on the PC, for working out more of it (never commit or upload it).

To try it on one PC, start h2live listening on this PC only, then two
launchers:

```
set H2LIVE_BIND=127.0.0.1
set H2LIVE_DATA=C:\h2work\live-data
target\release\h2live.exe
h2launch --live 127.0.0.1 --instance a --name Alpha
h2launch --live 127.0.0.1 --instance b --name Bravo --pad none
```

## The lobby (milestone 4, being built)

`h2launch` with no flags (or `--lobby`) opens a window of its own on Halo
2's start screen; A (or Start) opens the main menu, with XBOX LIVE and
SETTINGS. XBOX LIVE shows the sign-in screen the first time (a gamertag
and the server, h2.mohnjorrow.com by default, kept in `lobby.txt`) and
signs in at once after that, then the playlists with your level in each
and the party. A searches (party leader), X lists the players online to
invite or join, B goes back to the main menu. When the server makes a match the pregame lobby
shows the map, the game and the players for five seconds, then the engine
starts in its own window (`h2launch --session <file> --events ...`, which
tells the lobby on its standard output what the engine does). When the game
ends the carnage report comes up with each player's place, score, deaths
and level, and the level change once the server has counted the game.
Custom Game, the row after the playlists, lets the party leader pick a game
type and a map; the server makes an unranked match of it for the party,
with the leader hosting (LAUNCHER_CUSTOM). Settings, the row after that
(or SETTINGS on the main menu, or X on the sign-in screen), holds the controller settings ("Controls"
below), kept in `lobby.txt` and passed to every engine the lobby starts.
The details and how to test it without MCC are in `lobby.md`.

## Controls

The engine is told the player's controls three ways, all without touching
its code (`crates/h2launch/src/controls.rs`):

- **Buttons**: host slot 116 (`get_player_gamepad_mapping`) returns 66
  bytes, one per game action (host-interface.md 8.5), each naming the pad
  button that does it (0 LT, 1 RT, 2-5 the d-pad, 7 Back, 8/9 the stick
  clicks, 10 LB, 11 RB, 12 A, 13 B, 14 X, 15 Y, 0xFF none). Before this
  the default was all zero, which puts every action on LT: that is why A,
  B and RT did nothing in the first PC test. Start (pause) and the menus'
  buttons aren't in it; the engine reads those from the raw buttons.
- **Sticks**: the launcher moves the axes between the sticks before they go
  into the input state (slot 36/37), the same for the pad and for the input
  script's `stick` lines (they stand for the physical sticks). The
  profile's own `stick_preset` (0x1C9) stays 0, so the engine doesn't swap
  them again; `--set-profile 0x1C9=u8:1` tries the engine's own instead
  (use it with `--sticks default` only).
- **Keyboard and mouse, and the rest**: the profile (slot 34). Its table at
  0x42C gets 66 entries, entry *i* = action *i* and up to five Windows key
  codes (it was all zero before); look sensitivity goes to both 0x1B5 and
  0x1B6, look inversion (the thumbstick's) to 0x1D (0x1E, the mouse's,
  stays 0; see below), automatic look centering to 0x23, vibration to 0x1F
  (and the launcher sends no rumble to the pad when it's off), mouse
  sensitivity to 0x410. `button_preset` (0x1C8), `lefty_toggle` (0x1CA)
  and `swap_triggers_and_bumpers` (0x1D7) stay 0 so the engine applies no
  swap over ours. As HaloX does, the launcher also scales the mouse itself
  (raw counts x sensitivity x 0.0625 x 0.10, so 0.01 at MCC's default
  1.6) and flips its Y when the mouse is inverted, before the motion is
  copied into the right stick: HaloX writes 0x410 only because "vehicle
  code also reads" it.

The button layouts, action by action (`*` an estimate):

| # | action | Default | Southpaw | Boxer | Green Thumb | Bumper Jumper | Recon |
|---|---|---|---|---|---|---|---|
| 0 | jump | A | A | A | A | LB | A |
| 1, 65 | switch grenade, next grenade | RB | RB | RB | RB | A | D-right |
| 2, 3 | action, reload | X | X | X | X | B | RB |
| 4, 13 | switch weapon, dual wield (hold) | Y | Y | Y | Y | Y | Y |
| 5 | melee | B | B | LT | RS | RB | B |
| 6 | flashlight | LB | LB | LB | LB | X | D-up |
| 7 | throw grenade | LT | RT | B | LT | LT | LT |
| 8 | fire | RT | LT | RT | RT | RT | RT |
| 9 | crouch | LS | LS | LS | LS | LS | LS |
| 10 | zoom | RS | RS | RS | B | RS | RS |
| 15 | Banshee bomb | B | B | B | B | RB* | B |
| 20 | scoreboard | Back | Back | Back | Back | Back | Back |
| 21, 22 | vehicle functions 2, 3 | A | A | A | A | LB* | A |
| 24 | boost/e-brake (MCC's name) | LT | RT | B* | LT | LT | LT |
| 49 | left gun (MCC's name) | LT | RT | LT | LT | LT | LT |
| 55 | reload secondary | X* | X* | X* | X* | B* | RB* |
| 56 | previous grenade | - | - | - | - | D-left* | D-left |
| 64 | flashlight alt | - | - | - | - | D-up* | X |

Every other action has no button (movement is on the sticks). Start is
the pause in every layout. Where they come from and how sure each is:

- Default, Southpaw and Green Thumb: Halo 2's own BUTTON LAYOUT screen
  and the Xbox decompilation (high). Boxer: melee on LT and grenades on B
  are high; the left gun on LT medium-high (Vista's pane says "Melee/Use
  Left Weapon"); the boost on B medium: our notes follow Halopedia there
  (controller-plan.md 2.3, as the from-scratch game did), while Vista's
  "Use Left Weapon" on LT may take the boost with it. The PC run settles
  it (below).
- Recon: MCC's Halo 2 column of its default layout (high), which also
  puts "Select Next Grenades" (65) on D-right with "Switch Grenades" (1).
- Every layout puts 65 on the button of 1, as Recon does, so the grenade
  button works whichever of the two the engine reads.
- Bumper Jumper isn't a Halo 2 layout: Halo 3's, as the from-scratch game
  had it. Jump on LB, melee on RB and reload/action on B are high, grenades
  on A medium-high, the flashlight on X medium, and the Banshee bomb and
  vehicle functions following melee and jump onto the bumpers an estimate.
  The d-pad keeps Recon's other flashlight button (up) and previous
  grenade (left), as MCC's universal Bumper Jumper is Recon with jump and
  melee on the bumpers: an estimate. D-right stays free (65 is on A, with
  1).
- libmcc and MCC's settings file name three actions differently (13 swap
  weapon or Dual-Wield, 24 secondary fire or Vehicle Function 1, 49 dual
  wield or Fire Secondary). 4 and 13 always share a button, and so do 24
  and 49 except in Boxer, so the table is right under either naming there.
- The Banshee bomb on B and vehicle functions 2 and 3 on A: Xbox Halo 2
  fixes two actions to A and B in every layout, and MCC's Halo 2 puts
  those there (medium).

Thumbstick layouts (what the engine gets as its move stick and look stick,
from the controller's left stick L and right stick R; Halo 2's screen
calls turning "Rotate Left/Right"):

| layout | move: strafe, forward | look: turn, up |
|---|---|---|
| Default | L.x, L.y | R.x, R.y |
| Southpaw | R.x, R.y | L.x, L.y |
| Legacy | R.x, L.y | L.x, R.y |
| Legacy Southpaw | L.x, R.y | R.x, L.y |

The Legacy ones first snap each stick to its nearer axis, except within 35
degrees (left stick) or 10 degrees (right stick) of a diagonal, as Halo 2
does. Halo 2 snaps right after its square map, with no dead zone before
the snap; the launcher snaps the raw, round range (the engine square-maps
it later), so the angles are measured on a slightly different shape: an
estimate. A stick inside XInput's dead zone (7849 left, 8689 right) on
both axes is left alone, so the snap, which makes the stronger axis up to
1.41 times bigger, can't push a resting stick past the engine's dead zone.

Keyboard and mouse (the profile's table). halo2.dll has keys of its own:
HaloX's reading of it (reference only) found a binding table the engine
fills itself, at halo2+0x15EB7A0 (0x17C4 per player, from +0x1C, 60
actions of 0x64 bytes, in the engine's own action numbering), with Space
jump, Left Ctrl crouch, G throw grenade, Tab switch weapon (and the
scores), Q (and the middle button) melee, E action, the left button fire
and the right button zoom; and the engine's player input code checks W A
S D, R (reload), F (flashlight) and G by key code. So the table gives
those keys the same actions and never another one: F does only the
flashlight, G only throws. The rest are MCC's Halo 2 defaults and work
only if the engine reads the table: the right button for the left gun
(and action 24), 1 also switches weapons, 2 swaps grenades (actions 1 and
65), 4 is also the flashlight, C dual wields (action 13), Q the Banshee
bomb, R both guns' reload, and Left Ctrl and Space the vehicle functions
2 and 3. Z and X, which MCC uses for zoom in and out, are left out:
halo2.dll's own table has Z on an action of its own, and Halo 2 zooms
with one button. The lobby lists the engine's keys plainly and MCC's with
a `?`.

The empty table is not known to be why keys did nothing in the first PC
test. Whether keys reach the engine at all comes first: HaloX's findings
point to the engine's own thread reading key state (the
`AttachThreadInput` question) and to window focus.

Checked on the owner's PC (2026-10-10, ed90716, real engine, scripted
input and a read-only look at the player's biped):

- Default and Bumper Jumper work in matches: jump, crouch, fire, reload,
  melee, grenade, weapon switch, zoom and scores each land on their
  button (Bumper Jumper: LB jumps, RB melees, B reloads). Swapping
  grenades and the flashlight couldn't be seen in that test. A layout
  picked on the lobby's CONTROLLER screen is saved and a match started
  from the lobby uses it. The engine window came to the front in 0.2 s,
  and AttachThreadInput succeeded on its second try.
- Southpaw and Legacy sticks map as in the table; stick look inversion
  works.
- Look sensitivity: a full right stick for 0.5 s turns 60.8 degrees at 1,
  74.3 at 3 and 121.6 at 10, so 1 to 10 is only about twice as fast.
- Mouse sensitivity 0.8, 1.6 and 3.2 give 28.7, 57.3 and 114.6 degrees
  per 100 counts. With 0x1E set the engine flipped the mouse a second
  time, so 0x1E now stays 0 and only the launcher flips it.
- The engine reads 0x42C: its own table (`kb0`) changes with ours, and
  with `--no-key-bindings` W, Space, Left Ctrl and G do nothing. A
  scripted left click didn't fire; not followed up (keyboard and mouse
  are outside the owner's MVP).

Not known yet, to check on the owner's PC:

- Whether keys reach the engine at all: its window must be in front (the
  lobby passes the foreground on, and the window asks for it once a second
  for 30 s until it has it, unless another window is brought to the front
  meanwhile; the log says `the window is in front`), and the engine's
  thread must share the window's input (`AttachThreadInput: engine thread
  ... now shares`). `--diag` logs the input sent each second.
- Whether the engine reads the profile's keyboard table at all. A
  read-only look at its own table is `--watch kb0=0x15EB7BC:hex64`, once
  as it is and once with `--no-key-bindings`: if the two differ, the
  engine reads 0x42C. If they don't, try again with a
  `keyboard_mouse_button_preset` (0x428, which stays 0) set, say
  `--set-profile 0x428=i32:1` (and 2, 3), before concluding that it
  ignores 0x42C: the engine may use the custom table only under some
  preset value (HaloX copies MCC's `KeyboardMouseButtonPreset` there).
  Pressing 1, 2, 4 and C (MCC's keys only) shows it in play.
- Whether MCC's look sensitivity byte uses Halo 2's 1 to 10, and whether
  the zeroed look acceleration (0x1B7) and dead zones (0x1B8, 0x1BC)
  matter. MCC's defaults are said to be 12% axial and radial dead zones
  and look acceleration 5; read `LookAxialDeadZone`, `LookRadialDeadZone`
  and `LookAcceleration` from the owner's GameUserSettings.ini (nothing
  committed) before guessing their units. If a resting pad drifts the
  view, add Halo 2's per-axis dead zone (9000/32767 with rescale,
  decomp #4) before the Legacy snap, or write MCC's values there.
- With `--pad any` (the default), player 1's controller is the first
  connected one until another is used (a button pressed, or a trigger or
  stick pushed far from rest) while it has been idle for 2 s, so an idle
  or virtual controller in an earlier slot can't take over, and a pad
  whose reading never changes can't either (the log says `pad N is in
  use; it is player 1's controller`).

Checks with a controller: hold Y by a magnum to dual wield, LT fires the
left gun, X reloads both, B drops a Banshee bomb, LT boosts a Ghost, A in
the Banshee; in Boxer, which of B and LT boosts a Ghost (B is the guess);
RB swaps grenades once per press (not twice: 1 and 65 share it); the 0xFF
entries cause no stray actions.

## Input script

`--input-script <file>` feeds timed synthetic input to local player 1 through
the engine's input callback (it never presses real keys). One command a line:

```
base state1                 # time 0 = when the engine reports the map loaded
5     stick L 0 1 for 2     # left stick full forward for 2 s
7.5   press A               # tap A (0.15 s)
8     press RT for 1        # triggers can be pressed like buttons
9     trigger L 0.5 for 1
10    key W for 2           # sets keyboard[W] in the input state
14    mouse 300 0 for 1     # 300 mouse counts right, spread over 1 s
15    click left for 0.2
16    screenshot
17    log reached the end
18    quit
```

`stick` lines are the controller's physical sticks, so `--sticks` moves
them as it does a pad's; presses go through the button layout.

`base` chooses time 0: `input` (the engine's first input poll; the default
when there is no `base` line), `state1` (map loaded, as in this example) or
`launch` (program start).

## Building on Linux (for CI or a cross-build)

```
cargo build --release --target x86_64-pc-windows-gnu -p h2launch   # mingw-w64
```

CI builds the whole workspace on Linux; the non-Windows `main` just prints a
line, so the gate passes without a Windows machine.

## What is settled and what is a guess

The background, the exact tables and the open questions are in
`brief.md` in this folder and the research behind it. The launch order, the
engine and data-access vtable slots, the host slots that need real behaviour,
the game-options layout and the profile recipe come from that research
(HaloX's tested behaviour, libmcc's headers, and a static read of the owner's
own `halo2.dll` 1.3528 that confirmed HaloX's offsets for that build; the
Xbox decompilation the project uses elsewhere is of a different build).
Values that are estimates are marked as such in the code:
the stick look sensitivity scale and mouse scale in the profile, the parts
of the button layouts marked in "Controls", the keyboard and mouse
bindings, the Legacy stick snap's shape, and whether keyboard and mouse
work without the engine detours HaloX uses (the launcher tries
`AttachThreadInput` instead). The PC run is what will confirm them.
