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
  camera roll sign and the 70 degree field of view are unverified.
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
  build --release --target x86_64-pc-windows-gnu -p h2launch`.
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
  the music in 0.5 s (an estimate), then the stream pauses.
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
  decoded in ... ms`, then `sound: playing to the audio device`;
- the music plays on the start screen and the main menu, and fades over
  about 5 s on the playlists (a change within 6 s of the last waits);
  back on the main menu it starts again from its beginning;
- the cursor, select and back sounds play with the pad and the keyboard
  (and a click on a main menu row); A on the start screen sounds select,
  advance, then transport; a screen or popup opened with A sounds select
  then advance, and with X, Y or a bumper advance alone; the playlists
  after signing in, a failure screen, the pregame lobby and the carnage
  report advance as they open;
- holding Esc on the start screen (the quit popup opening and closing
  with each repeat) never stops the music;
- a friend request to us sounds as a message;
- silence through a match (nothing of the menu music playing on after
  it, even when the match started within 6 s of the music starting), and
  the music from its start again on the main menu after it;
- with `H2LOBBY_VISTA=off`: silence, `sound: the menus are silent (no
  Halo 2 Vista files)` and no error.

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

- **In-game GAME MENU** (Leave Game, Settings, Party Privacy, End Game)
  drawn in the match engine's `end_frame`, if the overlay works.
- **Intro movie** at start-up, if one is found in the player's files (check
  J), with a skip option, and the **attract movie** after 75 s idle on the
  start screen or main menu.
  - Vista's WMV plays through menu-preview's Media Foundation code.
  - Bink 1 needs FFmpeg, which is LGPL, so it would go in a separate crate.
  - Bink 2 has no open decoder: play it through the engine, or skip it.
- **SYSTEM LINK**, if a LAN path is built.
- **SPLIT SCREEN** with more than one pad, if the engine takes them.
- **Key rebinding.**

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
