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
--diag                  More detail in the log: dumps of the options, the
                        variant copy's effect, new calling threads, every
                        event-manager slot's first call, the input sent each
                        second, and two read-only engine values about
                        keyboard polling (the poller's branch flag and its
                        key gate array; read, never written).
--help                  Full help.
--version               The build the exe was made from.
```

`--set-option` and `--set-profile` take `<offset>=<type>:<value>`, where the
offset is decimal or `0x...` and the type is one of `u8 i8 u16 i16 u32 i32
u64 i64 f32 hex`. They exist so a value can be tried without a rebuild (for
example `--set-option 0x03=u8:0` to match HaloX's player-limit bytes).

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
