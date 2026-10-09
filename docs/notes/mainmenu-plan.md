# Halo 2 main menu and background: findings and plan

Research only. No repo files were changed. Scratch work is in
`$SP = /tmp/claude-0/-home-claude-halo2-rs/7b79b5a1-e128-5fc1-a97f-78225a2dfd44/scratchpad`.
Repo base: `/home/claude/halo2-rs` at 464878e.

## 0. Short answers

| Question | Answer | Confidence |
|---|---|---|
| Is the menu background a movie? | **No.** It is a live 3D scene, `mainmenu.map`: the Covenant carrier *Solemn Penance* next to the Mombasa tether over New Mombasa. A looping script flies the camera through 12 camera points. | High. Sources: the map's own script, camera points and tags, plus Halopedia "Main menu". |
| Is any movie involved? | Only `movie\intro_60.wmv` (18 s, 1280x720), with `intro_low_60.wmv` for low resolutions. Halo 2 Vista plays it once at startup, **before** the start screen. `credits_60.wmv` belongs to the campaign ending, which is out of scope. | High that the intro plays at startup (Project Cartographer has a `skip_intro` option: "0 - Normal Intro / 1 - No Intro"). Medium on what it shows; 18 s fits the studio logo sequence. |
| Startup order in H2V | intro movie, then the start screen (HALO 2 logo and "PRESS ANY KEY TO CONTINUE" over the flythrough), then the main menu (same logo, list fades in). The flythrough runs behind every menu screen. | High for the screens and their tags; medium for exact timing. |
| Can the remake already render the scene? | **Yes.** Pointing `h2viewer` at mainmenu.map loads 48 textures and 38,738 triangles (carrier, tether, city, bridge, sky). A scratch prototype plays the scripted camera path. | Verified here with screenshots. |
| Biggest visual gaps today | Sky shader blending (a black card behind the tether), the viewer's fixed grey fog, no flak or engine particles, no bloom, and the wrong blend for the menu water. The camera roll sign is not verified. | Seen in the screenshots. Causes are partly inferred; see section 1.2. |

## 1. What the original looks like

### 1.1 Background scene (mainmenu.map)

Sources: `cams` and `attach` examples in `$SP/mm-src/crates/blam-cache/examples/`, the tag list in `$SP/mm-tags.txt`, and the decompiled scripts in the earlier transcript.

- **Scenario** `scenarios\ui\mainmenu\mainmenu`, one BSP `scenarios\ui\mainmenu\mainmenu` (sbsp, lightmap `mainmenu_mainmenu_lightmap`). BSP bounds: x -210..237, y -219..251, z -50..298.
- **Sky** `scenarios\skies\solo\earthcity\main_menu`. Its model is `cinematic_newmombasa`. Shaders:
  - `bridge_sky_dome` (template `sky_two_add_clouds`);
  - `lightning_c` (`sky_one_alpha_env`, bitmaps `cloud_explosion` and `cloud_explosion_mask`);
  - `ec_mts` (`sky_one_alpha_env`, `mountain_range`);
  - `cine_fogband`;
  - `nm_clouds_low` (`sky_two_alpha_clouds`).

  Patchy fog `earthcity\cinematic_newmombasa\africa` (fpch) with bitmap `bridge_fog`.
- **Objects**:
  - the carrier `ui_carrier`, with an engine light at marker `exhaust`;
  - five flak sceneries, whose `effe` tags use `prt3` particles;
  - `plasma_screen` scenery (plasma_alpha shader);
  - `progress_symbol`;
  - player model objects `ui_player1..4`, `ui_player1b..4b`, `ui_player1c`, `ui_player1d`, using bipeds `alpha_masterchief` and `alpha_elite`. These are for the profile and lobby screens, see 1.6.
- **Startup script**:

  ```
  (cinematic_lighting_set_primary_light 45 240 0.45 0.45 0.45)
  (cinematic_lighting_set_secondary_light -45 30 0.1 0.1 0.1)
  (cinematic_lighting_set_ambient_light 0.1 0.1 0.1)
  (rasterizer_bloom_override true) (rasterizer_bloom_override_threshold 0.3)
  (object_uses_cinematic_lighting ui_player* true) (cinematic_lightmap_shadow_enable)
  (object_set_function_variable ui_carrier grav_lift_control 1.0 0.0)
  ```

  The cinematic lighting only applies to the `ui_player*` models. The carrier's gravity-lift function is switched on.
- **Camera**: continuous script `mainmenu_flythrough`:

  ```
  (camera_control true)
  (camera_set ui_path_01 0)   (sleep 90)
  (camera_set ui_path_02 500) (sleep 250)   ; same for 03..09
  (camera_set ui_path_10 400) (sleep 200)   ; same for 11, 12
  (camera_set ui_path_01 300) (sleep 300)
  ```

  It runs at 30 ticks/s, so one loop is 2,990 ticks, about 99.7 s.
  - Each move is cut off halfway by the next `camera_set`. The camera never comes to rest except at the end.
  - The loop finishes on ui_path_01, so the restart is seamless.
- **Camera points** (position; yaw, pitch, roll in degrees). Roll goes up to ±45°, so the camera banks hard.

  ```
  ui_path_01 (-2.54,-35.52,219.59)  105.6  -1.4   4.9
  ui_path_02 (-7.74, -9.68,226.43)  110.1  18.5 -41.2
  ui_path_03 (-33.85,21.24,229.66)  112.0 -12.9  11.9
  ui_path_04 (-50.06,34.51,265.15)   81.6  -5.9 -34.5
  ui_path_05 (-49.81,89.53,280.58)    6.4 -44.3  -8.1
  ui_path_06 (-10.45,129.74,258.24) -60.9 -27.2  37.2
  ui_path_07 (29.10,121.24,246.99) -100.2  10.4  45.2
  ui_path_08 (50.27,93.34,239.22)  -138.8  30.6  24.0
  ui_path_09 (16.08,46.23,242.63)  -106.9  26.3  44.3
  ui_path_10 (-5.76,7.97,228.73)   -100.4  21.1  36.0
  ui_path_11 (-18.46,-14.11,225.48) -14.8 -35.8   8.8
  ui_path_12 (-13.98,-34.02,221.87)  49.0  -1.9  -9.1
  ```

  There are also five unused points: `scene_01_alphahalo` … `scene_05_empty`, all with zero orientation.
- **Music**: `sound\ui\main_menu_music\main_menu_music` (lsnd, with `in` and `loop` tracks), from the UI globals, fade time 5,500 ms. Halopedia: "the Gregorian chant in *Prologue*". The remake already plays this.

### 1.2 What the scene looks like in the remake today (prototype)

These frames are from a scratch build of the remake with the prototype camera. They are **not** reference shots of Halo 2.

- With today's menu drawn on top: `$SP/lantest/runs/mm-fly/shots/t0..t90.png`, contact sheets `sheet1.png` and `sheet2.png`.
- With the menu hidden: `$SP/lantest/runs/mm-bare/shots/t3.png`, `t45.png`.
- With the menu hidden and fog off: `$SP/lantest/runs/mm-nofog/shots/t3.png`, `t45.png`. Comparison sheet: `$SP/mm-mock/fog-compare.png`.

Visible problems, with my best guess at the cause:

1. **A black rectangle with brown clouds behind the tether.** Probably a sky shader drawn without the right blend.
   - In `crates/blam-cache/src/shader.rs`, `transparent\sky_two_add_clouds` falls through to the generic `transparent\` case, giving `Blend::Alpha` with no mask.
   - `lightning_c` (`sky_one_alpha_env` with `cloud_explosion` plus `cloud_explosion_mask`) takes its alpha from the diffuse map, not from the mask.
   - Confidence: medium. Check with a shader dump of those four sky shaders.
2. **Grey wash.** The viewer's fog is fixed in `gpu.rs` (about line 328): `distance/400`, squared, mixed toward a constant `(0.62,0.70,0.80)`. The menu scene sits 200–400 units from the camera, so it fogs heavily. Turning fog off (`H2_FOG=0` scratch switch) gives the carrier its contrast back, but the scene is still mostly grey: the BSP shaders are `matte_grey`, `x03_metal_grey` and so on, and the sky is grey clouds. The real menu may be fairly grey too. **This needs a reference capture.**
3. **Purple-grey water sheet** (`nm_arial_water`). Check its template; the water path in shader.rs uses a fixed tint.
4. **No flak bursts or carrier engine glow** (prt3 particles, a light at a marker), no bloom, and no gravity-lift beam (an object function variable).
5. **Camera roll.** The prototype uses `rot = Rz(yaw) * Ry(-pitch) * Rx(roll)`. The horizon tilts as expected at points 06–10, but the **sign of roll is not verified**.

### 1.3 Start screen and main menu screen (UI tags)

The UI tags are in mainmenu.map: `wgtz ui\main_menu` points to `wigl ui\ui_shared_globals` and to 133 screen entries (131 distinct `wgit` tags). The globals hold 26 list `skin`s.

- Dumps: `$SP/mm-wigl.txt` (globals and skins), `$SP/mm-screens.txt` (all screens).
- Tool: `$SP/mm-src/crates/blam-cache/examples/uidump.rs`. Run `uidump <map> [globals|screens|<filter>|strings <filter>]`.
- Field layouts: Assembly's Halo2 plugins, saved in `$SP/mm-web/Halo2/{wgtz,wigl,wgit,skin,unic}.xml`.

**UI coordinate system** (inferred from the tags, then checked with a mock):
- origin at the screen centre, +y up, about 1200 units of screen height;
- the 4:3 safe area is about ±800 x ±600, and 16:9 is about ±1067 x ±600;
- one bitmap pixel is one unit, times the element's `scale` (0 means 1.0);
- screen bitmaps are placed by their **top-left**. List-skin bitmaps are placed by their **bottom-left**; Assembly's skin plugin notes this.

The mock `$SP/mm-mock/mainmenu-mock.png` composes the main_menu screen from the exported bitmaps over a flythrough frame using these rules. It comes out as the familiar Halo 2 layout: a chrome logo centred just above the middle, with the list centred below it. That is good evidence the model is right.

**Main menu screen** (`ui\screens\game_shell\main_menu_screen\main_menu`, screen_id 0x6):
- **Flags** "No Header Text"; button key NONE, so **no header and no button legend** on this screen.
- **Logo**: `ui\screens\game_shell\start_screen\start_screen`, 1024x128 chrome-blue "HALO 2", top-left at (-511, 90), so centred on x. Screen animation 29: fade in over 250 ms.
- **Tracks**: thin horizontal lines that scroll slowly. Each starts at x -1200 with scale 1.2, so 2458 units wide, and scrolls at a `wraps/s` U offset. Their alpha is very low; the maximum in the bitmaps is 24/255.

  | y | bitmap | wraps/s |
  |---|---|---|
  | 120 | track2b | 0.010 |
  | 128 | track4 | 0.030 |
  | -40 | track6 | 0.050 |
  | -44 | track7c | 0.080 |
  | 124 | track7d | 0.020 |
  | -370 | track5 | 0.100 |
  | -376 | track7c | 0.050 |
  | -510 | track_timeline2a | 0.010 |
  | -500 | track_timeline2b | 0.060 |
  | 500 | track7c | 0.035 |
  | 550 | track5 | 0.080 |

  Also `track_brace` (frame 1) at (300, -483).
- **List**: skin 11 (`ui\list_skins\main_menu\main_menu`), wraps, 6 visible, bottom-left (-178, -80), animation 34.
  - Item height is about **50 units**. The text block is 50 high, and the bitmaps carry the "ignore for list size" flag. Inferred.
  - Text is **centred**: text flags 0, bounds l-62..r418 relative to x -178, so centred on screen x 0. Font `main_menu`, colour (0.62, 0.74, 0.84).
  - A second text block in (0.29, 0.33, 0.37), 5 units lower, is probably the drop shadow. Medium confidence.
  - Per item: `list_bkd` (480x62 blue glow bar, alpha ≤ 86/255) at offset (-60, -40), and `list_bkd_rings_right` at (-54, -40).
  - Two **multiply** sheen strips: `list_bkd_multiply` at (150, -40) scrolling +0.2 wraps/s, and `list_bkd_multiply_left` at (-60, -40) scrolling -0.2.
  - Bitmap flags "swap on relative list position": the frame probably changes for the first, middle and last items. Medium confidence.
  - Focus animations from skin 11's item animations: focused fades 0.5 → 1.0 over 120 ms; unfocused fades 1.0 → 0.5 over 200 ms; hover states 0.5 ↔ 0.7 over 90 ms. **Non-focused items sit at 50% alpha.**
- **Bottom-right text** at bounds [t-562 l376 b-600 r610], font `split_hud_msg`, (0.64, 0.72, 0.87), left-justified. The code fills it; probably the version/build or the profile name. Unknown.
- **Item strings** in `main_menu_screen` (unic):
  - CAMPAIGN, LIVE, NETWORK, SPLIT SCREEN, SETTINGS, GUIDE, QUIT, LOADING;
  - "\u{e101}  SIGN OUT" (B glyph).

  The H2V order and which items show are set in the exe, not the tags, so they are unknown. The Xbox original used CAMPAIGN / XBOX LIVE / MULTIPLAYER / SYSTEM LINK / SETTINGS, from memory, medium confidence. The remake's labels (MULTIPLAYER, SYSTEM LINK) match the Xbox wording.

**Start screen** (`start_screen`, screen_id 0x9): the same logo, tracks and brace as the main menu, plus:
- "PRESS ANY KEY TO CONTINUE": font `title`, colour (0.64, 0.72, 0.87), flags **pulsating**, centred in [t-65 l-400 b-105 r400];
- the build number in orange (1.0, 0.5, 0.0), font `large_body`, at [t-170 l-200 b-190 r200]; leave this out;
- the bottom-right text, as on the main menu.

The ESRB screen ("Game Experience May Change During Online Play") exists but should be left out.

### 1.4 Shared look for sub-screens

- **Background** for every sub-screen: `game_shell_background` (screen_id 0x5).
  - `framing_center` (2048x1303, navy, mostly opaque) at (-1070, 654), scale 1.08, so it covers the whole 16:9 screen.
  - 17 track lines on top at scale 1.1, scrolling 0.01–0.15 wraps/s.
  - The flythrough still runs behind it but is **mostly hidden**: the framing's alpha leaves a soft window. Check against the exported `$SP/mm-bm/ui_global_bitmaps_framing_center_a.png`.
- **Header**: font `title`, colour (0.68, 0.76, 0.85), in the "full" header bounds [t567 l-730 b520 r50], top-left.
  - Smaller dialogs use the large, half and quarter bounds: large [t390 l-500 b310 r55], half [t158 l-500 b73 r255], quarter [t182 l-252 b142 r350].
  - Example headers: "SETTINGS", "EDIT PLAYER: <name>", "PREGAME LOBBY", "NETWORK GAME BROWSER".
- **Button legend** at bottom right, "full" bounds [t-500 l100 b-540 r730].
  - The text comes from `ui\global_strings\button_key_types`, picked by the screen's `button_key` enum. Examples: 1 = "\u{e100} SELECT \u{e101} BACK", 2 = "… SELECT … CANCEL", 9 = "\u{e100} SELECT", 0x19 = "ACCEPT/CANCEL".
  - Code points U+E100..E103 are the A/B/X/Y button glyphs drawn by the H2V font.
  - Each string has a `_keyboard` variant without glyphs, e.g. "SELECT BACK" or "ACCEPT CANCEL". Some are mangled in the tag, like "SELECTBACK", so build keyboard legends in code.
- **Default list skin** (skin 0): `item_background` (400x42) with `end_cap` (56x68), text font `body`, colour (0.56, 0.69, 0.81), left-justified, bounds [t38 l50 b-8 r440]. Settings lists use skin 2 (`setting_lists`): label at x 20–500 and value at x 550–1000, `large_body`, plus `hilite` and `hilite_bracket` bitmaps.
- **Corner art and flavour text**: e.g. `ul_06`, `br_03`, `bl_01`, `tr_02`, plus small "callout" texts in `subtitle` font (0.25, 0.39, 0.54) that drift in. These are decoration only.
- **Transitions** (wigl screen animations, used through each element's animation index):
  - 00: fade;
  - 01–15: slide in from x +1024 to 0 while fading in, durations 320 ms down to 180 ms in 10 ms steps, and the outro slides to -1024. Different elements use different indexes, which gives the staggered "fly-in" look;
  - 29: fade over 250 ms;
  - 30/31: in from z +1000;
  - per-element `delay` values of 50–375 ms.

  The keyframe timing within a period is not decoded; my dump printed start 0 for every key. Use evenly spaced keys until that is checked.
- **Overlay**: wigl `overlay_color` argb(0.85, 0.00, 0.08, 0.17) and `overlay_alpha_mod` 0.10. Probably the navy dimming behind dialogs; screens can turn it off with the "Disable Overlay Effect" flag. Medium confidence.

### 1.5 Fonts

- Halo 2 Vista ships fonts **outside** the maps, in `maps\fonts\`: `font_table.txt` plus extensionless font files. The table lists the 12 engine slots in order: terminal, body, title, super_large, large_body, split_hud_msg, full_hud_msg, english_body, hud_number, subtitle, main_menu, text_chat.
- The uploaded install here has **no fonts folder**, so the files could not be inspected. Format, from FontPackager `TableIO.cs` and `CharacterTools.cs` in `$SP/mm-web/fp/`, which has no licence, so re-implement from the description rather than copying code:
  - at 0x200: version `0xF0000001`;
  - i16 ascend, descend, lead height, lead width;
  - i32 character count, max compressed size, max decompressed size, compressed size, decompressed size;
  - kerning pairs (count, then u8, u8, i16 each), then 8 x i32;
  - at 0x400: a 65,536-entry u32 table from code point to character index;
  - at 0x40400: per-character records of 16 bytes: u16 display width, u16 data length, u16 width, u16 height, i16 origin x, i16 origin y, u32 data offset;
  - glyph pixels are a small run-length code over ARGB4444.
- The A/B/X/Y glyphs U+E100..E103 should be present in these fonts. Not verified.

### 1.6 Sub-screens John reaches, as Halo 2 lays them out

- **MULTIPLAYER** (the remake's local custom game). Closest H2 screen: `pregame_lobby` (screen 32).
  - Header "PREGAME LOBBY" and button key 1.
  - Four big `button` bitmaps top-left in a 2x2 grid: START GAME (-690..-360, 480..440), GAME SETUP (-320..8), FIND GAMES and GAME DETAILS on the row below at y 410.
  - "Quick Options:" with game type and "on <map>" lines at x -675.
  - A 16-row player list on the right, x 118–276, using player skins: emblem, name, rank.
  - Party status text and a chat box at the bottom right.
  - Game setup goes through `game_engine_category_listing`, `mp_variant_select` and `mp_map_select_lobby`. The map list is skin 10 with `mp_map_list` pictures.
- **SYSTEM LINK** → `network_squad_browser` (screen 24).
  - Header "NETWORK GAME BROWSER", tab "NETWORK GAMES".
  - Column heads at y 495: "Host | Game Name" (x -604), "Map" (-250), "Gametype" (-60), "Variant" (135), "Players" (330), "Status" (435).
  - 19-row list in skin 5 at (-670, 412). Empty text: "There are no other games on the network at this time".
  - A `list_frame` bitmap at (-760, 580), a help box ("Create a new network game.") and a map picture `unknown_map` at (410, -175).
  - Player list `player_skin_syslink`, 2 columns x 8 rows.
- **PLAYER PROFILE** → `edit_profile_menu` (screen 3).
  - Header "EDIT PLAYER: <name>", default-skin list at (-540, -220), description text below.
  - Corner art `bl_01` and `tr_02`; `display`, `profile` and `edit_profile` bitmaps.
  - Sub-screens:
    - **Appearance** (screen 6): skin 8 list at (-550, -40), help text, and two **model_scene**s that draw scenario objects `ui_player1c` / `ui_player1d` from mainmenu.map's own scene. Camera at (21.6, 56.4, -49.9), fov 55, viewport [t470 l-620 b-450 r510]. This is how H2 shows your Spartan or Elite, **not over a multiplayer level**.
    - `choose_emblem`, `choose_primary_color`, `choose_player_model`.
    - Controller and keyboard settings.
- **GAME OPTIONS** → variant editing screens with settings lists in skin 2.
- **ONLINE** (h2live) has no direct H2 equivalent. Use the shared shell (header, default list skin, legend). Avoid `xbox_live_*` art and the "LIVE" wording, so nothing implies Xbox Live.

### 1.7 Sounds (wigl)

| Event | Sound tag |
|---|---|
| cursor | `sound\ui\cursor1` |
| select | `forward1` |
| error | `flag_fail` |
| advance | `advance` |
| retreat | `back1` |
| tab | `forward1` |
| countdown | `game_sfx\multiplayer\countdown_for_respawn` |

The remake already loads cursor1, forward1, back1 and advance from these tags. Add `flag_fail` for errors and check that each event uses the tag above.

## 2. What the remake shows today (464878e, 1280x720, lockout.map)

Screenshots:
- `$SP/lantest/runs/mm-now/shots/01-main.png`
- `02-multiplayer-lobby.png`
- `02b-game-options.png`
- `03-system-link.png`
- `04-profile.png`
- `05-online.png`
- `06-main-again.png`

Contact sheet: `$SP/mm-mock/now-sheet.png`. Script: `$SP/lantest/runs/mm-now/run.sh`.

The remake's layout (menu.rs):
- 640x480 units, anchored left: rows at x 48, 300 wide, 26 high, a 32 step;
- a 5x7 pixel font;
- the title top-left with a rule underneath;
- solid blue highlight rows;
- a translucent panel on the right;
- the hint line "ENTER OR A: SELECT   ESC OR B: BACK".

The 3D background is an orbit around the loaded level (Lockout), darkened by 0.35.

Differences from Halo 2:

| Area | Remake now | Halo 2 |
|---|---|---|
| Background | Orbit of the current MP level, darkened | mainmenu.map carrier flythrough (99.7 s loop, banking camera) |
| Startup | Straight to the main menu | intro movie → start screen → main menu |
| Title | "HALO 2" in a pixel font, top-left, with a rule | Chrome logo bitmap, centred, no rule |
| Main list | Left-aligned boxes, solid highlight | Centred text over glow bars; focused item 100% alpha, others 50%, moving sheen |
| Main legend | "ENTER OR A: SELECT" | None on the main menu |
| Decoration | None | Scrolling track lines and the track brace |
| Text colour | (0.72, 0.84, 1.0), no shadow | (0.62, 0.74, 0.84) with a dark offset shadow |
| Fonts | 5x7 bitmap | H2V `maps\fonts` (main_menu, title, body …) |
| Transitions | Instant | Fades (250 ms) and staggered slides (180–320 ms) |
| Sub-screen backdrop | Level orbit plus a panel | Navy `framing_center` with tracks over a mostly hidden flythrough |
| Headers and legends | Pixel font, top-left; legend bottom-left | Title font in the header box at top-left; legend at bottom right with A/B glyphs |
| MULTIPLAYER | Settings rows plus map and player panels | PREGAME LOBBY: 2x2 buttons, quick options, player list |
| SYSTEM LINK | "GAMES ON YOUR NETWORK / SEARCHING…" | NETWORK GAME BROWSER: columns, list frame, help box |
| PLAYER PROFILE | 9 rows plus an emblem tile, red Spartan standing in Lockout | EDIT PLAYER list with corner art; Appearance sub-screen draws `ui_player1c/1d` from mainmenu.map with its own camera |
| Initial focus | PLAYER PROFILE for a new profile | First item; for John this is H2's default, but keep the remake's choice if it is deliberate |
| Music and sounds | Same tags | Same; add `flag_fail` |

## 3. Implementation plan

Rules for every step:
- Read everything at runtime from the user's maps folder: mainmenu.map, `maps\fonts`, `..\movie`. **Commit no extracted assets.**
- When a file is missing, fall back to today's behaviour.
- Coordinate with the worktree `wf_a876195b-c46-9`. It is rewriting menu.rs for H2 dialogs, the scoreboard and the carnage report, so land this work on top of it and reuse its dialog drawing.

### Step 1: UI tag reader (blam-cache) and `h2tool ui`
1. Add `blam-cache/src/ui.rs`, ported from the scratch `uidump.rs`. Offsets come from Assembly's Halo2 plugins in `$SP/mm-web/Halo2`.
   - `UiGlobals`:
     - the sounds;
     - music tag and fade time;
     - header and button-key bounds per size;
     - button-key strings, resolved through `text::unicode_strings`;
     - `screen_anims` (intro/outro/ambient period and keyframes of alpha and position);
     - overlay colour and alpha.
   - `Screen`: flags, `screen_id`, `button_key`, string list, header string, panes. Each pane holds:
     - lists (skin, visible count, bottom-left, animation, delay);
     - texts (flags, animation, delay, font, colour, bounds, string id);
     - bitmaps (flags, animation, delay, blend, frame, top-left, wraps per second, tag, depth, scale);
     - buttons;
     - model_scenes;
     - player-skin blocks.
   - `ListSkin`: arrows, item animations, texts, bitmaps.
   - Look screens up by tag name, e.g. `ui\screens\game_shell\main_menu_screen\main_menu`, not by index.
2. Add `h2tool ui <map> [globals|screens|<filter>|strings <filter>]` so later work can re-check layouts.
3. Tests: parse a **synthetic** wgit and skin byte block, the way `blam-cache/src/tests.rs` builds synthetic maps, because CI has no game files. Add an `#[ignore]` test that runs against a real mainmenu.map from `H2_MAPS` and checks:
   - main_menu has the logo at (-511, 90);
   - the list uses skin 11 at (-178, -80);
   - game_shell_background has `framing_center` at (-1070, 654) with scale 1.08.

### Step 2: The menu scene, kept loaded alongside the level
1. Load mainmenu.map with the existing scene loader in scene.rs, the same loader that renders it today when passed as the level.
2. **Recommended**: keep it resident as a second GPU scene.
   - Split `Gpu`'s per-scene data (`meshes`, `materials`, `hud_textures`, lightmap bind groups) into a `GpuScene`.
   - `Gpu` then holds `level: GpuScene` and `menu: Option<GpuScene>`, and `Frame` gains a `scene: SceneSlot`.
   - The cost is small: 48 textures and about 39k triangles.
   - The alternative, swapping scenes (H2's own way), reloads mainmenu.map after every game and Lockout before every game. Not worth it.
3. While `!in_game`, draw the menu scene with the flythrough camera in place of today's orbit, and drop the 0.35 darkening on screens that use the H2 background. If mainmenu.map is missing or fails to load, keep today's orbit.
4. Draw the menu scene's objects (carrier, flak sceneries, plasma screen) from its scenario as the level loader already does. Hide the `ui_player*` bipeds except in model scenes (step 7).

### Step 3: Flythrough camera
1. Read the script **from the map**: find `mainmenu_flythrough` with `blam_cache::script::scripts` and walk its `begin` body, collecting `(camera_set <point> <ticks>)` and `(sleep <ticks>)`. Keep the table in `$SP/mm-src/crates/h2viewer/src/menupath.rs` as the fallback.
2. Camera points come from `scenario::camera_points`, which reads name, position at 0x28 and orientation at 0x34 in radians.
3. Orientation: `Quat::from_rotation_z(yaw) * Quat::from_rotation_y(-pitch) * Quat::from_rotation_x(roll)`. Forward = rot·X, up = rot·Z. **Check the roll sign against a reference capture**; flip it if the horizon tilts the wrong way.
4. Interpolation must keep the speed it had when a move is cut off halfway. The prototype uses a cubic Hermite from (current position, current velocity) to (target, at rest) over the move's length, and a smoothstep slerp for rotation. The campaign's `CameraMove` uses a plain smoothstep from rest, which jerks to a stop every 8 s on this script.
   - A first version may simulate from t=0 every frame (about 13 segments, cheap). Later, keep the state incrementally.
   - Rotation also restarts from rest at each retarget. Use a velocity-matched slerp (a squad between neighbours) if it looks jerky.
5. Time base: seconds since the menus were entered; loop at 2,990 ticks; restart at 0 after a game. For screenshot tests add a debug override `H2_MENU_TIME=<s>`, as the prototype does.
6. Field of view: `camera_set` keeps the cutscene default. cutscene.rs uses 70 (horizontal). Start with 70° horizontal and match it to the capture.
7. Camera roll needs an up vector in the menu view. The prototype added `View.up` and used `look_to_mat4(eye, fwd, up)` for both the world and the sky view; `FlyCamera` has no roll.

### Step 4: Make the scene look right
In order of payoff:
1. **Sky shader blends** in `blam-cache/src/shader.rs`:
   - add `transparent\sky_two_add_clouds` as additive, with its mask;
   - for `sky_one_alpha_env`, use the mask/alpha map when there is one;
   - dump `lightning_c`, `bridge_sky_dome`, `cine_fogband` and `nm_clouds_low` with h2tool to confirm the map slots.

   Acceptance: no black rectangle behind the tether in the t=3 s frame.
2. **Fog per scene.** Replace the fixed `/400` grey fog with values from the scenario's sky tag (atmospheric fog colour and start/end distances). Failing that, turn fog off for the menu scene; `H2_FOG=0` in the scratch build shows the difference. Patchy fog (`africa` fpch) can come later.
3. **Water** `nm_arial_water`: check its template and give it the right tint, or draw it semi-transparent.
4. **Bloom** (threshold 0.3): optional. A small two-pass bloom on the menu scene only. Skip if it costs frame time.
5. **Carrier engine glow and gravity lift**: optional. Light at marker `exhaust`; set object function `grav_lift_control` to 1 if the remake supports object functions.
6. **Flak**: the five flak sceneries' `effe` tags use prt3 particles. If `effects.rs` can't draw prt3, leave them out; see section 5.

### Step 5: UI widget renderer
1. **Space**: `px = (W/2 + x·s, H/2 − y·s)` with `s = H/1200`, so everything scales with window height and wide screens show more background at the sides. In split screen the menu owner's viewport uses the same rule.
2. **Bitmaps**: load each referenced `bitm` from mainmenu.map once into `menu_textures`, keeping the sequence frames. rank.rs, emblem.rs and mapinfo.rs already read mainmenu.map this way. Draw quads with:
   - top-left anchoring for screen bitmaps and bottom-left for skin bitmaps;
   - size = bitmap pixels × scale, where 0 means 1;
   - U offset = `t × wraps_per_sec` with a repeating sampler (tracks and sheen);
   - blend 0 = alpha, 1 = multiply (`dst × src`);
   - sort by `depth` ascending, so lower depth is drawn further back. Inferred from main_menu: tracks at depth 1, glow bars at 0, sheen at 2, text at 12.
3. **Text**: bounds in UI units, flags (left/right/centre, pulsating), colour × element alpha, and the shadow for skin 11.
4. **Lists**: rows stack downward from the list's bottom-left at the item height. That is the max height of the skin's text bounds and its bitmaps without the ignore flag: 50 for skin 11; check others against their art. Show the up/down arrows when the list scrolls. Wrap when flag bit 0 is set.
5. **Animations**: each element follows the wigl screen animation named by its index:
   - intro when the screen opens (after its delay), outro when it closes;
   - alpha and x/y/z offsets interpolated between keyframes;
   - list items also follow the skin's item animations (focus/unfocus/hover).

   Respect the remake's "reduce motion" or test modes if they exist. Screenshots need a way to skip to the settled state, e.g. `H2_MENU_SETTLED=1`.
6. **Shell**: build each sub-screen as game_shell_background, then its own screen, then the header text and legend.
   - Header and legend bounds come from `wigl` by dialog size: full, large, half or quarter.
   - Legend text: the button-key string for controllers (A/B glyphs). With keyboard and mouse use the `_keyboard` meaning, built in code, e.g. "ENTER SELECT   ESC BACK".
   - Do this per input device. The remake already knows which device is in use.

### Step 6: Fonts
1. At startup read `<maps>\fonts\font_table.txt`. The line order maps to the 12 slots. Decode each listed file with the format in 1.5 into an RGBA glyph atlas per font: ARGB4444 to RGBA8, kerning pairs, ascent/descent/leading.
2. Render glyphs at the UI scale. The fonts were made for 480p and 720p, so allow bilinear filtering when scaling up.
3. **Fallback**: if `maps\fonts` or a slot is missing, use today's 5x7 font with a per-slot size, e.g. title 2x body. Keep A/B glyph fallbacks as small drawn button icons or the words "A"/"B".
4. Ask John for his `maps\fonts` folder **for local testing only** (never committed). The uploaded install here does not include it.

### Step 7: Screens

Change presentation only. Keep the rows, actions and network code.

1. **Start screen**, optional but authentic, after the intro movie: logo, tracks, brace, and a pulsating "PRESS ANY KEY TO CONTINUE" over the flythrough. Any key, button or click goes to the main menu with the select sound. Skip it with `--no-start-screen` or the profile option "Skip intro".
2. **Main menu**: logo, tracks, brace, and a skin 11 list with the remake's items: ONLINE, MULTIPLAYER, SYSTEM LINK, PLAYER PROFILE, QUIT. Centred, no header, no legend. Bottom-right shows the profile name, or nothing.
   - Keep the remake's labels: they match the Xbox Halo 2 wording, and "LIVE"/"NETWORK" are Vista wording that would wrongly suggest Xbox Live.
   - Focus starts on the first item, unless the new-profile focus on PLAYER PROFILE is deliberate.
3. **MULTIPLAYER lobby**: lay out the remake's local lobby like `pregame_lobby`.
   - START GAME and GAME SETUP as the top button row.
   - "Quick Options:" lines for game type, map, score and bots.
   - The remake's player list on the right using the player skin (emblem, name).
   - The map picture from `mapinfo.rs`.
   - Leave out FIND GAMES, GAME DETAILS and the chat box unless they map onto remake features.
4. **SYSTEM LINK**: the `network_squad_browser` layout. Columns Host | Game Name, Map, Gametype, Variant, Players, Status; the empty-list text; the help box. Fill them from the remake's LAN browser.
5. **PLAYER PROFILE**: `edit_profile_menu` with the default list skin and corner art. Appearance (model, colours, emblem) uses screen 6's layout.
   - The **player model is drawn from the menu scene's `ui_player1c` object** using the screen's model_scene camera and viewport.
   - If the menu scene is missing, keep today's model drawing.
6. **GAME OPTIONS**: settings lists in skin 2 (label left, value right, hilite bracket).
7. **ONLINE**: shared shell plus default list skin. No Xbox Live art or wording.
8. Dialogs, scoreboard and carnage report: take these from wf-9's work. Their H2 tags (`error_dialog_*`, `pcr`, `pcr_lobby`) use the same renderer.

### Step 8: Intro movie (Windows; optional, lowest priority)
1. **Where**: `<maps>\..\movie\intro_60.wmv`, or `intro_low_60.wmv` when the window is under 720 px high. Play once at startup, before the start screen. Skip it on any key, button or click, on the profile option "Skip intro", with `H2_SKIP_INTRO=1`, or when the file or decoder is missing (go straight to the start screen).
2. **Windows decoder**: Media Foundation through the `windows` crate. 0.62 is already in Cargo.lock through wgpu/cpal. Add it as a `[target.'cfg(windows)'.dependencies]` entry with features `Win32_Media_MediaFoundation`, `Win32_System_Com` and `Win32_Foundation`.
   - Start up with `MFStartup` and create the reader with `MFCreateSourceReaderFromURL`, passing attributes with `MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING = TRUE`.
   - Set the video stream to `MFVideoFormat_RGB32` and the audio stream to `MFAudioFormat_Float`.
   - A thread calls `ReadSample` and sends `(timestamp, BGRA frame)` and audio chunks over channels.
   - The render loop uploads the frame for the current audio-clock time into a 1280x720 texture and draws a full-screen letterboxed quad. Audio goes into the existing cpal mixer.
   - WMV9/WMA decoders ship with Windows; John has `wmvdecod.dll`.
   - Cost: about 300–450 lines and a few days, including A/V sync and skipping. Media Foundation is Windows-only and safe to leave off other targets.
3. **Linux and tests**: no Media Foundation. Put the player behind a small `MovieSource` trait.
   - On Linux, an optional backend spawns `ffmpeg -i <file> -f rawvideo -pix_fmt bgra -` and a second for `-f f32le` audio, only if `ffmpeg` is on PATH (it is here).
   - Otherwise a null source makes the player skip.
   - The uploaded install here has no `movie` folder. Tests can make a 2 s clip with `ffmpeg -f lavfi -i testsrc=size=1280x720:rate=30 -t 2 -c:v wmv2 test.wmv`, generated at test time and never committed.
4. Bink is not involved. H2V's movies are WMV/ASF and no Bink dll exists.

## 4. Tests and screenshot checks
- **Unit tests** (CI, no game files):
  - UI tag parsing on synthetic blocks;
  - flythrough script extraction on a synthetic `Scripts`;
  - camera path: continuous position and velocity across retargets (sample at 1/60 s and check no step exceeds a bound); the loop ends at ui_path_01; total 2,990 ticks;
  - UI-space-to-pixel mapping at 4:3, 16:9 and 21:9;
  - font decoding on a hand-made 2-glyph file;
  - button legend selection for keyboard and controller.
- **Local checks** (`#[ignore]`, need `H2_MAPS`): the real tags parse; mainmenu.map's scene loads; the script yields 13 moves.
- **Screenshots** on Xvfb at 1280x720. Reuse `$SP/lantest/runs/mm-fly/run.sh` (`H2_MENU_TIME`, mouse parked at 1279,719, `--onlyvisible` window search).
  - Main menu at t = 3, 30 and 60 s.
  - The start screen.
  - Each sub-screen settled (MULTIPLAYER, SYSTEM LINK, PLAYER PROFILE and Appearance, GAME OPTIONS, ONLINE).
  - The same at 1920x1080 and 1024x768 to check scaling.
  - Compare against `$SP/mm-mock/mainmenu-mock.png` for layout.
  - With mainmenu.map renamed away, the old orbit must still work, and with no fonts folder the 5x7 font.
- **Reference capture from John** (most valuable single check): a 100 s video or a few screenshots of his own H2V main menu and start screen, plus one sub-screen, at 1280x720. Use it to settle:
  - roll sign, field of view and colour or fog;
  - item order and labels;
  - what the bottom-right text shows;
  - font sizes;
  - whether the list really sits at 50% alpha.

## 5. What to leave out, and why
- **Campaign** item, difficulty, saved games, `credits_60.wmv`: the campaign is out of scope.
- **Xbox Live and GFWL screens** (`xbox_live_*`, friends, clans, voice mail, messages, optimatch, the "LIVE" label and `live_icons`): they don't exist in the remake and would imply Xbox Live. ONLINE stays the remake's own h2live screens in H2 style.
- **Split screen, the Guide, sign-out, the ESRB notice, the build number, the virtual keyboard**: not part of the remake's flow (or PC-only noise). Text entry stays the remake's own.
- **prt3 flak particles, bloom, patchy fog, the gravity-lift function**: polish that needs systems the remake lacks. The scene reads well without them. Add them when an effects system exists.
- **The voice and chat panes** in the lobby: no voice or chat yet.
- **Bink or a pre-rendered background video**: none exists in Halo 2 Vista. Rendering a video from the scene would mean shipping derived game footage, which the asset rule forbids.
- **The mouse cursor art** (`cursors\default`): keep the OS cursor.

## 6. Open points for John or the coordinator
1. Can John send screenshots or a short capture of his own start screen and main menu (section 4)? They are the reference for the camera roll sign, colours, item order and font sizes.
2. Can John send his `maps\fonts` folder for local testing (not for the repo)?
3. Labels: keep ONLINE / MULTIPLAYER / SYSTEM LINK / PLAYER PROFILE / QUIT (recommended), or use H2V wording?
4. Intro movie and start screen at every launch, with a "skip intro" option? Recommended: on, with the option.

## 7. Scratch artefacts (not for the repo)
- `$SP/mm-src`: `git archive` of 464878e with:
  - examples `uidump.rs`, `cams.rs`, `attach.rs`;
  - `h2viewer/src/menupath.rs` (the flythrough prototype);
  - scratch `main.rs` switches `H2_MENU_TIME`, `H2_MENU_SMOOTH` and `H2_MENU_HIDE`, and a `gpu.rs` switch `H2_FOG`.

  Built into `/home/claude/target-feel/release/h2viewer`.
- `$SP/mm-wigl.txt`, `$SP/mm-screens.txt`, `$SP/mm-tags.txt`: tag dumps.
- `$SP/mm-bm/`: bitmaps exported for inspection only (logo, tracks, list skins, framing).
- `$SP/mm-web/`: Assembly plugins, FontPackager sources, Cartographer `Config.cpp` (`skip_intro`).
- `$SP/mm-mock/`:
  - `mainmenu-mock.png`: the H2 layout composed over a remake frame;
  - `now-sheet.png`: today's screens;
  - `fog-compare.png`: fog on and off;
  - `mock.py`.
- Screenshot runs: `$SP/lantest/runs/mm-now`, `mm-fly`, `mm-bare`, `mm-nofog`, `mm-scene`.
