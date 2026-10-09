# Movement and combat: verified decomp findings (Xbox retail decomp vs halo2-rs)

Verifier's pass over the first reader's 13 "movement-combat" findings. For each one I re-read the decompiled code, checked whether each function is matched or only a draft (config/functions.csv), re-read the remake code (main fcec0f4; combat branch wf_a876195b-c46-8 at eab5456; weapons branch wf_a876195b-c46-6 at 859e24e), and re-read the tag values from the H2V maps myself. My tag reader is at scratchpad/decomp/verify_mc/ (vm.py, weap.py, vehi.py, bip.py, fpj.py, fpk.py, mag.py, wj.py). The first reader's tools/h2map.py has since been overwritten by another agent and no longer runs dump3.py.

Re-checked on resume (2026-10-09 18:30 UTC): main is still fcec0f4 and the combat and weapons branches are unchanged, so every remake line reference below still holds. Match status re-read from config/functions.csv; e8420, e6900, f6bd0, 105ba0, 1665c2, 100880, 1dae20, 1dae50 and baf40 are matched, everything else cited is todo.

Status legend: "todo" = a draft rebuilt from disassembly that does not match the retail bytes yet; "matched" = byte-identical. Unless a finding says otherwise, every decomp function cited here is a todo draft. Confidence is medium at best unless tag data or a matched function backs the claim.

String ids were resolved from the H2V string table (Xbox index N is H2V index N-2 above about 0x530): melee_strike_1..4, melee_1sthit/2ndhit/3rdhit, melee_dash(_airborne), melee_lunge(_airborne), reload_empty, reload_full, ready, put_away, throw_grenade, throw_overheated, left_hand, exit.

Findings are ranked by how much a player would notice.

---

## 1. Grenade throw: exactly along the aim, from 5 cm left of the eye, at the biped's grenade speed, released on the throw animation's keyframe, without the thrower's running velocity (impact: high)

- **Decomp:** src/unknown_0a76b0.cpp:
  - function_e7fb0 (0xe7fb0, l.971, todo) starts the throw.
  - unit_action_throw_grenade_update (0xe8380, l.1083, todo) runs it each tick.
  - function_e7730 (0xe7730, l.679, todo) puts the grenade in the hand.
  - function_e7900 (0xe7900, l.765, todo) releases it.
  - unit_action_throw_grenade_interrupted (0xe8420, l.1112, **matched**) handles an interrupted throw: it calls e7900 with spread=true, but for a player e7900 clears spread (only the AI path keeps the 0.6-1.4 random lob), so a player's interrupted throw still goes out at full speed along the aim.
  - The platform velocity comes from src/unknown_109e00.cpp function_109e00 (todo).
- **Halo 2:**
  - The unit plays throw_grenade (or throw_overheated) with a 0.1335 blend.
  - The grenade is created at the `left_hand` marker on tick 2, and the grenade count drops then.
  - Release happens when the tick counter reaches the throw animation's event time. The Spartan's third-person throw_grenade:var0 has its primary keyframe at frame 8 of 38, so release is on tick 8.
  - **Origin:** eye (cafc0) + aim x matg player information +0x60 (0.0) + left x +0x64 (0.05) + up x +0x68 (0.0). "left" is up x aim.
  - **Velocity:** aim x biped +0x1b0 (10.0 for masterchief_mp and elite_mp), plus the velocity of the platform the thrower stands on. The platform velocity is added only when the platform's definition flag allows it (109e00 with the flag required). The velocity is set absolutely: the code subtracts the projectile's current velocity before applying the impulse.
  - There is no upward bias, and none of the thrower's own running velocity is added.
  - The view unzooms, unless the thrower is seated.
  - If the throw is interrupted after tick 2, the grenade is still released along the aim (e8420 calls e7900).
  - e7900 deletes the grenade if a world test of the thrower's eye point fails (bc1d0).
- **Remake now:**
  - main crates/h2sim/src/game.rs:1630-1660 throws instantly.
    - Direction: dir = (f + u*0.15).normalize() at :1648.
    - Position: eye + f*0.25 - 0.05z.
    - Velocity: dir*7.0 + body.velocity*0.5 at :1653. The 7.0 is rules.frag/plasma.speed (game.rs:455/463).
    - The count drops at once, with a 0.6 s cooldown (:225).
  - The combat branch (game.rs:1802-1843) gets some of this right: release at 8/30 s, busy for 37/30 s, speed 10 read from +0x1b0. It still uses f + u*0.15 (:1832), eye + f*0.25 - 0.05z, and + 0.5 x body velocity (:1837), and it drops the count at release.
- **Change:**
  - Throw along the aim with no bias.
  - Spawn at eye + left*0.05, reading matg player information +0x60/+0x64/+0x68 rather than constants.
  - Set velocity = aim * biped +0x1b0 + the velocity of the ground or platform under the player. Do not add any of the player's own running velocity.
  - Take the release tick from the third-person throw_grenade primary keyframe (8 ticks). Drop the count on tick 2.
  - Release a throw that is interrupted after tick 2.
  - The grenade starts inside the thrower's capsule, so its collision must ignore the owner.
- **Vista:** same tags and engine.
- **Evidence:**
  - Code: drafts read line by line. 109e00 returns only the platform's linear and angular velocity.
  - Tags from lockout.map, read with my reader: matg player info +0x60/+0x64/+0x68 = 0 / 0.05 / 0; masterchief_mp and elite_mp +0x1b0 = 10.0; masterchief combat:rifle:throw_grenade:var0 is 38 frames with a primary keyframe at 8.
  - **Confidence:** medium. All the functions are drafts, but the formula is explicit and the 8-tick keyframe agrees with the combat branch's own choice.

## 2. Reload length is the first-person reload animation, not magazine +0x10 or a 2 s default (impact: high)

- **Decomp:**
  - src/weapons.cpp function_105a80 (0x105a80, l.1254, todo). The retail call list in config/functions.csv names 105ba0, 1665c2 and 1685a6.
  - function_105ba0 (l.807, **matched**) picks FP state 7 when the magazine is empty, else 8.
  - src/unknown_165ce5.cpp first_person_weapon_state_animation (0x1665c2, l.504, **matched**) maps 7 to reload_empty and 8 to reload_full.
  - first_person_weapon_animation_ticks (0x1685a6, l.695, todo): type 0 is the frame count; type 3 is the primary keyframe, or the frame count when there is none.
  - The magazine update in weapons.cpp (l.~2508-2535) loads the rounds (102b90) when ticks_0c reaches 0, at l.2527.
  - src/unknown_100880.cpp function_100880 (**matched**) keeps the unit's reload action alive while a magazine state is 1..6.
  - Magazine +0x10 is not read anywhere in the decompiled weapon code.
- **Halo 2:** a reload lasts the frame count of the first-person reload_empty (empty magazine) or reload_full animation, at 30 frames per second. The rounds go in at that animation's primary keyframe if it has one, otherwise at the end. Spartan FP values from the H2V lockout.map:

  | Weapon | Frames | Seconds | Rounds go in |
  | --- | --- | --- | --- |
  | Magnum | 47 | 1.57 | end |
  | Battle rifle | 58 | 1.93 | end |
  | SMG | 50 | 1.67 | end |
  | Sniper | 72 | 2.40 | end |
  | Rocket launcher | 112 (Elite 116) | 3.73 | end |
  | Flak | 90 | 3.0 | end |
  | Brute shot | 95 | 3.17 | end |
  | Needler | 64 | 2.13 | frame 44 (1.47 s) |
  | Carbine | 69 | 2.30 | frame 62 |

  The rocket and flak graphs list only reload_full.

  Putting a weapon away also takes time: the FP put_away up to its allow-interruption event, or its end. That is 4 frames (0.13 s) for most weapons, 16 for the sniper and 9 for the sword, and the ready animation follows it.
- **Remake now:**
  - crates/h2sim/src/weapon.rs:299-303 uses magazine +0x10 if it is above 0, otherwise DEFAULT_RELOAD_TIME 2.0 (weapon.rs:9).
  - Tag +0x10 values: magnum 0 (so 2.0), BR 0 (2.0), SMG 2.0, sniper 0 (2.0), rocket 5.0, flak 5.0, brute shot 2.0, needler 1.0, carbine 0 (2.0).
  - Rounds go in only when the timer ends (weapon.rs:553-560).
  - switch_weapon (game.rs:1472-1482) has no put-away phase.
  - The weapons and combat branches are the same.
- **Change:**
  - In h2viewer's weapon_def (main.rs:182-192, which already does this for ready), set reload_time from the FP reload_full frame count. Add a separate empty-magazine time from reload_empty, and a load point at the primary keyframe or else the end.
  - Keep magazine +0x10 only as a fallback when there is no FP animation.
  - Shell-by-shell reloads (the shotgun) are out of scope; the weapons branch handles them.
  - Optionally add the FP put_away time before ready on a switch.
- **Evidence:**
  - Frame counts and events read from the masterchief and dervish fp_* jmads in lockout.map (fpk.py); magazine values read with mag.py.
  - **Confidence:** medium-high. The state mapping is matched, the call graph comes from retail, and the numbers come from the tags.

## 3. A reload stopped more than half way to its load point still loads the rounds (impact: high)

- **Decomp:**
  - src/weapons.cpp function_1015a0 (0x1015a0, l.1932, todo).
  - function_102b90 (l.1904, todo) loads the rounds.
  - function_104080 (l.1458, todo) resets every magazine to state 0.
  - src/unknown_0a76b0.cpp function_e73c0 (l.560) is the reload's interrupted handler, and function_e8460 (l.1121) is the put-away.
  - These stop requests 0/0xa (the reload): weapon switch e8980 (l.1308), drop e8de0 (l.1422), melee e9ed0 (l.2131) and throw e7fb0 (l.971).
- **Halo 2:**
  - When a reload in magazine state 1 is stopped by a switch, a drop, a melee or a throw:
    - If `ticks_0c * 2 < ticks_0e` (less than half the ticks to the load point remain), the rounds go in: min(rounds reloaded, reserve), capped at the magazine size.
    - Otherwise the reload is lost.
  - For most MP guns the load point is the end of the reload, so the threshold is half the reload: BR 29 frames (0.97 s), magnum 0.78 s, SMG 0.83 s, sniper 1.2 s, rocket 1.87 s. For the needler it is 22 frames (0.73 s), and for the carbine 31 frames.
  - Shell-by-shell reloads (states 2/3) are not covered: shells already loaded simply stay.
- **Remake now:**
  - crates/h2sim/src/weapon.rs:528-531: put_away() sets reloading = None, so a reload is always lost. The weapons branch (weapon.rs:533) and combat branch (weapon.rs:534) do the same.
  - Melee and throws do not touch the reload at all (game.rs:1375-1381).
- **Change:**
  - Add an "interrupt reload" step for put-away, switch, drop, melee and throw: if more than half of the time to the load point has passed, move the rounds into the magazine now; otherwise cancel.
  - Have melee and throws call it (see 7).
- **Evidence:**
  - Drafts read; the 104080 reset and the matched 100880 confirm what "lost" means.
  - The finding's "half of the reload" is exact only for weapons whose reload animation has no primary keyframe. I corrected it to "half way to the load point".
  - **Confidence:** medium.

## 4. Melee hit test: 25 rays from the eye over the weapon's damage pyramid, blocked by walls (impact: high)

- **Decomp:** src/unit_object_type.cpp function_cf3d0 (0xcf3d0, l.7128, todo); the eye position comes from src/unknown_0cafc0.cpp function_cafc0 (todo).
- **Halo 2:**
  - From the eye, 25 rays are cast: aim x depth + i x across x (+0x1b0 / 2) + j x up x (+0x1b4 / 2), for i, j in -2..2. Each ray stops at the first thing it hits.
  - The nearest biped hit wins; failing that, the first object hit (vehicles too). The level only matters for impact effects and breakable surfaces.
  - The weapon tag fields are "damage pyramid angles" (yaw, pitch) at +0x1b0/+0x1b4 and "damage pyramid depth" at +0x1b8.

  | Weapons | Yaw | Pitch | Depth |
  | --- | --- | --- | --- |
  | Every hand-held MP gun | 0.2184 (12.5°) | 0.1058 (6.1°) | 0.6 |
  | Energy sword | 0.4197 (24°) | 0.2412 (13.8°) | 0.9 |
  | Useless sword | 0.3497 | 0.201 | 0.75 |
  | Vehicle guns | 0.2 | 0.2 | 0.8 |

  - Caveat: the draft adds the angle values directly as offsets at the depth, giving ±0.218 wu at 0.6, about ±20°. If retail also scales by the depth, the fan is about ±12.5°. A draft cannot settle this.
- **Remake now:**
  - crates/h2sim/src/game.rs:215-219: MELEE_RANGE 0.9 to the target's centre at 60% of its height, and MELEE_CONE 0.6 rad (34°), for every weapon.
  - melee() at game.rs:1585 takes no World, so melee hits through walls, and vehicles can never be meleed.
  - The combat branch's strike() (game.rs:1745) is the same.
- **Change:**
  - Read +0x1b0/+0x1b4/+0x1b8 into the weapon definition in blam-cache weapon.rs.
  - Cast the 5x5 grid from the eye against the world (each ray stops at a wall), player capsules and vehicles. Prefer the nearest player.
  - To pick between ±20° and ±12.5°, compare in-game reach, or wait for cf3d0 to match.
- **Evidence:**
  - The cf3d0 loop and the hit preference were read directly.
  - Tag values from lockout.map (weap.py).
  - **Confidence:** medium for the method (walls blocking, sizes from tags); low for the exact angular spread.

## 5. Melee timing comes from the first-person melee_strike animation; firing stays locked for the whole swing (impact: medium-high)

- **Decomp:**
  - src/unknown_0a76b0.cpp:
    - function_e99a0 (l.1877, todo) picks a random FP melee_strike_1..4 that the weapon has.
    - function_e9a20 (l.1917, todo) sets the damage tick from the primary keyframe (1dae20, **matched**), the end tick from the secondary keyframe (1dae50, **matched**), and the length from the frame count. It uses the FP animation, falling back to the third-person channel.
    - unit_action_melee_attack (e9ed0, l.2131) refuses a new melee until counter >= end tick. With no secondary keyframe, that means waiting for the whole animation.
    - unit_action_melee_attack_update (ea090, l.2183) applies damage when counter == damage tick. The action stays active until the frame count runs out.
  - src/unit_object_type.cpp function_c58f0 (l.6095; check at l.6272) holds the weapon (no fire) while action 26/27 is active.
  - A reload (e7110, l.460) or weapon switch (e8980) stops the melee action.
- **Halo 2:**
  - Damage lands at the FP primary keyframe, for example tick 5 (0.17 s) for the BR.
  - The next melee may start at the FP secondary keyframe, or at the end of the animation if there is none.
  - The gun cannot fire until the animation ends, unless reload or switch cuts it short. That cut is the B-X-R / B-Y-Y trick.
  - Spartan FP melee keyframes, as hit / end / total frames ("-" means no secondary keyframe, only an allow-interruption event):

    | Weapon | Hit / end / total |
    | --- | --- |
    | Magnum | 5 / 17 / 29 |
    | Needler | 5 / 16 / 29 |
    | Plasma pistol | strike_1 5/15/29; strike_2 8/15/34 |
    | BR | strike_1 5/20/30; strike_2 5/-/30 |
    | Carbine | strike_1 5/-/31; strike_2 5/-/27 |
    | Plasma rifle | 6 / 16 / 34 |
    | Shotgun | strike_1 3/13/32; strike_2 6/20/31 |
    | SMG | 6 / 25 / 30 |
    | Sniper | 4 / 20 / 35 |
    | Rocket and flak | 5 / - / 35 |
    | Brute shot | 3 / - / 28 (three variants) |
    | Sword | 8 / 14 / 24 |
    | Beam rifle | 3 / 11 / 24 |
    | Flag | 6-7 / 15 / 22 |

  - Third-person keyframes differ for some weapons: SMG 3/16, sword 3/14, and the needler and brute shot use allow-interruption rather than a secondary keyframe.
- **Remake now:**
  - main uses instant damage and MELEE_COOLDOWN 0.8 (game.rs:220, :1375, :1605), and firing is allowed during a melee.
  - The combat branch times melee per weapon from the third-person combat:<style>:melee_strike_1 (h2viewer main.rs:248-262; game.rs:1673-1688). It keeps the player "busy", for both firing and melee, until min(secondary keyframe, allow-interruption).
- **Change:**
  - Time each swing from the weapon's FP graph (Spartan or Elite), choosing a random available melee_strike_n.
  - Keep "next melee allowed" (secondary keyframe, else end) separate from "can fire" (end of the animation, unless reload or switch cuts it short).
  - Use third-person keyframes only as a fallback.
- **Evidence:**
  - Keyframes read with fpk.py from both the masterchief and dervish fp graphs; the third-person graph was read too.
  - **Confidence:** medium.

## 6. Melee while dual wielding drops the left-hand gun, then swings (impact: medium-high)

- **Decomp:**
  - src/unknown_0a76b0.cpp unit_action_melee_attack (0xe9ed0, l.2130-2175, todo): after its checks it stops requests 8, 0x12, 0 and 0xa. It then performs request 0x13 through function_e6900 (src/unknown_0e6900.cpp, **matched**; g_4677c8[19] = unit_action_drop_weapon), unzooms (c86e0), and starts the swing (e9a20).
  - unit_action_drop_weapon (0xe8de0, l.1422, todo): type 0x13 means hand 2 with drop mode 3.
  - unit_object_type.cpp function_ce520 (l.2944) independently uses 0x13 for "drop the second hand's weapon".
- **Halo 2:** a melee press while holding two guns throws the left gun down as a pickup. The player is single-wielding from then on and does the normal melee of the right-hand gun. Throws also issue request 0x13 (e7fb0); see the Vista note.
- **Remake now:**
  - main crates/h2sim/src/game.rs:1372-1376 melees and keeps both guns.
  - The combat branch (game.rs:1446-1451) refuses melee entirely while dual wielding.
- **Change:** on a melee press while dual wielding, drop the left gun as a pickup (the same path as a swap drop), return to single-wield, then run the normal melee. Do not block it.
- **Vista:** same engine. On a keyboard, melee is its own key, so this path is reachable there too. If the remake ever lets a separate grenade key throw while dual wielding, the same drop applies (e7fb0 issues 0x13). Whether H2V's input layer sends that throw lives in player_control, which is not decompiled.
- **Evidence:**
  - The type 0x13 dispatch goes through the matched dispatcher and table.
  - My recollection of Halo 2 (melee while dual wielding costs you the left gun) agrees.
  - **Confidence:** medium.

## 7. Throws and melees stop reloads and switches, unzoom, and lock the trigger (impact: medium)

- **Decomp:**
  - e7fb0 (l.971): stops 0x16, 8, 0x12, 0, 0xa and 0x1b, issues 0x13, and unzooms when not seated.
  - e9ed0 (l.2131): stops 8, 0x12, 0 and 0xa, and unzooms. It refuses a melee while a throw (0x16) is active, while triggers[0].state (weapon +0x20c) is 1 or 2 (charging or charged), or when weapon +0x12c bit 9 is set.
  - weapons.cpp function_101440 (l.543, todo) refuses a throw for "prevents grenade throwing" (+0x12c bit 6).
  - unit_object_type.cpp c58f0 (l.6272) holds the weapon (no fire) while bit 22 (throw), 26/27 (melee) or that hand's switch is set.
- **Halo 2:**
  - A throw or a melee cancels any reload (the rule in 3 applies) and any switch, unzooms, and blocks firing until it ends.
  - A throw also cancels a melee, and a melee is refused during a throw.
  - Melee is refused while the plasma pistol is charging or charged.
- **Remake now:** main does none of this (game.rs:1375-1381, 1585-1660). The combat branch has a "busy" lock that blocks firing, but no unzoom and no reload or switch cancel.
- **Change:**
  - On a throw or melee start: zoom = 0, interrupt the reload with the rule in 3, cancel any pending switch, and block firing until the action ends.
  - Refuse melee during a throw.
  - When overcharge is added (neither main nor any branch has it), refuse melee while charging.
- **Tags:** +0x12c bit 6 is set only on the flag, bomb and ball, which main already blocks through objective.is_some() (game.rs:1633). Bit 9 is set on no MP weapon, so neither bit needs new code.
- **Evidence:**
  - Drafts read. The +0x20c offset matches the s_weapon layout: barrels at 0x1a4, 2 x 0x34, so triggers start at 0x20c.
  - **Confidence:** medium.

## 8. Flipped vehicles: per-seat turnover time, counted only while resting upside down; Banshee pilots never auto-exit (impact: medium)

- **Decomp:**
  - src/vehicles.cpp function_f6bd0 (0xf6bd0, l.4400, **matched**). It is called for every vehicle type from vehicles.cpp l.1056.
  - src/bipeds.cpp function_dd360 (0xdd360, l.581, todo) issues request 0x1e (unit_action_vehicle_exit_immediate).
  - Turret gunners read the counter of the top vehicle through function_baf40 (**matched**).
- **Halo 2:**
  - Vehicle +0x34b counts ticks while the vehicle is touching something (a havok contact or a contact point) and up.k < 0. For type 5, the Banshee, the threshold is up.k < -0.075.
  - The counter resets when that stops, and caps at 255 ticks.
  - A rider exits immediately once the count exceeds round(30 x seat +0x20 "turnover time"), or round(30 x 0.55) if the field is 0.
  - This check does not run on a game_engine mode-4 client.
  - Coagulation seat values:

    | Seat | Turnover time (s) |
    | --- | --- |
    | Warthog driver and passenger | 0.65 |
    | Chaingun gunner | 1.0 |
    | Gauss gunner | 0.65 |
    | Warthog boarding seats | 0.2 |
    | Ghost | 0.5 |
    | Spectre driver and passengers | 0.65 |
    | Scorpion driver | 1.0 |
    | Scorpion riders | 0.1 |
    | Wraith driver | 1.0 |
    | Banshee pilot | 10.0 |
    | c_turret_ap | 10.0 |
    | h_turret_ap | 16.0 |

  - The Banshee pilot (10.0), c_turret_ap (10.0) and h_turret_ap (16.0) exceed the 8.5 s cap, so they never auto-exit.
  - A vehicle lying on its side (0 <= up.z) or tumbling in the air never ejects anyone.
- **Remake now:**
  - crates/h2sim/src/game/vehicles.rs:18: BAIL_OUT_AFTER = 1.5 s for every seat.
  - vehicles.rs:811-820 counts whenever upside_down() (vehicle.rs:475: up.z < 0.3, which includes lying on its side), whether touching anything or not (tricks excepted).
- **Change:**
  - Read seat +0x20 in blam-cache vehicle.rs (seat block at unit +0x1C8, 0xB0 each).
  - Count per vehicle only while up.z < 0 (or < -0.075 for vehicle type 5, at +0x1F0) and in contact; reset otherwise; cap at 255 ticks.
  - Eject each rider when the count exceeds that seat's time.
- **Evidence:**
  - The counter function is matched.
  - Seat and type values were read from coagulation.map (vehi.py): warthog type 1, ghost 4, Banshee 5, scorpion 0, turrets 6.
  - **Confidence:** high.

## 9. Sword lunge is a slide over several ticks that ends in melee_lunge, not a 60% teleport (impact: medium). Narrowed from the first reader's finding.

- **Decomp:**
  - e9a20 mode 2 (l.~1985-2000 and 2075-2090): plays melee_dash, or melee_dash_airborne when airborne (e4050).
    - Target: the aim-assist target (unit +0x1c8) if its lock (+0x1e4) is >= 1.0 and it is a biped or vehicle; otherwise the request's target.
    - Calls bipeds.cpp function_dee60 (l.513, todo), which switches to biped physics mode 6. Up to 60 ticks.
  - ea090 (l.2183) ends the dash when mode 6 ends, or when biped +0x3ec drops to round(30 x 0.15) = 4 ticks (ground) or round(30 x 0.22) = 7 ticks (air). It then plays melee_lunge or melee_lunge_airborne through e9a20 mode 0.
  - cf3d0 takes melee_lunge's damage from weapon +0x1ec.
- **Halo 2:**
  - The attacker is driven toward the target over up to 2 s. Biped +0x3ec is evidently the predicted ticks to contact (an inference, not shown).
  - The strike comes at the FP melee_lunge primary keyframe: frame 3 on the ground, 5 in the air (sword FP graph).
  - Damage is energy_blade +0x1ec = dash_melee (150/150) plus dash_melee_response.
  - Only the energy blade has a +0x1ec block in MP, so the dash is effectively sword-only, as in the remake.
  - The mode-6 speed (character physics) and the decision to lunge and its range (player_control) are not decompiled. The sword's tag aim assist is autoaim 6.0 and magnetism 8.0 wu, but its link to the lunge range is unproven.
  - Not yet actionable: a normal melee with a target also starts mode 6 toward it (dee60 with flag false, e9a20 l.~2060), after a line-of-sight check (de9d0, l.1367). That suggests every weapon gets a short pull toward its target, but the target comes from player_control.
- **Remake now:** crates/h2sim/src/game.rs:1585-1616 lunges with LUNGE_RANGE 2.2 and teleports the attacker 60% of the way in one step (:1610). Damage is rules.lunge_damage 150, which equals dash_melee. The combat branch strike() (game.rs:1745-1771) is the same.
- **Change:**
  - Replace the teleport with a fast push toward the target, run over ticks (cap 2 s).
  - When about 4 ticks (ground) or 7 ticks (air) from contact, play the lunge and strike 3 or 5 frames later.
  - Read the lunge damage from +0x1ec.
  - Keep the range and speed as estimates.
- **Evidence:**
  - Drafts read.
  - Tag data: lunge blocks (weap.py) and sword FP keyframes (fpk.py).
  - **Confidence:** medium for the slide and the strike timing; low for the speed and range.

## 10. Crouching takes 0.2 s, and speeds and acceleration blend with the crouch amount (impact: medium-low)

- **Decomp:**
  - src/bipeds.cpp function_dd7d0 (l.204, todo): crouch changes by tick x biped +0x250 per tick.
  - src/unknown_1e67a0.cpp function_1e6120 (l.83, todo): forward, back and side speeds, and acceleration, are a linear lerp from matg player information run to crouched values by the crouch fraction.
- **Halo 2:**
  - masterchief_mp and elite_mp have +0x220 = 0.2 s and +0x250 = 5.0/s.
  - Speeds go from 2.25 / 2.0 / 2.0 with accel 9.6 (+0x2c..+0x38) to 0.9 / 0.65 / 0.6 with accel 4.8 (+0x3c..+0x48), in proportion to the crouch fraction.
- **Remake now:**
  - main crates/h2sim/src/player.rs:127 eases crouch at dt*10 (0.1 s).
  - player.rs:138 switches the whole speed set once crouch > 0.5.
  - The combat branch already reads crouch_time (player.rs:251) but keeps the 0.5 switch (player.rs:146).
- **Change:** take the combat branch's crouch_time, and lerp speeds and acceleration by crouch.
- **Evidence:**
  - Tag values read with bip.py from lockout.map.
  - **Confidence:** medium.

## 11. Coyote time: for 5 ticks after walking off an edge you can still jump (impact: medium-low)

- **Decomp:**
  - src/unknown_0e4050.cpp function_e4050 (todo): "airborne" means not on the ground (flags_348 bit 0, set from the physics move result at bipeds.cpp l.966-973) for at least round(30 x 0.18) = 5 ticks, while physics mode is 1 or 3 and there is no parent.
  - bipeds.cpp function_e4680 (l.3609, todo) and function_e4770 (l.3648, todo) allow a jump when e4050 is false. They also require more than 0.16 s since the last jump (biped +0x39c, reset on each jump), and a landing state (+0x34c) other than 1.
- **Halo 2:** for about 0.17 s after leaving an edge you can still jump. The 0.16 s minimum since the last jump is what stops a second jump inside that window.
- **Remake now:** crates/h2sim/src/player.rs:172 jumps only when grounded is exactly true.
- **Change:** track the time since the player was last grounded. Allow a jump while it is under 5/30 s and more than 0.16 s have passed since the last jump.
- **Evidence:**
  - Drafts read. The usages agree with each other: e9a20 uses e4050 to pick melee_dash_airborne, and it uses the same +0x399 counter for airborne ticks.
  - **Confidence:** medium.

## 12. Vehicle entry also checks facing, the entry marker's cone and relative speed (impact: low)

- **Decomp:**
  - src/unit_object_type.cpp function_c9040 (l.3459, todo).
  - It is called through function_c6fb0 (l.1598) for each of the seat's entry markers (seat +0xc), from function_c8bb0 (l.4639), the seat chooser, which also covers players.
- **Halo 2:** a seat can be entered only if all of these hold:
  - the distance from the eye to the entry marker is < seat +0x98 "entry radius";
  - the angle between the player's aim and the direction to the marker is <= +0xa0 "entry marker facing angle";
  - π minus the angle between that direction and the marker's forward is <= +0x9c "entry marker cone angle";
  - the speed relative to the vehicle is <= +0xa4 "maximum relative velocity".

  Coagulation values (radius / cone / facing / max relative speed):

  | Seat | Radius | Cone | Facing | Max rel. speed |
  | --- | --- | --- | --- | --- |
  | Warthog driver | 1.25 | 1.222 | 1.222 | 3.0 |
  | Warthog passenger | 1.25 | 1.222 | 1.222 | 3.5 |
  | Warthog boarding seats | 1.5 | 1.222 | 0.87-1.05 | 3.5 |
  | Ghost | 1.25 | 1.92 | 1.047 | 3.0 |
  | Banshee | 1.25 | 1.396 | 1.047 | 3.0 |
  | Scorpion driver | 1.75 | 1.396 | 1.222 | 3.0 |
  | Wraith driver | 2.0 | 0.873 | 1.047 | 3.0 |

- **Remake now:** crates/h2sim/src/game/vehicles.rs:146-210 (vehicle_action) tests only distance against entry_radius (+0x98) at :190. You can get in facing away, or into a vehicle speeding past.
- **Change:** in vehicle_action, add the facing test (aim vs. direction to the entry point, <= +0xa0) and the relative-speed test (<= +0xa4). The cone test needs the entry marker's orientation (seat +0xc marker), so add it once the marker is read.
- **Evidence:**
  - Drafts read; the field names match H2's seat block.
  - Values from vehi.py.
  - **Confidence:** medium.

## 13. A jump tops up speed relative to the ground or platform; it never replaces it (impact: low)

- **Decomp:** src/bipeds.cpp function_e4770 (l.3648, todo).
- **Halo 2:**
  - Jump speed = biped +0x1f8 (3.08) x (1 - matg player information +0x7c (0.8) x stun). Stun is inert in MP.
  - The engine adds only what is needed to bring velocity·up, relative to the velocity of the ground or platform under the player (109e00), up to the jump speed. A faster existing upward speed is kept.
- **Remake now:** crates/h2sim/src/player.rs:172-173 sets velocity.z = jump_velocity absolutely.
- **Change:** velocity.z = max(velocity.z, ground_velocity.z + jump_velocity).
- **Evidence:**
  - Draft read.
  - **Confidence:** medium. The effect shows only on moving platforms.

---

## Rejected

- **Melee damage scales with movement (finding 7).** The reading of the draft is accurate:
  - e9a20 sets the scale to 0 (standing), 0x7f/255 (forward speed over 0.9 x run_forward) or 255 (more than 15 ticks airborne).
  - cf3d0 passes it on, cfc90 stores it in damage +0x54, and damage.cpp d9b60 computes (1-s) x lower + s x upper.
  - d6660 (l.506) defaults the scale to 1.0, so ordinary damage uses the upper bound.

  That would make strike_melee 20 standing, about 40 running and 60 in the air. But all of it is unmatched draft code, the finding itself says do not change, and it contradicts no remake bug until checked. Test it in H2V first (standing vs. running melee on a full-shield target).
- **2nd and 3rd melee hit damage blocks (smash_melee on the 3rd hit), part of finding 6: refuted for MP.**
  - Every MP weapon's FP graph has melee_strike_1 (some also have _2 and _3), for both the Spartan and the Elite.
  - e9a20 then always picks a random melee_strike_n.
  - The 1sthit, 2ndhit, 3rdhit chain runs only when the current animation is melee_1sthit.
  - cf3d0 maps every strike_n to block +0x1bc.

  So the 2nd and 3rd blocks are never used. The remake's single block (blam-cache WEAP_MELEE_DAMAGE 0x1BC) is right.
- **"Every weapon can lunge and falls back to the normal hit", part of finding 4: refuted.**
  - cf3d0 falls back from +0x1ec to weapon +0x190, then to biped +0x168, not to +0x1bc.
  - All of these are NONE for every MP weapon and for masterchief_mp and elite_mp, so a non-sword dash would deal no damage.
  - The dash is sword-only. A possible short pull on a normal melee is noted under 9.
- **"Remake ready time is 0.5 s for every weapon" and "weapon state 5/6/9/10 animations set the timing", part of finding 11.**
  - h2viewer main.rs:182-192 already sets ready_time from each weapon's FP `ready` length.
  - The 1058b0 state map drives the third-person weapon model's own animations, which most MP weapon graphs do not have (wj.py).
  - The FP reload animations are reload_empty and reload_full, not reload_1.
  - Weapon +0x13c is read by blam-cache (weapon.rs:29/398) but used nowhere, so it is harmless.
  - The reload part survives as finding 2.
- **"Grounded acceleration and animation persist during coyote time", part of finding 10.** Not supported. dc5c0 (l.1093) feeds 1e6120's "airborne" argument from the animation state == user_animation, not from e4050. Only the jump part survives.
- **"Exit carries the exit animation's velocity instead of a teleport", part of finding 13.** eac90 (l.2615) does add the exit animation's root velocity at the end of the exit animation. But the code read does not show how far the animation carries the rider, or what the rider's velocity is when detached. That is not enough to specify a replacement for the free-spot search, so I am not changing it. Note it for later.
- **0.3 s jump-press rule (e4680).** As drafted, a fresh press would need 9 ticks of holding before a jump, which contradicts Halo 2's instant jump. The draft is likely wrong. No change; the first reader agreed.
