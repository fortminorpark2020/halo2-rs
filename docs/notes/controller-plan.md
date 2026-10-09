# Plan: native Xbox controllers and selectable controller layouts (Halo 2's own + Bumper Jumper)

Repo `/home/claude/halo2-rs` at `464878e`. This plan was researched read-only. It builds on worktree
`wf_a876195b-c46-7` (commits `58df034`, `dccba29`), which adds `camera::Controls`
(look sensitivity 1-10, mouse sensitivity, invert look), a `Screen::Controls` sub-screen off PLAYER
PROFILE, the profile keys `look_sensitivity=` / `mouse_sensitivity=` / `invert_look=`, and Halo 2's
stick look curve and aim assist. The brief placed that work in `input.rs`, but the look curve, friction
and adhesion are in `camera.rs` (`StickLook`, `look_function`, `friction`) and `local.rs`
(`look_with_stick` / `turn_with_stick`). `input.rs` is unchanged there. That worktree also has guests
use `Controls::default()`.

User's request (verbatim): "also make sure native xbox controller support works in it and also has the
button mapping and stick mapping selections like bumper jumper, bumper jumper wasnt in the original halo 2
but I want to add it in to this recreation".

---

## 0. Decisions in one screen

1. **Backend: switch gilrs to XInput on Windows.**
   `gilrs = { version = "0.11.2", default-features = false, features = ["xinput"] }`.
   - John's wired Xbox Series X|S pad is already live on XInput slot 0.
   - XInput delivers input whether or not the window has focus. Windows.Gaming.Input (WGI), gilrs'
     default, delivers it only to a focused window.
   - XInput has 4 slots, which matches Halo 2's 4 local players.
   - I verified the change builds:
     - `cargo check`, `cargo clippy --all-targets -D warnings` and `cargo test -p h2viewer` (92 pass) on Linux.
     - `cargo build --release --target x86_64-pc-windows-gnu` (mingw).
   - Turn off gilrs' default filters and own the dead zone. XInput's built-in dead zones (0.24 / 0.27)
     would otherwise stack on ours.
2. **XInput is global, so add per-window pad claims.**
   - Every process sees every pad. With two windows on one PC, one pad would drive both.
   - A pad nobody has claimed works only the focused window. Seating it claims it with an OS file lock,
     so it keeps driving that window when it's unfocused and is ignored by the other.
3. **Layouts are data in `input.rs`.**
   - Button layouts: Halo 2's four (DEFAULT, SOUTHPAW, BOXER, GREEN THUMB), read out of
     `mainmenu.map`'s own BUTTON LAYOUT screen, plus BUMPER JUMPER and, optionally, MCC's RECON.
   - Thumbstick layouts: Halo 2's four (DEFAULT, SOUTHPAW, LEGACY, LEGACY SOUTHPAW).
   - Menus keep working on the physical buttons (A select, B back, X team, Start, Back, d-pad and left
     stick).
4. **Settings are per local player.**
   - Player one's settings and guests 1-3's are saved in `profile.txt` (`button_layout=`,
     `thumbstick_layout=`, `guestN_...=`).
   - The rows use Halo 2's wording: THUMBSTICK LAYOUT, BUTTON LAYOUT, LOOK SENSITIVITY, LOOK INVERSION.
   - A guest edits theirs from their own pause menu (CONTROLLER SETTINGS, Halo 2's in-game name).
5. **Diagnostics.**
   - I already built a tiny windowless `padprobe.exe` (XInput) that can be run on John's PC now. See §7.
   - The game gets the same probe as `H2_PADS=<seconds>`, and `H2_PADS=log` to log pads while playing.

---

## 1. Windows backend

### 1.1 What gilrs 0.11.2 has (read from `~/.cargo/registry/src/*/gilrs-0.11.2`, `gilrs-core-0.6.8`)

**Features.**
- `gilrs-0.11.2/Cargo.toml`: `default = ["wgi"]`, `wgi = ["gilrs-core/wgi"]`, `xinput = ["gilrs-core/xinput"]`.
- `gilrs-core-0.6.8/Cargo.toml`: `wgi = ["windows"]` (windows `>=0.44,<=0.62`) and
  `xinput = ["rusty-xinput", "winapi"]`. Both are target-specific (`cfg(target_os="windows")`) optional
  dependencies.

**Backend selection** (`gilrs-core/src/platform/mod.rs`):
- `wgi` selects `windows_wgi/`, and `xinput` without `wgi` selects `windows_xinput/`.
- `compile_error!("features gilrs/xinput and gilrs/wgi are mutually exclusive")` fires if both are on.
- `compile_error!("Windows needs one of the features gilrs/xinput or gilrs/wgi ...")` fires if neither
  is on. This check is not cfg'd to Windows, so Linux also needs one of the two features.
  `features = ["xinput"]` satisfies it.

**Default backend: WGI.** The gilrs README says: "Windows defaults to using Windows Gaming Input instead
of XInput. If you need to use XInput you can disable the `wgi` feature ... and enable the `xinput`
feature." and "Windows Gaming Input requires an in focus window to be associated with the process to
receive events."

**XInput backend** (`windows_xinput/gamepad.rs`):
- **Slots and ids.** 4 slots (`MAX_XINPUT_CONTROLLERS = 4`). The gilrs `GamepadId(n)` is XInput slot n:
  `finish_gamepads_creation` makes ids `0..last_gamepad_hint()`.
- **Polling.**
  - A `gilrs` thread polls every 10 ms (`EVENT_THREAD_SLEEP_TIME`).
  - It checks empty slots every 100 iterations (`ITERATIONS_TO_CHECK_IF_CONNECTED`), so a plug-in is
    noticed within about 1 s.
  - It sends `Connected` / `Disconnected` events.
- **Triggers.** Separate axes 0-255 (`AXIS_LT2` / `AXIS_RT2`), which gilrs turns into
  `Button::LeftTrigger2` / `RightTrigger2` values.
- **Rumble.** `XInputSetState` with strong and weak motors (`ff.rs`).
- **Identity.**
  - `name()` is always `"Xbox Controller"`, `uuid` is nil, and there is no vendor or product id.
  - The battery comes from `XInputGetBatteryInformation`.
  - The Guide button is never reported, because `XInputGetState` doesn't have it.
- **Dead zones.**
  - `AXES_INFO` carries XInput's dead zones: `XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE` (7849),
    `RIGHT_THUMB_DEADZONE` (8689) and `TRIGGER_THRESHOLD` (30).
  - With default filters on, `gilrs/src/ev/filter.rs::deadzone` applies `d / range * 2`:
    0.24 left stick, 0.265 right stick and 0.235 triggers, rescaled.
  - Under WGI the same filter applies only `DEFAULT_DEADZONE = 0.1` (`gilrs/src/gamepad.rs:41`), because
    WGI's `AxisInfo.deadzone` is `None`.
- **DLL loading.** The DLL is loaded at runtime by rusty-xinput (xinput1_4 → 1_3 → 1_2 → 1_1 → 9_1_0).
  `objdump` of the mingw build shows no xinput import, only the DLL-name strings.

**WGI backend** (`windows_wgi/gamepad.rs`):
- **Enumeration.** `RawGameController::RawGameControllers()` is polled every 8 ms, with
  Added / Removed handlers. Pads that aren't Xbox-type come through as `RawGameController` with SDL
  mappings (uuid from vendor and product id).
- **Identity.** Names come from `DisplayName()`, plus vendor and product ids.
- **Rumble.** `Gamepad.SetVibration` on the main motors (gilrs doesn't expose the trigger motors).
- **Steam crash workaround** (issue #132).
- **Needs a focused window.**

**Axis to button.** `gilrs/src/gamepad.rs:639` sets `axis_to_btn_pressed = 0.75` and
`axis_to_btn_released = 0.65`. A trigger only produces a `ButtonPressed` at 75% pull, but `input.rs`
treats it as held from 0.3 (`TRIGGER`). Fix this with `GilrsBuilder::set_axis_to_btn`.

### 1.2 Comparison for this game

| | XInput (`features=["xinput"]`) | Windows.Gaming.Input (default `wgi`) |
|---|---|---|
| Native Xbox controller (360 / One / Series, wired, wireless or adapter) | Yes. The same API family Halo 2 Vista targeted; its BUTTON LAYOUT diagram is an Xbox 360 pad. | Yes |
| Works with the window unfocused (two windows on one PC, alt-tab) | **Yes.** "XInput has 'global' focus" (DirectXTK GamePad wiki). | **No.** "requires an in focus window" (gilrs README). This is the known gap. |
| Number of pads | 4 (XInput's limit, and Halo 2's 4 local players) | 8+ |
| Hot-plug | Yes, polled (about 1 s to notice a new pad) | Yes, event plus 8 ms poll |
| Triggers as separate analog axes | Yes, 0-255 each | Yes, 0-1 each |
| Rumble | Yes, 2 motors | Yes, 2 motors (trigger motors not via gilrs) |
| Non-Xbox pads (DualShock / DualSense, Switch Pro, generic HID) | No, unless something exposes them as XInput (Steam Input when launched through Steam, DS4Windows) | Yes, via RawGameController and SDL mappings |
| Pad names / VID / PID | Always "Xbox Controller", none | Real names and ids |
| Stable id across processes | **Yes:** id = slot, which is what makes the claims in §5.9 work | No: per-process discovery order |
| Builds for x86_64-pc-windows-gnu | **Verified** (h2viewer release build, 57 s; standalone padprobe.exe) | It's what ships today |
| Builds on Linux (CI ubuntu) | **Verified**: check, clippy `-D warnings`, 92 tests pass. The feature only adds Windows-target deps. | Yes |

**John's PC** (coordinator's read-only check):
- Wired Xbox Series X|S pad `VID_045E&PID_0B12`, stock driver, "XINPUT compatible HID device"
  `IG_00`, live on XInput slot 0 via xinput1_4.dll.
- The windowless WGI probe returned 0 pads. That's consistent with WGI wanting a focused window and
  enumerating late, so it's not evidence the pad is invisible to WGI, but it is exactly the setup
  (console, no focus) where WGI gives nothing.
- Steam is running, but Steam Input only wraps games launched from Steam, which ours isn't. Nothing else
  (DS4Windows / ViGEm / HidHide) is installed. Windows 11 build 26300 has xinput1_4.dll.

### 1.3 Recommendation: XInput

Reasons:
1. It is the native Xbox controller API, and John's pad is confirmed on it.
2. It is the only gilrs backend that keeps working when the window isn't focused. This is needed for two
   game windows on one PC (System Link testing, two players with a window each) and for alt-tabbing.
3. 4 pads is all Halo 2 splitscreen needs.
4. It gives the stable slot ids that the per-window claims need.

What it costs:
- PlayStation, Switch and generic pads stop working natively. Mitigation: add the game to Steam as a
  non-Steam game so Steam Input exposes them as XInput, or use DS4Windows.
- Pad names are generic.

If non-Xbox pads in the background become a requirement later, the next step up is SDL3 (HIDAPI, XInput
and RawInput, with `SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS`). That adds a C dependency and mingw
packaging work, so it's not worth it now.

### 1.4 Exact changes

`crates/h2viewer/Cargo.toml`:
```diff
-gilrs = "0.11.2"
+# XInput on Windows: Xbox controllers work whether or not the window has focus
+# (gilrs' default, Windows.Gaming.Input, reports only to a focused window).
+# Linux and macOS are unaffected (the feature only adds Windows dependencies).
+gilrs = { version = "0.11.2", default-features = false, features = ["xinput"] }
```

`Cargo.lock`, exactly as cargo produced it in a scratch copy:
- Adds `rusty-xinput 1.3.0`, `winapi 0.3.9`, `winapi-i686-pc-windows-gnu 0.4.0` and
  `winapi-x86_64-pc-windows-gnu 0.4.0`.
- In `gilrs-core`'s dependency list: `+ "rusty-xinput"`, `+ "winapi"`, `- "windows 0.62.2"`.
  `windows 0.62.2` stays in the lock for other crates.

No other workspace crate depends on gilrs, so feature unification can't switch WGI back on. If a future
dependency pulls gilrs with default features, the build fails loudly with "mutually exclusive".

Optional, only if we want a WGI comparison build:
```toml
[features]
default = ["xinput"]
xinput = ["gilrs/xinput"]
wgi = ["gilrs/wgi"]   # cargo build --no-default-features --features wgi
```
with `gilrs = { version = "0.11.2", default-features = false }`. I don't think we need this. The
standalone probe in §7 covers the comparison.

`input.rs` `Pads::new` uses the builder:
```rust
const TRIGGER: f32 = 0.3;          // pulled
const TRIGGER_RELEASE: f32 = 0.2;  // let go
let gilrs = GilrsBuilder::new()
    // Our own dead zone, the same on every backend: XInput's built-in ones (0.24 / 0.27 / 0.24)
    // would otherwise stack on it.
    .with_default_filters(false)
    // A trigger press (Boxer's melee, a tapped shot) at the same pull that counts as held.
    .set_axis_to_btn(TRIGGER, TRIGGER_RELEASE)
    .build();
```

`DEAD_ZONE` becomes the whole dead zone: `0.2`, radial, per stick, on raw values. Today it's gilrs' 0.1
plus `DEAD_ZONE` 0.11 rescaled, which comes to about 0.2 total, so the feel the look-curve work tuned is
kept. Update the doc comment on `DEAD_ZONE` ("after gilrs' own ..."). Turning the default filters off also
drops the jitter filter, which is harmless.

---

## 2. Halo 2's own controller settings (read from the game's maps)

**Method.**
- `h2tool text mainmenu.map '<unic>'` read the string lists.
- `h2tool hex` plus a scratch parser (`scratchpad/scripts/wgit.py`) read the `wgit` screen tags.
  Each pane is 0x4C bytes. Text blocks are 0x2C bytes: bounds `i16 ×4` at +0x1C, string id at +0x24.
- `h2tool sid` resolved the string ids.
- `h2tool bitmap` rendered `ui\screens\game_shell\settings_screen\player_profile\button_config` image 0
  (to scratch only). It is an Xbox 360 controller with leader lines:
  - left column: LT, LB, left stick, Back;
  - right column: RT, RB, X, Y, B, A, Start, right stick.
- The label slots in the wgit panes line up with those lines exactly.
- The maps are Halo 2 Vista (Project Cartographer), build `11081.07.04.30.0934.main`.

### 2.1 Halo 2's wording

From the `unic` tags in `mainmenu.map` (and, for in-game, `lockout.map` via shared.map):

- **Profile path.** EDIT PLAYER → "Controller"
  (`...\player_profile\edit_profile_menu`: `controller_settings: "Controller"`,
  "Configure your Thumbstick and button layout, or change various controller functions.").
- **Screen `...\player_profile\controller_settings`.**
  - Header "CONTROLLER".
  - Rows: "Thumbstick Layout", "Button Layout", "Look Sensitivity", "Look Inversion",
    "Automatic Look Centering", "Controller Vibration", "Restore Defaults".
  - Values "Enabled" / "Disabled", and "1".."10".
- **In game** (`lockout.map`).
  - `ui\screens\multiplayer\mp_menu\mp_menu` "GAME MENU": "Settings" → `player_settings` "PLAYER SETTINGS":
    "Controller Settings".
  - `mp_menu\controller_settings\strings` header "CONTROLLER SETTINGS", rows: "Thumbstick Layout",
    "Button Layout", "Look Sensitivity", "Look Inversion", "Auto Look Centering", "Controller Vibration",
    "Keyboard Settings", "Dual Wield Inversion", "Use Default Settings".
- **`...\player_profile\look_sensitivity`.** "LOOK SENSITIVITY", "1 (Low)" … "3 (Default)" …
  "5 (High)" … "7 (Very high)" … "10 (Insane)". "Choose the speed at which you look around. Faster is not
  always better."
- **`...\player_profile\invert_look`.** "LOOK INVERSION", "If this is enabled, moving the Thumb- stick up
  causes you to look down."
- **`...\player_profile\button_settings`.**
  - Header "BUTTON LAYOUT".
  - Names: "Default", "Southpaw", "Jumpy", "Boxer", "Green Thumb". In-game forms: "DEFAULT",
    "SOUTHPAW", "JUMPY", "BOXER", "GREEN THUMB". `controller_settings` also calls Default "Standard".
  - Labels: `button_fire` "Use Right Weapon", `button_throw_grenade` "Use Left Weapon", `button_reload`
    "Reload", `button_switch_weapons` "Switch Weapons", `button_melee` "Melee Attack", `button_jump`
    "Jump", `button_swap_grenades` "Swap Grenades", `button_flashlight` "Flashlight/Team Chat",
    `button_zoom` "Zoom View", `button_crouch` "Crouch", `button_score` "Multiplayer Score",
    `button_pause` "Pause Game", `button_boxer_grenade` "Throw Grenade", `button_boxer_melee`
    "Melee/Use Left Weapon". `button_lean_left` / `button_lean_right` ("Lean Left" / "Lean Right") are
    unused leftovers.
  - Help:
    - Default: "The default setting. This is Bungie's recommended button layout for the typical player."
    - Southpaw: "This setting is identical to the default setting except the triggers are swapped for
      lefties."
    - Jumpy: "For those who like to bounce around like bunnies, we made it easier by mapping jump to the
      left trigger."
    - Boxer: "If you're tired of hitting the wrong button in the heat of a melee fight, this setting is for
      you."
    - Green Thumb: "This setting allows you to melee attack by mashing harder on the right thumbstick."
- **`...\player_profile\thumbstick_settings`.**
  - Header "THUMBSTICK LAYOUT".
  - Names "Default", "Southpaw", "Legacy", "Legacy Southpaw". In-game forms "DEFAULT", "SOUTHPAW",
    "LEGACY", "LEGACY SOUTHPAW".
  - Descriptions:
    - Default: "Default controls your movement with the left thumbstick and your look with the right
      thumbstick."
    - Southpaw: "Southpaw swaps left and right thumbsticks from the default setting. A special option for
      lefties."
    - Legacy: "Legacy is based off older console FPS configurations. Looking and strafing are combined."
    - Legacy Southpaw: "Legacy Southpaw swaps left and right thumbsticks from the legacy setting. An option
      for ancient lefties."
  - Labels: "Move Forward", "Move Backward", "Strafe Left/Right", "Look Up", "Look Down",
    "Rotate Left/Right".
- **Vibration.** "CONTROLLER VIBRATION", "If this is enabled, your controller will vibrate in response to
  game events."
- **The Xbox 360 pad.** "The glorious Xbox 360 Controller. There are few things in life that feel better
  in the palm of your hands."

### 2.2 The `button_settings` screen: which label sits on which button

The `wgit` tag `...\player_profile\button_settings` has 8 panes:
- Panes 0-3 are the Xbox 360 diagram (Vista's, drawn over `button_config` image 0) for Default, Southpaw,
  Boxer and Green Thumb.
- Panes 4-7 are the same four layouts on a second, smaller diagram. Its bitmap (image 1) is blank in
  Vista; it's the leftover original-Xbox controller layout, with Start in the left column and
  White / Black at the top right.
- **There is no Jumpy pane.** Jumpy exists only as strings, and Halopedia lists four Halo 2 layouts.
  Halo: CE (Xbox) had Jumpy (LT jump, A throw grenade), so the strings were inherited and the layout was
  cut.

Both diagrams agree on every slot. One exception: in Boxer's original-Xbox pane, LT is labelled "Melee
Attack" and B "Use Left Weapon", whereas Vista's pane labels LT "Melee/Use Left Weapon" and B "Throw
Grenade". The `boxer_*` strings were added later, so the Vista pane is the clarified one.

### 2.3 Halo 2's button layouts (Xbox original and Halo 2 Vista)

On the original Xbox, **White = Vista's LB** and **Black = Vista's RB**. That comes from the
label positions (Vista: left vs right bumper; original pane: White listed above Black) and matches
Halopedia's H2 table and the Halo fandom wiki's Xbox and 360 lists. The remake already has this
(`input.rs`: LB → Vision, RB → SwitchGrenade).

| Button (Xbox / Xbox 360) | DEFAULT | SOUTHPAW | BOXER | GREEN THUMB |
|---|---|---|---|---|
| Right trigger | Use Right Weapon | **Use Left Weapon** | Use Right Weapon | Use Right Weapon |
| Left trigger | Use Left Weapon | **Use Right Weapon** | **Melee/Use Left Weapon** | Use Left Weapon |
| Black / RB | Swap Grenades | Swap Grenades | Swap Grenades | Swap Grenades |
| White / LB | Flashlight/Team Chat | Flashlight/Team Chat | Flashlight/Team Chat | Flashlight/Team Chat |
| A | Jump | Jump | Jump | Jump |
| B | Melee Attack | Melee Attack | **Throw Grenade** | **Zoom View** |
| X | Reload | Reload | Reload | Reload |
| Y | Switch Weapons | Switch Weapons | Switch Weapons | Switch Weapons |
| Left stick click | Crouch | Crouch | Crouch | Crouch |
| Right stick click | Zoom View | Zoom View | Zoom View | **Melee Attack** |
| Back | Multiplayer Score | Multiplayer Score | Multiplayer Score | Multiplayer Score |
| Start | Pause Game | Pause Game | Pause Game | Pause Game |
| D-pad | (not on the diagram; menus) | | | |

What the labels mean in play (Halopedia H2 and Halo fandom):
- **"Use Left Weapon"**: throw grenade; dual wielding, fire the left gun; in vehicles, secondary fire,
  e-brake or boost.
- **"Reload"** is X's "Reload weapon/Swap weapon/Action": tap to reload (both guns when dual wielding, as
  the sim already does). Hold to pick up or swap a weapon, board or take a flag.
- **"Switch Weapons"**: tap to switch; hold by a one-handed gun to dual wield.
- **"Flashlight/Team Chat"**: the flashlight, or the Arbiter's active camouflage. The remake has no voice,
  so it's just FLASHLIGHT.

**Confidence.**
- Default / Southpaw / Green Thumb: **high**. The game's own diagram (two copies) agrees with Halopedia.
- Boxer: **high** that LT melees and B throws grenades. **Medium-high** that LT also fires the left gun
  when dual wielding (Vista's label says so; the old pane and Halopedia put "left weapon" on B). For
  vehicles, follow Halopedia: B is boost / e-brake.
- White = flashlight and Black = swap grenades: **high**.
- D-pad unused in play: **medium** (fandom lists D-pad up "Teamspeak" and D-pad down "lower weapon", a
  hidden combo).

### 2.4 Halo 2's thumbstick layouts

From the `wgit` tag `...\thumbstick_settings`, panes 0-3 (with a 4-7 duplicate set), each labelling the
up/down and left/right of each stick over `thumbsticks` image 0 (an Xbox 360 pad):

| THUMBSTICK LAYOUT | Left stick up/down | Left stick left/right | Right stick up/down | Right stick left/right |
|---|---|---|---|---|
| DEFAULT | Move Forward / Backward | Strafe Left/Right | Look Up / Down | Rotate Left/Right |
| SOUTHPAW | Look Up / Down | Rotate Left/Right | Move Forward / Backward | Strafe Left/Right |
| LEGACY | Move Forward / Backward | **Rotate** Left/Right | Look Up / Down | **Strafe** Left/Right |
| LEGACY SOUTHPAW | Look Up / Down | **Strafe** Left/Right | Move Forward / Backward | **Rotate** Left/Right |

**Confidence: high.** The table is the game's data and matches the description strings. The fandom wiki
lists Legacy the same way: "Left Stick - Move Forward/Backward | Rotate Left/Right; Right Stick - Look
Up/Down | Strafe Left/Right".

The stick clicks are not part of the thumbstick layout: crouch and zoom stay where the button layout puts
them (**medium** confidence: the thumbstick screen has no click labels).

---

## 3. Bumper Jumper, and MCC's Halo 2

**Halo 3 Bumper Jumper** (Halopedia H3:Control schemes; Halo fandom "Halo Controls"):
- LB Jump, RB Melee Attack.
- A "Reload/Swap Left Weapon/change grenade type", B "Action/Reload Right Weapon".
- X Use Equipment, Y Swap Weapons.
- LT Use Left Weapon, RT Use Right Weapon.
- Left stick crouch, right stick Zoom View, D-pad up team chat, Back score, Start menu.

It appeared first in Halo 3 (Haloology, "The Art of Jumping and Melee": "the left bumper is used for
jumping, while the right bumper is used for melee ... the B button is now used to reload and the A button
is used to switch grenades").

**Halo: Reach Bumper Jumper** (fandom): A Switch Grenades, B Action/Reload, X Armor Ability, Y Swap
Weapons, LB Jump, RB Melee, LT grenades, RT fire, click sticks crouch and zoom.

**MCC.**
- At launch (Halo Bulletin 16/10/2014, wiki.halo.fr) MCC added universal schemes "Recon", "Reclaimer",
  "Zoom Shoot", "Bumper Jumper" and "Green Finger" that work in every game, kept each game's original
  layouts "minus Southpaw" (replaced by "Bumpers and triggers can be swapped"), and kept the thumbstick
  layouts "Default, Southpaw, Legacy, and Legacy Southpaw".
- For Halo 2 the original layout is listed as "HALO 2 DEFAULT (AVAILABLE IN HALO 2 CLASSIC AND HALO 2:
  ANNIVERSARY)".
- An MCC moderator points Halo 2 players to the "Universal bumper jumper" binding (Steam thread
  2574320091912291101).
- Halopedia's MCC page tabulates MCC's **Halo 2 Universal Default (Recon)**:
  - Jump A, Crouch left stick click, Fire RT, Fire Secondary LT, Zoom right stick click, Melee B.
  - Reload RB ("reloading both weapons while dual-wielding"), Throw Grenade LT, Action RB.
  - Change Weapon Y, Dual-Wield Y, Switch / Next Grenade D-pad right, Previous Grenade D-pad left.
  - Toggle Flashlight D-pad up, Toggle Flashlight Alt X, Scoreboard View/Back.
- I found no text source with a per-button table for MCC's Universal Bumper Jumper *in Halo 2*. Its
  definition (Recon with jump and melee on the bumpers) plus Halo 3 and Reach give the mapping below.

**The remake's BUMPER JUMPER**, for every Halo 2 action:

| Halo 2 action | DEFAULT (Halo 2) | **BUMPER JUMPER** | Confidence for BJ |
|---|---|---|---|
| Jump | A | **LB** | high (all BJ versions) |
| Melee | B | **RB** | high |
| Reload; hold: pick up / swap weapon, board vehicle, take flag or bomb ("Action") | X | **B** | high (Halo 3 B "Action/Reload", Reach B "Action/Reload", MCC Recon puts Reload+Action together) |
| Reload while dual wielding (both guns, as Halo 2) | X | **B** | high |
| Switch weapon (tap) | Y | Y | high |
| Dual-wield pickup (hold) | Y | Y | high |
| Throw grenade / fire left gun when dual wielding / vehicle boost and secondary ("Use Left Weapon") | LT | LT | high |
| Fire / right gun ("Use Right Weapon") | RT | RT | high |
| Switch grenades | Black / RB | **A** | medium-high (Halo 3 A changes grenade type, Reach A Switch Grenades) |
| Flashlight / Arbiter's active camo | White / LB | **X** | medium (X is equipment / armor ability in Halo 3 / Reach; MCC Halo 2 Recon has "Toggle Flashlight Alt" on X) |
| Crouch | left stick click | left stick click | high |
| Zoom (nothing while dual wielding, as Halo 2) | right stick click | right stick click | high |
| Scoreboard | Back | Back | high |
| Pause | Start | Start | high |

Help line in Halo 2's voice (ours): "Jump and melee move to the bumpers, so your thumb never leaves the
right thumbstick."

**Other MCC layouts.**
- **RECON** (MCC's Universal Default) is cheap to add from the same table and is what MCC players' hands
  know. I'd include it as the sixth entry (§5.1).
- Halo 3's Walkie Talkie and MCC's Zoom and Shoot / Reclaimer / Green Finger aren't worth it now.
- JUMPY: leave it out. Halo 2 cut it, and its grenade / left-weapon placement under dual wielding is
  unknown.

---

## 4. The remake today (main `464878e`)

**`crates/h2viewer/src/input.rs` (200 lines).**
- `Pads { gilrs: Option<Gilrs>, sticks }` with `Gilrs::new()` (default filters on).
- `presses()` turns gilrs `ButtonPressed` straight into game meanings, hard-coded to Halo 2 Default:
  - RT2 → `Fire`, East → `Melee`, West → `Reload`, North → `SwitchWeapon`, LT2 → `Grenade`;
  - `LeftTrigger` (LB) → `Vision`, `RightTrigger` (RB) → `SwitchGrenade`, RightThumb → `Zoom`;
  - Start → `Join`, Select → `Leave`, South → `Claim`;
  - d-pad → `Up` / `Down` / `Left` / `Right`;
  - left-stick pushes past 0.6 (released under 0.35) also → `Up` / `Down` / `Left` / `Right`;
  - plus `Connected` / `Disconnected`.
- `state(id)` returns `PadState { left, right, fire, jump, crouch, zoom, action, switch, grenade, scores }`,
  equally hard-coded: South = jump, LeftThumb = crouch, West = action, North = switch, triggers > 0.3,
  Select = scores.
- Today's `PadPress` mixes **physical** meanings (Claim = A, Join = Start, Leave = Back, Up..Right) with
  **game** meanings (Melee, Reload, Fire...). This is what the layout table replaces.

**`main.rs`.**
- `App::pad_pressed`:
  - `Disconnected` / `Connected` → `pad_lost` / `pad_back`;
  - in menus → `flow::menu_pad`;
  - A / Start / RT skip a cutscene;
  - an unowned pad's A → `local::new_pad_for` + `take_pad`; Start → take or `add_local`;
  - an owned pad's Start → `pause(k)`;
  - otherwise it sets `locals[k].taps` (`Taps { fire, zoom, melee, reload, switch_weapon, throw_grenade,
    switch_grenade, vision }`), and Zoom is ignored while dual wielding.
- `update` reads `pads.state(id)` for look (`camera.look_stick`; in worktree 7 it's `l.look_with_stick`
  with `controls`, which is player one's profile for the keyboard player and `Controls::default()` for
  guests).
- The scoreboard is drawn while `PadState.scores` (Back) is held.
- `WindowEvent::Focused(false)` releases the mouse and keys. Focus isn't otherwise tracked.

**`flow.rs`.** `press(PadPress) -> Press` for menus: Up..Right → menu directions, Claim (A) → Select,
Melee (B) → Back, Join (Start), Leave (Back), Reload (X) → Team. `menu_pad` handles lobby seats
(`Seat { pad, team, picked }`). **Menus are meant to be physical but go through game names**: B is
"Melee" and X is "Reload".

**`local.rs`.**
- `LocalPlayer { keyboard, pad, lost_pad, taps, ... }`.
- `command()` ORs the keyboard and `PadState` into `h2sim::Command`. It has `cmd.reload |= p.action`
  ("Halo 2's X both reloads and picks up"), and zoom is suppressed while dual wielding.
- `new_pad_for(locals, PadPress::Claim | Join)`.
- **Prompts hard-code buttons:**
  - pick-up / dual prompts `["E","X"]` / `["Q","Y"]` (`local.rs:1121`);
  - `vehicle_prompt` "X" (`local.rs:1187`);
  - `objective_prompt` ("X","Y") (`objective.rs:567`);
  - campaign `switch_prompt` "X" (`campaign.rs:758`).

**How the sim reads `Command`** (`h2sim/src/game.rs:1349-1440`, `game/dual.rs`, `game/vehicles.rs`):
- `throw_grenade` throws only when not dual wielding and fires the left gun when dual wielding.
  In a vehicle it is boost or secondary (`vehicles.rs:506,542`).
- `zoom` also fires the left gun, which is the mouse's path.
- `reload` reloads both hands.
- `action` held picks up, boards or takes flags.
- `switch_weapon`: tap switches, hold takes a second gun.
- `vision` toggles the flashlight or camouflage.

So **every layout can be expressed in the viewer**, with no change to h2sim.

**Per-player settings.** There is one `Profile` (player one), held in `Menu.profile` and saved as
`%APPDATA%\halo2-rs\profile.txt`. Splitscreen guests have **no** profile:
- their name is `guest_name(name, k)` → `NAME(k)`, and their look is `look.guest(k)`;
- `flow::Seat` holds only pad and team;
- in worktree 7, guests get `Controls::default()`.

**Menus and pads.** Over a game, only the pause menu's owner's controller works it (`pad_in_menu`,
`menu_owner`).

---

## 5. Design

### 5.1 Data-driven layout table (`input.rs`)

```rust
/// A controller's buttons by where they are, with Xbox 360 names (an original Xbox
/// controller's White and Black are LB and RB, as Halo 2 Vista has them). gilrs calls the
/// bumpers LeftTrigger/RightTrigger and the triggers LeftTrigger2/RightTrigger2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadButton { A, B, X, Y, LB, RB, LT, RT, LeftStick, RightStick, Back, Start, Up, Down, Left, Right }
impl PadButton {
    pub const ALL: [PadButton; 16];
    pub fn from_gilrs(b: gilrs::Button) -> Option<PadButton>;
    /// For prompts: "A", "LB", "RT", "LEFT STICK", "UP"...
    pub fn label(self) -> &'static str;
}

/// What a button does in a game: the labels of Halo 2's BUTTON LAYOUT screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Function {
    RightWeapon,        // "USE RIGHT WEAPON"       fire
    LeftWeapon,         // "USE LEFT WEAPON"        grenade / left gun dual wielding / vehicle boost
    ThrowGrenade,       // Boxer: "THROW GRENADE"   grenade (vehicle boost), never the left gun
    MeleeOrLeftWeapon,  // Boxer: "MELEE/USE LEFT WEAPON"
    Reload,             // "RELOAD"                 and the action button: hold to pick up, board
    SwitchWeapons,      // "SWITCH WEAPONS"         tap switch, hold dual wield
    Melee,              // "MELEE ATTACK"
    Jump,               // "JUMP"
    SwapGrenades,       // "SWAP GRENADES"
    Flashlight,         // "FLASHLIGHT"             (Halo 2: "Flashlight/Team Chat"; the Arbiter's camo)
    Zoom,               // "ZOOM VIEW"
    Crouch,             // "CROUCH"
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonLayout { #[default] Default, Southpaw, Boxer, GreenThumb, BumperJumper, Recon }
impl ButtonLayout {
    pub const ALL: [ButtonLayout; 6];
    pub fn key(self) -> &'static str;     // profile.txt: "default", "southpaw", "boxer", "green_thumb", "bumper_jumper", "recon"
    pub fn from_key(s: &str) -> Option<ButtonLayout>;
    pub fn name(self) -> &'static str;    // "DEFAULT", "SOUTHPAW", "BOXER", "GREEN THUMB", "BUMPER JUMPER", "RECON"
    pub fn help(self) -> &'static str;    // Halo 2's help strings (ours for BJ and Recon)
    pub fn table(self) -> &'static [(PadButton, Function)];
    pub fn function(self, b: PadButton) -> Option<Function>;
    pub fn button(self, f: Function) -> Option<PadButton>;   // first one, for prompts
}

use Function::*; use PadButton::*;
const DEFAULT: &[(PadButton, Function)] = &[
    (RT, RightWeapon), (LT, LeftWeapon), (RB, SwapGrenades), (LB, Flashlight),
    (A, Jump), (B, Melee), (X, Reload), (Y, SwitchWeapons), (LeftStick, Crouch), (RightStick, Zoom)];
const SOUTHPAW: ...      // DEFAULT with (RT, LeftWeapon), (LT, RightWeapon)
const BOXER: ...         // DEFAULT with (LT, MeleeOrLeftWeapon), (B, ThrowGrenade)
const GREEN_THUMB: ...   // DEFAULT with (B, Zoom), (RightStick, Melee)
const BUMPER_JUMPER: &[(PadButton, Function)] = &[
    (RT, RightWeapon), (LT, LeftWeapon), (LB, Jump), (RB, Melee),
    (A, SwapGrenades), (B, Reload), (X, Flashlight), (Y, SwitchWeapons), (LeftStick, Crouch), (RightStick, Zoom)];
const RECON: &[(PadButton, Function)] = &[   // MCC's Universal Default, Halo 2 column (Halopedia)
    (RT, RightWeapon), (LT, LeftWeapon), (RB, Reload), (A, Jump), (B, Melee), (X, Flashlight),
    (Up, Flashlight), (Left, SwapGrenades), (Right, SwapGrenades), (Y, SwitchWeapons),
    (LeftStick, Crouch), (RightStick, Zoom)];
```

Back (scoreboard) and Start (pause, or join as a new player) stay **physical** and out of the tables.
They are the same in every Halo 2 layout, and Start / Back also mean join / leave.

**Thumbstick layout:**
```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StickLayout { #[default] Default, Southpaw, Legacy, LegacySouthpaw }
impl StickLayout {
    // keys "default", "southpaw", "legacy", "legacy_southpaw"; names "DEFAULT", "SOUTHPAW", "LEGACY", "LEGACY SOUTHPAW"
    /// (movement: x strafe, y forward; look: x turn, y pitch) from the two sticks
    /// (each already through the dead zone).
    pub fn apply(self, left: Vec2, right: Vec2) -> (Vec2, Vec2) {
        match self {
            Self::Default => (left, right),
            Self::Southpaw => (right, left),
            Self::Legacy => (Vec2::new(right.x, left.y), Vec2::new(left.x, right.y)),
            Self::LegacySouthpaw => (Vec2::new(left.x, right.y), Vec2::new(right.x, left.y)),
        }
    }
}
```

### 5.2 Readings, state and presses

```rust
/// A controller this frame by its buttons: the sticks through the dead zone, the buttons held.
pub struct PadReading { pub left: Vec2, pub right: Vec2, pub held: ButtonSet /* u16 bitset of PadButton */ }

/// What a controller asks of the game this frame under its player's controls.
pub struct PadState { pub movement: Vec2, pub look: Vec2, pub held: FunctionSet, pub scores: bool }

impl Controls {   // worktree 7's camera::Controls, gaining `buttons: ButtonLayout, sticks: StickLayout`
    pub fn pad_state(&self, r: &PadReading) -> PadState;   // pure: the testable heart
}

pub enum PadEvent {
    /// A button went down: the d-pad included.
    Down(PadButton),
    /// The left stick pushed one way (menus only; never the d-pad's in-game functions,
    /// or walking would toggle Recon's flashlight).
    Stick(Dir),
    Connected,
    Disconnected,
}
impl Pads {
    pub fn events(&mut self) -> Vec<(GamepadId, PadEvent)>;   // replaces presses()
    pub fn reading(&self, id: GamepadId) -> Option<PadReading>; // replaces state()
}
```

- `PadState.left` / `right` become `movement` / `look`. Update worktree 7's `turn_with_stick`
  (`pad.right` → `pad.look`, and "moving" uses both) and `command()`
  (`p.left` → `p.movement`).
- Inverted look still flips `look.y` whatever stick it comes from. The look function and aim assist apply
  per look axis, so Legacy works unchanged.

**Resolving functions into `Command`** (`local.rs`), one function shared by held state and taps. It is
evaluated at the tick, with the dual wielding state of that tick:
```rust
fn apply(cmd: &mut Command, f: Function, dual: bool) {
    match f {
        RightWeapon => cmd.fire = true,
        LeftWeapon => cmd.throw_grenade = true,            // the sim: grenade, left gun, or boost
        ThrowGrenade => cmd.throw_grenade |= !dual,        // never fires the left gun
        MeleeOrLeftWeapon => if dual { cmd.throw_grenade = true } else { cmd.melee = true },
        Reload => { cmd.reload = true; cmd.action = true }  // Halo 2's X: reload, hold to pick up
        SwitchWeapons => cmd.switch_weapon = true,
        Melee => cmd.melee = true,
        Jump => cmd.jump = true,
        SwapGrenades => cmd.switch_grenade = true,
        Flashlight => cmd.vision = true,
        Zoom => cmd.zoom |= !dual,                          // as today: nothing dual wielding
        Crouch => cmd.crouch = true,
    }
}
```

**Taps.** `LocalPlayer` gains `pad_taps: FunctionSet`. `App::pad_pressed`, for an owned pad's
`Down(b)`, does `if let Some(f) = l.controls.buttons.function(b) { l.pad_taps.insert(f) }`, and
`command()` applies them. Keyboard `Taps` stay as they are.

**Physical buttons** keep their own meaning in `pad_pressed`:
- A on an unowned pad claims player one; Start joins, or pauses an owned pad; Back is the scoreboard.
- Cutscene skip stays A / Start / RT.
- `local::new_pad_for` takes `PadButton::A` / `PadButton::Start`.

### 5.3 Menus unaffected by the layout

`flow::press` maps physical events:
- `Down(Up|Down|Left|Right)` or `Stick(dir)` → direction;
- `Down(A)` → Select, `Down(B)` → Back, `Down(X)` → Team;
- `Down(Start)` → Join, `Down(Back)` → Leave.

This is the same behaviour as today, without going through "Melee" or "Reload". Halo 2 (and Halo 3's
Bumper Jumper) also keep A / B for menus. Menus also always use the left stick, whatever the thumbstick
layout.

### 5.4 Prompts name the layout's button

Add `LocalPlayer.controls: Controls`, refreshed from the profile each frame (it's `Copy`), and
`fn pad_label(&self, f: Function) -> &'static str` = `self.controls.buttons.button(f).map_or("?", PadButton::label)`.

| Call site | Today | Becomes |
|---|---|---|
| `local.rs:1121` pick-up prompt | `["E","X"]` | `["E", pad_label(Reload)]` |
| `local.rs:1122` dual prompt | `["Q","Y"]` | `["Q", pad_label(SwitchWeapons)]` |
| `vehicle_prompt` | `"X"` | button label passed in |
| `objective_prompt` | take / drop `("X","Y")` | `(pad_label(Reload), pad_label(SwitchWeapons))` |
| `campaign::switch_prompt` | `"X"` | button label passed in |

So Bumper Jumper reads "HOLD B TO PICK UP BATTLE RIFLE", and Recon reads "HOLD RB TO DRIVE WARTHOG".

### 5.5 Profile fields and saved names (`profile.rs`)

On top of worktree 7's `look_sensitivity=` / `mouse_sensitivity=` / `invert_look=`:
```
button_layout=default            # default | southpaw | boxer | green_thumb | bumper_jumper | recon
thumbstick_layout=default        # default | southpaw | legacy | legacy_southpaw
guest1_button_layout=bumper_jumper
guest1_thumbstick_layout=default
guest1_look_sensitivity=3
guest1_invert_look=no            # guest2_..., guest3_... likewise (guest numbers as in NAME(1))
```
- Unknown or out-of-range values keep the default, the same rule worktree 7 uses for sensitivity.
- The settings stay on this PC. Like the look settings, they're not sent over System Link.
- Struct: `Profile { controls: Controls /* player one */, guests: [Controls; MAX_LOCAL - 1], .. }` with
  `fn controls_of(&self, local: usize) -> Controls` and `controls_of_mut`. Local 0 is player one, and
  local k is guest k (`add_local` names guests `guest_name(name, locals.len())`).
- Settings follow the **player slot**, not the controller: A on another pad taking over player one keeps
  player one's layout.
- `Controls` (in `camera.rs` in worktree 7) gains `buttons: ButtonLayout`, `sticks: StickLayout`, with
  `Default` = Halo 2's defaults.

### 5.6 Menu rows (`menu.rs`, building on worktree 7's `Screen::Controls`)

- Profile row: rename worktree 7's "CONTROLS" to **"CONTROLLER"** (Halo 2's EDIT PLAYER row). The screen
  title is **"CONTROLLER"** from the profile, or **"CONTROLLER SETTINGS"** from the pause menu (Halo 2's
  main-menu and in-game headers).
- Rows, in Halo 2's order:
  1. **THUMBSTICK LAYOUT**: DEFAULT / SOUTHPAW / LEGACY / LEGACY SOUTHPAW.
  2. **BUTTON LAYOUT**: DEFAULT / SOUTHPAW / BOXER / GREEN THUMB / BUMPER JUMPER / RECON.
  3. **LOOK SENSITIVITY**: 1-10 (worktree 7).
  4. **LOOK INVERSION**: rename worktree 7's "INVERT LOOK" to Halo 2's wording. The values can stay the
     remake's ON / OFF (Halo 2 says ENABLED / DISABLED).
  5. **MOUSE SENSITIVITY**: player one only, hidden on a guest's screen (no mouse). Halo 2 Vista keeps it
     under KEYBOARD/MOUSE.
- Left / Right / A cycle the values, as other value rows do (add them to `Row::adjust` and the
  value-row list). Each change returns `Action::SaveProfile`.
- `header()` shows the highlighted layout's help line. This is Halo 2's text, e.g. BOXER "IF YOU'RE TIRED
  OF HITTING THE WRONG BUTTON IN THE HEAT OF A MELEE FIGHT, THIS SETTING IS FOR YOU."
- The right-hand panel (where the profile's model turns) lists the selected button layout as Halo 2's
  diagram does: `RT  USE RIGHT WEAPON`, `LT  USE LEFT WEAPON`, `LB  JUMP` ... For the thumbstick layout,
  it lists `LEFT STICK  MOVE / ROTATE` and similar.
- Pause menu: add **CONTROLLER SETTINGS** (Halo 2's in-game screen; Halo 2 reaches it through GAME MENU →
  Settings → Controller Settings) between RESUME and END GAME. It opens `Screen::Controls` for
  `menu_owner`, and Back returns to PAUSED on that row.
- `Menu` gains `controls_for: usize`: 0 from the profile, the pause menu's owner from the pause menu.
- Later, optional: CONTROLLER VIBRATION (needs rumble), AUTO LOOK CENTERING, USE DEFAULT SETTINGS.

### 5.7 Per-splitscreen-player settings

- `App` reads `self.menu.profile.controls_of(k)` for local k each frame and copies it into
  `locals[k].controls`. This replaces worktree 7's `if l.keyboard { profile.controls } else { Controls::default() }`.
- `pad_state`, look, taps and prompts all use `l.controls`.
- A guest's pause menu is worked only by their controller (existing `pad_in_menu`), so a guest edits only
  their own settings.
- Two guests on the same layout don't interfere.

### 5.8 Pad claims and focus (needed because of XInput)

Add a small `Claims` type in `input.rs` (or `pads.rs`):
```rust
/// Controllers this window plays with, held against other copies of the game on this PC
/// (XInput gives every process every controller; slot n is the same pad in each).
pub struct Claims { dir: PathBuf, held: HashMap<usize, File> }
impl Claims {
    pub fn claim(&mut self, id: GamepadId) -> bool;   // open dir/pad{slot}.lock, File::try_lock(), keep it open
    pub fn release(&mut self, id: GamepadId);
    pub fn taken_elsewhere(&self, id: GamepadId) -> bool;
}
/// Whether this window takes a controller's input.
pub fn accepts(ours: bool, elsewhere: bool, focused: bool) -> bool { ours || (!elsewhere && focused) }
```

- **Lock files.** `std::env::temp_dir()/halo2-rs/pad{slot}.lock` with `std::fs::File::try_lock`
  (stable since Rust 1.89; the toolchain here is 1.99). The OS drops the lock if the game crashes.
- **When locks are taken.** Claim when a pad gets a seat or a local player: `take_pad`, `add_local`,
  lobby seat pushes, and `seats[0].pad = Some(id)`.
- **When locks are released.** Release when no seat or local references it as `pad` or `lost_pad`.
  - A disconnected pad stays claimed, so it comes back to the same window.
  - Check on leave and game end.
- **Focus.** Track focus in `App.focused` (`WindowEvent::Focused(b)`). Ignore every event and reading
  from a pad this window doesn't accept.
  - An unclaimed pad works only the focused window, so the window you're looking at gets A or Start.
  - A claimed pad keeps working its window while you're in the other one, or alt-tabbed.
- **Platforms.** On Windows only. Elsewhere `accepts` is always true; gilrs ids aren't stable across
  processes there.
- **README note.** Your controller keeps playing while you're in another window; leave the game (Back in
  the lobby, or quit) to free it.

---

## 6. Unit tests to write

**`input.rs`**
- `halo_2s_layouts_match_its_button_layout_screen`:
  - DEFAULT is exactly the ten pairs read from `button_settings` pane 0.
  - SOUTHPAW = DEFAULT with the triggers swapped.
  - BOXER = DEFAULT with LT `MeleeOrLeftWeapon` and B `ThrowGrenade`.
  - GREEN THUMB = DEFAULT with B `Zoom` and RightStick `Melee`.
- `every_layout_does_everything_once`:
  - Every layout reaches fire, a grenade, melee, jump, reload / action, switch weapons, swap grenades,
    flashlight, zoom and crouch.
  - No button maps twice.
  - Start and Back are never in a table.
- `bumper_jumper_jumps_and_melees_with_the_bumpers`: LB Jump, RB Melee, B Reload, A SwapGrenades,
  X Flashlight, Y SwitchWeapons.
- `recon_reloads_with_rb_and_lights_with_x_or_up`.
- `layouts_are_saved_by_name`: `key` ↔ `from_key` for all six, unknown → `None`. Same for
  `StickLayout`.
- `stick_layouts_move_and_look_as_halo_2_describes`. Left (0.5, 0) and right (0, 0.5) under each of the
  four layouts give:
  - DEFAULT: movement (0.5, 0) and look (0, 0.5);
  - SOUTHPAW: movement (0, 0.5) and look (0.5, 0);
  - LEGACY: movement (0, 0) and look (0.5, 0.5);
  - LEGACY SOUTHPAW: movement (0.5, 0.5) and look (0, 0).
- `the_dead_zone_is_ours_on_every_backend`: 0.19 → 0, full → 1, half-way about 0.37. Applied per stick
  before Legacy recombines them.
- `gilrs_buttons_have_xbox_names`: `LeftTrigger` → LB, `LeftTrigger2` → LT, `Select` → Back, `South` → A,
  d-pad → Up..Right.
- `the_left_stick_is_not_the_dpad`: a stick push is `PadEvent::Stick`, never `Down(Up)`.
- `trigger_presses_and_holds_agree`: `TRIGGER_RELEASE < TRIGGER`, and `pad_state` holds a trigger at 0.3.
- `prompts_name_the_layouts_button`: Reload → "X" (Default), "B" (Bumper Jumper), "RB" (Recon);
  SwitchWeapons → "Y".
- Claims:
  - `a_controller_plays_in_one_window`: two `Claims` on one temp dir; the first claims, the second can't,
    and after release it can.
  - `unclaimed_controllers_work_only_the_focused_window`: the `accepts` truth table.
  - `a_lost_controller_stays_claimed`.

**`local.rs`**
- `bumper_jumper_jumps_with_lb_and_swaps_grenades_with_a` (via `command()` with a `PadState`).
- `boxers_left_trigger_melees_or_fires_the_left_gun`: single → melee and no grenade; dual →
  `throw_grenade` (left gun) and no melee.
- `boxers_b_throws_grenades_but_never_fires_the_left_gun`.
- `green_thumb_melees_with_the_right_stick_and_zooms_with_b`: B does nothing while dual wielding. This
  generalises the existing `clicking_the_right_stick_dual_wielding_fires_nothing`.
- `southpaw_fires_with_the_left_trigger`.
- `legacy_turns_with_the_left_stick` (`turn_with_stick` with look from `left.x`).
- `taps_resolve_when_the_tick_runs` (a tapped Boxer LT that started single and is now dual fires the left
  gun).
- `pick_up_and_vehicle_prompts_name_the_layouts_button`.

**`profile.rs`**
- `controller_layouts_are_saved_by_name`: round trip `button_layout=bumper_jumper`,
  `thumbstick_layout=legacy_southpaw`; bad values → defaults; older profiles → defaults.
- `guests_keep_their_own_controls`: `guest2_button_layout=boxer` parses into `guests[1]`, player one is
  unchanged, and `to_text` writes it.

**`menu.rs`**
- `the_controller_screen_cycles_halo_2s_layouts`: rows in Halo 2's order. Right cycles DEFAULT →
  SOUTHPAW → BOXER → GREEN THUMB → BUMPER JUMPER → RECON → DEFAULT, Left reverses, each change returns
  `SaveProfile`, and the labels match.
- `a_guests_pause_menu_edits_their_own_controls`: `controls_for` = owner, and player one is untouched.
- `mouse_sensitivity_is_only_player_ones`.
- Extend worktree 7's `every_screens_rows_end_above_the_hint` (pause now has 4 rows; the controller
  screen 5).

**`flow.rs`**
- `menus_use_the_controllers_own_buttons`: B backs out and X picks a team even for a player on Bumper
  Jumper (where B reloads) or Recon.

CI already runs `cargo clippy --all-targets -- -D warnings` and `cargo test` on ubuntu. All of the above
are pure and need no controller.

---

## 7. Diagnostic for John's PC

### 7.1 Ready now: standalone `padprobe.exe` (XInput, no window)

- Built from `scratchpad/padprobe/src/main.rs` with gilrs 0.11.2 `features=["xinput"]`, default filters
  off and axis-to-button at 0.3 / 0.2.
- Output: `scratchpad/padprobe/dist/padprobe.exe`, 1.8 MB, mingw, imports only KERNEL32 / USERENV /
  WS2_32 / msvcrt / ntdll / bcryptprimitives / api-ms-win-core-synch. It loads xinput1_4.dll at runtime.
- sha256 `ea5fa38ae60b7b92da64abad7a56b2d2713a998ee17398a84e41e80e137bdf5f`. The Linux build runs and
  prints "no controllers connected" here.

Run it through the remote session (PowerShell):
```powershell
.\padprobe.exe 30          # 30 seconds; PADPROBE_NO_RUMBLE=1 to skip the half-second buzz
```

It prints:
- the backend line;
- every connected pad: `pad 0 (XInput slot 0 on Windows): "Xbox Controller", ... power Wired, rumble true`;
- a 0.5 s rumble on each pad;
- for 30 s, every press with both layouts' meanings, e.g.
  `pad 0: LB down   (DEFAULT: FLASHLIGHT; BUMPER JUMPER: JUMP)`;
- trigger values (`pad 0: RT 0.62`) and stick positions (10 Hz);
- plug and unplug events.

**Pass** if pad 0 is listed, A/B/X/Y/LB/RB/LT/RT/Back/Start, both stick clicks and the d-pad all print,
the sticks reach ±1.00, and it buzzes. Because it has no window, it also proves input arrives without
focus, which a WGI build can't do.

### 7.2 In the game

Add to `input.rs`:
- **`H2_PADS=<seconds>`** (a number). At the very top of `run()`, before `find_map()`, so it needs no
  maps and no window: run the same probe as padprobe and exit. Output adds which button layout each slot's
  player would use.
- **`H2_PADS=log`**. Play as usual. Print:
  - `controllers: XInput (gilrs 0.11.2)` at start, then each pad on connect and disconnect;
  - claims (`pad 1 claimed by this window`, `pad 1 belongs to another window: ignored`);
  - focus changes;
  - each owned press as `pad 0 (player 1, BUMPER JUMPER): LB → JUMP`.
- `const BACKEND: &str = if cfg!(windows) { "XInput" } else if cfg!(target_os = "linux") { "evdev" } else { "none" };`

On John's PC: `$env:H2_PADS="20"; .\h2viewer.exe; Remove-Item Env:H2_PADS`, then `$env:H2_PADS="log"`
for a game. Document both in the README's testing list next to `H2_LIST_WEAPONS`.

---

## 8. Order of work and coordination

1. **Backend PR (small, ship first).** Covers §1.4: the Cargo change, `GilrsBuilder`, the 0.2 dead zone
   and `H2_PADS`. John confirms with padprobe or `H2_PADS`. This touches only `Cargo.toml`, `Cargo.lock`
   and `input.rs`, so it doesn't conflict with worktree 7.
2. **Claims and focus (§5.8).** Needed before two windows on one PC work with XInput. This touches
   `input.rs` and `main.rs` / `flow.rs` seat changes.
3. **Layout table and the event / state refactor** (§5.1-5.4). This needs worktree 7 merged first,
   because both edit `local.rs::command` / `turn_with_stick` and `main.rs::update`.
4. **Profile, menus and per-guest settings** (§5.5-5.7). Builds on worktree 7's `Controls`, `Screen::Controls`
   and profile keys, including the two renames (CONTROLS → CONTROLLER, INVERT LOOK → LOOK INVERSION).
5. **README.**
   - The controls paragraph: the layouts and their names, plus the thumbstick layouts.
   - Prompts follow the layout.
   - XInput: the background-input note, non-Xbox pads through Steam Input, and the claims.
6. **Later.** Controller Vibration (XInput rumble via gilrs `ff`; Halo 2's damage effects carry vibration
   data) and Auto Look Centering.

Note: my scratch builds used `CARGO_TARGET_DIR=/home/claude/target-feel`. The **release build overwrote
`/home/claude/target-feel/x86_64-pc-windows-gnu/release/h2viewer.exe` with an XInput build of `464878e`**
(scratch clone `scratchpad/h2copy`). Rebuild from the intended branch before shipping anything from
that folder.

---

## 9. Sources

**Game data** (no files copied into the repo; scratch outputs in `scratchpad/ctl/`):
- `mainmenu.map`:
  - `unic` `ui\screens\game_shell\settings_screen\player_profile\{button_settings, thumbstick_settings,
    controller_settings, look_sensitivity, invert_look, aim_assist}`, `...\edit_profile_menu`;
  - `wgit` `...\button_settings` (panes 0-7) and `...\thumbstick_settings` (panes 0-7);
  - `bitm` `...\button_config`, `...\thumbsticks`.
- `lockout.map` (through `shared.map`): `ui\screens\multiplayer\mp_menu\{mp_menu, controller_settings\strings,
  player_settings\strings}` and `ui\hud\hud_messages`. Halo 2's own prompts use per-action button glyphs
  (`\u{e45a}` action, `\u{e46a}` dual wield).

**gilrs source:**
- `gilrs-0.11.2/{Cargo.toml, src/gamepad.rs, src/ev/filter.rs, src/mapping/mod.rs}`;
- `gilrs-core-0.6.8/{Cargo.toml, src/platform/mod.rs, src/platform/windows_xinput/*, src/platform/windows_wgi/*}`.

**Web:**
- gilrs README: https://docs.rs/crate/gilrs/latest
- DirectXTK GamePad wiki (XInput "global" focus): https://github.com/Microsoft/DirectXTK/wiki/GamePad
- Halopedia, Halo 2 control schemes: https://www.halopedia.org/H2:Control_schemes
- Halopedia, Halo 3 control schemes: https://www.halopedia.org/H3:Control_schemes
- Halopedia, MCC control schemes (Halo 2 Recon table): https://www.halopedia.org/MCC:Control_schemes
- Halopedia, Halo: CE control schemes (Jumpy): https://www.halopedia.org/CE:Control_schemes
- Halo fandom, Halo Controls (Halo 2 Xbox and 360 lists; Halo 3 and Reach Bumper Jumper; Legacy): https://halo.fandom.com/wiki/Halo_Controls
- Haloology, "The Art of Jumping and Melee" (Bumper Jumper's origin in Halo 3): https://haloology.wordpress.com/2014/02/14/the-art-of-jumping-and-melee/
- Halo Bulletin 16/10/2014, MCC universal schemes: https://wiki.halo.fr/Halo_Bulletin_16/10/2014
- Bungie.net forum, MCC universal controller settings: https://www.bungie.net/my/Forums/Post/73425735
- Steam, "Halo 2 Bumper Jumper with reload and swap on B and Y?": https://steamcommunity.com/app/976730/discussions/0/2574320091912291101
