# The launcher's Halo 2 menus

The plan for re-creating the original Xbox Halo 2 menus in the launcher.
Phases 0 and 1 were built on `launcher-menu` (based on
`launcher-friends`): the lobby opens on Halo 2's start screen and main
menu, drawn from the tags. Phase 2 (widened, section 7) goes on on
`launcher-menu2`, one sub-phase at a time.

The owner asked (2026-10-10): "I also want to re-create the original halo 2
menu from the halo 2 xbox version", and "make the main menu a priority". At
21:45 UTC the same day he reported that in the build he had just tried
neither the controller nor the keyboard did anything, and asked for button
layouts such as Bumper Jumper to be built in. Both are covered here
(sections 6 and 9.1).

Today the lobby is a plain window of our own (`crates/h2launch/src/lobby`:
winit, softbuffer, drawn on the CPU), with Halo 2's fonts from MCC's
`halo2\h2_fonts` and the rank icons from MCC's `mainmenu.map`. The goal is
for it to look and behave like Halo 2 on the Xbox, from the start screen to
the carnage report, while the h2live features stay as they are.

Sources, with how sure each is:

- **H**: read from the Xbox retail decompilation
  (`/home/claude/kirklandsig/halo2-decompiled`, CC0), or from the classic UI
  tags that code uses.
- **M**: tag text or layouts from Halo 2 Vista's files, or the Halo 2
  manual (pp. 11-15). Vista may differ from the Xbox.
- **L**: memory or inference.

Earlier work this builds on:

- `docs/notes/mainmenu-plan.md`: the menu scene, the UI tags and the
  coordinate system.
- `origin/menu-preview` (896a83d): the from-scratch game's re-creation of
  Vista's menu (`blam-cache/src/ui.rs`, `h2viewer/src/menu.rs`, `menuart.rs`,
  `flythrough.rs`, `intro.rs`).
- `mcc-maps.md`: the format-13 reader.

## Rules for every part

- **No game files in the repository.** No MCC or Vista bytes, dumps, pictures,
  sounds or movies. Screenshots that show Halo 2's art or fonts stay on the
  owner's PC. Positions, sizes and colours written as facts are fine (as
  `mainmenu-plan.md` and menu-preview already do).
- **Text.** Short labels (SETTINGS, BUTTON LAYOUT, START GAME) are written in
  code. Halo 2's longer texts (help lines, status sentences) are read from
  the player's own `mainmenu.map` at run time. Where that can't be read yet,
  we write our own short words instead.
- **No patches, hooks, byte writes or calls by RVA into halo2.dll.** If a
  step can only work with one, it stops and the owner is asked.
- **No campaign.** The CAMPAIGN row and every "Switch To: Co-op" item are
  left out.
- **Tests need no game files.** UI tags are made up in tests, as
  `mcc::synthetic` does for the rank icons.
- **The server stays as it is.** Phases 1 to 3 change only how the lobby
  looks and moves between screens: no new message kinds, and `LIVE_PROTOCOL`
  stays 3. Anything that would need the server (a CLOSED party, delaying a
  countdown) is marked as later.

## 1. The target

### 1.1 The flow

```
(intro movie, if one is found)
start screen ── A / Start ──> (first run: type a gamertag) ──> main menu
main menu:
  XBOX LIVE ──> "Please Wait" (signing in) ──> XBOX LIVE screen
      Quickmatch ─────────────> MATCHMAKING ──> loading ──> game ──> CARNAGE REPORT
      Optimatch ──> playlists ─> MATCHMAKING                                   |
      Create Party ──> PREGAME LOBBY <──────────────────────────────────────────┘
                         START GAME (leader): matchmaking, or a custom game
                         GAME SETUP: playlist, or map and game type
  SPLIT SCREEN ──> a local PREGAME LOBBY ──> an offline game on this PC
  SYSTEM LINK  ──> later (section 1.4)
  SETTINGS ────> Player Profile (Controller, Appearance, ...)
Y on any Live screen: the players list (FRIENDS | PLAYERS)
```

B goes back one screen everywhere:

- On the main menu, B asks before signing out and goes back to the start
  screen.
- On the start screen, B (or Esc) asks before quitting. That is a PC extra;
  the Xbox had no quit.
- Closing the window always quits.

### 1.2 Screens

The screen numbers are the Xbox's, from the decompilation. Vista's tags
number some screens differently, so the reader finds screens by tag name,
not by number. "Today" names the lobby's current screen (`app.rs`, `Screen`)
that each one replaces.

| Halo 2 screen | Here | Today | Phase |
|---|---|---|---|
| Start screen (0x9): logo, tracks, pulsing "PRESS START" | as Halo 2, with A, Start or Enter to go on | (none) | 1 |
| Main menu (0x6): CAMPAIGN, XBOX LIVE, SPLIT SCREEN, SYSTEM LINK, SETTINGS | the same rows without CAMPAIGN (see 1.4 for System Link) | (none) | 1 |
| Select Player (0xC1) and Profile Name keyboard (0xA7) | the gamertags used on this PC, plus a new one typed in | SignIn | 2 |
| "Please Wait" (0xB6), "SIGNING INTO XBOX LIVE" | while connecting to h2live | Connecting | 2 |
| XBOX LIVE (0xBA): Quickmatch, Optimatch, Create Party, Content Download | the first three; picture and help line per row, "Latest news" from the server's message of the day (sent with CHALLENGE) | Live (its list half) | 2 |
| Choose matchmaking playlist (0xDB; 0xED from the lobby) | our launcher playlists: name, your level icon, "N players in this playlist" (the icon and the count are our additions) | Live (playlists) | 2 |
| PREGAME LOBBY (0xE): START GAME, GAME SETUP, mode block, party status, 16-row player list | the party hub | Live (party panel), Party, Custom | 2 |
| Game Setup (0x17), Party Privacy (0x19), Select New Leader (0xB1), Boot Player (0xB8), Select Map (0xCE), Select Game (0xCF) | as Halo 2, over our party and custom-game messages | Party, Custom | 2 |
| Y menu players list (0x1A): FRIENDS, CLAN, PLAYERS tabs | FRIENDS and PLAYERS (everyone signed in); level icon and status columns | Friends, Players | 2 |
| Player options (0x1F) | Join Party, party invite, friend request, Remove Friend, Make Party Leader, Boot From Party, and View Gamer Profile (our service record) | popups | 2 |
| (no Xbox screen) | the service record, as a Halo 2 sub-screen, reached from the player options | Record | 2 |
| SETTINGS (0x13), Edit Player (0x26), Controller (0x27), Button Layout (0x28), Thumbstick Layout (0x29) | as Halo 2, plus PC rows (mouse, server address) | (none) | 2 |
| MATCHMAKING (0xD2): three steps, status line, players by team, "Game starts in N" | our search, the server's stages and the pregame roster | Searching, Pregame | 3 |
| Lobby countdown, START GAME in a custom game | counted down in the leader's lobby, then the custom game starts | Custom | 3 |
| LOADING (0xD1), PLEASE WAIT (0xE7) | while the engine starts and loads | InGame | 3 |
| POSTGAME CARNAGE REPORT (0x10; 0xB9 from the lobby) | pages of our results | Carnage | 3 |
| Dialogs: OK (0x8), OK/Cancel (0x7), large error (0xF0), multiple choice (0xE6) | every popup and failure | Failed, popups | 2 |
| Invite and message notice (0xC0) | lower-left notice with the gamertag | notices | 2 |
| In-game GAME MENU (0xC3) | only with the end_frame overlay (section 2) | B in the lobby window | 4 |
| Intro movie, attract movie after 75 s idle | only if the player has a movie file | (none) | 4 |

The XBOX LIVE screen lays out as follows (H tags):

- the list on the left;
- a picture and a help line on the right that change with the row;
- "Latest news:" and a message box below;
- at the lower left, a callout saying Y opens friends and recent players at
  any time. Here, as in Halo 2, Y opens the players list on every Live
  screen.

These Live screens seem to keep the Xbox's 640 by 480 coordinates, centred
on the screen (L). That is checked before their numbers are reused.

The lobby's current keys move to Halo 2's:

- A or Start selects; B or Back goes back.
- The d-pad or the left stick moves. The stick counts only past about 90%
  (H; the lobby uses 60% today). Holding repeats every 250 ms (H).
- X opens options (Y menu) or changes teams (lobby).
- In the Y menu, LB and RB change tabs, and so do Left and Right. Halo 2's
  own choice isn't known (L).
- On the carnage report, Left and Right change pages (M).

Today LB opens a service record and RB the friends list. Those shortcuts
stay as PC extras but leave the button legends.

### 1.3 What every screen keeps (H unless marked)

- **Background.**
  - The start screen and main menu show the flythrough in full.
  - Every other screen sits on `game_shell_background`: the navy
    `framing_center` with slow horizontal track lines, leaving a soft window
    onto the scene.
  - Dialogs dim what is behind them with the overlay colour
    argb(0.85, 0, 0.08, 0.17) (M).
- **Header** at the top left, in the `title` font, colour (0.68, 0.76, 0.85).
  It comes in four sizes (full, large, half, quarter) from the globals'
  rectangles.
- **Button legend** at the bottom right, picked by the screen's legend
  number. The glyphs U+E100 to U+E105 (A, B, X, Y, Black, White) come from
  Halo 2's font.
  - After a keyboard key, the legend uses key names instead. Those legends
    are built in code, because Vista's keyboard variants are mangled in the
    tags.
  - The lobby doesn't yet track whether a pad or the keyboard was used
    last; 2.6 adds that (section 7, Phase 2's corrections).
- **Transitions.** Fades of 250 ms. Pieces slide in from the right one after
  another over 180 to 320 ms, and slide out to the left when a screen
  closes.
  - The key timing inside each animation was never decoded, so keys are
    spread evenly until it is (as in menu-preview).
- **List focus.** The focused row is at full alpha and the others at 50%
  (skin 11 on the main menu). The fades are 120 ms in, 200 ms out, and
  90 ms for hover.
- **Sounds and music.** See section 5.

### 1.4 Left out, and why

- **Campaign.** The CAMPAIGN row, "Switch To: Co-op", saved games and the
  credits movie are left out: the project has no campaign.
- **Xbox Live account screens.**
  - Pass code, "Register New Account", the ESRB notice, Content Download,
    the build number and Game Demos.
  - Signing in is to h2live with a gamertag.
- **Clans, messages, voice and feedback.**
  - The CLAN tab, the Msg and Voice columns, Send Message, Mute, and Leave
    Feedback.
  - h2live has none of them. They can come back if it gains them.
- **Engine-drawn 3D in the UI.** The spinning Halo ring on the matchmaking
  screen and the 3D player on Appearance are `model_scene`s, which the
  engine's own UI code draws. Our screens can't draw them unless option B
  is built (section 2).
- **SYSTEM LINK, for now.** The launcher has no LAN games.
  - Until it does, the row is hidden. Showing it with an error dialog, as
    the Xbox did without a cable, is the other choice. That is the owner's
    call (section 11).
- **SPLIT SCREEN with several pads.**
  - The row opens a local lobby and an offline game (`--offline` with
    `--map` and `--variant`).
  - It is one player, unless a PC check shows the engine takes a second
    local player.

## 2. How it is drawn: three options

### (A) The engine draws the real menu scene; we draw the UI over it

- **What it is.** halo2.dll runs `mainmenu.map` in UI-shell mode
  (`options::mode::UI_SHELL`, 4). Its real flythrough, with particles,
  bloom and lighting, is the background. Our UI is drawn into the engine's
  frame in the `end_frame` host callback (`win/gfx.rs`, `on_end_frame`)
  with a small Direct3D 11 renderer.
- **Process.** The menu engine is a child process, as a match engine is
  (`h2launch --menu-engine`).
  - The lobby keeps `App`, the h2live link and all menu state, so an engine
    crash doesn't lose the party.
  - Each frame the lobby sends the child a draw list on its standard input,
    where `quit` goes today. A draw list is at most tens of kilobytes. The
    child loads the art itself, from the same files.
  - The child forwards the player's input back to the lobby, and gives the
    engine neutral input in slot 36 so the engine stays idle.
  - For a match, the lobby closes the menu engine and starts the match
    engine, as now. After the game, it starts the menu engine again and
    shows the carnage report over the scene.
- **The renderer** is about 800 to 1,200 lines (estimate):
  - shaders compiled at run time with `D3DCompile` from
    `d3dcompiler_47.dll`, which ships with Windows;
  - plain, multiply and additive blending;
  - wrap and clamp samplers, chosen for each axis (art scrolling across
    is clamped up and down);
  - texture atlases for art and glyphs;
  - the engine's context state saved and put back each frame
    (`ID3D11DeviceContext1::SwapDeviceContextState`).
- **Fidelity:** the highest. It is the real scene in MCC's renderer.
- **Risks:**
  - UI-shell mode may only boot with a patch. An earlier project patched
    the map table to boot it (`pivot/mcc-engine-hosting.md`).
  - The engine's own classic UI may draw, take input or crash.
  - A state leak would spoil the engine's next frame.
  - It is Windows-only and tied to build 1.3528.
- wgpu can't be used here: it has no D3D11 backend, so it can't draw on the
  engine's device. The renderer uses the `windows` crate, whose D3D11 and
  DXGI features h2launch already turns on.

### (B) We render the flythrough ourselves with wgpu

- **What it is.** Pull a backdrop-only renderer out of menu-preview's
  h2viewer: the level, sky, scenery, shader and fog paths, plus
  `flythrough.rs` and `menuscene.rs`. Then move the lobby window to wgpu.
- **Cost.**
  - It means real refactoring: `scene.rs` and `gpu.rs` are tied to h2sim,
    rigs and audio.
  - For Vista's maps, about 1 to 2 weeks (estimate).
  - For MCC's format 13 it needs the whole geometry, shader and lightmap
    reader, none of which exists: several weeks plus research. Until then,
    players with only MCC get no scene.
- **Fidelity:** medium-high. There are no particles or bloom, and the
  70 degree field of view is unverified.
- It also revives the renderer the pivot meant to stop maintaining.

### (C) Halo 2's 2D UI over a still background

- **What it is.** Every screen is drawn from the UI tags in our own window.
  - The start screen and main menu get the navy framing or a dark gradient
    instead of the scene.
  - No video of the scene exists to ship, and rendering one would be
    derived footage, which the asset rules forbid.
- **Fidelity.** Sub-screens look close to Halo 2, because the framing hides
  most of the scene there anyway. The start screen and main menu lose the
  moving carrier, which is the most recognisable part.
- **Risks:**
  - the format-13 gaps in section 3;
  - CPU drawing speed. Six to eight full-screen layers at 1080p may be too
    slow on the CPU (estimate, to be measured). The fix is a D3D11 quad
    backend for the lobby window, the same renderer A needs.
- **A later variant.** Grab one frame of the scene from the engine in
  UI-shell mode at run time and keep it only in the player's launcher
  folder. That needs only A's first proof.

### Recommendation

1. **Build C's pieces first.** Every option needs them:
   - the format-13 UI tag reader;
   - a shared UI crate that turns screens into a draw list with no GPU;
   - the CPU backend for tests.
2. **Run A's proofs on the owner's PC alongside.** That experiment is
   already under way. If they pass, the same draw list goes into
   `end_frame`, and the start screen and main menu get the real scene.
3. **Keep B as a fallback only**, for the case where A fails and the owner
   wants the moving scene. It would cover Vista owners first.

### The decision rule

The PC experiment answers two questions:

- Does halo2.dll run `mainmenu.map` in UI-shell mode by itself?
- Does a quad we draw in `end_frame` show up and leave the next frame
  untouched?

| What the PC shows | What we do |
|---|---|
| UI shell boots with no patch and plays the flythrough at 60 fps. Our quad shows, and the next frame is clean. The engine draws no UI of its own, or only what our framing covers. | **A.** Every shell screen is drawn in the menu engine's window. The lobby window stays as the fallback and for tests. |
| UI shell boots, but our overlay can't draw or spoils frames. | **C** in our window. The still-frame variant can come later. |
| UI shell boots, but the engine shows its own menus or takes input that can't be stopped without a patch. | Ask the owner. **C** until he answers. The engine's own menus lead to MCC's online code, which we don't drive. |
| UI shell needs a patch to boot. | **C.** Ask the owner whether a patch is acceptable. **B** only if he wants the moving scene and has Vista. |
| The overlay works in a match engine, whatever UI shell does. | Use it in phase 4 for the in-game GAME MENU and invite notices. |

Two more things to measure before choosing A:

- **The time to swap engines.** That is closing the menu engine, starting a
  match engine, and starting the menu engine again after the game. If it
  is long, the carnage report shows in the lobby window while the menu
  engine reloads.
- **Whether the engine plays `main_menu_music` by itself in UI shell.** Its
  UI code starts the music, so it may not play (section 5).

### Shape of the code

- `blam-cache` keeps `ui.rs` (brought over from menu-preview; it is ours)
  and `font.rs` (already here).
- **New crate `crates/h2ui`, with no GPU.** Its modules:
  - `layout`: Halo 2's UI space (origin at the centre, +y up, about 1,200
    units tall; the 16:9 safe area is about ±1067 by ±600), header and
    dialog rectangles, and the layout numbers menu-preview kept as
    fallbacks (`Place`, the pregame and browser boxes, `LEGENDS`).
  - `anim`: screen and item animations, the focus fades and the pulse.
  - `text`: glyph layout in Halo 2's fonts, with the advance measured from
    the glyph's left edge (`advance + origin x`), as the lobby found on the
    real fonts. menu-preview's `menuart.rs` still has the old spacing bug.
  - `art`: a picture-source trait, implemented for MCC (`mcc::Map` with
    `Textures`) and Vista (`MapSet`).
  - `paint`: menu-preview's `Painter`, now writing a `DrawList`. That is a
    list of textured quads with a blend mode (plain, multiply or additive)
    and a wrap flag for each axis, plus solid fills. Text arrives as glyph
    quads, so a backend draws only quads.
- **Backends.**
  - The lobby's CPU `Canvas`, for Linux, CI and headless PNGs. It works in
    gamma space, so menu-preview's linear-colour conversion is not ported.
  - D3D11 on Windows, for the lobby window if the CPU is too slow, and for
    `end_frame` under A.
- **The lobby's `App` stays the state machine.** menu-preview's `menu.rs`
  (6,596 lines) is tied to h2viewer and is not ported; it serves as a
  reference for the screen ages and fades.
  - `app.rs`'s `draw_*` functions (from line 1755) are replaced screen by
    screen with ones that build the draw list.
  - New `Screen` values: `Start`, `Main`, `Settings` and the settings
    sub-screens.

## 3. What comes from Halo 2 Vista and what from MCC

Since Phase 2 (widened, section 7), Halo 2 Vista as Project Cartographer
installs it comes first, and MCC second: Vista has everything the menus
use (music, sounds, the scene and the intro too), and MCC's own files
have the same art, layouts, fonts and rank icons, which is what friends
without Cartographer get. One finder (`lobby/sources.rs`) picks the
Vista folder: `H2LOBBY_VISTA` (the folder or its `maps`, or `off`), else
`vista =` in `lobby.txt`, else the usual folders (`ranks::MAP_DIRS`).
The log says which source each piece came from.

| What | First | Second | Last |
|---|---|---|---|
| Fonts | Vista `maps\fonts` (2.1) | MCC `halo2\h2_fonts` (done) | a system font (done) |
| Screen layouts (UI tags) | Vista `mainmenu.map` | MCC `mainmenu.map`, format 13 | numbers in `h2ui::layout` |
| UI pictures | Vista `mainmenu.map` | MCC `textures.dat` | flat shapes in Halo 2's colours |
| Rank icons | Vista (done) | MCC (done) | numbers (done) |
| Map pictures and names | Vista | MCC `mainmenu.map` (`matg` UI level data) | names from `names.rs` |
| Help and status texts | Vista's string tables | MCC's | our own short words |
| Button glyphs | Halo 2's fonts (U+E100 to U+E105) | | a letter in a circle |
| UI sounds and music | Vista's sound tags (menu-preview's decoders; 2.2) | MCC's, once researched (section 5; later) | silent |
| The flythrough | our renderer on Vista files (2.4, 2.5) | | the still background (C) |
| Intro movie | Vista `movie\intro_60.wmv` (Media Foundation; 2.3) | | skipped |

Settings in the same style as `H2LOBBY_RANKS`:

- `H2LOBBY_VISTA=<folder>` names the Cartographer folder (or its
  `maps`), and `off` uses none: menus, fonts and rank icons from MCC.
- `H2LOBBY_MENU=<map or folder>` points at another `mainmenu.map`, and
  `off` gives the flat look. Tests use the flat look.
- `H2LOBBY_SOUND=off` silences the menus (from 2.2).

### What the format-13 reader still needs

`blam_cache::mcc` today finds tags, reads meta, and decodes single-chunk
A8R8G8B8 images. The UI tags very likely use Vista's layouts unchanged in
format 13.

The model behind that: tag blocks, tag references and data references stay
8 bytes, and runtime pointers widen to 8. It reproduces the PC's 168-byte
bitmap entry and `bitm`'s bitmaps block at 0x44. It also gives the same
offsets as menu-preview's `ui.rs` for:

- `wgtz`, `wigl` and `wgit` with every sub-block;
- `skin`, `unic`, and `matg`'s UI level data.

So the plan is a `ui::Reader` implementation for `mcc::Map`, after which
`ui.rs`'s parsers run unchanged. What `mcc.rs` must gain:

1. **Nested blocks:** the count at `at`, the address at `at + 4`, counted
   from the tag index's start. This is proven for `bitm`; it needs checking
   for nested `wgit` blocks.
2. **Tag references:** group then datum, 8 bytes. Expected; not seen yet.
3. **The string-id table**, predicted in the 0x380 header: count at 0x30,
   data offset 0x34, data size 0x38, index offset 0x3C (a u32 per string).
   The id packing is Vista's (length in the top 8 bits, index in the low
   24). Index numbers may differ from Vista's, so match strings by name.
4. **The English string table**, for help texts. Use the header's locale
   globals at 0x2E4/0x2E8 when they aren't 0xFFFFFFFF, otherwise `matg`
   +0x190. Index entries are 8 bytes (string id, offset). This is optional,
   since our own words stand in.
5. **Bitmap sequences** (block at 0x3C, 0x3C bytes each, with sprites), for
   frames such as `track_brace` and the button pictures.
6. **`textures.dat` records in more than one chunk.** `mcc.rs` refuses them
   today, and `framing_center` (2048 by 1303) is the likeliest to need
   them. Also the row pitch, and the tile mode.
7. **Pixel formats beyond A8R8G8B8:** DXT1/3/5, A8, AY8, A8Y8, A4R4G4B4,
   P8. The Vista decoders in `blam_cache::bitmap` apply if the format
   numbers match.
8. **Pointers with a top bit set.** `mcc.rs` refuses them now. If UI images
   use them, try them with the bits masked off, as Reclaimer does.

## 4. Text and wording

- Short labels and keyboard legends are written in code, in Halo 2's
  capitals.
- Halo 2's longer texts are read at run time where the reader can (point 4
  above).
- **MCC's `mainmenu.map` may hold the Xbox wording rather than Vista's.**
  Examples: "PRESS START" against "PRESS ANY KEY TO CONTINUE", and "XBOX
  LIVE" and "SYSTEM LINK" against "LIVE" and "NETWORK". Check G in section 9
  dumps its main-menu and Live-menu strings to the console. We use the Xbox
  words either way; the check says whether they can come from the file.
- **ONLINE, not XBOX LIVE.** The owner decided (2026-10-11 03:20) that
  the launcher doesn't use Microsoft's Xbox Live service, so wherever
  Halo 2 said XBOX LIVE the launcher says ONLINE, in the same font and
  style: the main menu row, the screen headers and the sign-in wording.
  - `pivot/legal.md` asks that nothing suggest Microsoft made or backs the
    launcher; the sign-in screen also says, in small print, that this is a
    private server.
  - The Xbox Live logo is never drawn. The `xbox_live_menu` pictures are
    used only if a PC check shows they don't carry it.

## 5. Sounds and music

What Halo 2 does (H):

| Event | Sound |
|---|---|
| focus moves | `sound\ui\cursor1` |
| select | `forward1` |
| refused | `flag_fail` |
| screen opens | `advance` |
| back | `back1` |
| Y menu tab | `forward1` |
| message or invite arrives | `receive_message` |
| lobby countdown, each of 3, 2, 1, 0 | `countdown_for_respawn` |
| a key on the on-screen keyboard | `virtual_keyboard_click` |
| the start screen goes on to the main menu | `transport` (0x22f28c) |

**The music** is `sound\ui\main_menu_music\main_menu_music`: an `in` track,
then a `loop`, at its track's gain (-6 dB in Vista's).

- It starts or stops at most once every 6 s (0x235b9c); a change asked
  for sooner waits. Each start is a new looping sound, from `in` again.
- Stopped, it fades out over its track's own fade-out (5.0 s in Vista's,
  the track's fade-at-stop flag set; no fade-in).
- The globals' 5500 ms at 0x1C0 is not a fade: it is how long before the
  idle attract movie the music is stopped (the decompilation's "silence
  time", 0x223976).

- It is switched on at the start screen and the main menu.
- It is switched off on the XBOX LIVE screen, the System Link browser and
  the pregame lobby.
- Every other screen keeps the current state. So Settings plays music, but
  the Live screens, matchmaking and the lobby are silent. We copy that
  rule.

**Where the sounds are.**

- In MCC, Halo 2's sounds seem to live in the maps' sound data, played by
  halo2.dll through Miles. They are not in FMOD or Wwise banks, and no
  `sounds_*.dat` has been seen.
- The MCC `snd!` and `ugh!` layouts changed, `lsnd` grew to 0x34 bytes
  with tracks at 0x24, and the codec is probably Opus (compression 5).
- How the chunks are framed, and which file they point into, is unknown
  until checks I and J (section 9).

**Plan.**

1. The lobby gets audio output: `cpal`, as h2viewer uses. Today it has
   none.
2. On Vista: menu-preview already plays these sounds from Vista's tags,
   with Xbox ADPCM and WMA decoders. WMA goes through `crates/wma`, kept
   separate because it is LGPL. That serves the owner first.
3. On MCC: after checks I and J, read MCC's `snd!` and `ugh!`, and decode
   Opus through the `opus` crate (libopus, BSD). Put it in its own small
   crate, as `wma` is. Read the music loop in pieces, not all at once.
4. Until then, players with only MCC get a silent menu.
5. Under option A, our UI sounds still come from us, since we never call
   into the engine. The music comes from the engine only if it plays it by
   itself in UI shell; otherwise from us.

## 6. Controls and button layouts

The owner asked for this at 21:45. It doesn't wait for the menu art.

**Halo 2's own layouts (H):**

- Button Layout (0x28): Default, Southpaw, Boxer and Green Thumb.
- Thumbstick Layout (0x29): Default, Southpaw, Legacy and Legacy Southpaw.

Bumper Jumper and Recon come from later Halo games and are added on the
owner's request. They appear after Halo 2's four, on the same screen.

menu-preview already defines all six button layouts and the four stick
layouts as tables of button to action, in
`origin/menu-preview:crates/h2viewer/src/input.rs` (the `ButtonLayout`
enum, and tables from line 209). They also exist on `controller-wip`.
These tables are ours and move into h2launch.

**How a layout reaches the engine.** No patch is needed: the host already
hands the engine a gamepad mapping in slot 116
(`get_player_gamepad_mapping`, `profile.rs`'s `gamepad_mapping`, one byte
per action). Today the launcher sends `PadMap::Zero` (all zero) by default,
or `PadMap::Halo2`, whose table is an estimate. Zero still played in
milestone 1, so either the engine ignores an all-zero mapping, or our button
numbering isn't what the engine reads. Check C in section 9.1 finds out
which, and then picks one of three ways:

1. **Slot 116 is followed.** All six layouts become slot-116 tables, built
   from menu-preview's ones.
2. **Slot 116 is ignored, and the profile's preset bytes are followed**
   (`BUTTON_PRESET` 0x1C8, `STICK_PRESET` 0x1C9; never written today).
   Halo 2's four layouts go through them. Bumper Jumper and Recon go
   through way 3.
3. **Neither is followed.** The launcher swaps buttons, and sticks or axes,
   in the XInput state before it fills slot 36.
   - That works for every layout that only moves Default's actions to
     other buttons. Southpaw, Bumper Jumper and the stick layouts are such
     layouts.
   - Boxer gives the left trigger a melee that changes with dual wielding,
     so it can't be done this way.
   - A unit test checks which layouts are pure moves.

**Where it lives.**

- `lobby.txt` gains `buttons = <name>`, `sticks = <name>`,
  `look_sensitivity` (1 to 10; Halo 2's default 3), `look_inverted` and
  `vibration`.
- The lobby passes them to the engine child as flags (`--buttons
  bumper_jumper`, `--sticks legacy`), next to `--pad-map`.
- Phase 0 adds a plain list for them in the current lobby screens, so the
  owner can switch layouts at once.
- Phase 2 replaces it with Halo 2's CONTROLLER and BUTTON LAYOUT screens
  (layout in 7.3).

**Keyboard.** Key rebinding (the profile's keyboard mapping at 0x42C) is
later, after the PC shows that the engine reads it.

## 7. Phases

Each phase ends with the gate (`cargo fmt`, clippy, the release tests), a
Windows cross-build, and a run on the owner's PC.

### Phase 0: groundwork, and input first

1. **Input.** The 21:45 report comes first, since every PC check needs
   input. There are two paths to check:
   - the lobby window: winit for keys and gilrs for pads, which ignores the
     pad while another window is in front (`lobby/window.rs`, `focused`);
   - the engine: slot 36, filled from XInput, key messages and raw mouse,
     with `AttachThreadInput` (`win/input.rs`).

   The PC log says which one failed.
2. **Layouts in the lobby** as a plain list, passed to the engine
   (section 6).
3. **`ui.rs` from menu-preview** into blam-cache, with its tests.
4. **`ui::Reader` for `mcc::Map`**, and the reader additions in section 3
   (points 1 to 3 and 5 to 8; the string table can wait).
5. **A console probe** for the owner's PC, `blam-cache/examples/mcc_ui_probe.rs
   <MCC mainmenu.map> [<Vista mainmenu.map>]`. It runs checks A to H of
   section 9 and prints only.
6. **`crates/h2ui`** with `layout`, `anim`, `text`, `art` and `paint`, and
   the CPU backend into the lobby's `Canvas`.

**Done when:**

- the probe passes on the owner's PC;
- a test screen made from synthetic tags draws headless on Linux.

#### What Phase 0 built (2026-10-10)

Steps 3 to 6, and the drawing half of phase 1's two screens. Steps 1 and 2
(input and layouts) are separate work.

- `blam_cache::ui` reads the UI tags of both kinds of map (`Menus::open`),
  and `blam_cache::mcc` gained the format-13 pieces of section 3. Their
  predicted layouts are listed in `mcc-maps.md`.
- `crates/h2ui`: `layout`, `anim`, `text`, `art`, `paint`, the CPU
  backend (`cpu`) and the two screens (`screens`).
  - `tags` builds both screens' layouts from `wgit start_screen`, `wgit
    main_menu`, `wgit game_shell_background` (the framing), the list's
    `skin` (its place, rows shown, the focus fades) and `wigl`'s screen
    animations. A piece the tags don't have keeps the built-in numbers,
    and a bitmap whose picture is missing draws its flat shape.
  - `art::read` decodes the pictures those layouts name, MCC's through
    textures.dat and Vista's from the map, only the images their frames
    show.
  - `shell::Shell::open(<mainmenu.map or its folder>, log)` does both and
    logs once where each piece came from (lines starting `menus:`). It
    never fails; with nothing readable it is the flat look.
  - The still background (the navy and the framing) is a list of its own,
    `screens::background`: it doesn't move, so it is drawn once for a
    window size and copied in under each frame (`cpu::Kept`).
- Not done: the lobby's `app.rs` doesn't use any of it yet (that waits
  for the lobby settings branch to merge) and `H2LOBBY_MENU` isn't read
  (both done in phase 1; see "What Phase 1 built").
  On the container's 2.1 GHz Xeon at 1080p, with stand-in pictures of the
  real sizes, a main menu frame takes about 7 ms on the CPU and the kept
  background about 30 ms once (flat: about 2 ms and 8 ms); the owner's
  numbers decide on the D3D11 backend.

**On the owner's PC** (from the clone; nothing is saved but the PNGs, and
those stay on the PC, as they hold Halo 2's art and glyphs). With `<MCC>`
for `C:\Program Files (x86)\Steam\steamapps\common\Halo The Master Chief
Collection` and `<Vista>` for the Project Cartographer folder:

```
cargo run --release -p blam-cache --example mcc_ui_probe -- "<MCC>\halo2\h2_maps_win64_dx11\mainmenu.map" "<Vista>\maps\mainmenu.map"
cargo run --release -p h2ui --example menu_png -- C:\h2work\menu-png --mcc "<MCC>\halo2\h2_maps_win64_dx11\mainmenu.map" --vista "<Vista>\maps\mainmenu.map"
```

- The probe prints checks A to H of section 9.2; its output goes into
  this file in our own words.
- `menu_png` prints the `menus:` lines for each map, then each screen's
  quads and its CPU time a frame at 1920x1080 and 1280x720. It writes
  `start-<look>-<size>.png` and `main-<look>-<size>.png` for the looks
  flat, standin, mcc and vista. Text is in Halo 2's fonts from the
  folder beside the map (MCC's `halo2\h2_fonts`), or from a fonts folder
  given after the output folder.
- The ignored tests that read the real files (PowerShell):

```
$env:H2_MCC_MAPS = "<MCC>\halo2\h2_maps_win64_dx11"
$env:H2_MAPS = "<Vista>\maps"
cargo test --release -p blam-cache mainmenu -- --ignored
cargo test --release -p h2ui real_ -- --ignored
```

### Phase 1: start screen and main menu (the first thing the owner sees)

1. The new `Start` and `Main` screens, in front of today's sign-in.
2. **Start screen:**
   - the logo, its top left at (-511, 90), fading in over 250 ms;
   - the tracks and the brace;
   - the pulsing "PRESS START".
   - The pulse is 1.5 s, an estimate, until its animation is read.
3. **Main menu:**
   - list skin 11 at (-178, -80);
   - Handel Gothic in (0.62, 0.74, 0.84) with its drop shadow;
   - the glow bars and the scrolling sheen strips;
   - unfocused rows at 50%;
   - the gamertag at the bottom right (L: Halo 2 probably showed the
     profile there).
   - The rows are XBOX LIVE, SPLIT SCREEN, SYSTEM LINK (if shown) and
     SETTINGS.
   - The last row chosen is focused again on return.
4. **Background:**
   - Under C, these two screens show the framing at a low alpha, or a dark
     navy gradient when there is no art.
   - If A has passed by the time this phase starts, the two screens are
     drawn in the menu engine's window over the flythrough instead.
   - If A passes later, phase 1b moves them there.
5. XBOX LIVE leads to today's screens, so everything after the main menu
   keeps working while phase 2 restyles it.
6. Sound comes in phase 2, unless the engine plays the music by itself.

**Done when:** the owner sees both screens from MCC's files alone (Vista
moved aside), at 1080p, without stutter. The log says how long a frame
takes on the CPU. If it's over 16 ms, the D3D11 backend for the lobby window
moves into phase 2.

#### What Phase 1 built (2026-10-11)

Steps 1 to 5 under option C, without sound (step 6). Split screen and
system link aren't offered: the main menu has XBOX LIVE and SETTINGS.
Two things differ from 1.1 on purpose (the owner's Phase 1 decisions):
B on the main menu goes to the start screen without asking, and keeps
the player signed in and in the party; and the gamertag form comes after
XBOX LIVE, not between the start screen and the main menu.

- `lobby/app.rs`: `Screen::Start` and `Screen::Main` in front of the
  sign-in screen; the lobby opens on `Start` and no longer signs in at
  start-up.
  - Start: A (Enter, Space, the pad's A or Start) or a click anywhere opens
    the main menu; B opens the Quit popup, the only place it is now
    reachable. A click straight after one that changed the screen or shut
    a popup (the second half of a double-click, within 0.4 s) doesn't go
    on, and on the main menu only focuses a row, so a double-click on
    PRESS START (which lies over XBOX LIVE) stops on the main menu.
  - Main: up and down move an `anim::Focus` (clamped) with the skin's
    fades; a click focuses a row (`screens::main_menu_row_at`), or picks it
    if it had the focus. The row picked last keeps the focus on return.
  - XBOX LIVE goes to the playlists when signed in, to the signing-in
    screen while that goes on, else signs in with the gamertag and server
    last signed in with (without the form, and whatever was typed there
    and abandoned), else opens the form. SETTINGS opens the settings
    screen, which comes back to the main menu. B goes to the start screen.
  - The small print is the gamertag signed in, else the one saved.
  - B on the sign-in, signing-in (calling it off), offline and playlists
    (when not searching) screens now goes to the main menu; on the
    playlists the player stays signed in with the party kept.
  - A server lost while on either screen (or in the settings opened from
    the main menu) leaves the player there, with the reason as a notice.
    An invitation still pops up over the main menu, and accepting it
    shows the playlists; a match still opens the pregame lobby.
- Drawing: on these two screens the lobby's chrome isn't drawn. The still
  background is kept (`cpu::Kept<u32>`, straight into the canvas's
  pixels, which are the same 0x00RRGGBB), and the screen is drawn over it
  with `cpu::draw_u32`. The menus' clock is the seconds since the lobby
  opened, and each screen's intro plays from when it was opened. Popups
  and notices are drawn over them with the lobby's own pen.
- Loading (`lobby/mod.rs`, `menus`): `H2LOBBY_MENU` (a map or its folder,
  or `off`), else the rank icons' candidates (`ranks::candidates`: MCC's
  mainmenu.map, then Vista's folders, then `maps` beside the launcher;
  2.1 puts Vista's first);
  the first that gives pictures is used, else the first that opened,
  drawn flat with its layouts, else the flat look. `Shell::open` logs its
  `menus:` lines to `lobby.log`. The fonts are `Fonts::read` on
  the lobby's Halo 2 fonts folder, else the built-in font.
- The log says each screen's CPU time a frame once per window size,
  averaged over its first 30 frames there, with the background's one-off
  time: `menus: 1920x1080, start screen 3.4 ms a frame on the CPU
  (background 20 ms once)`. That answers "Done when" from the owner's log.
- Headless: the labels `start` and `main`; `see` finds PRESS START, the
  rows and the small print once they have faded in, and the automatic
  screenshot of either screen waits for that (on the start screen, until
  PRESS START is back at the top of its pulse), holding the script. On the container, flat, at 1280x720: about
  1 ms a frame and 6 ms for the background.

### Phase 2 (widened, 2026-10-11)

This replaces the first Phase 2 plan. What was still true of it is kept
below, under the sub-phase it now belongs to.

**The owner's direction.** The menus should look and feel as close to
the original Xbox Halo 2 as possible; that comes before anything else.
On the main menu he asked for Cartographer's music, intro movie and look.
Every screen works with a pad alone. The word is ONLINE, never XBOX LIVE,
and the Xbox Live logo is never drawn. No campaign, and nothing touches
halo2.dll.

**Where Halo 2's files come from** (`lobby/sources.rs`, section 3):

1. **Cartographer first**: Halo 2 Vista as Project Cartographer installs
   it gives the art, the layouts, the fonts, the rank icons, the UI
   sounds, the music, the 3D menu scene and the intro movie.
2. **MCC second**: its own Halo 2 files give the art, the layouts, the
   fonts and the rank icons (the same as Vista's: 133 of 133 screens).
   They give no music, scene or intro yet (format 13's sounds and
   geometry aren't read, and its movies are Bink 2).
3. **Then the built-ins**: the flat look, the built-in font, level
   numbers, silence, the still background and no movie.

The finder decides once, when the lobby opens: `H2LOBBY_VISTA` (the
Cartographer folder or its `maps` folder), else `vista =` in
`lobby.txt`, else the first of `ranks::MAP_DIRS` that holds a
mainmenu.map. `off` in either place shows what friends without
Cartographer get: then nothing comes from a Vista folder at all, not even
through `MAP_DIRS`. Nothing from either install goes into the repository
or the friends' zip.

**Decisions.**

- **Drawing.** The lobby keeps its CPU canvas (softbuffer). Only the 3D
  menu scene uses the GPU: wgpu renders it offscreen on its own thread,
  and the frame is read back into the canvas, on the start screen and
  main menu only. Every other screen keeps the still
  `game_shell_background`, and the scene sleeps there. A GPU presenter
  for the whole UI waits until some screen takes more than 12 ms at
  1080p on the owner's PC.
- **Audio.** A mixer inside h2launch, copied from our
  `h2viewer/src/audio.rs`, on cpal. `App` only emits cues and says
  whether music is wanted; the window owns the device, so App's tests
  need none.
- **Screens from tags.** `h2ui::tags::screen_layout` reads any wgit by
  its full path. Each lobby screen binds its slots to
  `Words::{Tag, TagOr, Ours, Hide}`, and a slot the binding doesn't name
  is hidden. Format tokens (U+E400 to U+E4FF) take our values; a token we
  don't know hides its text.
- **Wording.** `wording()` turns Xbox Live into ONLINE (never XBOX LIVE)
  and hides Bungie, clan, voice and the other Xbox wording.
- **Scale**, per catalog entry and pane: Vista's 1:1, or the Xbox's with
  positions ×2.5 and pictures ×1.25 (checked against the owner's Xbox
  screenshot and three bitmaps only). The quarter and half dialogs are
  decided in 2.7 by a header-fit check on the real map; a dialog's header
  and legend boxes take its pane's scale. `H2LOBBY_SCALE` overrides any
  entry for tuning.
- **Code shape.** `App` stays the state machine. Each new screen's
  binding goes in a child module, `lobby/app/<screen>.rs`, declared in
  app.rs so it sees App's private fields. Old `draw_*` functions go when
  their screen is replaced.
- **Input.** Controller first. The stick counts past 0.9 (H), and a held
  direction repeats every 250 ms (H; the first delay is an estimate).
  Legends follow the device used last: A, B, X and Y glyphs after a pad,
  key names after the keyboard. LB and RB keep their PC shortcuts but
  stay off the legends (1.2), except as the Y menu's tabs.
- **Server.** No change: `LIVE_PROTOCOL` stays 3. The leader's lobby
  mode, map and playlist live on the leader's PC. Other players' emblems
  and colours come later.
- **Sign-in.** Its internals belong to `launcher-accounts`, which bumps
  `LIVE_PROTOCOL` itself; the sign-in restyle waits for it.

**Sub-phases**, each leaving every flow working (old screens stay
reachable until their replacements land):

| # | Sub-phase | The owner sees |
|---|---|---|
| 2.1 | Sources (Cartographer first), the lobby flows test, per-screen timing, this plan | log lines only |
| 2.2 | Menu music and UI sounds | music on start/main, button sounds |
| 2.3 | Intro movie (Media Foundation) | the intro before the start screen, skippable |
| 2.4 | The menu scene as data (new crate `h2scene`) | nothing |
| 2.5 | The scene behind start/main (wgpu offscreen) | the darkened flythrough |
| 2.6 | Halo 2's shell on every lobby screen; legends by device; stick repeat | framing, tracks, header and legend everywhere |
| 2.7 | h2ui: any screen from its tags (library), catalog, menu_png | nothing |
| 2.8 | Halo 2's dialogs; the invite notice | every popup and failure as a Halo 2 dialog |
| 2.9 | The ONLINE menu, OptiMatch's playlists, Please Wait | Halo 2's Live screen, worded ONLINE |
| 2.10 | The pregame lobby as the party's home | a Cartographer-style PREGAME LOBBY |
| 2.11 | Game Setup, the Select Game/Variant/Map pickers, map pictures, Party Privacy | Halo 2's custom game setup |
| 2.12 | Player Options; the back stack | Halo 2's per-player dialog |
| 2.13 | The Y menu (FRIENDS, PLAYERS) | Halo 2's players list on Y |
| 2.14 | The service record restyled; an on-screen keyboard | the record as a Halo 2 screen; Add Friend with a pad |
| 2.15 | SETTINGS and CONTROLLER | Halo 2's settings screens |
| 2.16 | BUTTON LAYOUT and THUMBSTICK LAYOUT | the controller picture with labels, Bumper Jumper |

Where an open question to the owner has no answer yet, the plan's
default holds: members see Cartographer's pregame layout with START GAME
and GAME SETUP dimmed, and its chat box stays empty; no QUIT row on the
main menu; LB and RB switch the Y menu's tabs; the intro plays on every
launch, skippable; the scene at 30 fps; Quickmatch with no playlist
chosen yet opens OptiMatch; MCC music later; emblems and colour bars
later; the button layouts offered stay as they are.

**What the first plan said that still holds**, by sub-phase:

- 2.9: "Please Wait" while connecting, then the ONLINE screen. Optimatch
  opens the playlists, with your level icon and the player count from
  the server's searching and playing numbers. Create Party opens the
  pregame lobby: on h2live you are always in a party, so only the screen
  changes.
- 2.10, the pregame lobby: START GAME (leader) searches the playlist or
  starts the custom game; GAME SETUP opens the setup screens. On the left
  the mode block ("Matchmaking", the playlist, or the custom game's type
  and map with its picture), the party's privacy and one status line; on
  the right up to 16 players with their level icons, the count and who
  leads. Select New Leader and Boot Player do today's PROMOTE and KICK. B
  asks, then leaves the party. Y opens the players list; X changes teams
  in a custom game.
- 2.11: GAME SETUP offers Change Match Settings and Switch To: Custom
  Game in matchmaking, or Change Map, Change Rules and Switch To:
  Matchmaking in a custom game. Party Privacy offers Open and Invite
  Only; Closed needs the server and comes later.
- 2.12: the player options do today's popups (Join Party, invite, friend
  request, Remove Friend, Make Party Leader, Boot From Party), and View
  Gamer Profile opens our service record.
- 2.8: an invitation shows Halo 2's lower-left notice with the gamertag
  and `receive_message`; today's A-accepts, B-declines popup becomes the
  multiple-choice dialog, and the invitation also shows in the Y menu.
- 2.13: the FRIENDS tab is today's friends list, requests first; the
  PLAYERS tab is everyone signed in, with level icons and a status column;
  X opens the player options.
- 2.14: the on-screen keyboard (controller-only players need one to
  type a gamertag; `controller-wip` has one to borrow from).
- 2.15: SETTINGS, Edit Player, then CONTROLLER (Thumbstick Layout,
  Button Layout, Look Sensitivity, Look Inversion, Automatic Look
  Centering, Controller Vibration), plus the PC's rows
  (mouse sensitivity, and the server address, which moves here from the
  sign-in screen). Appearance only if the profile bytes are shown to work,
  and without the 3D model.
- 2.16, BUTTON LAYOUT (0x28, H tags): the list at the top left, the help
  text at the top right, the `button_config` controller picture in the
  middle, and a label beside each button that changes with the focused
  layout. Bumper Jumper and Recon use the same picture, with labels from
  our tables.
- **Done when** (for the whole phase): a whole session (sign in, invite,
  party, search, custom game, friends, service record, a layout change)
  works on the owner's PC through the new screens, with keyboard, mouse
  and a pad.

**Rules for every sub-phase.**

- **The gate**, before every commit: `cargo fmt --all`, `cargo clippy
  --workspace --all-targets -- -D warnings`, `cargo test --workspace
  --release` (which runs `crates/h2launch/tests/flows.rs`), and `cargo
  build --release --target x86_64-pc-windows-gnu -p h2launch`, whose
  exe must import neither of Media Foundation's DLLs (2.3's review: the
  intro loads them when it plays, so an N edition without them still
  starts the launcher): `x86_64-w64-mingw32-objdump -p
  target/x86_64-pc-windows-gnu/release/h2launch.exe | grep -iE
  'mfplat|mfreadwrite'` must print nothing.
- **Docs**: README.md in plain words, "What Phase 2.N built" here,
  lobby.md (labels, variables, recipes), PROGRESS.md.
- **Pinned tests** (strings, pixels, positions) are updated to the new
  truth, never loosened.
- **Game files**: nothing from them is committed or pasted (maps,
  pictures, fonts, sounds, movies, the tags' words, screenshots). Names,
  offsets, positions, sizes and colours in our own words are fine.
  Real-file tests are `#[ignore]` and read `H2_MAPS` or `H2_MCC_MAPS`.
  The container's Cartographer copy is for those tests and for
  scratchpad PNGs only.
- **New crates** get a stub line in `Dockerfile.live`.
- **The engine child** is polled every tick on every screen, and
  `Told::next`, `on_match_over`, LeaveGame and `shutdown` stay wired.
- **Frame order.** The intro: black, then the movie frame. The start
  screen and main menu: the scene frame (or the still background), the
  h2ui screen, then the Pen's popups and notices. Every other screen: the
  kept shell still layer, the moving tracks (every frame), the h2ui body
  (or the Pen's, until that screen converts), the header and legend, the
  overlay and dialog, then the notices.
- **CPU budget.** Every screen at most about 10 ms a frame at 1080p on
  the owner's PC; the start screen and main menu with the scene at most
  about 9 ms on the UI thread. The per-screen timer logs each one; a
  screen over budget moves its still parts into a kept layer.

**The new-screen checklist**, for every screen added or restyled:

- a `label()`;
- its music rule (on, off or keep);
- its focus index in `cue_state`;
- its name in the per-screen timer (`timer_name`);
- its strings pushed into `drawn` once its intro is over;
- `menu_settled` waiting for its intro;
- `pad_is_the_lobbys` and `keys_are_the_games`, if it can show while the
  engine runs;
- `invite()`'s screen list, where invitations should show;
- the flows scripts (`crates/h2launch/tests/flows/`) and lobby.md's
  recipes, in the same commit as any label or navigation change;
- the engine child still polled every tick, with `Told::next`,
  `on_match_over`, LeaveGame and `shutdown` still wired.

**Corrections to the sections above.**

- 1.3 said the lobby tracks whether a pad or the keyboard was used last.
  It doesn't; 2.6 adds that.
- Phase 3 said A cancels the countdown for the leader. In the
  decompilation it is X, which cancels it for the leader and delays it
  for the others.
- The first plan had Quickmatch take the party's last playlist or the
  busiest one. The decompilation shows it uses the playlist chosen last
  in OptiMatch; with none chosen yet, Quickmatch opens OptiMatch (the
  default above).
- The playlist screen's level icon and player counts (1.2) are our
  additions; Halo 2's playlist list had neither.

#### What Phase 2.1 built (2026-10-11)

No visible change; the log says more.

- `lobby/sources.rs`: `Vista { root, maps }` (`mainmenu()`, `fonts()`
  when `maps\fonts` holds `font_table.txt`, `movies()`), and `find`, in
  the order above. A folder named in `H2LOBBY_VISTA` or `lobby.txt` that
  holds no mainmenu.map (nor its `maps` folder) is passed over, and the
  log says so. `lobby.txt` gained `vista = <folder>` (`MenuPrefs`, the
  menus' own lines, written after the controls; later sub-phases add
  theirs there).
- The lobby finds the folder once (`Config::vista`) and logs either
  `lobby: Halo 2 Vista files (Project Cartographer) in <maps>` or `lobby:
  no Halo 2 Vista folder; menus from MCC, no music, scene or intro`.
- Vista first, MCC second: `ranks::candidates` lists the Vista
  mainmenu.map found, then each of `MAP_DIRS` not listed yet (none when
  Vista is off), then MCC's, then `maps` beside the launcher. The rank
  icons and the menus both use it. The fonts are `H2LOBBY_H2FONTS`, else
  Vista's `maps\fonts`, else MCC's `halo2\h2_fonts` (`fonts_choice`);
  the lobby's text and h2ui both read either through
  `blam_cache::font::read_table`.
- The per-screen timer: every screen's CPU time a frame, averaged over
  its first 30 frames, logged once per screen and window size. The menus
  keep `menus: 1920x1080, main menu 3.9 ms a frame on the CPU (background
  20 ms once)`; the others say `lobby: 1920x1080, playlists 2.7 ms a
  frame on the CPU`. On the lobby's own screens the whole draw is timed.
- The flows test (`crates/h2launch/tests/flows.rs`, lobby.md "The flows
  test"): real headless lobbies against an h2live and relay in the test
  process. All seven are in the gate: matchmaking, a party searching
  together (`party_matchmaking`, three lobbies), a custom game, the
  party screen with three lobbies, friends, a setting kept across runs,
  and a taken gamertag and the quit popup (about 45 s in the container,
  side by side; the longest is `party_matchmaking` with its 20 s
  countdown). A party of two can't search any default launcher
  playlist alone (Head to Head takes parties of one, and Team Snipers
  wants a third player to even its teams), so in the matchmaking flow
  BRAVO accepts ALPHA's invitation, then leaves, and both search Head
  to Head. The party search is `party_matchmaking`'s: ALPHA and BRAVO,
  a party of two, search Team Snipers, and CHARLIE searching it alone
  evens the teams.
- Scripts may end a line with a `# comment`; the headless lobby runs
  without any font (its text isn't drawn, but `see` still reads it).
- On the container, flat, at 1280x720: about 1 ms a frame on the start
  screen and main menu, 1.5 to 4 ms on the lobby's own screens; with the
  Cartographer copy's art, about 4 to 5 ms on the start screen and main
  menu (20 ms for the background once).

**On the owner's PC (P1)**, from `lobby.log`:

- it names the Cartographer folder for the menus (`menus: reading
  ...mainmenu.map (Halo 2 Vista's)`), the fonts (`lobby: Halo 2's fonts
  from ...\maps\fonts`) and the rank icons (`lobby: rank icons from ...
  (Halo 2 Vista's)`);
- with `H2LOBBY_VISTA=off` it names MCC's for all three;
- each screen on show for about a second or more at his window size
  (30 frames, added up over visits; resizing starts the count again)
  logs a ms line; a short one, such as signing in, may not.

#### What Phase 2.2 built (2026-10-11)

Halo 2's menu music and sounds, from Cartographer's files (section 5).

- **The tags.** `blam_cache::ui::Sounds::all` holds all 13 of the
  globals' sound references (at 0x90, 8 bytes each); the five named
  fields stay. `h2tool ui <mainmenu.map> globals` prints all 13. In
  Halo 2 Vista's mainmenu.map they are, by index: 0 `cursor1`, 1
  `forward1`, 2 `flag_fail`, 3 `advance`, 4 `back1`, 5 `transport`, 6
  `cursor1` again, 7 `virtual_keyboard_click`, 8 `receive_message`, 9
  `forward1` again, 10 `countdown_for_respawn` (under
  `sound\game_sfx\multiplayer`), 11 none and 12 `pickup_health`.
  Accessors: `receive_message()` (8), `countdown()` (10),
  `keyboard_click()` (7), `tab()` (9) and `transport()` (5: the start
  screen going on to the main menu, H, 0x22f28c; review round 1 renamed
  it from a guessed `login()`). The globals' 0x1C0 is `music_silence_ms`
  (was `music_fade_ms`; section 5), and `sound::Track` now reads its
  flags, fade-in and fade-out (0x4, 0xC, 0x10).
- **The mixer** (`lobby/audio.rs`), copied from `h2viewer/src/audio.rs`
  without the game's voices and placing: linear resampling, looping, a
  random next loop, per-voice fade-outs, 16 voices at most and MASTER
  0.55. Beyond 16 the oldest menu sound makes room, never the music
  (review round 3: the quietest went, and the music, at 0.50 under every
  menu sound's 0.71, was the one dropped once a held Esc's repeats
  stacked 16 `back` sounds, and it never came back on that screen).
  New: `pause`, which stops the cpal stream, and `Output::{Device, Null,
  Wav}`. (A fade back up, `fade_to`, went in review round 1: Halo 2
  starts the music again from its beginning; round 3 narrowed the
  mixer's fade to fade-outs.) cpal (0.16, as h2viewer) is new in
  h2launch.
- **Loading** (`lobby/sound.rs`, `load`, on its own thread): the globals
  from `Menus::open`, then `MapSet::open` on the same map (with shared.map
  beside it) to decode each sound's first variation at its tag's gain.
  The music is the globals' looping sound: its first track's start, then
  its loop's variations, picked at random one after another (by name if
  that fails). In the container's copy: cursor 0.36 s, select 0.20 s,
  back 0.77 s, error 0.51 s, advance 1.02 s, message 0.69 s, countdown
  0.09 s, key click 0.17 s, transport 1.39 s; the music's start 29.7 s,
  then loops of 56.5, 35.4, 75.5 and 150.0 s, gain 0.50 and fade-out
  5.0 s (its track's); about 1 to 1.6 s to decode.
  It never panics; failing, the menus are silent and the log says why.
- **Gains**, from the tags: menu sounds at their tag's gain (-3 dB each
  in Vista's), as Halo 2 plays them (0x236299, at a scale of 1); the
  music at its track's gain times its start's own (-6 dB and 0 dB: 0.50).
  Found by name, without its looping sound, the music takes 0.5 and a
  5 s fade (estimates from Vista's track). Review round 1 dropped the
  from-scratch game's 1.6 on the menu sounds, which put them 4 dB louder
  against the music than the tags do. A match's engine starting fades
  the music in 0.5 s (an estimate), then the stream pauses. (After P2
  each also takes its sound class's gain, -4 dB in Vista's, and the mix
  is no longer scaled by 0.55: "After P2" below.)
- **`MenuSound`**, owned by the window and the headless loop: it starts
  the loader when Vista's files were found, `H2LOBBY_SOUND` isn't `off`
  and `lobby.txt`'s `menu_sound` (new in `MenuPrefs`) isn't `off`, and
  opens the output only once sounds have come: the device with a window,
  nowhere headless, a WAV with `H2LOBBY_AUDIO_WAV`. Each tick it plays
  `App::take_cues()`, starts or fades the music by `App::music_on()` as
  Halo 2 does (section 5: at most once every 6 s, each start a new voice
  from the music's start, each stop its track's fade-out), and while
  `App::engine_running()` fades every voice at once (`Audio::fade_all`:
  the music, a stop's fade still running and a menu sound), drops the
  cues, then stops every voice (`Audio::stop_all`) and pauses, so nothing
  from before a match plays on after it. (Until review round 2 only the
  playing music faded: a stop held by the 6 s rule and still fading when
  the engine started was frozen by the pause and finished on the
  carnage report.)
- **App** stays free of cpal. `music_rule(screen, current)` is Halo 2's
  (section 5): on at Start and Main; off on Live (searching too), Party,
  Custom, Pregame and InGame, and whenever an engine runs; the rest keep
  it. The cues come from a snapshot (`cue_state`: the screen, the popup,
  the invitation shown, the focused row of the list on show, the custom
  game's game and map and the controls, and a refused flag) before and
  after `input` and `click` (their bodies are now `input_inner` and
  `click_inner`): refused gives Error; another screen, popup or
  invitation gives Back on B (or Esc, Backspace, the b key); else, when a
  screen or a popup opened, Advance, after Select when the press was A
  (Enter, Space, the pad's A or Start, or a click that picks; review
  round 3: X, Y, the bumpers and the other shortcuts opening something
  give Advance alone); else Select (the players and friends lists, the Y
  menu's tabs, and a popup only closing; see the next point); only the
  row or a value changing gives Cursor. `refuse()`
  (a notice and the flag) is used for the leader-only notices, a friend
  who can't be joined, a player who can't be asked (with the server's
  own words: already a friend, already asked, asked you; ourselves only
  sounds), and no maps for a custom game. A search started pushes Select
  then Advance itself (Halo 2's matchmaking screen loading; ours stays
  on the playlists), a custom game asked for Select only (Halo 2's START
  GAME is a button that loads nothing), and the screens the lobby opens
  by itself push Advance: the pregame lobby (`on_match`, not when the
  same match comes again with a new host), the carnage report
  (`poll_game`) and, since review round 3, the playlists once signed in
  (`signed_in`: Halo 2's main menu loads its online screen by itself,
  0x230ba6) and the failure screen (`fail`, and `poll_game` when the
  server was lost during a match, unless the carnage report's advance
  has just sounded; Halo 2's error dialogs from 0x1486b8 load forward),
  once each. A new invitation on show
  pushes Notify once (`tick`; one that has left the invitations list is
  forgotten, so the same party inviting again sounds again), and so does
  the server's notice of a friend request to us (Halo 2 sounds every
  message, 0x2b740a). Presses that only send a request push their own
  sound, as nothing on screen changes: the leader's B stopping a search
  Back; make leader, party privacy, an invitation, a friend request,
  accepting or declining one and taking ours back Select. A on the start
  screen pushes Select, Advance (the main menu loading), then Transport.
  Countdown and KeyClick wait for
  the countdown and the on-screen keyboard. The sign-in functions are
  unchanged but for `signed_in`'s Advance.
- **Select, then advance, on a screen opening** (review round 2, from
  the decompilation). One press that opens a screen plays two sounds in
  Halo 2, in the same frame: the press's `forward1` (sound 1, from the
  list item's A at 0x24ac05 or the button's at 0x2540e8, after its
  handler has loaded the screen) and the new screen's `advance` (sound
  3: every screen and dialog load calls 0x147f6d, which plays it unless
  the load came from the back history or is the start screen's, id 9).
  Forward loads (the main menu's rows, `dialog_ok` and `dialog_choice`)
  have type 0; only B's reloads from the history set bit 0 (0x23553f),
  so B plays `back1` alone. 0x236299 plays any sound below 13 through
  0x1896c0 at a scale of 1 and drops none, so both are heard (until
  round 2 this note wrongly said 0x236299 wasn't decompiled, and the
  lobby played `select` alone). A Y-menu tab change plays only the tab's
  sound (9, `forward1`, from the Y menu's tab bar, 0x2b41ea) and loads
  no screen, and a dialog closing on a choice runs an animation, not a
  load, so both stay `select` alone. The start screen's A gives
  `forward1`, `advance` (the main menu, id 6, type 0, from 0x22f28c),
  then `transport`. Only A and Start play `forward1` (0x24ac05 and
  0x2540e8 check the press); Y opens the Y menu through 0x1494af and
  0x1496c7, which play nothing, so that is `advance` alone, and so are
  the lobby's shortcuts (X, Y, LB, RB) opening a screen or a popup
  (review round 3; until then they sounded `select` too).
- **LGPL** (review round 2). Since 2.2 the launcher calls
  `blam_cache::sound`'s WMA decoding, so `crates/wma` (ported from
  FFmpeg, LGPL 2.1 or later) is in `h2launch.exe`, linked statically. It
  stays a crate of its own (AGENTS.md). The next friends' zip built with
  2.2 (the READ ME isn't in the repository) must: say in its READ ME that
  `h2launch.exe` contains the `wma` crate, a WMA decoder ported from
  FFmpeg under the LGPL 2.1 or later; carry `crates/wma/LICENSE`; and
  carry `crates/wma`'s source or the owner's written offer to provide it,
  plus an offer of h2launch's own source or object files so the library
  can be relinked (LGPL 2.1 section 6). The repository stays private, so
  "see the repository" doesn't do.
- **Checked in the container** (2026-10-11), headless with the uploaded
  Cartographer folder: the start screen, then the main menu, down and up
  gave a 23 s WAV with music from its first 0.05 s; a second run with no
  presses, taken away from it sample for sample, left only the select
  and two cursor sounds, each where its press was. Two lobbies through a
  custom game (ALPHA recording, the stand-in engine playing 4 s): music on
  the start screen and main menu (and the sign-in form), its fade over
  5.5 s from the playlists (5 s since review round 1), silence through
  the match, the select and back sounds after it, and the music from its
  start again on the main menu.

**On the owner's PC (P2)**, with Cartographer's folder found:

- `lobby.log` has `sound: from ...mainmenu.map: cursor ...; music ...;
  decoded in ... ms`, then `sound: playing to <the device's name> (the
  default audio device)` (since the follow-ups below; until then just
  "the audio device");
- the music plays on the start screen and the main menu, and fades over
  about 5 s on the playlists (a change within 6 s of the last waits);
  back on the main menu it starts again from its beginning;
- the cursor, select and back sounds play with the pad and the keyboard
  (and a click on a main menu row); A on the start screen sounds select,
  advance, then transport; a screen or popup opened with A sounds select
  then advance, and with X, Y or a bumper advance alone; the playlists
  after signing in, a failure screen, the pregame lobby and the carnage
  report advance as they open;
- holding Esc on the start screen never stops the music (when P2 was
  written the quit popup opened and closed with each of Windows'
  repeats; since 2.3's review round 2 a held key's repeats are dropped
  for every button, so it opens once and stays);
- a friend request to us sounds as a message;
- silence through a match (nothing of the menu music playing on after
  it, even when the match started within 6 s of the music starting), and
  the music from its start again on the main menu after it;
- with `H2LOBBY_VISTA=off`: silence, `sound: the menus are silent (no
  Halo 2 Vista files)` and no error.

**After P2 (the PC check's follow-ups, 2026-10-11).**

- **The music's level.** P2's recording had the music on the start
  screen and main menu at about -41 dB RMS (peaks 0.03 to 0.10 of full
  scale), which sounded too quiet. The chain, checked against the tags
  and the decompilation: Halo 2 plays a sound at its playback
  parameters' base gain (0x10 of its gestalt playback entry), plus its
  sound class's gain at the scale it plays at (the sound classes tag,
  `sncl`: 0x5C bytes a class, the bounds at 0x20 for a scale of 0 and
  0x24 for 1, interpolated in amplitude by 0x1251e0), plus a looping
  sound's track gain and the permutation's own (0 for all of these),
  added in decibels and capped at 0 dB (0x2197f0). Menu sounds play at a
  scale of 1 (0x236299 through 0x1896c0), and so does the music
  (0x235b5d starts its looping sound with 1.0, which 0x18a7b0 takes as
  its scale). In Halo 2 Vista's mainmenu.map the menu sounds and the
  music are class 50, whose bounds are 0 and -4 dB, so the menu sounds
  are -3 - 4 = -7 dB (0.45) and the music 0 - 4 - 6 = -10 dB (0.32).
  2.2 played them at -3 and -6 dB, then took the from-scratch game's
  `MASTER` 0.55 (-5.2 dB, headroom for a battle's sounds, not in any
  tag) off the whole mix: 1.2 dB under the tags. Now the class's gain is
  read (`blam_cache::sound::class_gains`, the bounds at a scale of 1)
  and `MASTER` is 1, so both play at the tags' level. No mix-up between
  decibels and amplitude, and no halving by resampling or by channels,
  was found. The quiet start is the music's own:
  its start's first 5 s are -28 dB RMS in the file, the next -25 dB,
  the whole start -18.6 dB and the loops -15.5 to -19 dB. Recorded again
  in the container (start screen, then the main menu at 25 s): -38, -35,
  -30, then -25 dB RMS a 5 s stretch, peaks up to 0.29. Louder than that
  is the PC's own volume, as on the Xbox it was the TV's.
- `sound: playing to` names the device (cpal's name), and the music and
  the engine's changes are logged once each (lobby.md).
- `H2LOBBY_AUDIO_WAV` keeps recording silence while the engine runs, and
  `H2LOBBY_SCRIPT` is for headless lobbies only (the windowed lobby logs
  that it ignores it); both said in lobby.md.

#### What Phase 2.3 built (2026-10-11)

Halo 2's intro movie before the start screen, from Cartographer's
`movie` folder, decoded by Media Foundation (section 5's table).

- **`lobby/intro.rs`.** A `MovieSource` decodes a movie on a thread of
  its own and sends, down a `sync_channel` of `AHEAD` (4), the whole
  soundtrack first (`Decoded::Sound`, a `Clip`), then each picture in
  order (`Decoded::Picture`: its time, size and pixels), or why it
  failed (`Decoded::Failed`, its last word). `MfSource` (Windows only)
  is a close port of `menu-preview`'s `h2viewer/src/intro.rs` Media
  Foundation code (not its ffmpeg path): Media Foundation's five
  functions looked up in System32's `mfplat.dll` and `mfreadwrite.dll`
  when it first plays (`LoadLibraryExW` with
  `LOAD_LIBRARY_SEARCH_SYSTEM32`, then `GetProcAddress`; kept loaded),
  never linked, COM and `MFStartup` on the thread, a source reader per stream from `MFCreateSourceReaderFromURL`
  with `MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING`, the sound read whole
  as 16-bit PCM, the pictures as RGB32 through `IMF2DBuffer::Lock2D`
  row by row at its pitch, then `MFShutdown`. Its blue, green, red,
  unset bytes become opaque RGBA on that thread, so drawing needn't
  swap them. `FakeSource` (tests) makes solid pictures and a short
  sound, and can hold the channel open (a stall) or fail. Off Windows
  there is no movie player (`intro::opener` is `None`).
- **`Intro`** starts its clock at the decoder's first word, shows the
  latest picture whose time has come and passes over earlier ones also
  due (counted as late), ends when the decoder has finished and both
  the last picture (held 1/30 s, an estimate) and the soundtrack have
  played out, and gives up after `STALL` (5 s) without a word, as
  `menu-preview`'s did. A decoder that ends with no pictures fails.
- **Which file.** `Plan::find` takes Vista's `movies()` folder when it
  holds `intro_60.wmv` or `intro_low_60.wmv`. The file is chosen at the
  first draw: the low one in a window under 720 pixels tall, else the
  full one, and the other when that one is missing. A file that is
  there but can't be decoded isn't retried with the other (it ends the
  intro, below).
- **When it plays.** `mod.rs`'s `intro_plan`: a window (not headless),
  `H2LOBBY_INTRO` not `off`, `lobby.txt`'s new `intro` line (`on` or
  `off`, `MenuPrefs`, written after `menu_sound`) not `off`, a Halo 2
  Vista folder, a movie player (Windows) and a movie. Otherwise the
  lobby opens on the start screen as before, and the log says why
  (`intro: none (<why>); the lobby opens on the start screen`). It
  plays at every start (the plan's default).
- **`Screen::Intro`** (label `intro`, timer name `intro`, music off,
  focus 0, no strings in `drawn`; `menu_settled` is true on it, and
  `pad_is_the_lobbys`, `keys_are_the_games` and `invite()` need nothing,
  since no engine runs and nobody is signed in yet). Decoding starts at
  its first draw, when the window's height is known. Each draw is black,
  then the picture showing, as large as fits in the middle
  (`intro::letterbox`), as one h2ui quad drawn by `h2ui::cpu` from a
  small `Textures` (`MovieFrame`), through a `Kept` that draws again only
  for a new picture or size.
- **Skipping.** Any button: A, B, X, Y, LB, RB, LT, RT (the pad's Start
  and Back are A and B), Enter, Esc, Backspace, Space, a letter, or a
  click anywhere. A direction (stick, d-pad, arrows) and Tab don't, as
  in Halo 2 (0x156ab0 skips on every button but the d-pad and the stick
  clicks). The triggers (`Input::Lt`, `Input::Rt`, gilrs's
  `LeftTrigger2` and `RightTrigger2`) do nothing on any other screen. Leaving it sounds nothing (`cue_for`: the press is no list
  item's or button's, and the start screen's load plays no `advance`).
  The second click of a double-click doesn't go on from the start
  screen (the existing guard).
- **The end.** At its end, on a skip, on a stall, or when it fails
  (`intro: <why>; skipped`; Windows N without the Media Feature Pack
  lands here, as `intro: Media Foundation isn't installed (...);
  skipped`), the start screen comes in: its intro animations start at
  its first draw, and its music starts (music rule on).
- **Sound.** `App::take_cues` became `take_sound() -> Vec<SoundOut>`:
  `Movie(clip)` when the soundtrack comes, `StopMovie` when the intro is
  left, then the cues as `Cue(c)`. `MenuSound` opens the output when
  the soundtrack comes (even before the menu sounds have loaded), plays
  it once as music (never dropped for room) at its own level (gain 1:
  Halo 2 plays its movies through Bink straight to the sound card at
  Bink's full volume, 0x8000 on the front left and right in 0x156090,
  with no sound class or track gain), and on `StopMovie` fades it out
  in 50 ms (our choice, to avoid a click). With the menus' sound off
  (`H2LOBBY_SOUND=off`, `menu_sound = off`) no device opens and the
  movie plays silent.
- **Frames.** `App::frame_interval()`: 16 ms while the intro plays (its
  pictures come 60 a second), else 33 ms; `window.rs`'s `next_wake`
  takes it.
- **Tests (no game files).** `intro.rs`: pictures by their time and late
  ones passed over; the end after the last picture and the sound; a
  silent, stalled (on its thread) or failing decoder; the letterbox of
  a 16:9 movie in 4:3, 16:9 and 21:9 windows; the file by window height;
  why it is skipped; a player only on Windows. `app.rs` with
  `FakeSource`: it plays before the start screen and goes on at its end
  (black first, pictures letterboxed, the soundtrack then `StopMovie`,
  no cue, the music after); every button and a click skip it and a
  direction doesn't; a stall and a failure reach the start screen.
  `mod.rs`: only with a window, a player and a movie. `sound.rs`: the
  soundtrack once at its own level until skipped. `settings.rs`: the
  `intro` line.
- **Measured** in the container (release, by itself, not the timer's
  line, since the container plays no movie): one picture of 1280x720
  drawn into the window takes about 12 ms at 1280x720, 25 ms at
  1920x1080 and 32 ms at 2560x1440 on the CPU. The owner's PC is
  faster; the timer's `lobby: 1920x1080, intro <n> ms a frame on the
  CPU` (the average of the first 30 new pictures' draws, so it needs
  about half a second of pictures before a skip) and the end line's
  late count say how it does there. Pictures that come due while one is
  drawn are passed over, so the movie keeps time with its sound either
  way.
- **Review fixes (round 1).** Media Foundation is loaded at run time
  (above): linked, its DLLs were load-time imports, and Windows refused
  to start the whole exe (the lobby and each match's engine) on an N
  edition without them; the gate now checks the cross-built exe imports
  neither. The soundtrack plays at gain 1, not the music's 0.32. The
  triggers skip the intro. The timer counts only draws of a new picture
  (black frames while Media Foundation reads the soundtrack, and a
  picture drawn again, made it read far too low). New tests: the
  timer, the triggers, the movie chosen by the window's height (not its
  width) through `App`, the mix at the tags' gains with nothing taken
  off, and a sound's own and class decibels (`sound_db`); the stall
  test waits for the last picture before moving the clock.
- **Review fixes (round 2).** A key held down no longer goes on past
  what it pressed. The window passed Windows' repeats of a held key
  (`KeyEvent::repeat`) as presses, so Enter held about 0.6 s to skip
  the intro went on to the main menu, then picked ONLINE and signed in
  (with three menu sounds), and a held Esc opened and shut the Quit
  popup with each repeat. Halo 2 sends a button only on its first frame
  down and repeats only directions, every 250 ms
  (`docs/notes/decomp/controller.md` section 7; the pad already behaved
  so, as gilrs never repeats a button). Now `window.rs` sends repeats to
  `App::key_repeated`, which drops A, B, LB and RB (Enter, Esc, PageUp,
  PageDown) and, outside a field (`App::typing`: the sign-in form, the
  Add Friend popup), the letters, Space and Backspace; the arrows, Tab
  and typing in a field still repeat. `launcher-accounts` has the same
  function; merged, it keeps that branch's popups in `typing` (its
  password and new-account fields) and this one's LB and RB, which are
  buttons (Halo 2's White and Black) and open screens. Test: a key held
  to skip the intro leaves the start screen showing, with no popup and
  no cue; on the main menu the arrows repeat and the buttons don't; in
  the sign-in form letters and Backspace repeat. The README and the
  launcher README now say it opens on the intro movie first.
- Not here: the attract movie after idling, the credits, the scene.

**On the owner's PC (P3)**, with Cartographer's `movie` folder found
(`C:\Games\Halo 2 Project Cartographer\movie`):

1. The intro plays with its sound, louder than the menu music, and
   `lobby.log` has `intro: playing intro_60.wmv (<W>x<H> window)`
   (`intro_low_60.wmv` in a window under 720 pixels tall) and `sound:
   the intro's soundtrack, <n> s (gain 1.00)`.
2. After a second or two, A skips it: the log has `intro: skipped at
   <t> s`, no select or advance sound plays, the start screen comes in
   and its music starts. The d-pad, the stick, the arrows and Tab don't
   skip it; B, X, Y, the bumpers, the triggers, Enter, Esc, Space, a
   letter and a click do. Holding Enter (or Esc) for a second or more
   to skip it leaves the start screen showing, with no Quit popup.
3. Left to play out, the log has `intro: played to the end (<t> s, <n>
   pictures, <k> late ones passed over)`; a small late count is fine.
4. After 30 new pictures the log has `lobby: 1920x1080, intro <n> ms a
   frame on the CPU` (the container measured about 25 ms for one picture
   at 1920x1080).
5. With `H2LOBBY_INTRO=off` the log has `intro: none (H2LOBBY_INTRO=off);
   the lobby opens on the start screen`; with `H2LOBBY_VISTA=off`,
   `intro: none (no intro movie: no Halo 2 Vista folder); the lobby
   opens on the start screen`; with `intro = off` in `lobby.txt`,
   `intro: none (intro = off in lobby.txt); ...`. Each opens on the
   start screen with its music.
6. On a Windows N PC without the Media Feature Pack, if one is at hand:
   the launcher still starts, and the log says `intro: Media Foundation
   isn't installed (...); skipped`.

**After P3 (2026-10-12).** P3 passed on the owner's PC on fcfb9e1, with
these limits: it was driven by keys and clicks posted to a 1600x900
window, so no pad button (A among them) was tried, and check 6 was
skipped (no Windows N PC at hand).

1. `intro: playing intro_60.wmv (1600x900 window)`, the audio device
   named, the soundtrack 18.0 s at gain 1.00 and louder than the menu
   music after it.
2. Enter, Esc, Space, letters, Backspace and a click skipped it at about
   2 s, with no select or back sound; the arrows and Tab didn't. Enter
   or Esc held for 1.2 s gave one skip and no Quit popup, and the start
   screen's music started from its beginning.
3. Left to play: `played to the end (18.0 s, 540 pictures, 1 late)`
   (2 late in another run).
4. The intro took 7.9 to 8.2 ms a frame on the CPU at 1600x900, and the
   start screen 2.1 ms. That isn't the 1080p the list asked for; P4's
   1080p measure can take it too.
5. The three `intro: none` cases each opened on the start screen.

It also found that the windowed lobby, started from its own console,
exited with code 101 in about 1 launch in 7: `println!` panicked when
its write raced the console being freed (`FreeConsole`). The log's
console echo now ignores write errors (18279ad on `launcher-menu`, 0
failures in 30 launches on the PC after; merged here as 81b2e07).

#### What Phase 2.4 built (2026-10-12)

The 3D scene behind the start screen and main menu, read into plain data
by a new crate, `crates/h2scene`, on any thread and with no GPU. Nothing
visible changes; 2.5 draws it.

- **`blam-cache`** gained what `menu-preview` had added for it:
  `model::sky_atmosphere` (a sky's `Atmosphere`: its atmospheric `Fog`,
  the fog over the sky's model, and the clear colour when its flags ask
  for one) and `shader::read_menu_shader`, which reads the cloud
  templates the menu's way (two maps multiplied, doubled and added for
  the added clouds, as bright as the colour's alpha) and shows plain
  glows (`ShaderInfo::multiply`). `read_shader`, which h2viewer and
  h2tool use, is unchanged.
- **`h2scene`**, copied from `menu-preview`'s h2viewer (ours):
  - `flythrough.rs`: the `mainmenu_flythrough` script's
    `camera_set`s and `sleep`s at 30 ticks a second, each move a curve
    that starts at the camera's speed and turn and is cut off by the
    next. Its 70-degree field of view is still unverified (marked so).
    New: `seconds`, `ticks`, `points`, `points_used` and `moves`, for
    the example. Review round 1 changed how the points turn and the
    curve (below).
  - `camera.rs`: `look_to` (a view that banks with the camera's up),
    `Lens` with the reversed-Z projection and `fov_magnification`, and
    `menu_view_proj` (the camera at a time and its view-projection for
    an aspect, as h2viewer drew it: 70 degrees across at any width from
    4:3 up, near 0.05, far 2,000).
  - `data.rs`: `Vertex` (position, normal, uv, lightmap uv, light; 56
    bytes, `vertex_bytes` and `index_bytes` give the buffers' bytes, so
    the renderer needs no casting crate), `Material` (colour map, the
    second map's `AuxKind` (glow, mask, change colour, emblem, multiply,
    multiply twice), `Blend`, `pass()`: Opaque, Alpha or Additive),
    `MeshData::from_sections` (batched by material and lightmap page; a
    damaged section drops its bad triangles instead of panicking),
    `TextureData { w, h, mips }` with `mip_chain` (2x2 boxes down to
    1x1) and the grey fallback, and `SceneData { level, bsps, meshes,
    objects, sky, atmosphere, textures, materials, flythrough,
    failed_textures }`.
  - `light.rs`: `LevelLight`, the baked light on the floor under a
    point, which lights each piece of scenery.
  - `load.rs`: `load(mainmenu, log) -> Result<SceneData, String>`, a port
    of `Backdrop::load` and the `Loader` parts it used (level, lightmap
    pages, bitmaps, menu shaders, sky, atmosphere, scenery, placements);
    the game's HUD, weapons, sounds, rigs and h2sim are left out.
    Warnings go to `log`. An error, never a panic (a damaged map's panic
    is caught): a missing file, a file that isn't a Halo 2 map, MCC's
    format 13 ("MCC maps have no geometry reader yet"), no level
    geometry, or no flythrough.
  - `examples/scene_info.rs`: BSPs, triangles, textures, materials by
    pass, scenery by tag name, fog, camera points, the loop and the load
    time.
- **Workspace**: `crates/h2scene` is a member; `Dockerfile.live` stubs
  its `src/lib.rs` as it does h2ui's (checked by resolving the workspace
  from only what `.dockerignore` lets through, with and without the
  stub).
- **Tests (no game files).** The flythrough's (copied: reading the
  script, the loop back to rest, no jumps in speed or turn at the cuts);
  the lens (70 degrees across 4:3, depth reversed, a banked
  view tilts, the menu view centred on where the flythrough faces); `from_sections` on made-up sections (batches, shared
  batches, lighting, damaged sections), the mip chain's sizes and
  averages, the vertex bytes; `LevelLight` over two made-up floors and
  a lightmap page; the sky's fog and clear colour and a postprocess
  colour's alpha (copied); `load` on a missing path, on junk (empty,
  text, zeros, a Vista header with nothing behind it) and on a format-13
  header; `SceneData` is `Send`. Ignored, with `H2_MAPS`:
  `reads_the_real_main_menu`.
- **Measured in the container** on Cartographer's `mainmenu.map`
  (`H2_MAPS=... cargo test --release -p h2scene -- --ignored` and the
  example):
  - 1 BSP, 38,738 level triangles (the plan's count exactly), 6,997 in
    the scenery as placed and 184 in the sky: 45,919 in all;
  - 39 textures with the fallback (19.4 MB with their mips), none that
    failed to decode. The plan's 48 was h2viewer's loader for a whole
    level, which adds the players' bodies and weapons;
  - 40 materials: 27 opaque, 7 alpha-blended, 6 additive;
  - 10 pieces of scenery: the carrier (5,510 triangles), two plasma
    screens, the progress symbol (1,348), a cinematic anchor (6) and
    five one-triangle stand-ins for the firing effects; 9 of them over
    a floor that lights them. Whether Halo 2 draws the anchor and the
    stand-ins is for 2.5's look check;
  - the sky's fog: a brown haze, density up to 0.5, from 0 to 150
    units; clear colour a dark grey;
  - 18 camera points, 12 of them flown through in 13 moves; a loop of
    2,990 ticks (99.7 s), starting on the first path point;
  - the load: about 1.9 s the first time (the file read from disk),
    about 110 ms once the file is in the disk cache.

**On the owner's PC (with P4)**, from the clone (`C:\Halo 2 Rust
Project` or the work folder):

```
cargo run --release -p h2scene --example scene_info -- "C:\Games\Halo 2 Project Cartographer\maps\mainmenu.map"
```

It should print the counts above (1 BSP, 45,919 triangles, 39 textures
and 0 not decoded, 40 materials as 27/7/6, 10 objects, 18 camera points
with 12 flown through, 2,990 ticks and 99.7 s) and a load time; and on
MCC's `halo2\h2_maps_win64_dx11\mainmenu.map`, `error: MCC maps have no
geometry reader yet`.

**Review fixes, round 1 (2026-10-12).**

- **The camera points turn as Halo 2 turns them.** Its `camera_set`
  (0x16c2f0) makes a point's forward and up through 0x11df60 and
  0x141ce0: yaw about +z first, then pitch, and the roll last, about
  the world's +x (`camera::halo_rotation`). The port rolled about the
  camera's own forward instead, which banked the menu's camera up to 45
  degrees and pointed it into the sky at points 7 to 10. Now, on
  Cartographer's map, the camera banks at most 13 degrees over the whole
  loop (3 on average) and looks between 47 degrees down and 5 up; the
  review found five of the twelve points level to a tenth of a degree. Placed scenery turns the same
  way (0x0d5060, which fills in a scenario object's placement, uses the
  same conversion through 0x11df60), so `placement_matrix` does
  too; the carrier turns by yaw only and doesn't change, the plasma
  screens do. h2viewer's own `placement_matrix` (the parked engine)
  still rolls the old way; fix it there if that engine is picked up
  again.
- **The camera follows Halo 2's curve.** The observer solves, each
  frame, a fifth-order curve from its position, velocity and
  acceleration to the point at rest (0x16ff10, followed by 0x1701f0 and
  0x170630), for moving and for turning. The port's cubic forgot the
  acceleration at every cut, so it jumped there; `Motion` now carries
  acceleration and the spin's rate of change, and each move is that
  curve (the review measured the two paths up to 6.7 units apart on
  the real loop). Still
  open: the code that counts a move's ticks down hasn't been found; one
  curve per move assumes it does (`camera_time` returns the ticks left,
  and the script's last 300-tick move ends exactly as the loop starts
  again). Halo 2 turns by a rotation vector in the world's frame, this
  by one in the camera's frame at the start of the move, which is close.
- **A damaged map can't take the lobby down.** `LevelLight::new` leaves
  out a triangle with a vertex that isn't a finite number or that spans
  more than 1,024 grid cells (2,048 units; the level is about 470
  across), which would otherwise fill memory until the process
  aborted, past `load`'s panic catch. The script reader looks at no
  more than 4,096 expressions and 64 nested `begin`s, so a script that
  leads back into itself can't recurse until the stack overflows.
- **Tests.** Damaged vertices are left out of the light grid; damaged
  scripts (a `begin` that is its own argument, `begin`s fanned out five
  deep, 1,000 nested) end; moves to points the map lacks are dropped
  and the rest fly, and no script, an empty one or one that never
  waits gives no flythrough; acceleration and turning acceleration
  don't jump at the cuts (under 2% of the strongest a frame, where the
  old cubic jumped 5 to 54%); a level point stays level (the seventh
  path point's rounded angles look 46 degrees down with no bank), and
  placed scenery turns the same way; `halo_rotation` gives 0x141ce0's
  forward, left and up. `shader::shader_info` (the template match, now
  a pure function under `read_shader` and `read_menu_shader`) is tested
  for the menu's three templates both ways (a glow shown, clouds
  multiplied, added clouds doubled and as bright as slot 0's alpha),
  and h2scene's `material` for which second map wins (glow, mask,
  change colour, multiply; doubled only above 1; emblem) and for asking
  for the colour map first. `reads_the_real_main_menu` now pins every
  number above exactly (triangles, 39 textures, 40 materials as
  27/7/6, 10 objects with 9 lit, the fog's 0.5 from 0 to 150, the clear
  colour, 18 points with 12 flown in 13 moves, 2,990 ticks); if the
  owner's map differs, change them here and in the test together.

**Review fixes, round 2 (2026-10-12).**

- **A lightmap page that can't be decoded is tried once.** The pages
  are atlases many clusters and instances share; a page that failed
  wasn't remembered, so it was read and inflated again for each
  section that used it and counted each time, and the log's and
  `scene_info`'s "not decoded" count could run far past the number of
  bitmaps. It is now remembered either way (its sections are unlit), as
  a failed bitmap already was (texture 0). Test: on a map with no tags,
  a page asked for five times is counted once, another page of it once
  more, and a bitmap asked for three times once.
- **A `camera_set` to no point is skipped.** The reader turned a point
  of -1 (the script's none) into point 0, so a damaged script would cut
  the camera across to the first path point mid-loop. Halo 2's
  `camera_set` (0x16c2f0) does nothing for a point below 0 or past the
  last, so the move before runs on through the `sleep` after it; the
  reader now skips it the same way. A move to a point past the map's
  last is still dropped with its wait, as before. Test: (camera_set 0
  0) (sleep 90) (camera_set -1 100) (sleep 50) (camera_set 1 500)
  (sleep 250) reads as two moves, the first waiting 140 ticks.
- **Docs.** The code that places scenario objects is cited as 0x0d5060
  (it fills in a placement through 0x11df60); round 1 gave 0x0b7300,
  the address the decompilation's file for it is named after, which is
  another routine.
  `docs/notes/mainmenu-plan.md`'s notes that the roll's sign was
  unverified and should be flipped if the horizon tilted are marked
  superseded; only the 70-degree field of view is still unverified.

#### What Phase 2.5 built (2026-10-12)

The darkened flythrough behind the start screen and the main menu, drawn
by our own wgpu renderer offscreen and copied into the lobby's CPU
canvas (option B for these two screens). Every failure falls back to
the still background, and nothing touches the GPU while a match runs.

- **`h2scene::render`** (`Renderer`), from `menu-preview`'s h2viewer
  `gpu.rs` (ours), cut down to the scene: an instance from the
  environment (so `WGPU_BACKEND` picks the backend), a low-power
  adapter with no surface, refusing a software one (lavapipe, WARP)
  unless allowed, `downlevel_defaults` limits raised to the adapter's
  texture sizes, and an uncaptured-error handler and a device-lost
  callback that mark it failed instead of panicking (checked after
  each submit, so the error's own words reach the log). Textures are
  sRGB with 2.4's mip chains (a level past the device's limit is left
  out); the mesh shader without the players' camouflage, overshield,
  emblems or HUD; five pipelines (opaque, alpha, additive, sky, sky
  alpha). Each frame: the sky pass (near 1, far 10,000) then the world
  pass (near 0.05, far 2,000, reversed Z in `Depth32Float`), opaque,
  then alpha, then additive; then the veil pass into a `Bgra8UnormSrgb`
  target; the copy to a buffer with rows padded to 256 bytes; the map;
  and the unpacking into `0x00RRGGBB` pixels. `Eye::flythrough` is the
  camera at a time on the loop (2.4's `camera_set` rotation; the roll
  is settled and untouched).
- **The veil** (`Veil`), a pass of our own: in linear light each pixel
  keeps `saturation` of its colour (the rest is its Rec. 709 grey), is
  multiplied by the navy, then has `k` times the navy added. Halo 2
  Vista's own numbers for the darkening aren't known; ours are
  estimates fitted in the container (lavapipe, Cartographer's map, a
  frame every 5 s of the loop) to the owner's Cartographer screenshot,
  whose mean is about (31, 39, 61) and whose middle half of blues runs
  from 47 to 71: the navy (101, 121, 176), `k` 0.03, and saturation 0.
  The scene alone averages about (80, 77, 76); veiled, 20 frames at
  1280x720 average (32, 40, 62), within 1 of the screenshot, with the
  middle half of blues from about 50 to 70. Saturation 0 because the
  screenshot is all one hue: multiplying the colours alone left the
  brown sky brown-black and the water purple. `H2LOBBY_VEIL=r,g,b,k`
  or `r,g,b,k,s` tries others (`off` for none).
- **Departure from the brief.** It asked for the veil as two blended
  draws over the scene (multiply by the navy, then add it). That can't
  grey the picture first, which the fit needed, so the scene is drawn
  into a texture of its own and the veil is a full-screen pass that
  reads it; `H2LOBBY_VEIL` takes the optional fifth number for the
  saturation.
- **`h2scene::backdrop`** (`Backdrop`, behind the `SceneSource` trait
  the lobby's tests fake): a thread that reads mainmenu.map, then on
  each ask opens the device if it has none (logging the adapter once
  and each wake's time), draws the newest ask (older ones are dropped)
  at `seconds` into the loop, and puts the frame in a one-slot mailbox
  (the frame it replaces is drawn into next, if the lobby has let go of
  it). Asked to sleep it drops the renderer and everything on the GPU
  but keeps the scene's data. Its state is Loading, Ready or Failed
  with why (an error or a panic on the thread). Draw and read-back
  times are logged once per size.
- **The lobby** (`App::scene_frame`, `draw_menu`): `mod.rs` starts the
  `Backdrop` when Vista's files were found, `H2LOBBY_SCENE` isn't `off`,
  `lobby.txt` doesn't say `background = still` (a new menu line), and
  the lobby has a window (or `H2LOBBY_SCENE=on`, which also allows a
  software GPU); otherwise it logs why not. On the start screen and the
  main menu each draw asks for a frame of the window's size at the
  flythrough's clock and copies the newest of that size in where the
  still background would go (the still background otherwise, at a new
  size until a frame of it comes). The first frame fades in over the
  still background in 0.25 s (`SCENE_FADE`, an estimate; Halo 2 had the
  scene from the start, so it shouldn't pop in). The flythrough's clock
  runs only on those two screens, at most 0.25 s a frame
  (`SCENE_STEP`). `tick` puts the scene to sleep on every other screen
  and while a match exists, and `on_match` does before the engine
  starts; it wakes when the menus next ask, the last frame showing
  meanwhile. A failure is logged once (`menus: the scene can't be
  drawn (<why>); the still background from now on`) and the still
  background stays for the run. The lobby still draws every 33 ms (the
  plan's 30 fps default); `H2LOBBY_SCENE_FPS=60` makes it 16 ms while
  the scene shows. The per-screen timer adds the copy's time (`the
  scene's frame copied in <n> ms`).
- **`examples/scene_shot.rs`**: frames at chosen times and size to PNGs
  with each one's mean colour and times, for the look check and for
  tuning the veil on a PC.
- **Tests (no game files).** Rows padded to 256 bytes and unpacked past
  the padding; sRGB round trips; the veil's grey, multiply and add, and
  its four- or five-number form; the mailbox keeping the newest, only
  the newest ask drawn, a missing map failing with why. A GPU test,
  skipped with no adapter (or only a software one, unless
  `H2LOBBY_SCENE=on`): a made-up red square in the sky and a lit green
  one in the level, through the real pipelines and veil, checked pixel
  by pixel against `Veil::apply_srgb` (it runs on lavapipe with
  `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
  WGPU_BACKEND=vulkan H2LOBBY_SCENE=on`). The lobby, with a fake scene:
  the scene's colour under the start screen's and main menu's rows once
  faded in, the still background before and a blend half way; none on
  the settings or the playlists; a sleep (once) on leaving and none
  asked for while a match is on (`on_match` sleeps it); the clock
  carrying on where it stopped and stepping at most 0.25 s; a failed
  scene giving the still background and one log line; the window's
  16 ms only with `H2LOBBY_SCENE_FPS=60` and the scene showing; the
  fade's alpha and the blend; the timer's line; `lobby.txt`'s
  `background`; and the reasons it isn't started. Ignored, with `H2_MAPS`
  (and `H2_SCENE_PNG` to write PNGs): `draws_the_real_main_menu`, at 10,
  40 and 70 s, not flat and navy.
- **Measured in the container** (lavapipe, a software GPU sharing the
  CPU, so only the shape counts): the scene read in about 0.1 s, awake
  (device, pipelines and 39 textures) in about 105 ms, 25 to 280 ms to
  draw a 1280x720 frame and 1.5 to 2.2 ms to read it back; the main menu
  8 to 18 ms a frame on the UI thread with the copy 0.6 to 2.1 ms. The
  real-map test's frames at 640x360 average (29, 37, 58), (35, 44, 68)
  and (36, 44, 68).
- **Looks like / still missing** (from the container's frames next to the
  owner's screenshot): the carrier, the city, the tether and the water
  in one navy, as in the screenshot. Missing: the carrier's gravity-lift
  beam, the flak and other particles, the engines' glow, and bloom. The
  70-degree field of view is still unverified. The cinematic anchor and
  the one-triangle stand-ins for the firing effects showed nowhere in the
  frames looked at.

**On the owner's PC (P5)**, with Cartographer's files found, from the
clone (`C:\Halo 2 Rust Project` or the work folder), built with `cargo
build --release -p h2launch` and run as usual (a variable set first in
the same console: `set H2LOBBY_SCENE=off` in cmd,
`$env:H2LOBBY_SCENE="off"` in PowerShell):

1. After the intro the start screen comes in over the still navy and,
   within a moment, the flythrough fades in behind it and plays on behind
   the main menu. `lobby.log` has `scene: ...mainmenu.map read in <t> s
   (45919 triangles, 39 textures, a 99.7 s flythrough)`, `scene: drawing
   on <your graphics card> (<kind>, <backend>)` and `scene: awake in <n>
   ms`.
2. In a maximised 1920x1080 window, after half a minute on the main
   menu, the log has `menus: 1920x1080, main menu <n> ms a frame on the
   CPU (the scene's frame copied in <m> ms; ...)` with `<n>` at most
   about 9, and `scene: 1920x1080, <a> ms to draw a frame and <b> ms to
   read it back (on its own thread)`.
3. SETTINGS, then B: `scene: asleep (its GPU device closed)`, then
   `scene: awake in <n> ms`, and the flythrough carries on from where it
   was.
4. A match (an offline one or a custom game): the log has `scene:
   asleep` before the engine's lines, and during the match Task
   Manager's GPU column (Details tab) shows the launcher's lobby process
   at about 0. After the match, back on the main menu, it wakes.
5. `H2LOBBY_VISTA=off`: the still background, and `scene: none (no Halo 2
   Vista folder); the menus have the still background`.
   `H2LOBBY_SCENE=off`, or `background = still` in `lobby.txt`: the
   still background, with that reason on the same line.
6. A forced failure, `WGPU_BACKEND=metal` (Windows has no Metal): the
   still background and one line, `menus: the scene can't be drawn (no
   GPU adapter (...)); the still background from now on`.
7. Resizing the window: the still background for a moment, then the
   flythrough at the new size.
8. Optional, the look: `cargo run --release -p h2scene --example
   scene_shot -- "C:\Games\Halo 2 Project Cartographer\maps"
   C:\h2work\scene 1920x1080 10 40 70` writes three PNGs (keep them on
   the PC) and prints each frame's mean; the container's lavapipe gave
   about (29, 37, 58), (35, 44, 68) and (36, 44, 68) at those times. A
   real GPU should be within a few levels; if the PC's screen looks
   lighter, darker or off-hue next to the Xbox's or Cartographer's
   menu, try `H2LOBBY_VEIL=101,121,176,0.03` with other numbers and
   send the ones that look right.
9. Optional: `H2LOBBY_SCENE_FPS=60` makes the flythrough smoother; its
   frame time is in the same log line.

### Phase 3: matchmaking, pregame, loading and the carnage report

**1. MATCHMAKING (0xD2).**

- The three-step indicator follows the server's stages:
  1. "Searching for the best possible game..."
  2. "Waiting for additional players..."
  3. "Attempting to join a game...", "Starting the game..."
- On the right, players sorted by team, with level icons.
- "Game starts in N" from the pregame roster that today's Pregame screen
  shows.
- B asks before leaving. Once a game is found, B is refused (`flag_fail`)
  for the first 60 s, as Halo 2 did (H).
- The spinning ring is left out (section 1.4).

**2. The custom game countdown** in the leader's lobby:

- "Game start countdown in progress"; X cancels it for the leader (the
  decompilation; this said A before).
- `countdown_for_respawn` at 3, 2, 1 and 0.
- Then the leader's lobby sends the custom game as today. The server
  doesn't change.
- The length is unverified. The code passes 10 and 3 (L).
- X delays it, for the others; that needs the server and comes later.

**3. LOADING... and PLEASE WAIT.**

- "LOADING..." in `super_large` while the engine starts.
- The PLEASE WAIT lines follow the child's events: "Connecting to game...",
  "Loading map...", "Starting game...".
- "Leaving game..." shows after B, while the engine closes.

**4. POSTGAME CARNAGE REPORT.**

- 16 rows, with page arrows at the top right; Left and Right change pages.
- Pages we can fill:
  - TEAM STATS (team games): team, place, score;
  - PLAYER STATS: place, score, best spree, average life;
  - KILLS: kills, assists, deaths, suicides.
- Best spree and average life come from the results block's inferred
  "in a row" and "seconds alive". They show "-" until
  `results::COUNTS_SEEN`, as kills and assists do today.
- PLAYER VS. PLAYER, MEDALS and HIT STATS need data the engine's results
  block hasn't given us yet. They come once it does.
- The level change from MATCH_OVER sits under the table.
- A selects a player, and B goes back to the pregame lobby.

**Done when:** a real matchmade game and a custom game on the owner's PC go
from START GAME to the carnage report and back to the lobby, on the new
screens.

### Phase 4: extras, each on its own

- **In-game GAME MENU** (Leave Game, Settings, End Game) drawn in the
  match engine's `end_frame`. Built on `launcher-gamemenu` (2026-10-11,
  below); it waits for the PC checks in section 9.5. Party Privacy stays
  on the lobby's Party screen for now.
- **Intro movie** at start-up, if one is found in the player's files (check
  J), with a skip option, and the **attract movie** after 75 s idle on the
  start screen or main menu.
  - Vista's WMV plays through menu-preview's Media Foundation code.
  - Bink 1 needs FFmpeg, which is LGPL, so it would go in a separate crate.
  - Bink 2 has no open decoder: play it through the engine, or skip it.
- **SYSTEM LINK**, if a LAN path is built.
- **SPLIT SCREEN** with more than one pad, if the engine takes them.
- **Key rebinding.**

#### What the in-game GAME MENU built (2026-10-11)

Halo 2's multiplayer GAME MENU (screen 0xC3) over a running match, in three
stages on `launcher-gamemenu`. Stages 1 and 2 are tested on Linux; the
engine side (stage 3) ran on the owner's PC on 2026-10-11 (section 9.5
has the results), and the follow-ups from those checks are in: the
reload message on by default, the controls saved once a visit, End
Game's result sent as a game cut short, and the leave notice.

- **What the player sees.** Start on the pad, or Esc in the game's window,
  opens one opaque panel; the match keeps running behind it. Rows, in Halo
  2's order, only the ones the launcher can back: Leave Game, Settings
  (straight to CONTROLLER SETTINGS, as Halo 2 did for a player with no
  voice options), and End Game for a custom game's host and in `--offline`
  runs. No Resume row: Start, B or Back close it, as in Halo 2. The players
  column (team colour, gamertag, HOST) comes from the session file. Leave
  Game and End Game ask first, with the focus on Cancel. The words are
  ours (`gamemenu::words`).
- **Logic** (`crates/h2launch/src/gamemenu/`, plain Rust): the pages and
  keys (`GameMenu`), the pad's presses with a 250 ms repeat (`edges`), and
  what the engine's input polls get (`gate`). **Panel**
  (`crates/h2ui/src/gamemenu.rs`): drawn on the CPU in Halo 2's fonts from
  MCC's `halo2\h2_fonts`, or the built-in font.
- **Lobby hand-off** (stage 2): the engine says `H2EVENT controls ...` (the
  eight `lobby.txt` pairs, after the word `defaults` for Use Default
  Settings) once the player leaves CONTROLLER SETTINGS (its value pages
  and the defaults question count as part of it) or the menu, a second
  after the last change, or before a quit, whichever comes first
  (`gamemenu::save`; check 6 saw 24 saves for 24 steps of Mouse
  Sensitivity when it went at every change). It says `H2EVENT left game`
  or `left end` before it quits; after `left end`, `ended` follows with
  the result the host's engine gives as it quits (see the quit, below).
  `H2EVENT menu off` comes if the menu turns itself off mid-game (the
  lobby's in-game screen then stops saying Start opens it). The lobby saves, through `App::set_control`, only the
  controls that changed from those the engine runs with (its own, with
  any control flag in `H2LOBBY_ENGINE_ARGS` over them), so a test's
  one-run flag isn't saved, and every one after `controls defaults`; it
  lands a player who left on ONLINE (a
  custom game's leader on Custom) and passes `--game-kind`. `PROTOCOL`
  and `LIVE_PROTOCOL` are unchanged.
- **The driver** (`win/menu.rs`) runs on the window thread, which ticks at
  least every 5 ms. It reads the pad the engine's polls picked (without
  disturbing their bookkeeping), the input script, and the menu keys our
  window got (Esc, the arrows, Enter, Backspace; auto-repeat ignored for
  Esc, Enter and Backspace). It opens once the fonts are loaded (on a
  thread of their own, from map load), while the game is on and no quit
  has started. It redraws the panel only when the page or the back
  buffer's size changes and logs how long that took.
- **The overlay** (`win/overlay.rs`): the panel becomes a texture of ours,
  made on the window thread through the shared device. In `end_frame`,
  before the screenshot capture and the Present, the render thread copies
  it into back buffer 0 with one `CopySubresourceRegion`: no pipeline
  state is touched, a busy lock never makes it wait (it keeps its own copy
  of the last panel), and the back buffer is let go before `end_frame`
  returns, so `ResizeBuffers` keeps working. The driver draws the panel for
  the swap chain's size, which only our own resize changes (on the window
  thread, while the engine waits in slot 2), so after a resize the next
  tick draws it again. A back buffer of another size, format or sample
  count is skipped, as a safety net. Any Direct3D error or panic turns the overlay off
  for the run. The 10 s summary gets an `overlay: N copies, F failures,
  mean X us, max Y us` line.
- **The input gate** (`win/input.rs`, rules in `gamemenu::gate`), for
  player 0 on slots 36 and 37; players 1 to 3 are unchanged.
  - Before the menu is ready, nothing changes.
  - Once it is ready, Start and Esc are the menu's and never reach the
    engine.
  - While it is open, the engine gets a neutral frame (no buttons,
    triggers, sticks or keys), keeping the cursor and the keyboard-or-pad
    flag it had when the menu opened, so the HUD's prompts don't switch.
  - Whatever is held when it closes stays masked until released
    (a trigger counts as released at 30 of 255 or below, or, on a pad
    whose trigger rests higher, 16 above its lowest reading since the pad
    was picked, at most 80; a trigger resting where it rests is never
    held), so closing with B doesn't
    melee and A on the last value doesn't jump. Closing adds the driver's
    last reading to that hold before clearing the open flag, so no poll
    can see a held button in between.
  - Slot 37 (the pad only) masks through a copy of the hold and never
    releases it; slot 36, which sees everything, does.
  - Losing focus drops the hold's keys and mouse buttons; another pad
    being picked drops its pad buttons.
- **Live controls** (`win/mod.rs`): one atomic holds the controls the menu
  set, read by the input polls (stick layout, look flip, mouse) and by
  rumble. Turning vibration off stops the motors at once, and while it is
  off every rumble call from the engine sends zeros (so one racing the
  switch can't leave the motors on).
- **Profile and mapping generations** (`win/host.rs`): host slots 34
  (`get_player_profile`) and 116 (`get_player_gamepad_mapping`) serve the
  newest generation. Generation 0 is the launch buffers, as before; when
  the menu closes having changed the controls, new bytes are built
  (`profile::profile_for`, plus `--set-profile`, and the button layout's
  mapping) and swapped in only when they differ. One generation a visit,
  not one a step: the engine gets neutral input while the menu is open,
  so it misses nothing by the wait. Old generations are never freed, since
  the engine may keep a pointer (at most 64 of each kind per run; past
  that the menu treats that kind as read only at load). The log says when the engine first gets a
  new one (`profile generation N first served at call #k (t s)`, and the
  same for the gamepad mapping). The engine doesn't ask again by itself
  (check 7), so after a change to Button Layout, Look Sensitivity or
  Auto Look Centering (or vibration turned on) the driver posts the
  reload message once the new generations are in place: engine message
  5 by default (`--reload-message N` picks another, 0 or `off` none),
  which makes the engine ask for slots 34 and 116 again (check 8). It
  isn't posted once the game has ended or a quit has started; the next
  game loads the saved controls anyway.
- **The quit.** Leave Game tells the lobby, lets the lobby take the
  foreground, and quits the engine by the user-quit path; the run's
  result line says `left from the game menu` or `ended from the game
  menu`. The result the engine gives as it quits is ours as a quitter
  and isn't passed on. End Game quits the same way, but its quit-time
  result is passed on (`H2EVENT ended`, `gamemenu::result_is_the_games`):
  on the host it holds every player's standing and score (check 14). The
  lobby sends it as LAUNCHER_RESULT with `finished` false, then
  LEFT_MATCH at once (`live::Told`); h2live then takes the host as
  leaving once the game was over rather than HOST_LEFT, so the match ends
  with no reason and counts for no one (custom games are unranked), and
  the host lands on the carnage report like the guests, whose engines
  end the game themselves when the host goes (check 14). That holds once
  the server's GO came; End Game before it (a guest still joining) has
  its result dropped by h2live, so the lobby sends LEFT_MATCH alone and
  keeps no result (`Told::ended_before_go`). "You ended the game." is
  left for that and for an engine that closes without a result. No engine
  message ends the game for everyone (check 15), so End Game doesn't use
  one.
- **Safety** (`gamemenu::watch`, Linux-tested; the driver does what it
  says). A panel not drawn within 1 s while the window isn't minimised
  closes the menu and lets go of the game's input; three of those, or an
  overlay failure (a failed copy, or a texture that can't be made for the
  panel), turn the menu off for the run, Start goes back to the engine,
  and the lobby is told (`H2EVENT menu off`). PLEASE WAIT is exempt from
  both (it lasts until the process ends, input neutral and Start the
  menu's): a failure seen under it hides the panel, and turns the menu off
  only once it closes. An End Game waiting on `--end-game-message` still
  quits on time if the menu is turned off meanwhile. A natural end while the menu is open closes it
  (`game menu: closed (game over)`), so the scoreboard shows, and it can't
  be opened again after the game has ended.
- **Flags.** `--no-game-menu` (the menu off; Start and Esc reach the
  game). `H2LAUNCH_OVERLAY_FAIL=1` (environment; the same, as a stand-in
  for an overlay failure, for check 11); `=stall` (the panel is never
  copied, as if never drawn) and `=copy` (the first copy fails, as a
  Direct3D error would) fail at run time instead, for check 17.
  `--reload-message N` (the reload message above: 5 unless given; 0 or
  `off` turns it off, and Button Layout, Look Sensitivity and Auto Look
  Centering are then tagged NEXT GAME once changed, as before).
  `--end-game-message N` (End Game posts N and quits 10 s later if the
  game hasn't ended; kept for research, check 15). `--game-kind` (the lobby
  passes it; it defaults to social with `--session`, to the kind of the
  server's match with `--live`, and to offline otherwise). The lobby
  leaves out its "Start or Esc ... opens the game menu" line when its
  engines run without the menu.
- **Settled on the PC.** `ENGINE_REREADS_PROFILE` and
  `ENGINE_REREADS_MAPPING` in `win/menu.rs` stay false: the engine
  doesn't ask for a new profile or mapping by itself (check 7). With the
  reload message on (the default) the three settings below that depend
  on them aren't tagged NEXT GAME and their value pages have no note
  (`gamemenu::Pickup::Reload`); with it off they are, as before. The
  engine doesn't read the keyboard by itself while the menu is open: W
  and Space didn't leak (check 5).

**Settings in the GAME MENU** (CONTROLLER SETTINGS, nine rows, the same
list and `lobby.txt` keys as the lobby's Settings):

| Setting | How a change is applied | Mid-match? | Where |
|---|---|---|---|
| Thumbstick Layout | the launcher swaps the sticks before the engine's poll | at once | launcher (`sticks.apply`) |
| Button Layout | a new gamepad mapping generation, then the reload message | when the menu closes (check 8); NEXT GAME with `--reload-message 0` | slot 116 |
| Look Sensitivity | a new profile generation (0x1B5, 0x1B6), then the reload message | when the menu closes (check 8); NEXT GAME with `--reload-message 0` | slot 34 |
| Look Inversion | the launcher flips the look stick's up and down when the live value differs from the launch one; profile 0x1D keeps the launch value, so the engine never flips it twice | at once | launcher (`gate::look_flip`) |
| Auto Look Centering | a new profile generation (0x23), then the reload message | when the menu closes (the engine re-read the profile in check 8, though nothing visible changed); NEXT GAME with `--reload-message 0` | slot 34 |
| Controller Vibration | the launcher's rumble follows it; off stops the motors; also profile 0x1F | off at once; on, from a game started with it off, NEXT GAME (the engine's profile said off, and the launcher can only pass on rumble the engine asks for; check 8 didn't try whether the reload message changes that) | launcher (`rumble`), slot 34 |
| Mouse Sensitivity | the launcher's mouse-to-look; also profile 0x410 | at once | launcher (`mouse_motion`) |
| Mouse Inversion | the launcher's mouse-to-look (profile 0x1E stays 0) | at once | launcher (`mouse_motion`) |
| Use Default Settings | every row above back to Halo 2's defaults | as each row | all of the above |

Every change is saved, once a visit to CONTROLLER SETTINGS (when the
player leaves it or the menu, a second after the last change, or before
a quit): through the lobby (`H2EVENT controls`) when the
lobby started the engine, or straight into `lobby.txt` in the instance's
settings folder in `--offline` runs, which also print the same `H2EVENT
controls ...` line in the log. Changes that end where they started save
nothing. Those runs save only the settings changed
in the menu, so a control flag given for one run (`--layout`,
`--no-vibration`, ...) isn't saved with them, even when a change lands
on Halo 2's defaults; Use Default Settings (said as such by the menu, not
guessed from the values) saves every one. The lobby does the same with
the engine's events: only what changed from the controls the engine runs
with, and every one after `H2EVENT controls defaults ...`. The PC checks
are in section 9.5.

## 8. Option A's work, if it is chosen

Separate from the phases, because it only starts after the PC proofs pass:

1. `h2launch --menu-engine`: the engine on `mainmenu.map` in UI shell,
   with neutral input in slot 36, reading draw lists on standard input and
   writing input lines on standard output.
2. The D3D11 quad renderer in `win/`. If the lobby window needs speed, it
   can draw there too.
3. The lobby hides its own window while the menu engine's is up, and shows
   it again if the menu engine fails to start or dies. A crash never costs
   the party.
4. Measure, and write down in this file, the time from START GAME to the
   match engine's map loaded, and from the game's end back to the menu.

## 9. Checks on the owner's PC

Read-only: print to the console, never save or upload dumps or pictures,
and write the results here in our own words.

### 9.1 Before anything else

**A. Input.**

- In the lobby window: arrows, Enter and Esc; a pad's d-pad, A and B.
- In a match: walking with the keys, looking with the mouse, and the pad's
  sticks and buttons.

The logs say which path drops input.

**Result** (2026-10-10): input was dead because the launcher handed the
engine an all-zero gamepad table (slot 116) and an empty keyboard table
(profile 0x42C). Fixed on `launcher-controls`; the pad works in matches
with Default and Bumper Jumper. Details in `README.md` here, "Controls".

**B. The UI-shell experiment** (being run separately). In this order:

1. Does `mainmenu.map` boot in UI shell with no patch, and play the
   flythrough at 60 fps?
2. Does the engine draw any UI or text of its own over it? Does it take
   input?
3. Does a coloured quad drawn in `end_frame` show, with the next frame
   untouched?
4. Can the menu engine be closed and a match engine started, and how long
   does each step take?
5. Does neutral input in slot 36 keep the engine idle?
6. Does `main_menu_music` play?

**Results** (owner's PC, 2026-10-10, halo2.dll 1.3528, read-only: only
options, host answers through `--slot-return` stubs, and diagnostics):

1. **Boot.** `--offline` with options `game_mode` (0x0C) = 4 and both map
   id fields (0x10, 0x14) = 0 boots with no patch. The engine opens
   shared.map, single_player_shared.map and mainmenu.map (twice), and
   nothing else; frames start at about 0.5 s and run at 60 fps (600 per
   10 s). It never calls set_game_state, so the launcher sees no game
   state at all. Map ids 1 and -1 behave the same. `game_mode` 3
   (multiplayer) with ids 0, 1 and 2 also loads only mainmenu.map, reaches
   no map-loaded state, and shows the same picture; on quit it goes
   through states 10, 7, 5. mainmenu's own map id wasn't found: halo2.dll
   builds scenario paths in code, and no plain table gives an id.
2. **What it shows.** A flat dark navy frame, rgb(21,28,51), with no logo,
   start screen, list or text. Stretched about 120 times, the frame holds
   a real moving scene under it (shapes change between 4, 16 and 25 s) at
   about 1/255 of full brightness, so something covers the scene almost
   completely; whether it is the carrier flythrough can't be told at that
   level. Scripted Start, A and d-pad presses (with `--pad-map h2`) don't
   change the frame. In the first 30 s the shell calls host slots 0, 1, 2,
   10, 23 (about 1,850 calls in the first 0.7 s: phase 1, then phase 5
   with progress stuck at 0.02; a match goes on to phase 4 and then
   loads), 32, 33, 34, 36, 39, 43, 47, 48, 51, 55, 57 get_string (79
   calls, 47 ids from two string tables), 88 get_player_xuid (once a
   frame), 94 (once), 99 (14), 100, 104 (once), 116 and 118, and events
   31 (once a frame), 142 and 147. A match calls 22, 54, 72, 74 and 75 and
   events 144 and 146 instead, and the shell never calls them. Returning 1
   from each of 57, 94, 99, 100, 104 and 118 in turn (`--slot-return`)
   changes nothing: the frame stays navy (mean colour 21, 28, 51 in every
   region measured). What MCC answers that makes the shell draw is still
   unknown; the stuck launch timer (phase 5 at 0.02) is the best lead.
3. **Drawing over it.** Both work, in UI shell and in a match, and leave
   the engine's frames clean:
   - copying a block of ours into back buffer 0 in `end_frame`
     (CopySubresourceRegion): about 1.6 us a frame;
   - a quad drawn with our own shaders (compiled at run time with
     d3dcompiler_47's D3DCompile) inside our own device context state,
     swapped in and out with `ID3D11DeviceContext1::SwapDeviceContextState`:
     about 22 us a frame including a new render target view each frame.
     Drawn on every other frame, the frames without it show no trace and
     the engine's picture is normal, so its state is restored.
   - `end_frame` comes before the engine's Present (one Present per
     end_frame). The back buffer is 960x540 R8G8B8A8_UNORM, one sample, in
     a 2-buffer DISCARD swap chain, so buffer 0 is always the one to draw
     into.
4. **Switching engines.** UI shell to first frame: about 0.5 s. Asking an
   engine to quit to its process exiting: about 5 s, nearly all of it the
   launcher's own wait for the engine to finish (5.0 s from quit to the
   final summary, both for the shell and for a match). A Lockout match
   from start to map loaded: 6.0 s. So menu to match is about 11 s today
   (about 6 s if the menu engine is closed at once rather than waited
   for), and match back to menu about 5.5 s.
5. **Neutral input.** With no pad and no script the shell stays idle:
   frames keep coming and nothing changes for as long as it runs (up to
   40 s tried).
6. **Music.** Nothing in the logs shows any (get_audio_setting is asked
   once; no sound-related host calls or events). It has to be checked by
   ear on the PC.
7. **Second round** (same rules). None of these changed the picture (still a flat 21, 28, 51 everywhere) or the launch timer (phase 5 at 0.02, then 1 at 0.00, then 5 at 0.02, then nothing):
   - answering 1 from update_launch_timer (slot 23);
   - posting engine messages 1, 5, 7, 8, 9 and 14 three seconds after the first frame;
   - answering true for setting 6, the only game setting the shell asks for;
   - giving the first eight video-setting floats and the profile brightness a middle value.
   GetGUID (event 147) already returns a valid pointer. The cover over the scene doesn't come from any host answer tried so far.

**Decision (2026-10-10):** option C. The menus are drawn in our own window over Halo 2's navy framing; the quad proof is kept for phase 4 (in-game overlays) and for option A if a way to uncover the shell's scene turns up.

**C. Layouts.**

1. Play a match with `--pad-map h2`, then with a table that swaps A and B.
   Does the engine follow slot 116?
2. If not, set `BUTTON_PRESET` to 1, 2 and 3 in the profile. Does Southpaw,
   Boxer or Green Thumb follow? The numbering is unknown.

**Result** (2026-10-10): the engine follows slot 116, so every layout is
built there and `BUTTON_PRESET` wasn't needed. Default and Bumper Jumper
were checked button by button; the other layouts wait until after the MVP.

### 9.2 MCC's UI tags (the probe, `mcc_ui_probe`)

Expected Vista values are in brackets.

**A. The header and string ids.**

- Print the header fields 0x18, 0x1C, 0x30 to 0x3C, 0x2D0, 0x2E4/0x2E8,
  0x2EC and 0x2F0/0x2F4.
- Pass: the string index and data lie inside the image, string 0 is empty,
  and the last string ends at the data size.
- Pass: `main_menu`'s header id (`wgit` +0x2C) resolves to a name whose
  length is `id >> 24`.

**B. Tag counts.**

- Count `wgtz`, `wigl`, `wgit`, `skin`, `unic`, `bitm`, `snd!`, `lsnd`,
  `ugh!`, `matg`, `goof` and `scnr` [1 wgtz, 1 wigl, 131 wgit, 38 skin;
  the wigl lists 26].
- List the `wgit` names found in only one of the two maps.

**C. `wgtz ui\main_menu`.**

- +0x0 is a `wigl` reference whose datum matches the index.
- +0x8 holds 133 screens, every one a `wgit` reference that resolves.
  Nested addresses land inside the tag's own meta.
- +0x10 is a `goof` reference, and +0x18 a `unic` one.

**D. `wigl`.**

- 0x44 [0.10] and 0x6C [0.85, 0, 0.08, 0.17].
- The first five sound references at 0x90 [cursor1, forward1, flag_fail,
  advance, back1].
- 26 skins at 0x138, header fonts at 0x160 below 12, and the full header
  rectangle at 0x178 [567, -730, 520, 50].
- The music at 0x1B8 [`main_menu_music`], and at 0x1C0 how long before
  the attract movie it is stopped [5500] (not a fade; section 5).

**E. `wgit main_menu`.**

- The NO_HEADER flag, screen id 6 and legend 0.
- The list in skin 11, 6 visible, at (-178, -80).
- The logo at (-511, 90).
- The 11 tracks of `mainmenu-plan.md` 1.3.

**F. `skin main_menu`.** Item animations of 120, 200 and 90 ms, with alphas
0.5, 1.0 and 0.7.

**G. Strings.**

- The English table resolves, and an entry's offset lands on the start of
  a string.
- Print the main menu's and the Live menu's labels, to see whether MCC
  holds the Xbox wording.

**H. Bitmaps.** For every picture the start screen, main menu and shell
background name, print:

- the size, format, flags and mip count;
- the native mip info and tile mode;
- the LOD offsets and sizes, and the pointer's top bits;
- the `textures.dat` record's chunks.

Then decode each one and compare it with Vista's by hash. Note any format
number MCC alone uses, and the sequences of `list_bkd`, `track_brace` and
the button pictures.

**One diff covers C to F.** Run `ui.rs` on both maps and diff the text. It
should match except for datums, string-id numbers and addresses, which the
parse turns into names.

**Results** (owner's PC, 2026-10-11, `launcher-menu` 24ed025, MCC build
2025.08.16; no engine run, the PNGs stayed on the PC):

- The probe passed 31 of 32 checks. The one failure was the check that
  every nested block lies inside its tag's meta: 285 don't (the first in
  `squad_lobby_player_list`'s skin). Since every screen and skin still
  matches Vista's, format 13 seems to share blocks between tags; the probe
  now prints this as a note.
- A: the string-id table is where predicted (0x30 count 7,907, 0x34 data,
  0x38 size, 0x3C index); the English strings are found through the
  globals at +0x190 (0x2E4 is -1). `main_menu` and `game_shell_background`
  have no header id; `settings_screen_header` resolves.
- B: 1 wgtz, 1 wigl, 131 wgit (the same names as Vista's), 38 skin tags
  (the wigl lists 26), 512 bitmaps. MCC's shared.map didn't open (its
  first tag name points past the names); the menus don't need it.
- C to F: every value above is as expected, and the parsed globals, skins
  and all 133 screens are the same as Vista's.
- G: 4,403 English strings. MCC renames two main menu rows: "LIVE" for
  XBOX LIVE and "NETWORK" for SYSTEM LINK, and adds SIGN OUT, QUIT and
  GUIDE. The Live menu keeps "XBOX LIVE" as its header, with Quickmatch,
  Optimatch, Create Party, Content Download, Gamer Details, Clan Details
  and Message of the Day. The launcher keeps the Xbox wording.
- H: all 56 images of the 33 pictures decode, every one A8R8G8B8 and the
  same pixels as Vista's. The start screen's logo is 1024x128, in two
  streams.
- `menu_png` found all 16 pictures it wants in both maps and Halo 2's fonts
  in `halo2\h2_fonts`. Described by the PC session: the main menu shows the
  navy background with the blue framing at the top left and bottom right,
  the faint scrolling tracks, the silver-to-blue HALO 2 logo, and the rows
  in Handel Gothic with the focused one brighter over its glow bar; the
  start screen shows PRESS START under the logo. No stripes, swapped
  colours or flips; Vista's look the same.
- CPU time a frame at 1920x1080 with MCC's art: start screen 3.5 ms, main
  menu 5.3 ms, plus the background drawn once (about 20 ms) when the window
  size changes. At 1280x720: 2.1 ms and 3.2 ms, background 13 ms. Well
  under 16 ms, so the CPU backend stays for phase 1.
- The ignored real-file tests pass (`mainmenu` in blam-cache, `real_` in
  h2ui).

### 9.3 Sounds and movies

**I. Sounds.**

- Find `ugh!` (header 0x2EC) and print its distinct codec triples.
- For `cursor1` and the music's `in` and `loop`, follow the chain from
  `snd!` to the pitch range, the permutation and the chunk.
- Print each chunk offset's top bits, and whether offset plus size fits:
  - this map;
  - the file as stored;
  - `shared.map`;
  - each `*.dat` beside it.
- Print the first 16 bytes on the console only. Look for `OggS`, an Opus
  pattern, the ASF header (30 26 B2 75), or something like ADPCM.

**J. Folders and movies.**

- List `halo2\`'s folders and `h2_maps_win64_dx11\*.dat` with their sizes.
- Search the MCC install for `*.bik`, `*.bk2`, `*.wmv`, `*.mp4`, `*.webm`
  and `*.usm`, printing each one's first 4 bytes. `BIK` and a letter is
  Bink 1; `KB2` is Bink 2.
- Search halo2.dll's strings (read-only) for `.bik`, `bink`, `intro`,
  `attract` and `movie`. Write down only the facts.

### 9.4 Look

The owner compares the start screen, the main menu and one sub-screen with
his memory of the Xbox, or with a capture of it, at 1080p and at 720p.

Things nobody has checked:

- the font sizes;
- whether unfocused rows really sit at 50%;
- the bottom-right text;
- how the Y menu tabs and the carnage report pages change;
- the countdown's length.

A 2004-era video of the Live flow would settle most of them.

### 9.5 The in-game GAME MENU

For the Remote Control session on the owner's PC, in this order. Same rules
as above, except that the launcher's screenshots are saved: they stay on
the PC, are described here in words, and are never uploaded or committed.
Nothing from MCC leaves the PC. The results of the first run (2026-10-11)
are after check 17; check 8 made the reload message the default.

Every run, unless a check says otherwise: `target\release\h2launch.exe
--offline --instance 9 --windowed 1280x720 --diag --input-script
C:\h2work\scripts\<name>.txt`, plus the check's own flags. Instance 9 keeps
the saved controls away from the owner's own. Each script starts with
`base state1` (times count from map load); keep every log.

1. **Build.** A fresh worktree `C:\h2work\gamemenu` on `launcher-gamemenu`
   (`git fetch`, `git worktree add`); `cargo build --release -p h2launch`
   and `cargo test --release -p h2launch`. Write down the commit.
2. **A raw Start, for reference** (not pass or fail). With
   `--no-game-menu`: `3 stick L 0 1 for 20`, `8 press start`, `9
   screenshot`, `12 press start`, `14 key esc`, `15 screenshot`, `18 press
   back for 1`, `18.5 screenshot`, `25 quit`. Record the first calls of
   host slots 7 and 8 (pause) with their arguments, whether the summaries
   stay near 600 frames per 10 s, whether the Spartan kept walking
   (shots 9 and 15), what the engine drew, and what each press logged.
3. **It draws and leaves no trace.** `5 press start`, `5.5 screenshot`, `6
   press down`, `6.5 screenshot`, `7 press a`, `7.5 screenshot`, `8 press
   b`, `8.5 press b`, `9.5 screenshot`, `12 quit`. Pass: `game menu:
   ready` with the fonts named; shot 5.5 is GAME MENU (Leave Game,
   Settings, End Game) over the live game; 6.5 has Settings focused; 7.5
   is CONTROLLER SETTINGS with its nine rows and values; 9.5 is a clean
   game frame; the summary has `overlay: N copies, 0 failures`; frames per
   10 s within 2% of check 2.
4. **Neutral input and the hold.**
   - A: `3 stick L 0 1 for 12`, `3 press RT for 12`, `6 press start`, `6.2
     screenshot`, `8 screenshot`, `10 press b for 2`, `13 screenshot`, `17
     quit`. Pass: from 6 to 10 the `input:` lines are neutral (marked
     `(game menu open)`) while frames go on; shots 6.2 and 8 show the
     Spartan in one place; no B (no melee) until it is let go at 12; RT
     masked until it is let go at 15 (no firing); the stick walks again at
     once after 10; shot 8 shows the focus still on Leave Game (a stick
     held when the menu opens doesn't move it).
   - B: `4.8 press a for 2`, `5 press start`, `6 press start`, `9 press
     a`, `12 quit`. Pass: one jump at 4.8 (before the menu); the menu
     opens at 5 and closes at 6; no A reaches the engine from 5 until it
     is let go at 6.8 (no jump at the close); the press at 9 jumps. (The
     first draft pressed A at 5.8 with the menu open, which opens the
     LEAVE GAME dialog, so the Start at 6 would have done nothing.)
   - C: `5 press start`, `6 press back for 1.5`, `9 quit`. Pass: no
     scoreboard while Back stays held after the close.
   - D, only with a pad whose trigger rests above 30 (the `--diag`
     `input:` lines show it at rest): open and close the menu without
     touching the trigger, then pull it. Pass: it fires.
5. **Keyboard.** `5 key esc`, `5.5 screenshot`, `6 key down`, `6.5 key
   enter`, `7 screenshot`, `8 key backspace`, `9 key esc`, `9.5
   screenshot`, `11 key esc`, `11.5 key w for 2`, `11.6 key space for
   0.2`, `12 screenshot`, `13.5 screenshot`, `14 key esc`, `16 quit`.
   Pass: Esc opens; Down and Enter open CONTROLLER SETTINGS (shot 7);
   Backspace goes back and Esc closes (9.5 is clean). Record, as a known
   limit rather than a failure: the `--diag` `poller flag` lines, and
   whether shots 12 and 13.5 show walking or a jump with the menu open.
6. **Settings that apply at once.** Each change is a menu path in 0.5 s
   steps.
   - Thumbstick Layout to Southpaw (`start, down, a, a, down, a, b, b`),
     then `stick L 0 1 for 1`: the input lines show it as look values and
     a shot shows the view turned, not moved.
   - Look Inversion to Inverted (`start, down, a, down, down, down, a, up,
     a, b, b`), with `stick R 0 1 for 0.5` before and after: `thumb_ry`
     changes sign; the view goes up before and down after.
   - Controller Vibration off: `game menu: rumble stopped (vibration
     off)`, and no rumble set after it.
   - Mouse Sensitivity 1.6 to 4 (row 6, right 24 times), with `mouse 300
     0 for 1` before and after: about 2.5 times the turn.
   - The log has the `H2EVENT controls ...` lines, and
     `%LOCALAPPDATA%\h2launch\9\lobby.txt` has the new values.
7. **Does the engine read the profile and mapping again?** (By itself:
   since the reload message became the default, run this with
   `--reload-message 0`.)
   1. 30 s untouched; note the `34 get_player_profile=N(+M)` and `116
      get_player_gamepad_mapping=N(+M)` summary lines.
   2. Button Layout to Bumper Jumper (`start, down, a, down, a, down x4,
      a, b, b`); 2 s later `press lb` with shots 0.1 and 0.3 s after it.
      Did it jump? Any `gamepad mapping generation 1 first served` line?
   3. Look Sensitivity 3 to 10, with `stick R 1 0 for 0.5` before and
      after; compare the turn. Any `profile generation N first served`?
   4. Auto Look Centering on: anything visible?
   5. Started with `--no-vibration`, Controller Vibration on (the row says
      NEXT GAME), then fire a weapon: a `first rumble` line with non-zero
      motors, and a `profile generation N first served` line?

   Result: for each setting, whether it took effect mid-match.
8. **Only if check 7 showed nothing.** Repeat it with `--reload-message 5`
   (message 5 was harmless in UI shell; the owner approved these
   experiments; it is the default now, so no flag is needed). Record
   whether the layout and sensitivity now apply, any other change, and
   any crash; stop at the first crash and keep the log. If 5 works, the
   next build makes it the default (it did; see the results).
9. **The game ends with the menu open.** `--set-option 0x354=i32:60`; `55
   press start`, `56 press down`, `56.5 press a`, `62 screenshot`, `63
   press start`, `66 screenshot`, `80 quit`. Pass: `game menu: closed
   (game over)` with `set_game_result`; shots 62 and 66 show the
   scoreboard and no panel; the Start at 63 doesn't open the menu;
   `restart_game` ends the run as before.
10. **Resize and minimise with the menu open.** `5 press start`, `40
    quit`; from PowerShell, resize the game's window to 1600x900 at 8 s
    and 960x540 at 14 s, minimise at 20 s and restore at 24 s; shots at
    10, 16 and 26. Pass: `swap chain resized` lines and no `ResizeBuffers(...)
    failed`; the panel redrawn at each size (`game menu: panel ...`
    lines); input works after. Record whether the menu closed while
    minimised (allowed).
11. **Fallback.** `H2LAUNCH_OVERLAY_FAIL=1` and check 2's script without
    `--no-game-menu`. Pass: `game menu: off (...)`; Start reaches the
    engine as in check 2 (the same slot 7 and 8 lines); no neutral input
    lines.
12. **Cost.** From checks 3 and 9: the overlay's mean and max
    microseconds a frame, frames per 10 s with the menu closed and open,
    and each `drawn in X ms`. Repeat check 3 at `--windowed 1920x1080`.
    Pass: a copy under 50 us, a redraw under 10 ms, the frame rate
    unchanged.
13. **Leave Game online.** Two lobbies as in `lobby.md`'s two-lobby run
    (instances 1 and 2) with real engines, against a local h2live or the
    project's server with two test accounts; a custom game on Lockout. The
    guest lobby gets `H2LOBBY_ENGINE_ARGS=--input-script <guest script>`,
    which sets Button Layout to Bumper Jumper at 5 s and at 15 s does
    `start, a, up, a`. Pass: the guest's log has `H2EVENT controls ...
    button_layout=bumper_jumper`, then `the engine says H2EVENT left
    game`; its engine exits within about 6 s; its lobby comes to the front
    on ONLINE (`live`) with "You left the game." or the server's text;
    instance 2's `lobby.txt` has `button_layout = bumper_jumper`; the
    server logs LEFT_MATCH; the host's game goes on or ends as today; the
    guest's next match logs `gamepad mapping: Bumper Jumper`.
14. **End Game as the custom game's host**, same two lobbies. Host engine
    script: `start, down, down, a, up, a`; the guest's opens the menu,
    takes a shot and closes it. Pass (since the follow-ups; the first run
    passed the earlier rules, see the results): the guest's menu has only
    Leave Game and Settings; the host's engine exits within about 6 s;
    the host's log has `result: End Game's, given as the engine quits`,
    and its lobby's `the engine says H2EVENT left end`, then `H2EVENT
    ended ...`, then `telling the server match ... ended (finished:
    false, 2 player results)` and `telling the server we left match ...`
    in that order; both lobbies show the carnage report with no "THE HOST
    LEFT" wording and no level change, and the host's A from it goes to
    Custom. Record what the guest's engine did before it was asked to
    quit (result, restart, frozen?) with times.
15. **An engine message for End Game** (offline, one id a run, approved by
    the owner). `--end-game-message N`, script `10 press start`, `10.5
    press down`, `11 press down`, `11.5 press a`, `12 press up`, `12.5
    press a`, `40 quit`, for N in 2 to 12, 15, 17, 19 and 20. For each:
    crash or not; the set_game_state, set_game_result and restart_game
    calls with times; shots at 15 and 20 (GAME OVER or scoreboard). Stop
    at the first crash. If an id ends the game (a result while not
    quitting, then restart_game), repeat check 14 with
    `H2LOBBY_ENGINE_ARGS=--end-game-message N` on the host: do both
    engines show the scoreboard, and both lobbies a carnage report?
16. **For the owner, with his pad, next time he plays.** Start opens and
    closes it; the d-pad and stick move the focus and repeat every 250
    ms; closing with B doesn't melee; A on a value doesn't jump; closing
    with Back doesn't flash the scoreboard; Bumper Jumper behaves as the
    menu says; the panel's size, colours and words against his memory or
    a video.
17. **Safety at run time** (offline; `H2LAUNCH_OVERLAY_FAIL` makes the
    overlay fail once the menu shows, as check 11 can't: there the menu
    is never ready).
    - A, the panel never drawn: `H2LAUNCH_OVERLAY_FAIL=stall`, `5 press
      start`, `8 press start`, `11 press start`, `14 press start`, `20
      quit`. Pass: about 1 s after each of the first three Starts, `its
      panel wasn't drawn for 1 s; closed it and let go of the game's
      input (n of 3)`, and the `input:` lines are neutral only in those
      seconds; after the third, `game menu: off for this run (its panel
      didn't show three times)`; the Start at 14 reaches the engine
      (check 2's slot 7 and 8 lines).
    - B, PLEASE WAIT holds: `H2LAUNCH_OVERLAY_FAIL=stall`, `5 press
      start`, `5.2 press a`, `5.4 press up`, `5.6 press a`, `20 quit`
      (Leave Game confirmed within the first second). Pass: no `closed
      it` line; `input:` lines neutral until the process ends about 5 s
      later; no slot 7 or 8 call.
    - C, a Direct3D error: `H2LAUNCH_OVERLAY_FAIL=copy`, `5 press start`,
      `8 press start`, `12 quit`. Pass: `overlay: H2LAUNCH_OVERLAY_FAIL=copy
      ...; the overlay is off for this run`, then `game menu: off for this
      run (the overlay failed)` on the next tick; the Start at 8 reaches
      the engine.
    - D, the lobby told: A again in a lobby match, with
      `H2LAUNCH_OVERLAY_FAIL=stall` set where the lobby starts. Pass: the
      lobby logs `the engine says H2EVENT menu off`, and its in-game
      screen drops "Start or Esc in the game's window opens the game
      menu."

#### Results on the owner's PC (2026-10-11, about 13:00 UTC, 672bf4a)

1. **Build**: pass, 208 unit tests and 5 end-to-end tests.
2. **A raw Start**: each Start or Esc called host slot 7 (`pause_game`)
   once, with its first and fourth arguments 0; slot 8 was never called.
   The engine drew nothing of its own for the pause, and Back showed its
   scoreboard.
3. **Pass.** The panel was 696 by 510 at 304,102 in the 1280x720 back
   buffer, with Leave Game, Settings and End Game, and PLAYERS (1) with
   Player marked HOST; CONTROLLER SETTINGS had its nine rows, and the
   frame after the close was clean. The overlay made 210 copies with no
   failure (mean 1.8 us, longest 11.5 us); a redraw took 0.6 to 1.0 ms;
   600 frames each 10 s, as without the menu.
4. **A, B and C pass**: neutral input while open; B held across the close
   didn't melee, and RT, A and Back held across it did nothing until let
   go. A stick held while the menu is open repeats Up every 250 ms, which
   is the menu's repeat, by design. D wasn't reported.
5. **Pass**: the legend said ENTER SELECT and ESC BACK, and W and Space
   held with the menu open didn't reach the game.
6. **Pass** in a running game: Southpaw, look inversion, vibration off
   (the rumble stopped) and mouse sensitivity all took effect at once.
   Two small notes: 24 steps of Mouse Sensitivity wrote `lobby.txt` 24
   times (now once a visit, `gamemenu::save`), and the dual-wield hint's
   button picture switched between the pad's and the keyboard's as the
   engine's keyboard-or-pad flag flipped while the script sat idle.
7. **The engine doesn't read either again by itself**: the call counts of
   slots 34 and 116 stayed flat after the changes, and Bumper Jumper, a
   new look sensitivity and auto look centering weren't used mid-match.
8. **`--reload-message 5` works, with no crash**: after each change the
   engine called slots 116 and 34 once more; Bumper Jumper took effect
   (LB jumped and A no longer did) and look sensitivity 3 to 10 turned
   two to three times faster; auto look centering showed nothing
   visible. So message 5 is now the default, `ENGINE_REREADS_PROFILE` and
   `_MAPPING` stay false, and the NEXT GAME tags are gone for those three
   settings unless the reload message is turned off.
9. **Pass**: the game's natural end closed the open menu (game over), and
   Start after it didn't open the menu.
10. **Pass**: resizing to 1600x900 and to 960x540 each gave one frame
    whose size didn't match (skipped) and then a redraw; minimising and
    restoring were fine.
11. **Pass**: with `H2LAUNCH_OVERLAY_FAIL=1` the menu stayed off and Start
    reached the engine.
12. **Pass**: copies took 1.6 to 2.1 us on average, under 21 us at most
    (once 85 us); a redraw 0.3 to 1.0 ms at 720p and 1.4 to 1.8 ms at
    1080p; 600 frames each 10 s with the menu open or closed.
13. **Pass but for the notice.** The guest's Leave Game worked, and Bumper
    Jumper picked in the game reached `lobby.txt` and the next match; but
    the guest's lobby said "The match was called off.". The host's engine
    had ended the Team Slayer game by itself once the guest's team was
    empty, and the server's verdict (no reason) reached the guest's lobby
    before its engine closed. Fixed: a leave from the GAME MENU now wins
    over a verdict with no reason ("You left the game."). The run's notes
    also said Custom Game was seven downs from the playlists; a headless
    run of this build from a fresh sign-in needs eight with the eight
    launcher playlists (`lobby.md`, screen 8).
14. **The host passed** (the END GAME question, the engine gone in 5.0 s,
    "You ended the game.", landed on Custom), and the result the host's
    engine gave as it quit held both players and their scores. The
    guest's engine gave its own result 0.02 s later (an ordinary GAME
    OVER), and its lobby showed the carnage report with "THE HOST LEFT.
    THE GAME DIDN'T COUNT."; its engine stayed up until the lobby asked
    it to close, 15.5 s after the verdict. Decision: keep the carnage
    report for the guests (the original showed everyone the end), and
    send the host's result before it leaves: End Game now passes the
    host's quit-time result on, and the lobby sends it as LAUNCHER_RESULT
    with `finished` false, then LEFT_MATCH, so h2live takes the host as
    leaving at the end. No HOST LEFT wording, nothing counts, and the
    host gets the carnage report too. Checked again below.
15. **No engine message ends the game for everyone**: with
    `--end-game-message N`, 2 ended the host's game only (repeated in two
    lobbies, the guest sat about 17 s in a game that had stopped for it);
    6 stopped the rendering (`set_game_state(3)`); 17 crashed (an access
    violation); the other ids tried did nothing. End Game doesn't post
    one; the flag stays for research.
16. and 17. Not run yet.

#### Re-check on the owner's PC (2026-10-11, about 23:40 UTC, 76414d6)

The engines ran with `--pad none`, because a pad plugged into the PC was
moving and its presses reached the first run.

- **14, through the real server** (h2.mohnjorrow.com, two lobbies on the
  PC): pass. The host's lobby logged `left end`, then `ended` with both
  players, then the result with `finished` false, then LEFT_MATCH; the
  server's verdict (not counted, no reason) came 46 ms later, and the
  host's engine was gone 5.0 s after End Game. Host and guest both landed
  on the carnage report ("Unranked: levels don't change"), with no HOST
  LEFT wording. The guest's engine ended its game by itself and its lobby
  closed it about 20 s later. Whether the party was freed wasn't looked at.
- **End Game before the server's GO** can't really happen: the host's
  menu only exists once its map has loaded, and the host's load waits for
  the guest, so both reach `maploaded` within 0.2 s and the guest's join
  goes at once. Forced by holding the guest's engine for 25 s, the server
  dropped the guest at LINK_WAIT and the host went on alone; End Game then
  sent a one-player result and the host got the carnage report. The
  "You ended the game." path is still unseen on the PC.
- **8, with no flag** (reload message 5 by default): pass. A new look
  sensitivity and Bumper Jumper were each posted when the menu closed and
  served at the engine's next read; LB jumped and the turn was faster; no
  NEXT GAME tags. With `--reload-message 0` the changes were never served,
  the tags and the footer came back, and LB didn't jump.
- **6**: pass. 24 steps of Mouse Sensitivity (1.6 to 4) gave one
  `H2EVENT controls` and one save, on leaving the page.
- **13's count**: a fresh sign-in starts on the first playlist; seven downs
  reach the seventh playlist and eight reach Custom Game, as `lobby.md`
  says. The earlier seven came from a script that had opened and closed
  the invite list first.

## 10. Tests

- **blam-cache.**
  - `mcc::synthetic::MapBuilder` learns to write made-up `wgtz`, `wigl`,
    `wgit`, `skin`, `unic` and `bitm` tags with sequences, a string-id
    table, a language table and multi-chunk `textures.dat` records.
  - `ui.rs`'s parse tests then run against both the fake Vista reader they
    have and a synthetic format-13 map.
- **h2ui.** Pure unit tests:
  - UI space to pixels at 16:9, 4:3 and 21:9;
  - animation values at chosen times (focus fades, slide-ins, the pulse);
  - the glyph advance (an L then an I don't touch);
  - a synthetic screen turned into a draw list with the expected quads,
    blend modes and order.
- **CPU backend.** A draw list with a made-up texture, blended plain,
  multiply and additive, against known pixel values.
- **Headless runs on Linux (CI-safe).**
  - `H2LOBBY_HEADLESS=1` with `H2LOBBY_MENU=off`, `H2LOBBY_H2FONTS=off` and
    the fake engine.
  - Scripts walk start, main, sign in, Xbox Live, Optimatch, the search, a
    game and the carnage report; and start, main, Settings, Controller,
    Button Layout, Bumper Jumper.
  - They use `wait` on the new screen names (`start`, `main`, `settings`,
    `controller`, `buttons`, `sticks`) and `see` on texts, with PNGs from
    `H2LOBBY_SHOTS`.
  - These pictures have no game art, and system fonts, so they may be kept
    as test output. Golden-image comparisons are not used, because system
    fonts differ between machines.
  - The two- and three-lobby runs in `lobby.md` are repeated on the new
    screens.
- **Ignored tests on the owner's PC** (`H2_MCC_MAPS`, `H2_MAPS`):
  - MCC's `mainmenu.map` parses;
  - the logo at (-511, 90), list skin 11 at (-178, -80), and
    `framing_center` at (-1070, 654) with scale 1.08;
  - every start-screen and main-menu picture decodes.
- **Windows-only, ignored.** The D3D11 backend draws a draw list on a WARP
  device to an off-screen texture, and it matches the CPU canvas within a
  small tolerance.
- **The layout tables.** Each of the six button layouts gives every action
  one button, and Halo 2's four match the BUTTON LAYOUT screen's labels. A
  test says which layouts are pure moves of Default (section 6, way 3).
- **The gate and the Windows cross-build**, as always.

## 11. Decisions for the owner

1. **The main menu rows.** Recommended: XBOX LIVE, SPLIT SCREEN (an offline
   game on this PC), SETTINGS, with SYSTEM LINK hidden until LAN games
   exist. The other choice shows SYSTEM LINK with an error dialog.
2. **The words "Xbox Live"** on screen, as the lobby already has them,
   without the logo (section 4).
3. **If UI shell needs a patch**, whether one is allowed, or C is enough.
4. **The intro movie** at every start, with a skip option, if a movie is
   found.
5. **Bumper Jumper and Recon:** which game's versions (Halo 3's or later
   ones). menu-preview's tables follow one definition; the owner confirms
   it on the BUTTON LAYOUT screen.

## 12. Risks

| Risk | What it costs | What we do |
|---|---|---|
| Input stays broken on the PC | Nothing can be checked | Phase 0 step 1, before anything else |
| UI shell needs a patch, or the engine's own UI shows | No real scene behind the menu | C; ask the owner |
| `end_frame` drawing spoils engine state | Flicker or a crash | Save and restore state; check the frame after; fall back to C |
| Format 13 differs from the model (nested blocks, multi-chunk textures, top pointer bits) | Pictures missing for MCC-only players | The probe first; Vista second; flat shapes last |
| The CPU is too slow at 1080p | Stutter on the menus | Measure in phase 1; D3D11 backend for the lobby window |
| MCC's sounds are Opus in an unknown framing | A silent menu for MCC-only players | Vista first; checks I and J; Opus in its own crate |
| The Xbox look is guessed in places (keys inside animations, the Xbox lobby layout, tab and page buttons, the countdown) | Small differences from the original | Mark guesses as estimates; check against a capture |
| halo2.dll updates beyond 1.3528 | A breaks; offsets move | A stays optional; C needs no engine knowledge |
| The `app.rs` restyle (4,300 lines) breaks lobby features | Regressions in friends, parties and custom games | Keep `App`'s logic and replace only drawing and navigation; rerun the multi-lobby scripts each phase |
| Halo 2's own text or art ends up in the repository | Breaks the asset rules | Text read at run time; PNGs with art stay on the PC; review every diff for strings |
| `crates/wma` (LGPL) is in `h2launch.exe` once the menus play music (2.2) | The friends' zip must meet the LGPL | Keep it a separate crate; the zip's READ ME names it, with its licence and source or a written offer ("What Phase 2.2 built") |
