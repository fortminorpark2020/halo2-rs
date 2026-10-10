# h2launch

`h2launch.exe` starts MCC's classic Halo 2 engine (`halo2\halo2.dll`) from
your Master Chief Collection install, without MCC, straight into an offline
multiplayer Slayer match on Lockout with one local player. A gamepad works;
keyboard and mouse should. It is a Windows program; on other systems it
prints that it runs on Windows only and exits (so the Linux CI build passes).

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
by hand instead, run it with no arguments and close the window to quit.

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
--pad-map <zero|h2>     Gamepad mapping handed to the engine (default zero,
                        which is what HaloX uses).
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
--pad <0-3|none|any>    Which controller is player 1's (default any).
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
the stick look sensitivity and mouse scale in the profile, the Halo 2 default
gamepad mapping behind `--pad-map h2`, and whether keyboard and mouse work
without the engine detours HaloX uses (the launcher tries `AttachThreadInput`
instead). The PC run is what will confirm them.
