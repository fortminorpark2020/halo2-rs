# Controller: verified findings (Halo 2 Xbox decomp vs halo2-rs main)

Decomp: /home/claude/kirklandsig/halo2-decompiled (original Xbox retail). "matched" means the function's bytes match retail. "todo" means a draft that does not match yet, so its logic is likely but not proven. "stub" or "not in src" means not decompiled. Statuses come from config/functions.csv.

Remake: /home/claude/halo2-rs at main (fcec0f4), read only. Tag values come from the Vista maps (lockout.map, coagulation.map), read with small read-only scripts in scratchpad/decomp/verify_ctl/ and verify_mc/.

Halo 2 Vista's PC input layer (XInput polling, mouse and keyboard, Vista-only options) is not in the decomp. Where a constant lives in code rather than tags, Vista may differ. Vista's mainmenu.map does contain the strings "CONTROLLER VIBRATION", "AUTOMATIC LOOK CENTERING", "Jumpy" and the Spanish "INVERTIR USAR A DOS MANOS" (dual wield inversion), so those options exist on Vista.

Ranked by how much a player would notice.

---

## 1. Vibration is missing; Halo 2 drives it from jpt! player responses
- **Decomp:**
  - src/unknown_2229d0.cpp: `rumble_player_play_effect` 0x222cb0 (matched, l.164-191); `rumble_player_evaluate` 0x222e90 (todo, l.193-242); `function_222a70` (todo, l.244-292).
  - src/unknown_1248b0.cpp: `function_124a40` (matched, l.313-331) and `function_124ee0` (matched, l.225-251).
  - src/unknown_153950.cpp: `function_153d10` (todo, l.521-575): picks the player response and plays it.
  - src/weapons.cpp l.2985-2986 (todo): the firing-damage reference; src/unit_object_type.cpp `function_d03e0` (todo, l.3357-3385).
  - src/unknown_13b390.cpp `function_13b390` (todo): the tag function evaluator.
- **Halo 2:**
  - Each local player has 8 rumble slots. A new effect takes the slot with the largest timer (the oldest).
  - Every update, for each slot and each motor (0 = left/low frequency, 1 = right/high frequency): if t < duration, add function(t/duration) × scale. Add a global scripted term, multiply by 65535, clamp to 0..65535, round. Timers advance after the evaluation.
  - The effect is the jpt! "player responses" block (+0x6C, 0x4C per entry). The first entry whose type fits is used: type 0 = shielded only, 1 = unshielded only, 2 = both (shielded means unit +0xF0 > 0). Low frequency {duration, function} is at +0x24/+0x28; high frequency at +0x30/+0x34. The function is a tag function: a 0x14-byte header, then parameters (a transition goes from p0 to p1).
  - Sources: every shot uses the barrel's firing effect "firing damage" jpt! (entry +0x20 + mode×8; mode = firing, misfire or empty), at scale 1.0. Damage taken uses the same 153d10 path. The damage-taken scale is unclear in the draft.
  - The motors are zeroed when the profile's Vibration is Off, while paused, while input is suppressed, with no game running, and while a network simulation world exists but is not yet in its running state (world substate != 4).
- **Vista tags (shared data, the same in lockout and coagulation):**
  - battle_rifle_trigger: type 2. Low frequency 0.034 s, 0.8 → 0.3. High frequency 0.034 s, 0.9 → 0.05.
  - magnum_trigger: type 2. Low frequency 0.067 s, 0.8 → 0.3. High frequency 0.067 s, 0.9 → 0.7.
  - The other reader's battle rifle numbers (0.9→0.7 / 1.0→0.8) are wrong.
- **Remake now:** only the H2_PADS probe buzz (crates/h2viewer/src/input.rs:963-1020). There is no rumble in the game and no Vibration setting in `Controls` (crates/h2viewer/src/camera.rs:175-232).
- **Change:**
  - Add a per-player 8-slot rumble mixer as above, fed from jpt! player responses and evaluated with the tag function.
  - Trigger it per shot from the firing damage, and on damage taken.
  - Add a per-profile Vibration On/Off. Zero the motors while paused and outside a running game.
- **Vista:** XInput rumble works on PC, and Vista has the "CONTROLLER VIBRATION" option. Vista's own mixer code was not checked.
- **Evidence:** I re-read the play, evaluate and output code; the slot choice and sum are explicit. I dumped the responses with verify_ctl/jptpr.py.

## 2. Sticks are mapped circle-to-square, and look gets a second diagonal boost
- **Decomp:** src/unknown_217c40.cpp l.103-120 (0x217c40 todo); src/unknown_185be0.cpp l.191-204 (0x185be0 todo).
- **Halo 2:**
  - After the dead zone, each stick is multiplied by 1/max(|sin a|, |cos a|) (a = stick angle), then by 1/32767, and each axis is clamped to ±1.
  - For look only, when |yaw| > 0.1 and |pitch| > 0.1, both are multiplied by sqrt(1 + (minor/major)²) and clamped to ±1.
  - The look function and the acceleration peg test use these values.
- **Worked example:** a full diagonal on a round-gate pad reads raw (23170, 23170).
  - Halo 2: 0.596 per axis after the dead zone, 0.843 after the square map, 1.19 after the look boost, clamped to 1.0. Both axes turn at the full rate, and the stick is past the 0.85 peg, so acceleration builds to 2.5×.
  - Remake: 0.707 per axis. With Vista's look function {0, 0.05, 0.1, 0.25, 0.58, 1} that is 0.43 of the rate, and it is under the peg, so there is no acceleration.
  - Movement at full diagonal is the same in both, because the sim clamps length to 1 (crates/h2sim/src/player.rs:137). Partial diagonal pushes are faster in Halo 2.
- **Remake now:** no square mapping. `PadReading::state` (input.rs:550-565) passes the radial `dead_zone` (input.rs:590-597) output to `StickLook::turn` (camera.rs:254-283) and to movement.
- **Change:**
  - After the dead zone, map each stick circle-to-square and clamp. Do it before the thumbstick layout swap, as Halo 2 does.
  - For look, apply the sqrt(1 + r²) boost when both axes exceed 0.1, and clamp.
  - Feed these values to `look_function` and to the peg test.
- **Vista:** 0x217c40 is game-level code, so Vista probably keeps it, but this was not checked.
- **Evidence:** I re-read both drafts; both steps are explicit. g_4e61dc is the poll's dead-zoned pad state (+0x40 = thumbsticks). I dumped the look function from lockout matg.

## 3. Vehicle seats set their own look rates by vehicle speed
- **Decomp:** src/unknown_185be0.cpp l.163-186 (0x185be0 todo).
- **Halo 2:**
  - When the player's unit sits in a seat whose yaw (+0x44/+0x48) or pitch (+0x4C/+0x50) rate bounds are non-zero, that axis's rate becomes lerp(min, max, f) in deg/s. This **replaces** the profile sensitivity rate, so sensitivity does nothing on that axis.
  - f = clamp((|vehicle velocity| - min speed ref +0x54) / (max +0x58 - min), 0..1), raised to the power of +0x5C when that is non-zero.
  - The look function and pegged acceleration still apply.
- **Vista tags** (coagulation.map; vehi seat block +0x1C8, 0xB0 per seat, the same layout the remake already reads):

  | Seat | Yaw (deg/s) | Pitch (deg/s) | Speed ref (wu/s) | Exponent |
  |---|---|---|---|---|
  | Warthog driver | 60→20 | 30 | 5.5..9.0 | 1.6 |
  | Ghost driver | 60→5 | 30 | 4.5..8.5 | 1.6 |
  | Spectre driver | 60→20 | 30 | 4..7 | 1.6 |
  | Banshee | 50→10 | 60→30 | 3..12 | 1.2 |
  | Scorpion | 40 | 36 | – | – |
  | Wraith | 35 | 30 | – | – |
  | Warthog chaingun/gauss gunner, Spectre gunner, h_turret_ap, minigun | 60 | profile rate | – | – |
  | c_turret_ap | 60 | 40 | – | – |

  Passenger and boarding seats are 0, so they keep the profile rates.
- **Remake now:** every seat looks with the on-foot rates × sensitivity (`turn_with_stick`, crates/h2viewer/src/local.rs:721-745). Seat +0x44..+0x5C are not read (crates/blam-cache/src/vehicle.rs:312-330). At default sensitivity a Warthog driver's camera turns at 120°/s, and up to 300°/s pegged. Halo 2 turns it at 60°/s, falling to 20°/s at speed. A Warthog gunner turns at 120°/s instead of 60°/s.
- **Change:** read the seat yaw/pitch rate bounds, speed references and exponent into `Seat`. While seated, replace each axis's base rate with the lerp when its bounds are non-zero, using the vehicle's speed.
- **Evidence:** I re-read the draft. I dumped every vehi seat in coagulation.map (verify_ctl/seats.py).

## 4. Thumbstick dead zone is per axis (27.5%) with rescale, not radial 0.2
- **Decomp:** src/unknown_1248b0.cpp: `k_thumbstick_dead_zone = 9000` (l.26), `input_thumbstick_dead_zone` (l.160-170), applied to all four axes in `function_124c00` (l.218-221, 0x124c00 todo).
- **Halo 2:** each axis on its own reads 0 while |v| ≤ 9000/32767 (0.2747). Above that, (|v| - 9000) is rescaled linearly to full range. The dead zone is a cross: a push near an axis gives a pure one-axis value. It is applied at poll time, before layouts and menus.
- **Remake now:** a radial 0.2 dead zone with rescale (input.rs:27 `DEAD_ZONE`, `dead_zone` l.590-597).
- **Change:** use a per-axis dead zone with linear rescale on both sticks, as a named constant, then the square map from finding 2.
- **Vista:** 9000 is the Xbox value. Vista's PC poll code is not in the decomp and may use another value, such as XInput's suggested 7849 (left) / 8689 (right). Treat 9000 as the Xbox reference.
- **Evidence:** I re-read l.160-221. The constant is an enum in the file, but 0x124c00 is still todo.

## 5. Pitch limits: ±85.5° on foot, the seat camera's range in vehicles, eased in
- **Decomp:** src/unknown_185b00.cpp `function_187510` l.701-768 (todo); `player_control_get_camera` 0x1871e0 (matched, l.267-299).
- **Halo 2:**
  - On foot the pitch is limited to ±1.4922565 rad (85.5°), unless the unit camera's pitch range (bipd +0xE0/+0xE4) is non-zero.
  - In a seat, the seat camera's pitch range (seat +0x6C/+0x70) replaces that limit. It is shifted by the vehicle's tilt and pinned to ±85.5°.
  - The limits move toward a new value by at most π/256 rad per tick (about 21°/s at 30 ticks/s), so entering a vehicle lowers the view smoothly.
- **Vista tags:** masterchief_mp and elite_mp have pitch range 0, so ±85.5° on foot. Warthog and Ghost drivers and Warthog passengers: ±0.785 (45°). Warthog gunner: -0.332..0.611. Banshee: 0, so ±85.5°.
- **Remake now:**
  - `MAX_PITCH` is 1.5 (camera.rs:28, clamped in `FlyCamera::turn` l.67), and the sim clamps to ±1.5 (crates/h2sim/src/game.rs:684).
  - Seat `pitch_range` is read but only clamps turret aim (crates/h2sim/src/game/vehicles.rs:490-497), not the player's view.
- **Change:**
  - Set the on-foot limit to 1.4922565.
  - While seated with a non-zero seat camera pitch range, limit the view to it.
  - Ease limit changes at π/256 per tick.
- **Evidence:** I re-read the draft constants and the matched camera choice. I dumped the seat ranges.

## 6. Hold-to-zoom: holding the zoom button over half a second makes it temporary
- **Decomp:** src/unknown_186ab0.cpp l.224-254 (0x186ab0 todo).
- **Halo 2:**
  - A zoom press saves the current level and steps to the next. If that lands on a zoom level, a hold counter starts at 0.
  - While the button is held, the counter adds 1 per tick (cap 127).
  - On release, if the counter is > 15 (about 0.53 s at 30 ticks/s), zoom returns to the saved level.
  - Zoom is cleared when the unit cannot zoom, during cinematics, while paused and in some UI states.
- **Remake now:** zoom only cycles on press (crates/h2sim/src/weapon.rs:547-550).
- **Change:** keep the saved level and a hold counter in the weapon state. On release after more than 15 ticks, restore the saved level.
- **Evidence:** I re-read the draft. The press, hold and release branches are explicit.

## 7. Menu navigation: 0.9 stick threshold, 250 ms auto-repeat, one button event per frame
- **Decomp:** src/unknown_190001.cpp: `function_1907bf` (matched, l.880-889), `function_190a3d` (todo, l.908-958), `function_1907d6` (todo, l.1012-1148).
- **Halo 2:**
  - First-frame presses are checked in this order: A, B, X, Y, Black, White, Start, Back, LT, RT, LThumb, RThumb. The first one found sends an event and ends menu input for that frame, for all pads. Buttons do not repeat.
  - Otherwise direction comes from the **left stick only**. Each axis is zeroed under 0x7332 (0.9 of the dead-zoned range). When the stick is neutral, the D-pad is used in the order right, left, up, down.
  - A new push sends at once. A direction held at 0.9 or more on consecutive frames repeats every 250 ms. X beats Y. The D-pad sends on press, again 250 ms later, then every 250 ms.
- **Remake now:**
  - The left stick pushes at 0.6 on raw values and releases under 0.35 (input.rs:33, `stick_push` l.810-818, events l.739-756).
  - There is no auto-repeat for the stick or the D-pad.
  - Every event is delivered.
- **Change:** use the 0.9 threshold on dead-zoned values and repeat every 250 ms for the stick and the D-pad. Optionally deliver one button event per frame in Halo 2's order.
- **Evidence:** I re-read all three functions; 250 ms and 0x7332 are explicit, and the repeat path traces as described.

## 8. Aim assist adhesion is scaled by the same strength as friction, and the follow rate is clamped
- **Decomp:** src/unknown_185be0.cpp l.232-246 (0x185be0 todo). The strength k and the target motion (i, j) come from `function_1a4900`, which is not decompiled (stub in src/stubs/lane_f.cpp:43-44).
- **Halo 2:**
  - yaw rate = (1 - friction·k) × stick rate + clamp(target term, ±π rad/s) × adhesion·k.
  - Pitch is the same, with ±π/2.
  - This applies only when k > 0 and either the look or the move input is non-zero.
  - friction = matg player control +0x0 (Vista 0.6), adhesion = +0x4 (0.7), each clamped to 0..1.
- **Remake now:**
  - Friction is 1 - friction × (1 - off/angle) (camera.rs:304-310). This has Halo 2's shape, with the remake's own k.
  - Adhesion adds the change in target angle × adhesion with no k and no clamp (local.rs:733-741), so it pulls at full strength even at the edge of the cone.
- **Change:** multiply the adhesion term by the same k used for friction, and clamp the followed target's rate to ±π rad/s (yaw) and ±π/2 rad/s (pitch). Keep k's shape as the remake's estimate, because 0x1a4900 is not decompiled.
- **Evidence:** I re-read l.237-244. Both terms use the same `local_15.k`.

## 9. Triggers use adaptive hysteresis (release 32/255 below the peak, re-press 64/255 above the trough)
- **Decomp:** src/unknown_1248b0.cpp l.27-28, `function_124c00` l.185-207 (0x124c00 todo).
- **Halo 2:**
  - For each analog button, "down" means the **previous** sample > threshold.
  - While down: threshold = max(threshold, value - 32). While up: threshold = min(threshold, value + 64).
  - So a trigger releases once the pull drops 32/255 (12.5%) below its peak, and presses again once it rises 64/255 (25%) above its lowest point. It can be re-fired without going back to rest. There is one sample of latency.
  - On PC only the triggers are analog, so this matters for LT and RT.
- **Remake now:** gilrs axis-to-button with a fixed press at 0.3 and release at 0.2 (input.rs:30-31, `open_gilrs` l.601-607).
- **Change:** keep per-trigger peak/trough thresholds with margins of 32 and 64 out of 255.
- **Vista:** Vista's PC poll code is not in the decomp and may differ.
- **Evidence:** I re-read the loop. The margins are enum constants, and the update order is as stated.

## 10. Automatic Look Centering option is missing
- **Decomp:** src/unknown_186ab0.cpp l.257-277 (todo); src/unknown_185b00.cpp `function_187510` l.746-753 (todo). The settings screen is c_auto_level_settings_screen (src/unknown_2b116a.cpp:1413-1462) with its list (src/unknown_2c4e9c.cpp:2717-2727). It is profile controller_flags bit 3 (include/screen_widgets.h:35-42). Vista has the "AUTOMATIC LOOK CENTERING" string.
- **Halo 2:**
  - On foot only, with the setting on, all of these must hold: last tick's |forward move| > 0.5, this tick's pitch delta < 0.0001 (signed in the draft), and aim assist not active. Then a counter adds 1 per tick (cap 127). Centering engages once the counter is > matg player control +0x6E (Vista: 15 ticks).
  - While engaged, pitch moves toward level by at most matg +0x5C (Vista 0.5) × unit speed × seconds per tick × |pitch| × 2/π per tick.
  - Seat cameras with a non-zero auto-level target always use 0.08 instead of +0x5C.
- **Remake now:** not implemented. There is no setting (camera.rs:175-232), and `PlayerControl` does not read +0x5C or +0x6E (crates/blam-cache/src/physics.rs:209-232).
- **Change:** add the setting, read +0x5C and +0x6E (a short), and run the counter and rate as above. The decomp does not show the default (the other reader's "off as on Xbox" is unverified). Choose signed or absolute for the pitch test, and say which.
- **Evidence:** I re-read both drafts and dumped matg player control from lockout.map: +0x5C = 0.5, +0x6E = 15.

## 11. Lost controller: a reconnect dialog, with no pause in multiplayer
- **Decomp:** src/unknown_190001.cpp `function_18f9be` (l.1435-1459), `function_191135` (l.1359-1379), `function_19119c` (l.1383-1399), all matched; `function_146840` (matched, src/unknown_146240.cpp:273-282) = "game time is paused".
- **Halo 2:**
  - When a signed-in player's pad is lost, a reconnect dialog for that controller appears.
  - The game is paused only when the scenario type is 0 (campaign) and it is not already paused. Multiplayer keeps running.
  - The dialog cannot be dismissed until every lost signed-in pad is back; then the next lost pad's dialog shows.
- **Remake now:** a "CONTROLLER DISCONNECTED" HUD message only, and the player stands still (crates/h2viewer/src/main.rs:1338-1361).
- **Change:** show a reconnect dialog for that player that stays until the pad is back. Do **not** pause multiplayer for it. This matters for the controller branch's "disconnect pause" work.
- **Evidence:** all matched code. The scenario-type check, the paused test and the dismissal refusal are explicit. The other reader's "and not networked" was wrong.

## 12. Y switches weapons on release
- **Decomp:** src/unknown_185be0.cpp l.365-371 (todo); src/unknown_186ab0.cpp l.187-194 (todo).
- **Halo 2:** nothing switches while Y is down. On release, the switch flag (field_1c bit 0) is raised unless a hold latch (field_8f) was set; its setter is not decompiled. 186ab0 uses only this release flag to pick the next weapon (`function_cdeb0`), and ignores it while dual wielding.
- **Remake now:** `press_switch` switches on the press when no left-hand pickup is in reach, and on release otherwise (crates/h2sim/src/game/dual.rs:112-136).
- **Change:** always switch on release, unless the hold was used for a pickup.
- **Evidence:** I re-read both drafts. The weapon-switch request takes only the release flag.

## 13. A zoom press is ignored while either trigger is past half
- **Decomp:** src/unknown_185be0.cpp l.305-319 (todo).
- **Halo 2:** when not zoomed, a zoom press is dropped if the raw LT or RT analog is > 127. It is also dropped when the held weapon cannot zoom and the look stick magnitude² > 0.64; that has no effect in the remake, where such weapons cannot zoom. Zooming out is never blocked.
- **Remake now:** a zoom press always counts, except when dual wielding (local.rs `apply` l.1521-1544, zoom at l.1541; weapon.rs:547).
- **Change:** ignore zoom-in presses while either physical trigger is past half.
- **Evidence:** I re-read the draft. It reads the pad's raw analog bytes 6 and 7.

## 14. X/Y hold time comes from matg (0.2333 s), not 0.25 s
- **Decomp:** src/unknown_185be0.cpp l.360-364 (todo): hold threshold in ms = matg player control +0x7C × 1000.
- **Halo 2:** X and Y each have a "down" bit and a "held" bit. The held bit is set once the button's msec_down ≥ "minimum action hold time". On Vista lockout.map +0x7C = 0.2333 s. The consumers (reload vs pickup, dual pickup) are not decompiled.
- **Remake now:** `SWAP_HOLD = 0.25` is hard-coded (crates/h2sim/src/game.rs:223). It is used for the action hold (game.rs:2118), the dual pickup (dual.rs:125) and boarding (vehicles.rs:227, 469).
- **Change:** read +0x7C into `PlayerControl` and use it for the X and Y holds. Halo 2's boarding hold is unknown, so leave it.
- **Evidence:** I re-read the draft and dumped the matg value.

## 15. Legacy thumbstick layouts snap to axes
- **Decomp:** src/unknown_217c40.cpp `function_217c42` l.57-91, called l.156-160 (todo).
- **Halo 2:**
  - Legacy and Legacy South Paw only, after the square map.
  - Inside a zone around the diagonal (left stick 35° = 0.6108652 rad, right stick 10° = 0.1745329 rad), the dominant axis gets the full magnitude and the other gets (1 - angle-from-diagonal × 1.6370222) × magnitude.
  - Outside the zone, the dominant axis gets the full magnitude and the other gets 0. The results are clamped to ±1.
  - The axis assignments already match the remake.
- **Remake now:** the axes are swapped correctly, with no snapping (input.rs:483-492).
- **Change:** add the snapping for the two Legacy layouts.
- **Evidence:** I re-read the function and the layout cases.

## 16. Boxer: LT does nothing after dual wielding ends, until it is released
- **Decomp:** src/unknown_185be0.cpp l.322-334 (todo).
- **Halo 2:**
  - While both hands hold weapons, the action on the grenade trigger (grenade in Default, Green Thumb and South Paw; melee in Boxer) is zeroed, but its raw value still drives the left-weapon flags.
  - A latch keeps that action zeroed after dual wielding ends, until the trigger is released.
- **Remake now:**
  - Grenades fire on a press edge (crates/h2sim/src/game.rs:1380), so Default, South Paw and Green Thumb already behave this way.
  - In Boxer, LT is `MeleeOrLeftWeapon` (input.rs:251-262; local.rs:1528-1529). If the left gun goes away with LT held, the next tick sends melee with no previous melee, and the sim melees (game.rs:1375).
- **Change:** latch LT after dual wielding ends, until it is released, at least for Boxer's melee.
- **Evidence:** I re-read the draft and traced the remake's edge detection.

## 17. Back scoreboard fades in over 0.25 s and the right stick scrolls it
- **Decomp:** src/unknown_157450.cpp `function_15bb20` l.4133-4173 (todo); the fraction is read by `function_159170` (src/unknown_157450.cpp:863-880) and src/unknown_1600f0.cpp:81, and the scroll at l.1244-1252.
- **Halo 2:** while Back is held (read raw, not through the layout), a per-player timer rises to 0.25 s and falls at the same rate after release. Code that uses it reads it as a 0..1 fraction. The right stick's Y scrolls the board at 240 units/s at full push. The same timer also rises after death and at the end of a game.
- **Remake now:** the scoreboard is drawn instantly while Back or Tab is held (crates/h2viewer/src/main.rs:2481-2488; input.rs:562). It has no fade and no scroll.
- **Change:** add the 0.25 s fade in and out, and right-stick scrolling.
- **Evidence:** I re-read the draft. Input byte 0xd is Back's frames-down and 0x36 is the right stick's Y.

## 18. Look acceleration: the remake ramps one tick early
- **Decomp:** src/unknown_185be0.cpp l.205-218 (todo); `function_185b30` (matched, src/unknown_185b00.cpp l.361-388).
- **Halo 2:**
  - Per axis, when |mapped stick| ≥ peg (matg +0x48, 0.85), the rate is multiplied by 1 + (scale - 1) × clamp(timer/time). Yaw uses +0x4C (0.8 s) and +0x50 (2.5); pitch uses +0x54 and +0x58.
  - The ramp uses the timer **before** this tick's dt is added. The timer resets under the peg.
  - The look function is the same lerp as the remake's.
- **Remake now:** `held += dt` comes before the ramp (camera.rs:257), and the peg test uses the unmapped stick (camera.rs:254-283).
- **Change:** compute the ramp before incrementing, and test the mapped value (finding 2). The doc comment can cite the decomp instead of "the plain reading".
- **Evidence:** I re-read both functions. The remake's `look_function` matches 0x185b30.

## 19. Sensitivity tables confirm the remake's (3+s)/6 (comments only)
- **Decomp:** src/unknown_190001.cpp `function_190c34` and its tables, l.1259-1299 (matched); src/unknown_218420.cpp `input_preferences_set_defaults` (matched, l.25-49).
- **Halo 2:** sensitivity 1-10 sets absolute rates: horizontal {80, 100, ..., 260} and vertical {40, 50, ..., 130} deg/s. The default is 3 (120/60). These are profile values, not matg values.
- **Remake now:** (3+s)/6 × matg rates (camera.rs:211-213). This gives the same numbers because Vista matg has 120 and 60 (+0x44/+0x40). The doc cites Project Cartographer.
- **Change:** cite the decomp tables. Optionally use them directly, so a different matg cannot change the rates.
- **Evidence:** matched code; matg values dumped from lockout.

## 20. Aim assist zoom scaling matches the decomp (comments only)
- **Decomp:** src/unknown_1a58b0.cpp `function_1a50a0` (matched, l.1206-1278); src/weapons.cpp `function_101090` (matched, l.400-428).
- **Halo 2:** autoaim angle (+0x208) and magnetism angle (+0x210) are divided by the zoom magnification, and the ranges (+0x20C, +0x214) are multiplied by it. Magnification = min × (max/min)^(level/(count-1)). Weapon flag bit 5 at +0x12C means no assist unzoomed. The decomp's own field names for these are swapped, but the operations are as stated.
- **Remake now:** `aim_cone` does the same (crates/h2sim/src/game/autoaim.rs:199-206). The comment at l.30-33 says "the tags don't say how" and cites Halopedia.
- **Change:** no behaviour change; cite the decomp.
- **Evidence:** matched code.

## 21. The four Xbox button layouts match (no change)
- **Decomp:** src/unknown_218420.cpp l.25-49; src/unknown_190001.cpp l.1281-1294 (matched).
- **Halo 2:** the default action→button map is {A, Black, X, Y, B, White, LT, RT, Start, Back, LThumb, RThumb, D-left, D-right, A, B}. South Paw swaps the triggers and their analog values. Boxer puts melee on LT and grenade on B. Green Thumb puts melee on RThumb and zoom on B. The Xbox has only these four.
- **Remake now:** DEFAULT, SOUTHPAW, BOXER and GREEN_THUMB match (input.rs:225-276). Bumper Jumper and Recon are remake additions.
- **Change:** none. Note that Vista's mainmenu.map has a "Jumpy" layout string, so Vista has at least one more layout that the decomp cannot describe.
- **Evidence:** matched code; strings found in mainmenu.map.

---

## Rejected

- **"Being stunned slows look (stun turning penalty 0.8)".** The draft reading is right: look × (1 - unit stun × matg player information +0x7C). But unit stun (+0x2E4) is only written in the damage code when the jpt! stun (+0x34) > 0 and the game is multiplayer (src/unit_object_type.cpp:6813-6846). Every jpt! in the Vista MP maps has stun, max stun and stun time = 0 (94 tags in coagulation, 95 in lockout). The factor is always 1, so implementing it changes nothing. Also, the jump code (src/bipeds.cpp:3672) reads the same +0x7C, so which penalty that field holds is unsettled.
- **"Jump hold-off latch after UI or cinematics".** g_510c50 is the cinematic state: its first field is the letterbox amount (src/unknown_13cf00.cpp:129-150), and byte 5 means a cinematic is playing. That never happens in multiplayer, and nothing ties it to closing menus. The general per-button hold-off mask (field_c/field_e, 185be0 l.285-297) exists, but its setter is not decompiled.
- **"Crouch press only counts when airborne or moving slowly".** This is a draft-only reading. The helper `function_e4050` is also a draft. It means "a biped in state 1 or 3 for at least 0.18 s with no parent", which is not shown to be "airborne". Taken literally, a crouch pressed while running would be ignored, which contradicts common play. Confirm it in Vista before implementing.
- **"Flight-only look inversion option".** `function_218510` is matched (driver seat of a vehicle of type 3 or 5), but no Xbox UI sets controller_flags bit 2. The invert-look list has two items (src/unknown_2c4e9c.cpp:2603-2615), and nothing else writes the bit. Vista's maps have no "flight" invert string either. Players cannot reach the option.
- **"Aim assist target choice: inconsequential scale, model markers, left weapon".**
  - The inconsequential scale (matg +0x08) needs unit flag bit 19 at +0xBC (src/unknown_1a58b0.cpp:1588-1590). masterchief_mp (0x00448008) and elite_mp (0x00408008) do not have it, so it has no effect on players.
  - The left gun already autoaims per shot with its own definition (crates/h2sim/src/game.rs:1460-1466).
  - Marker aim points are already documented as a stand-in (autoaim.rs:26-29).
  - Target selection (0x1a4c00) is not decompiled, and scoring (0x1a60f0) is a draft.
- **Parts of other findings that were dropped:**
  - "Expose a B-action flag so Boxer can melee on B while dual." Flag 0x20000000 is set only when g_4e6948->state != 2, and state 2 is multiplayer (src/items.cpp:292, src/unknown_139296.cpp:376). It is never set in MP.
  - "Compute k with the half-then-linear falloff." 0x1a4900 is not decompiled. 0x1a4870 is only called by target scoring (0x1a60f0), not shown to produce k.
  - "Automatic Look Centering defaults to off on Xbox." The decomp does not show the default.
  - "Lost-controller pause only when not networked." The matched test is "not already paused" (function_146840).
  - "battle_rifle_trigger low 0.9→0.7, high 1.0→0.8." The tags give low 0.8→0.3 and high 0.9→0.05.
  - "Vehicle seats" kept, but the seat table was extended: gunner seats also override yaw to 60°/s.
  - "Team talk is White or D-up." True (0x57620 matched), but the remake has no voice.
  - "Order splitscreen players by port." `function_138640` is a draft that builds local session options. It is unclear where it applies, and the impact is negligible.
  - "Hold time ... medium impact." Kept at low: 0.2333 s vs 0.25 s is 17 ms.
