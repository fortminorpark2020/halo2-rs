# h2launch milestone 1: implementation brief

Written 2026-10-10 by the thread (the spec step of the research workflow did not
produce a file). This brief records the decisions; the exact tables live in the
research files below and must be read, together with their `.verify.md`
corrections, which win wherever they disagree with the base file.

`SP` = `/tmp/claude-0/-home-claude-halo2-rs/7b79b5a1-e128-5fc1-a97f-78225a2dfd44/scratchpad`

Research (all in `SP/launcher/research/`):
- `host-interface.md` + `host-interface.verify.md`: exports, engine object (11 slots), data access (6 slots), variant objects, host `i_game_manager` (121 slots, section 6.3), event manager (151 slots, section 7), data layouts (section 8: profile 0xACC, input 0x130, rumble, gamepad mapping, video/audio settings, font glyph), options (section 9), threads (12), defensive design (13), Rust ABI notes (14).
- `launch-sequence.md` + `launch-sequence.verify.md`: process setup, load order, window/D3D11/swap chain, frame loop, input, pause/resize/quit, files, audio, crash points, and the ranked "most likely to break the first run" list plus C1-C22 filled-in items (event manager, universal stub and non-zero slots, profile recipe, groundhog fallback, window-thread rule, XUID rule...).
- `game-options.md` + `game-options.verify.md`: the 0x2BF30-byte packed `s_game_options`, recipe (a) offline Lockout Slayer, map ids (Lockout = 44), variant loading through data access.
- `networking.md`: host network slots 41-43 (milestone 2; log only now).
- `results-ui.md`: set_game_result, events, fonts, profile appearance (milestone 3; log only now).
- `SP/launcher/pc-inventory.md`, `SP/launcher/pc-static-1.md`: John's install and the static facts read from his halo2.dll 1.3528 (sections, imports, delay-imports, function starts, strings).

## Goal of milestone 1

On John's PC (Windows 11, RTX 5080, MCC Steam build 2025.08.16, halo2.dll
1.3528.0.0), `h2launch.exe` with no arguments opens a window, loads MCC's
classic Halo 2 engine from his install, and starts an offline multiplayer
Slayer match on Lockout with one local player; the engine renders and plays
until the match ends or the window is closed. Gamepad (XInput) must work;
keyboard and mouse should, through the no-patch routes below.

## Hard decisions

1. **No code patches and no hooks into halo2.dll (or groundhog.dll).** The
   launcher talks to the engine only through what MCC's own shell uses: the DLL
   exports (`SetLibrarySettings`, `CreateDataAccess`, `CreateGameEngine`), the
   engine / data-access / variant objects they return, and the host interface
   and event manager we implement. Everything else is documented Windows API.
   No detours, no IAT patching, no byte writes into the module, no calls to
   internal engine functions by RVA. Read-only diagnostics (logging a value at a
   known RVA behind a `--diag` flag) are allowed. This keeps us on the same
   contract MCC uses and makes MCC patches less likely to break us.
2. **Keyboard and mouse without hooks**, in this order (launch-sequence.verify
   C9 routes 1-3): fill `keyboard[256]`/mouse in the input state; rely on the
   engine's GetAsyncKeyState branch while our window is foreground; and
   `AttachThreadInput(game thread id, window thread id, TRUE)` (game thread id
   from `GetThreadId` on the handle `initialize_game` returns; retry until it
   succeeds once the game thread has a message queue) behind a default-on flag
   `--attach-input` (`--no-attach-input` to compare). Route 4 (detours) is out.
3. **Launch order = HaloX's tested order** (launch-sequence.md section 0 as
   corrected by its verify file): DLL search setup (`SetDefaultDllDirectories`
   + `AddDllDirectory` for MCC root, `<root>\halo2`, `<root>\MCC\Binaries\Win64`),
   current directory = MCC root, window + D3D11 device + swap chain first, then
   `LoadLibraryExW` halo2.dll and `CreateDataAccess`; then on a launch worker
   thread: `SetLibrarySettings("en-US" x3)`, `CreateGameEngine`,
   `initialize_graphics(device, context, swapchain, NULL)`, optional groundhog
   (below), `preload_common_begin(device)`, `preload_level_begin(-1)`, build the
   options, `initialize_game(host, options)`. Window thread keeps pumping
   messages and never touches D3D or presents while the engine runs.
4. **groundhog.dll**: default OFF. Flag `--groundhog` loads
   `<root>\groundhog\groundhog.dll` and runs only its exports the way MCC does
   (SetLibrarySettings, CreateGameEngine, initialize_graphics(dev, ctx, swap,
   NULL), never initialize_game), after halo2's initialize_graphics and before
   the preloads (verify C4). If the default run fails, the PC test tries this.
5. **Host object**: vptr at +0, event manager pointer at +8, zeroed to at least
   0xB788 bytes total. Vtable of 256 entries: slots 0-120 per host-interface
   section 6.3 with all verify corrections; slots that need real behaviour per
   launch-sequence.verify C2 (10, 32-34, 36-37, 46-49, 51, 52, 59-61, 68, 62-67
   if fonts are on, 88, 97 returns its 2nd argument, 116 per A13); every other
   slot (including 121-255) is its own distinct logging stub that logs its index,
   thread id and first four integer arguments on its first call (then counts)
   and returns 0. Slots that return f32 (38, 69, and any other the tables mark)
   get `-> f32` functions so XMM0 is set. `extern "system"` with an explicit
   `this` first argument for every slot.
6. **Event manager**: static object, 151-entry table of logging return-0 stubs
   (mind the MSVC overload ordering of slots 4/5), slot 147 `GetGUID` returns a
   pointer to a static non-zero GUID; leave out-parameters untouched.
7. **Player and profile**: one random XUID per run (or a fixed one from
   `--xuid`) used everywhere: options player[0], slot 88, slots 34 and 116
   (verify C22). Profile per verify C3 / host-interface 8.1 with non-zero look,
   zoom and vehicle multipliers (1.0), volumes 1.0, FOV 0 (default), gamertag
   from `--name` (default "Player"). Static buffers that outlive the call.
8. **Game options**: recipe (a) from game-options.md with all game-options.verify
   and host-interface.verify A1/A2 corrections (packed 0x2BF30 buffer; zero
   everything, NOT -1 defaults; load the variant `.bin` bytes, data access slot
   4, variant slot 7 copy-to-options FIRST, then write map id 44 at 0x10 and the
   16-byte map id at 0x14, game mode 3 multiplayer, flags with bit 3, tick 60,
   host_address 123, player_count at 0xE8 = 1, player[0] at 0xF0 (stride 0x20),
   peer_count at 0x2F0 = 1, and whatever else the recipe lists). Build it with a
   small safe byte-buffer writer with named offset constants and unit tests that
   pin every offset. Variant default `halo2\hopper_game_variants\01_slayer.bin`;
   `--variant <name>` and `--map <name>` (map name to id table from
   game-options.md) for later.
9. **Fonts**: setting 6 = false by default (engine uses its own text path);
   `--host-fonts` turns it on and serves h2_fonts via the font slots only if
   that is simple; otherwise log the font calls. Log what the engine asks for.
10. **Network slots 41-44**: log and return 0 (milestone 2 adds the relay; the
    `h2relay` crate is being written on another branch, don't depend on it yet).
11. **Results/events**: log set_game_result (slot 6) size and call time, and
    set_game_state values; nothing more yet.

## Diagnostics (the PC session reads these back as text)

- Log file `h2launch.log` in `%LOCALAPPDATA%\h2launch\` (also printed to a
  console when started from one): timestamps, thread ids, every launch step with
  its result, every host slot's first call and a per-slot call-count summary
  every 10 s and at exit, set_game_state transitions, Present count if
  observable (V12: `IDXGISwapChain::GetLastPresentCount` in end_frame).
- Crash reporting: `AddVectoredExceptionHandler` logging access violations,
  illegal instructions, stack overflow and other fatal codes (ignore the usual
  first-chance noise like 0xE06D7363 C++ exceptions, 0x406D1388 thread naming,
  DBG_PRINTEXCEPTION), with faulting module + RVA, registers and a short stack
  of return addresses that fall inside loaded modules (module+RVA). Also
  `SetUnhandledExceptionFilter` to log and exit cleanly. Engine calls from our
  code: Rust can't catch SEH; logging plus process exit is fine for now.
- `--check`: no engine start. Prints the MCC root it found (Steam
  `libraryfolders.vdf` parsing, `HALOX`-style fallbacks, `--mcc <path>`), halo2
  file list, halo2.dll FileVersion, SHA-256 (expected
  DE65B4F4FDBF3F0A5EAB7431FE530DA17DD815599182DFD6AE9B7E21CF171946: warn, don't
  abort, on mismatch), PE TimeDateStamp/SizeOfImage, export names, the variant
  and map files it will use, and the D3D11 adapter.
- Unattended test switches: `--quit-after <secs>` (posts quit, waits, logs
  whether the game thread exited, exits with code 0 if the launch reached a
  running game state), `--screenshot <secs>[,<secs>...]` (copy the back buffer
  in end_frame to a PNG in the log folder at those times; images stay on
  John's PC), `--input-script <file>` (timed synthetic gamepad/keyboard input
  fed through the input-state slot, e.g. hold left stick forward 2 s, press A),
  `--windowed WxH`.
- Exit codes and a final one-line summary in the log
  (`RESULT: in-game=<yes/no> frames=<n> crashed=<code or none>`).

## Crate shape

- `crates/h2launch`, binary `h2launch`. Windows code under `#[cfg(windows)]`;
  on other platforms `main` prints that it runs on Windows only, so the
  workspace gate passes on Linux CI.
- Platform-independent modules with unit tests that run on Linux: options
  buffer builder (offsets pinned), profile builder, Steam VDF parser, PE header
  /export/version parser for `--check`, input-script parser, map name table,
  log summary formatting.
- Use the `windows` crate (pick a current version compatible with the
  workspace's lockfile; list exact features) or `windows-sys`. It must build for
  `x86_64-pc-windows-msvc` (John's PC: Rust 1.99, VS Build Tools 2022) and
  `x86_64-pc-windows-gnu` (this container, mingw: `cargo build --release
  --target x86_64-pc-windows-gnu -p h2launch`). Release profile with debug
  symbols kept (so crash RVAs in our own code can be mapped).
- Keep the repo rules: no MCC files, bytes or disassembly in the repo; facts
  in our own words; never paste HaloX/libmcc/BCS/Cartographer code.
- `docs/notes/launcher/` in the repo: copy of this brief plus a short
  `README.md` for the crate (what it does, flags, how to run on the owner's PC).
  Don't copy the research files themselves into the repo yet.
