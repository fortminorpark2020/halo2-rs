# The launcher's Halo 2 menus

The plan for re-creating the original Xbox Halo 2 menus in the launcher, on
`launcher-menu` (based on `launcher-friends`). Phase 0's reader and drawing
pieces are built, and the start screen and main menu draw from the tags;
the lobby doesn't show them yet ("What Phase 0 built", section 7).

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
| Choose matchmaking playlist (0xDB; 0xED from the lobby) | our launcher playlists: name, your level icon, "N players in this playlist" | Live (playlists) | 2 |
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
  - The lobby already tracks whether a pad or the keyboard was used last.
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

## 3. What comes from MCC and what from Halo 2 Vista

Other players have MCC only, so MCC comes first everywhere. The owner also
has Vista (Project Cartographer, `maps\mainmenu.map`), which serves as the
second source and as a check on the first. The log says which source each
piece came from, as it does for the rank icons.

| What | First | Second | Last |
|---|---|---|---|
| Fonts | MCC `halo2\h2_fonts` (done) | Vista `maps\fonts` | a system font (done) |
| Screen layouts (UI tags) | MCC `mainmenu.map`, format 13 | Vista `mainmenu.map` | numbers in `h2ui::layout` |
| UI pictures | MCC `textures.dat` | Vista `mainmenu.map` | flat shapes in Halo 2's colours |
| Rank icons | MCC (done) | Vista (done) | numbers (done) |
| Map pictures and names | MCC `mainmenu.map` (`matg` UI level data) | Vista | names from `names.rs` |
| Help and status texts | MCC's string tables | Vista's | our own short words |
| Button glyphs | Halo 2's fonts (U+E100 to U+E105) | | a letter in a circle |
| UI sounds and music | MCC's sound tags (once researched; section 5) | Vista's (menu-preview's decoders) | silent |
| The flythrough | the engine in UI shell (A) | our renderer on Vista files (B, only if built) | the navy framing (C) |
| Intro and attract movies | a movie in MCC, if one exists | Vista `movie\intro_60.wmv` (Media Foundation) | skipped |

Two settings, in the same style as `H2LOBBY_RANKS`:

- `H2LOBBY_MENU=<map or folder>` points at another `mainmenu.map`, and
  `off` gives the flat look. Tests use the flat look.
- `H2LOBBY_SOUND=off` silences the menus.

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
- **The words "Xbox Live".** The lobby already calls its first screen Xbox
  Live, and Halo 2 used them.
  - `pivot/legal.md` asks that nothing suggest Microsoft made or backs the
    launcher. So the sign-in screen says, in small print, that this is a
    private server not run by Microsoft.
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

**The music** is `sound\ui\main_menu_music\main_menu_music`: an `in` track,
then a `loop`, with a 5.5 s fade.

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
  for the lobby settings branch to merge) and `H2LOBBY_MENU` isn't read.
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

### Phase 2: the Xbox Live screens over our features, and settings

**1. The shell.** `game_shell_background`, headers, legends, transitions,
list skins, dialogs and the overlay dimming.

**2. Sign in and the Live menu.**

- Select Player and the gamertag keyboard. An on-screen keyboard is needed
  for controller-only players; `controller-wip` has one to borrow from.
- "Please Wait" while connecting, then the XBOX LIVE screen:
  - **Quickmatch** searches at once. The Xbox code doesn't say which
    playlist; we take the party's last one, or the one with the most
    players.
  - **Optimatch** opens the playlists.
  - **Create Party** opens the pregame lobby. On h2live you are always in a
    party, so this only changes the screen.

**3. The playlist screen**, with your level icon per playlist and "N players
in this playlist" from the server's searching and playing counts.

**4. The pregame lobby as the party's home.**

- START GAME (leader) searches the playlist, or starts the custom game.
- GAME SETUP opens:
  - Change Match Settings, Switch To: Custom Game;
  - or Change Map, Change Rules, Switch To: Matchmaking.
- Party Privacy offers Open and Invite Only. Closed needs the server and
  comes later.
- Y opens the players list; X changes teams in a custom game.
- The lines on the left:
  - "Matchmaking / In <playlist>", or the custom game's type and map with
    the map picture;
  - "Party is OPEN / INVITE ONLY";
  - one status line.
- On the right:
  - up to 16 players with their level icon and gamertag;
  - "N players in party";
  - "<gamertag> is the party leader".
- Select New Leader and Boot Player do today's PROMOTE and KICK.
- B asks, then leaves the party.

**5. The Y menu.**

- The FRIENDS tab is today's friends list, with requests first.
- The PLAYERS tab is everyone signed in.
- Columns: gamertag, level icon, status ("In your Party", "In matchmaking",
  "Playing custom game", "Offline" and the rest).
- X opens the options; the player options dialog does today's popups.
- View Gamer Profile opens the service record, restyled as a Halo 2
  sub-screen.

**6. Notices and dialogs.**

- An invite shows Halo 2's lower-left notice, "PARTY INVITE FROM
  <gamertag>", with `receive_message`.
- Today's A-accepts, B-declines popup stays, drawn as Halo 2's multiple
  choice dialog. The invite is also in the Y menu, as on the Xbox.

**7. Settings.**

- SETTINGS, Edit Player, then CONTROLLER: Thumbstick Layout, Button
  Layout, Look Sensitivity, Look Inversion, Automatic Look Centering and
  Controller Vibration.
- BUTTON LAYOUT is Halo 2's screen (layout below).
- PC rows: mouse sensitivity, and the server address (which moves here
  from the sign-in screen).
- Appearance (model, colours, emblem) only if the profile bytes are shown
  to work (`USE_ELITE`, the colour fields), and without the 3D model.

**8. Sounds and music,** from Vista first, then MCC (section 5).

**BUTTON LAYOUT (0x28)** (H tags):

- the list at the top left, the help text at the top right;
- the `button_config` controller picture in the middle;
- a label beside each button, which changes with the focused layout.

Bumper Jumper and Recon use the same picture, with labels from our tables.

**Done when:** a whole session (sign in, invite, party, search, custom game,
friends, service record, a layout change) works on the owner's PC through
the new screens, with keyboard, mouse and a pad.

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

- "Game start countdown in progress"; A cancels for the leader.
- `countdown_for_respawn` at 3, 2, 1 and 0.
- Then the leader's lobby sends the custom game as today. The server
  doesn't change.
- The length is unverified. The code passes 10 and 3 (L).
- X to delay, for the others, needs the server and comes later.

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
  `ugh!`, `matg`, `goof` and `scnr` [1 wgtz, 1 wigl, about 131 wgit, 26
  skin].
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
- The music at 0x1B8 [`main_menu_music`], and the fade at 0x1C0 [5500].

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
