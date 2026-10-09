# UI / HUD: decompilation vs halo2-rs (raw notes, in progress)

Decomp = /home/claude/kirklandsig/halo2-decompiled (Xbox retail). Status from config/functions.csv:
"matched" = byte-exact; "todo" = written but not matched (draft; logic may be off); no @retail = not decompiled.
Remake: main = /home/claude/halo2-rs; MM = main-menu worktree wf_a5629b98-87c-2.
H2V tag values below were read from the owner's maps with a scratch Python reader
(scratchpad/decomp/tools/h2map.py), not from the remake.

## Motion sensor (src/unknown_1a1c3e.cpp)
- range = hudg `ui\hud\default` +0x290 (H2V shared.map: 8.0 wu); +0x294 min speed 0.03; +0x298 32.0 (sensor scale).
  Remake h2sim/src/game/sensor.rs:9 SENSOR_RANGE = 25/3.048 = 8.20 wu (hard-coded).
- update (function_1a22b4, matched): every tick; the full object scan (clear + nearby + other) only every
  0.5 s (update_ticks = function_1469f0(0.5f)), between scans only the player units list refreshes.
  Scan radius = range*1.5 (motion_sensor_update_nearby_objects/other, todo), sample keeps <= range (2D x,y only).
- samples: 10 ring samples of <=16 blips per local player; sample_index decrements each tick.
- who shows (motion_sensor_object_visible 0x1a20e6 matched; _moving 0x1a1d55 matched):
  * player-controlled unit (unit+0x13c player index != NONE): moving if |v with z*0.33| >= matg player_information[0]+0x3C (sneak forward) * 1.05
  * other units/creatures: |v (z*0.33)| >= hudg+0x294 (0.03)
  * always: unit flags14a & 0x21, or function_e70e0(unit) not 0 and not 3 (decomp calls it zoom level; meaning uncertain)
  * unit flag 0x134 bit30 forces visible (when not hidden)
  * active camo (unit flag 0x134 bit 3) hides from the sensor only when g_4e6948->state != 2 (i.e. campaign); in MP camo does not hide.
  * debug global g_51e990 shows everything.
  Remake sensor.rs:38 `fast = sneak_forward * 1.15` (SNEAK_MARGIN) on full 3D speed; shot keeps you on 1.5 s (SHOT_SHOWS); allies always show.
  Halo 2: allies do NOT show unless moving/firing-flag (no "teammates always" rule in visible()).
- objects: nearby list = all player units (data players) within 1.5*range; other list = bipeds/vehicles/creatures (mask 0x1003)
  not player-controlled and creature def flag bc bit6 clear and object flag 10a bit2 clear. Max 16 total per player.
- blip type/colour (g_445320): 1 self (1,.5,0), 2 ally unit (1,1,0) yellow, 3 enemy unit (1,0,0), 4 ally/neutral vehicle (1,1,0),
  5 enemy vehicle (1,0,0), 6 none/pulse (.5,.5,1), 7 ally talking (1,1,.3), 8 enemy talking (1,.3,.3).
  Talking (voice, function_53750) in MP (state==2) -> type 7/8 and blip size forced "large".
  Vehicle: enemy if its team is enemy; else by driver(+0x24c)/gunner(+0x248) team; empty -> 4 (or 5 if first block type 0xa000082).
  Remake local.rs:209 SENSOR_ALLY (1,.85,.25), enemy hud::RED (1,.25,.2).
- blip size: unit def +0x194 "motion sensor blip size" (creature +0xc2) clamped 0..3 -> offset g_47ff9c {0,-0.75,+1.0}.
  Remake local.rs:1088 vehicle 0.24 / person 0.15 of radius, scaled by its own pulse.
- trail: render (0x1a2823, todo) draws all 10 samples; age i: fraction=(10-i)/10, brightness=fraction^2,
  size term = (1-fraction)^3.5*7+1 (+ size offset); quad half-extent = 2*(alpha*intensity+offset) px in 64x64 sensor space.
- sweep pulse (0x1a224e matched): period 2.1 s: t=fmod(time,2.1); scale = 1/((t+0.0625)*1.1) while t<2.0375 else 0.4.
  Remake SENSOR_PULSE = 1.0/s, blips swell 0.85..1 and fade 0.6..1 with it.
- sensor drawn into 64x64 target, then on screen at centre +-42 px (one view) or +-32 px (splitscreen, g_4ba04c>1) (0x2548f0 todo).
- not drawn when camera mode (function_155760) is 2 or 3 (observer/dead/transition).
- enemy_nearby (0x1a2b6b todo): enemy unit/vehicle within range; enemy_vehicle_ahead: within 4 wu, slower than 1.5, in 30 deg cone.

## HUD messages (src/unknown_24c7c1.cpp, lane O)
- 4 slots per local player, 63 chars. Shown: 4 (3 in splitscreen, minus 1 more if a top message shows in split).
- sorted active-first, newest first; drawn top-down from anchor function_1396c7(1) (weapon-HUD anchor, NOT decompiled)
  + 60 px down (35 px in splitscreen), left aligned, line advance = text height * hudg+0x80 (1.1).
- timing (24cdd8 todo / 24cf66 todo): full for hudg+0x58 (2.0 s) then alpha *= (1 - (age-up)/fade)^1.9 over hudg+0x5c (2.0 s); removed at 4.0 s.
  MM local.rs:261 message_alpha linear; main local.rs:534 5.0 s, alpha = min(left,1) (linear last 1 s).
- counted messages (24caac matched): same text not expired -> count++ and plural text, e.g. "Picked up  frag grenades".
- font: index 6 (5 in splitscreen); colour = player's HUD colour (13927e) * HUD alpha (1392a9); shadow colour g_4686d4.
- top "scripted/tutorial/timer" line flashes in: white & 1.5x size settling over 0.25 s.
- bottom: timed message (text_454) centred at viewport bottom-40 px, alpha (1-t/T)^0.4; below it (+18 px) the player
  status line (function_15f120 suppress=1): mulg runtime state responses (H2V shared.map multiplayer_globals +0x538):
  "Respawn in #local_spawn_time", "Waiting for space to clear", "Observing", "You are sitting out", "You are out of lives",
  "Welcome!" (first 3 s of round), "Enemy has your Flag!", "You are the Juggernaut!", "You control the hill",
  "Flag is contested", "Bomb is contested", "#n lives left" / "Last life!" (3 s after spawn), "You win!/Tie game!/You lose!".
  MM local.rs:1290 "RESPAWN IN N" mid-screen (12*t), none of the others.
- HUD messages hidden for a user once the scoreboard fade (timers[user]) reaches 1 (function_161b60 matched).

## Scoreboard (src/unknown_1600f0.cpp 0x1608e0 todo, rows 0x1600f0 matched / 0x1603f0 todo; timers in unknown_157450.cpp ~4130)
- shows while Back (button 13) held; fade-in 0.25 s (timers max = 0.25*30 ticks); also automatically 1 s after
  the game leaves "playing" (function_15b2f0), and while dead under some conditions.
- right stick Y scrolls it when taller than the screen: 240 px/s at full deflection.
- layout (640x480 px): x = centre - 134, width 268; title row 20 px (player status via 15f120 suppress=0:
  "Winning with #local_player_score of #score_to_win", "Tied with..", "Losing with..", "GAME OVER", "ROUND OVER",
  "WAITING FOR NEXT ROUND"); header row at +24: [blank] | "Name" | "Wins" (only if rounds) | "Score"
  (strings: multiplayer\global_multiplayer_messages mp_name/mp_wins/mp_score); header text grey .125 on white.
  Team rows (20 px) then 21 px gap then player rows (20 px). Columns: place 25, icon 20 (player rows), name, wins 50, score 54, net bars 7.
  Name width = 216 - score - wins (players), 236 - ... (teams).
- colours: row bg = team/player colour, alpha*0.25; text = colour*0.3+0.7; dead/quit/no-team rows text *0.4.
- each row fades in in turn: alpha_row = clamp((a - row/total*0.5)*2).
- place = rank/2+1; in team games players show their team's place. Score shown as time (" :SS" under a minute, "M:SS")
  for Oddball/KOTH/Territories (mode 3,4,8); else integer.
- no kills/assists/deaths columns in game. MM menu.rs:3989 draw_scoreboard: 514 wide, place|emblem|name|level|score|kills|assists|deaths.

## Menu input (src/unknown_190001.cpp, lane H)
- 0x1907d6 (todo): one button event per frame (first controller found wins), on first frame down only:
  A,B,X,Y,Black,White,Start(12),Back(13),LT(6),RT(7),LS(14),RS(15).
- stick axis counts only past 0x7332 (~90%); x beats y. d-pad: right,left,up,down when stick idle,
  on press or once held >= 250 ms (0x1907bf matched).
- repeat (0x190a3d todo): first push immediate; held -> every 250 ms (first repeat at 250 ms).
- main: input.rs:33 STICK_PUSH (0.6 push, 0.35 release after a 0.2 dead zone), no auto-repeat (task #60 adds one on the controller branch).

## Lists (src/unknown_24c177.cpp)
- 0x24c1c5 (todo): focus moves within visible items; at an end the data scrolls under the items;
  tag list flag bit0 (value7c) wraps focus last->first / first->last (only when the data's end is reached);
  a separate `wraps` member (never set in decompiled code) makes the data ring-scroll.
- cursor sound (UI sound 0) only when the focused datum changed (no sound at a blocked end).
- after a move: value74/value76 = 5 (likely up/down arrow highlight counters).

## UI sounds (wigl +0x90, 13 refs; 0x236299 todo) H2V mainmenu.map ui\ui_shared_globals:
 0 cursor1 (focus move), 1 forward1 (A/Start on a button or item), 2 flag_fail (refused), 3 advance (a screen opens,
 not for history rebuilds or screen id 9), 4 back1 (screen closed), 5 transport, 6 cursor1 (keyboard erase),
 7 virtual_keyboard_click (key typed), 8 receive_message, 9 forward1 (tab change, Y menu), 10 countdown_for_respawn
 (lobby countdown stage 3..0 tick), 11 none, 12 pickup_health.
 Sound tags flagged 0x80 pick the permutation named after the language (localized UI sounds).
 MM ui.rs:606 reads 0..4 only.

## Widget animations (src/unknown_22e27b.cpp 0x22e3cd)
- frame_time = period/(frames-1): same as MM menuart.rs:437. Modes: 0 hold, 1 loop, 2 bounce; outro (reverse) on close.
- list item animations: 0 on focus gain, 1 on focus loss, 2 when outside the window.
- screen transitions (default window get_transition): NOT decompiled.

## List wrap flags as the H2V tags set them (scratchpad/decomp/tools/uilists.py)
- wgit pane lists (+0x20 panes, 0x4C each; pane +0xC lists, 0x18 each; list +0 flags). bit0 = wraps.
  mainmenu.map: 151 lists wrap, 28 don't. shared.map: 76 wrap, 17 don't.
- WRAP: main_menu (screen 6, 0x3), confirmation_dialog (33, 0x3), error_dialog_ok_cancel (7), dialog_pause (18),
  mp_menu (195, the MP pause menu), pause_settings (259), change_teams (198), player_settings (196).
- NO WRAP: difficulty_lobby (1, 0x2), delete_profile_confirmation (48, 0x2), advanced_keyboard_settings (253, 0x2),
  settings panes 1-5 (19), button/thumbstick layout panes, player_selected_dialog (32), message_type_dialog (35),
  system_settings pane 1, matchmaking_progress panes.
- Remake MM menu.rs:1632-1637: every list wraps except Pause and Confirm (deliberate: "QUIT is never a nudge up
  from RESUME"). Halo 2's tags make the pause menu and confirmation dialog wrap, and the difficulty list not wrap.
  blam-cache ui.rs:73 LIST_WRAPS is read but unused.

## Text justification (253c8b.cpp 0x253765)
- text flags &1 -> left (0), &2 -> right (1), else centred (2); &4 pulsate; &8 editable (font forced to 1).
  Matches MM ui.rs LEFT_JUSTIFY 1 / RIGHT_JUSTIFY 2 / PULSATING 4.

## Item fades (22e27b.cpp 0x22e3cd + list item animations 0/1/2)
- the item animation's alpha scales the whole item including its text; MM menu.rs:686 TEXT_FLOOR 0.6 keeps
  text at >= 0.6 (comment says Halo 2 fades whole items to a third). Deliberate readability deviation.

## Buttons (253c8b.cpp 0x2540e8 todo)
- a button fires on event type 5 with param 0 (A) or 12 (Start) and plays UI sound 1 (forward1);
  direction events move focus (v8/v9) with sound 0 unless flags valuef4 &1 (vertical) / &2 (horizontal) block it.

## Press start screen (src/unknown_22f118.cpp, all matched except v3)
- its only widget is a button: A or Start activates (other buttons do nothing).
- handle_start (0x22f3b5 matched): game invite accepted < 15 min ago -> "join?" dialog; else if nobody signed in
  -> sign this controller in (or a "no room for a profile" dialog). Xbox profile flow; Vista differs.
- v3 (0x22f28c todo): when a user is already signed in, it goes straight on to the main menu and plays UI sound 5 (transport).
- MM menu.rs:1600-1605: any key or button (and a click) goes on with Sound::Forward.

## Main menu (src/unknown_14741b.cpp c_main_menu_screen)
- v19 0x230b32 restores the last selected item g_510a14 (0..4) when the screen is rebuilt.
- v10 0x230c2b: B (1) or Back (13) from a signed-in user opens a sign-out confirmation.
- MM: back_to_main (menu.rs:2051) lands on the item that led to the sub-screen (same effect for Back);
  B on the main menu does nothing (menu.rs:2091).

## Intro movie
- Xbox: d:\bink\intro_60.bik. loading.cpp 0x163890 (todo) plays it during the first-boot map copy with flags 0x3c6,
  not skippable until the copy is done; main_play_intro_movie (12be90.cpp 0x12bf90 matched) plays it with flags 0x1c6
  once (main_globals.unknown28, cleared after) when the shell (game options type 3) starts, never on the xdemo map.
- skip: function_155f80 (01e930.cpp 0x155f80 matched): flags&2 and the movie is skippable, any of A,B,X,Y,Black,White,
  LT,RT,Start,Back newly pressed (0x156ab0) ends it. D-pad and stick clicks do not skip.
- flags&1 loop; flags&0x10 fills the screen, else native size centred (0x156960 todo). 0x1c6/0x3c6 have bit 0x10 clear
  -> intro shown at native size centred (640x480 movie on a 640x480 Xbox screen = full screen anyway).
- MM intro.rs:2-5, main.rs:1280/2706: plays movie\intro_60.wmv (Vista file) every start (if intro_wanted), any key,
  mouse button or controller button skips; letterboxed to fit (intro.rs:486). Vista is a different exe; skip set is
  Xbox-only evidence.

## UI sounds: countdown
- c_screen_24fd74::update_countdown (250155.cpp 0x250a8b): UI sound 10 (countdown_for_respawn) at each new
  lobby countdown stage 0..3. MM has a matchmaking countdown (online.rs) with no sound.

## Fonts (src/font_loading.cpp)
- font_table.txt / font_table_<lang>.txt list up to 11 fonts (Xbox); Vista has 12 (MM blam-cache font.rs incl. TextChat).
- line height = ascent+descent+leading (default 10; 0x122540 matched). kerning: up to 96 pairs, chars <= 0xFF only (0x122570 todo).
- glyph pixels: run-length 4-bit alpha + 12-bit colour. Cache of 10 entries, header at 0x200, version 0xf0000001.
- HUD messages use font 6 (5 in splitscreen) = remake FullHudMessage/SplitHudMessage.

## Scoreboard extra
- main menu.rs:2701 draw_scoreboard: 514 wide, PLACE|PLAYER|SCORE|KILLS|DEATHS (2638), only while Tab/Back held (main.rs:2481).
- MM main.rs:2589-2606: also from game over until the carnage report; MM menu.rs:3989 adds emblem, level, assists.
- main local.rs:261 score_text "M:SS" (no leading-space form under a minute).

## Not decompiled / draft only (this area)
- screen transitions (c_window_channel_45997c::get_transition, include/unknown_234c64.h:68): not decompiled.
- HUD anchors: function_1396c7 (anchor positions) not decompiled.
- drafts (todo): sensor object_type/build_sample/render, 0x2548f0 sensor placement, 24cdd8/24cf66 HUD messages,
  1608e0/1603f0 scoreboard, 15f120/15ee30 status text, 1907d6/190a3d menu input, 24c1c5 list move, 236299 sound play,
  0x163890 loading/intro, 0x156960 movie draw, 22f28c press-start forward, 0x122570 kerning.
- meanings unresolved: unit flags14a & 0x21 and function_e70e0 (named zoom, decomp says "current zoom level")
  "always show" rule; unit flag 0x134 bit 30; who sets list `wraps` (ring scroll).

## Corrections / precision (later pass)
- sensor cadence (0x1a22b4 matched): a sample (positions + visibility) is built EVERY tick for every local player
  (build_sample 0x1a264e todo); the 0.5 s timer only refreshes the list of non-player objects (AI, vehicles).
  Player units are re-listed every tick. Trail = the last 10 ticks (1/3 s): newest sample bright and small,
  older ones bigger and fainter (scale = frac^2 is passed as brightness, (1-frac)^3.5*7+1 as size).
  Positions are stored as bytes in a 64x64 grid (0.25 wu cells at 8 wu range); the sensor turns with the player's yaw.
  The 2.1 s pulse value g_51e98c drives the sweep ring drawn by 0x2548f0 (todo), not the blip sizes.
- scoreboard timers live in function_15bb20 (todo) lines ~4132-4176; fade-out is symmetric (1 tick per tick -> 0.25 s).
  Dead auto-show needs: unit NONE, player+0x170 <= 1, !(flags & 0x4000), !function_15db30, game_time > 3 s, flags & 1;
  function_162c50 suppresses it.
- status type picker 0x15ee30 is "near" (almost matched); 15f120 todo.
- main: HUD messages main.rs:1862 "PICKED UP {what}" one line per pickup, no counting; local.rs:532-537 max 4, 5.0 s;
  local.rs:1204-1215 drawn bottom-left above the tracker, newest lowest, 11*t line for 8*t text.
