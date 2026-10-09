# Damage, shields and health: verified findings (decomp vs halo2-rs)

Reviewer pass over the "damage" reader's findings. Each one was re-read in the decompilation
(/home/claude/kirklandsig/halo2-decompiled, Xbox retail) and in the remake (main at fcec0f4; combat
branch worktree-wf_a876195b-c46-8 now at eab5456, not 63f0276 as the reader cited, so its line numbers
below are for eab5456). Tag values were re-read from the Vista coagulation.map with the scratch h2tool.

Matching status, checked in config/functions.csv: only d5bc0, fbfd0, fd8b0, 1df560, 15e020 and 1e9700
are "matched". Every other function cited (d9b60, d74e0, d6f90, d9110, d8cb0, d7b80, d6800, db670,
db210, dc230, d5de0, d6af0, d9640, f8200, f8eb0, faa60, fc330, fd560, e9a20, cf3d0, cfc90, cc010,
1588b0, 15dec0, e3700, de620) is "todo": written but not byte-verified. Read their control flow as
strong evidence, not proof. Vista code is not decompiled, so Vista-only changes cannot be ruled out
anywhere below.

Ranked by how much a player would notice.

---

## 1. Range falloff applies only to projectiles flagged "damage scales based on distance"

- Decomp: projectiles.cpp function_faa60, lines 2240-2251. The damage scale starts at the
  projectile's +0x188, which function_f8200 sets to 1.0 (line 1269). Only when proj flags (+0xbc)
  bit4 is set does it become clamp(1 - (distance travelled + this segment) / range, 0, 1). The range
  is projectile->speed, which function_fbfd0 (matched, lines 233-240) fills from the proj field at
  +0x16c (air damage range upper; +0x178 water damage range upper under water). The scale goes into
  damage +0x54 (line 2300), and damage.cpp function_d9b60 (lines 2315-2316) deals
  lerp(jpt lower +0x1c, jpt upper +0x20, scale). The lower bound of air damage range (+0x168) is never
  read.
- Halo 2: with no bit4 flag, a round does its jpt upper bound at any range. SMG (proj flags 0x0, jpt
  0-4.8): 4.8 everywhere. Plasma pistol bolt (0x400, jpt 4-7): 7. Plasma rifle bolt (0x400, 6-10): 10.
  Only shotgun_bullet has bit4 (flags 0x10, air damage range 5-8, max range 8): its pellets do
  lerp(5, 16.5, 1 - d/8), so 16.5 at the muzzle, 9.3 at 5 wu, and 5 at 8 wu. The drop starts at the
  muzzle; there is no full-damage plateau.
- Remake now: crates/h2sim/src/weapon.rs:669-676 WeaponState::damage_at gives full damage out to
  air_damage_range.0 and then lerps down to the jpt lower bound at air_damage_range.1, for every
  weapon. damage_range comes from weapon.rs:382-385. It is used by hitscan (game.rs:1562 and 1568)
  and by flying rounds (game/projectiles.rs:214). Results: the SMG falls from 4.8 to 0 over 0-40 wu
  (2.4 at 20 wu), the plasma pistol from 7 to 4, the plasma rifle from 10 to 6, and the shotgun keeps
  a full 16.5 out to 5 wu. All three branches have the same code.
- Change: blam-cache already reads the proj flags (crates/blam-cache/src/weapon.rs:556,
  `Projectile.flags`), but nothing uses them. Carry bit4 into WeaponDef. With bit4:
  scale = clamp(1 - travelled / air_damage_range.1, 0, 1) and damage = lower + (upper - lower) * scale.
  Without it: damage = upper.
- Evidence: the bit4 test, the +0x188 = 1.0 initial value, and that +0x170 is only written by fbfd0
  (no other writer in projectiles.cpp) were all re-read. f8eb0 line 1970-1971 also uses the same
  +0x170 as a fallback range for projectiles with no max range, which confirms it is the air damage
  range upper bound.
- Impact high (every SMG and plasma fight). Confidence high (faa60 is "todo"; fbfd0 is matched).

## 2. Health regenerates after 10 s without body damage, on its own timer (main has none)

- Decomp: damage.cpp function_d8cb0 line 1817 (the body stun is set only when body damage is above
  the hlmt body minimum stun damage, 0). function_d9110 lines 1555-1557 (the shield stun is set by
  shield damage, or by body damage while the shield is 0). function_d5de0 lines 3625-3662 (the body
  recharges after its own stun counter reaches 0, at the rate in damage info +0xd4, up to the
  recharge fraction in +0x38).
- Halo 2: masterchief_mp and elite_mp hlmt damage info have body stun time 10 s (+0x30), body recharge
  time 5 s (+0x34) and recharge fraction 1.0 (+0x38); shield stun 5 s (+0x98) and shield recharge 2 s
  (+0x9c). Health comes back to full 10 s after the last body damage. A hit that the shields fully
  absorb does not restart the health timer. Body damage taken with the shields at 0 restarts both.
  Caveat: the per-second rate at +0xd4 is filled by a tag post-process that is not decompiled; "full in
  5 s" assumes it is fraction / recharge time.
- Remake now: main has no health regeneration at all (crates/h2sim/src/game.rs:1313-1318 recharges
  shields only), so lost health never returns before death. The combat branch adds health_delay 10
  and health_recharge 5 (game.rs:346-347, 466-467), but both timers use the shared since_damage
  (game.rs:1375-1383), so a shield-only hit also delays health.
- Change: keep separate shield and body stun timers. Restart the body timer only when body damage
  (after shields) is above 0. Read the times from the hlmt damage info (+0x30, +0x34, +0x38 for the
  body; +0x98, +0x9c for the shield) instead of hard-coding them.
- Evidence: both stun conditions and the separate counters in d5de0 re-read; tag values re-dumped.
- Impact high on main (medium once the combat branch merges). Confidence high.

## 3. A stuck plasma grenade on main does not kill a full-shield Spartan (attached 400 is missing)

- Decomp: projectiles.cpp function_fc330 lines 1680-1705. When a projectile detonates while it has a
  parent object (it is stuck), the parent first takes the proj +0x10c damage (attached detonation
  damage; tagref at +0x108) directly through function_d7b80. Then the area damage (+0x104) is applied
  through function_d6c80 with no ignored object, so the victim takes both. Overflow:
  damage.cpp function_d9110 lines 1523-1532.
- Halo 2: plasma_grenade_attached_explosion is 400, group explosion_attached (x0.5 against
  energy_shield_thin). That is 200 against the 70 shield; the overflow is (200 - 70) / 0.5 = 260 to a
  45 body, so the victim dies before the 50-120 blast (radius 0.75-1.5) even lands.
- Remake now: main's explode (crates/h2sim/src/game.rs:1728-1736) applies only the area blast (up to
  120, through walls to the stuck victim). 120 x 0.5 = 60 to shields leaves a full-shield Spartan alive
  with about 10 shield. Defaults are at game.rs:462-469. The combat branch adds `attached` read from
  the tag (game.rs:1912-1918, h2viewer main.rs:215-220) and fixes this.
- Change: merge the combat branch's attached-damage path (proj attached detonation damage to the stuck
  victim, then the blast). The needle supercombine already applies its attached damage (400) on main
  (game/projectiles.rs:275), so nothing is needed there.
- Evidence: the damage-index choice (+0x160 / +0x130 / +0x10c) and the object choice were re-read;
  the shield overflow arithmetic was re-derived from d9110.
- Impact high. Confidence high.

## 4. Grenade blasts ignore the jpt lower bound (0 instead of 50 at the edge)

- Decomp: damage.cpp function_d74e0 lines 1007-1027: scale = 1 - (distance - radius min) /
  (radius max - radius min), clamped; beyond radius max the damage is zeroed (data +0x5c = 0, line
  1028). Since flags0c bit0 is clear for grenades, the scale goes into +0x54 (line 1031).
  function_d9b60 lines 2315-2316: damage = lerp(lower, upper, scale).
- Halo 2: frag_grenade_explosion radius 0.75-1.75, lower 50, upper 150: 150 inside 0.75 wu, falling
  linearly to 50 just inside 1.75 wu, then 0. plasma_grenade_explosion 0.75-1.5, 50-120. A frag 1.7 wu
  away does about 55.
- Remake now: grenades build Blast { damage: (0.0, def.damage) } (crates/h2sim/src/game.rs:1730-1735)
  from hard-coded rules (game.rs:454-469: frag 150, 0.75-1.75; plasma 120, 0.75-1.5), so a frag at
  1.7 wu does about 7.5. Projectile blasts (rockets) already use the tag lower bound through
  Blast::from_tags (weapon.rs:140-149). The combat branch still builds grenades with (0.0, damage)
  (its game.rs:1919-1924).
- Change: build grenade blasts from each grenade's detonation jpt with Blast::from_tags, so the lower
  bound, radii and push come from frag_grenade_explosion and plasma_grenade_explosion.
- Evidence: Blast::damage_at (weapon.rs:163-169) already implements the lerp correctly; only its input
  is wrong.
- Impact high. Confidence high.

## 5. Melee damage depends on how you come in: 20 standing, about 40 running, 60 after 0.5 s in the air

- Decomp: unknown_0a76b0.cpp function_e9a20 lines 2097-2110 (melee start): the scale byte is 0; 0x7f
  if (velocity . facing) / matg player information +0x2c (run forward, 2.25) is above 0.9; 0xff if the
  biped's airborne tick counter (+0x399, counted at bipeds.cpp 659-669 while the physics move reports
  airborne) is above 15. function_ea090 line 2194-2195 passes byte/255 to unit_object_type.cpp
  function_cf3d0, which clamps it into the hit (line ~7299); function_cfc90 line 3437 puts it into
  damage +0x54; function_d9b60 lerps lower to upper by it. The lunge (bipeds.cpp function_dee60) only
  switches the physics mode; it does not set velocity before the check, so the lunge does not raise
  the scale.
- Halo 2: strike_melee is lower 20, upper 60 (flags0c bit0 only stops area-distance scaling; melee is
  direct). So damage = lerp(20, 60, s): s = 0 standing or walking, s = 0.498 (about 40) when moving
  forward faster than 0.9 x 2.25 wu/s, s = 1 (60) after more than 15 ticks airborne. smash_melee
  (40-120) scales the same way. slice_melee (70) and dash_melee (150) have lower equal to upper.
- Remake now: always the jpt upper bound (crates/h2viewer/src/scene.rs:1226-1230 reads only
  upper_bound) or Rules.melee_damage 60 (game.rs:1585-1628). The combat branch's strike does the same.
- Change: at melee start, compute s (0; 0.498 if forward speed / run_forward > 0.9; 1.0 if airborne
  for more than 15 ticks) and deal lerp(jpt lower, jpt upper, s). Read lower_bound as well as the
  upper bound for the weapon's melee jpt.
- Evidence: the constants (0x7f, 0xff, 0.9, 0xf), the byte/255 conversion and the path into +0x54 were
  all re-read; run_forward at matg player info +0x2c is the same field the remake already reads
  (blam-cache physics.rs:135). Because a 20-damage standing melee is a big change to how fights feel,
  confirm with a play test on Vista before shipping it (two standing melees should not kill from full
  shields; a jumping melee should do 60).
- Impact high. Confidence medium (e9a20/cf3d0/cfc90 are "todo").

## 6. Player-fired rounds with "faster when owned by player" fly 1.5 times their tag speed

- Decomp: projectiles.cpp function_fd8b0 lines 286-296 (matched): proj flag bit10 gives 1.5 when the
  placement +0x5c is set, otherwise the difficulty row 10 (0.9/1.0/1.25/1.5; MP uses column 1 = 1.0).
  function_f8200 line 1272 stores it at projectile +0x16c; lines 1311-1317 multiply the initial
  velocity by it; f8eb0 line 1957-1958 multiplies the speed curve (initial to final) by it too.
  Placement +0x5c is the source object's player index when the source is a biped or vehicle
  (docs/unknown_0b7740.md lines 197 and 335); weapons.cpp line 3717 passes the weapon's owner unit
  (function_101fb0) as that source.
- Halo 2: plasma pistol bolt 11 -> 16.5 wu/s, plasma rifle bolt 14 -> 21, overcharged bolt 5 -> 7.5
  when a player fires them. Needles (0x9), rockets (0x340) and flak (0x200) do not have bit10. Ghost and
  Banshee bolts also have bit10 (15 -> 22.5, 30 -> 45), but whether +0x5c is set for vehicle weapons
  depends on the vehicle's player index, which was not checked.
- Remake now: rounds launch at flight.speed.0 (crates/h2sim/src/game/projectiles.rs:71) from
  weapon.rs:312, with no scaling, so player plasma bolts are a third slower than in Halo 2.
- Change: carry proj flag bit10 into Flight. For rounds fired by a player, multiply both the initial
  and final velocity by 1.5. Bots (actors) keep 1.0.
- Impact high (leading every plasma shot). Confidence medium-high (fd8b0 matched; the vehicle case is
  unverified).

## 7. Backsmack: an instant kill from anywhere in the victim's rear half, enemies only (main has none)

- Decomp: damage.cpp function_d7b80 lines 2882-2893: when the hit object is the outermost object
  (not seated), the damage was not passed on from a vehicle (flag 0x200), the jpt side effect (+0x10) is
  2 ("lethal to the unsuspecting") and function_cc010 is true, the victim is killed if it is not a unit
  or function_1df560 (matched) says the teams are enemies. unit_object_type.cpp function_cc010 lines
  1544-1566: true when the victim biped's facing (x, y) dotted with the damage direction is above 0,
  or always during animation 0x6000084; never when unit tag +0xbe bit0 (unit flag 16, "not instantly
  killed by melee") is set. For melee the damage direction is the attacker's aim vector (cfc90 line
  3434), not the line between the two players.
- Halo 2: any melee jpt with side effect 2 (strike, smash, slice, dash on coagulation) kills outright
  when the attacker's aim points within 90 degrees of the way the victim faces. Teammates and seated
  riders are never backsmacked.
- Remake now: main has no backsmack. The combat branch (game.rs:221-223 and 1786-1798) uses
  BACKSMACK_ANGLE pi/3 (60 degrees) from the attacker's position, and routes INFINITY damage through
  hurt, so with friendly fire on it kills teammates.
- Change: kill when dot(victim facing xy, attacker aim xy) > 0 (a 90-degree half-angle), only for
  enemies and unseated victims, gated on the melee jpt side effect == 2.
- Impact high on main (medium once the combat branch merges). Confidence high (the angle and team
  logic are clear; the functions are "todo").

## 8. Frag fuse: 0.5 s after the first floor bounce, but never before 1.5 s after the throw

- Decomp: projectiles.cpp function_f8200 lines 1284-1299 (timer and arming as per-tick rates);
  function_f8eb0 lines 1856-1877 (the timer runs once its start condition holds: 0 at once, 1 after
  the first bounce, 2 at rest, 3 any collision), lines 2037-2039 (the "first bounce" flag is set only
  when the surface normal's z is above 0.3, so wall bounces do not start it), lines 2151-2156 (a
  timed-out projectile still waits until armed).
- Halo 2: frag_grenade timer start 1, timer 0.5 s, arming 1.5 s: it goes off at the later of (first
  bounce off an upward-facing surface + 0.5 s) and (throw + 1.5 s). plasma_grenade: start 2 (at rest or
  stuck), timer 1.5 s, arming 1.5 s.
- Remake now: main uses fuse 1.0 from the first contact of any kind (crates/h2sim/src/game.rs:455-458,
  1697 and 1707) and no arming. The combat branch reads the timer (0.5) and arming (1.5) from the tag
  and waits for both (its game.rs:1896-1901, h2viewer main.rs:209-215), but still starts the frag timer
  on any contact, including walls.
- Change: merge the combat branch's fuse and arming. Also start a frag's timer only on a bounce whose
  surface normal z is above 0.3, and read the timer-start mode from proj +0xc0.
- Impact medium. Confidence high.

## 9. Overshield: invulnerable while it charges, drains over a fixed 90 s, and cannot be topped up

- Decomp: damage.cpp function_d9110 lines 1496-1499 (while shield_double_charged, the shield is not
  reduced and body damage is set to 0); function_d5de0 lines 3527-3540 (charge 1.0 per second up to
  3.0, then the flag clears) and 3543-3560 (multiplayer only: above the maximum shield it drains 2/90
  per second down to the maximum); function_d6af0 lines 548-556 (accepted only when the shield is at
  or below 1.0; sets the flag and clears the shield stun). The caller 00151ea0 (the pickup itself) is
  not decompiled.
- Halo 2: picking up an overshield charges from the current level to 3x at +1x per second, and the
  player takes no damage at all until it reaches 3x. It then drains from 3x to 1x over 90 s whatever
  the equipment's powerup time (60 s on coagulation). It cannot be taken while the shield is above
  100%.
- Remake now: the charge rate already matches (1x per second, crates/h2sim/src/game/powerups.rs:14-17,
  34-38), but damage applies normally while charging (hurt, game.rs:1750-1800, ignores
  overshield_charge). The drain is rules.overshield_time (powerups.rs:39-41), set from the equipment
  powerup time (h2viewer scene.rs:1880-1906, main.rs:176), so 60 s. Pickup tops up whenever at least
  25% of a layer is missing (powerups.rs:108-118).
- Change: ignore all damage while overshield_charge > 0. Drain at 2/90 of the base shield per second.
  Refuse the pickup when shield > base shield.
- Impact medium. Confidence medium (d6af0's caller is not decompiled).

## 10. Riders take a share of every hit on their vehicle; closed seats are not fully immune

- Decomp: damage.cpp function_d7b80 line 2919 calls function_db210 for every hit on a vehicle (the
  hit object itself). function_db210 lines 2964-3022: each seated rider takes the jpt damage again,
  scaled by (seat max transfer x jpt +0x48) when the rider is within the seat's falloff radius of the
  damage origin (the impact point for projectiles, faa60 line 2303), else (seat min transfer x jpt
  +0x4c); seats with no entry take nothing; it recurses into child vehicles (turrets).
  function_dc230 lines 1242-1266 (used at d7b80 line 2896): a direct hit on a seated rider is scaled
  by seat direct x jpt +0x44, unless jpt flags bit13 is set.
- Halo 2 (hlmt damage info +0xd8 seats, 0x14 each; coagulation): warthog_d and warthog_p direct 0.8,
  falloff 0.5, transfer 0.8/0.8; ghost_d direct 0.9, falloff 0.5, 0.9/0.9; banshee_d direct 0, falloff
  0.75, 0.8/0.8; scorpion_d direct 0.8, falloff 1.0, 0.4/0.4. jpt rider values (direct/max/min):
  rocket explosion 1/1/0.75, rocket impact 1/1/1, frag 1/0.5/0.5, BR 1/0.25/0, SMG 0.75/0.75/0,
  magnum 1/0.75/0.75. BR, carbine and sniper have flag bit13, so their direct hits on riders are not
  seat-scaled. A rocket on a Banshee does 150 x 0.8 = 120 to the pilot from the impact alone.
- Remake now: vehicle damage never reaches riders until the vehicle is destroyed
  (crates/h2sim/src/game/vehicles.rs:657-705). Riders in closed seats cannot be hit by rays or blasts
  at all (vehicles.rs:1053-1061, game.rs:1505, game/projectiles.rs:359-362); exposed riders take
  full damage.
- Change: on every hit on a vehicle, deal each rider the jpt damage x (seat max x jpt rider max) if the
  rider is within the seat falloff radius of the hit point, else x (seat min x jpt rider min). Scale
  direct hits on a rider by seat direct x jpt rider direct unless jpt flag bit13. Read the seats block
  (hlmt damage info +0xd8) and jpt +0x44/+0x48/+0x4c.
- Impact medium-high. Confidence medium (db210/dc230 are "todo"; a direct hit on a rider also passes
  through the vehicle first in d7b80's parent loop, which this change does not model).

## 11. An occupied vehicle at 0 health does not blow up until its last rider dies

- Decomp: damage.cpp function_d8cb0 lines 1804-1813: at 0 body, if function_db110 finds any unit
  seated in it, the vehicle is only flagged (unknown12) instead of dying. function_d6800 lines 674-692:
  when a seated rider dies and it was the last one in a flagged vehicle, the vehicle is marked
  (unknown13) and destroyed on the next tick (d5de0 lines 3498-3504). function_db670 lines 3369-3400,
  called from unknown_0a76b0.cpp function_ea8e0 line 2543 when a unit gets out: if no biped is left,
  the flag clears and the vehicle's body is raised to at least 25%.
- Halo 2: a vehicle brought to 0 health with anyone inside keeps taking hits (passing them to its
  riders, finding 10) until they die, and explodes when the last one dies. If they get out alive, the
  vehicle survives with at least 25% health.
- Remake now: destroy_vehicle runs as soon as health reaches 0 (vehicles.rs:657-672). It ejects and
  kills every rider and deals a guessed wreck blast of 100 over 2.5 wu (vehicles.rs:22-23, 674-704).
- Change: at 0 health with riders, mark the vehicle doomed instead of destroying it. Destroy it when
  its last rider dies. When the last rider gets out alive, clear the mark and set health to
  max(health, 25%). The wreck blast should come from the vehicle's destroyed effect and damage tags;
  b7360's internals are not decompiled, so keep the guess until then.
- Impact medium. Confidence medium (all "todo"; edge cases such as a rider killed by the same hit that
  empties the vehicle were not traced).

## 12. Vehicle damage comes from the material damage table, not a flat 0.25 below 50

- Decomp: damage.cpp function_d9020 lines 1370-1390 (body multiplier = matg damage table entry for
  the jpt's general (+0x50) and specific (+0x54) groups against the struck material's general and
  specific armor, via function_d5bc0, matched). The struck material is the hit collision region's
  material, or damage info +0xce (indirect material) for blasts.
- Halo 2: warthog, ghost and banshee indirect materials (72, 78, 82) are hard_metal_thick_* with
  general armor hard_metal_thick. Against hard_metal_thick: bullet_slow 0.25, bullet_fast 0.5 (BR,
  sniper), plasma_slow 0.1, plasma_fast 0.25 (carbine, beam rifle), melee 0.25, cutting 0.25,
  burning 0, explosion_attached 0 (a plasma grenade stuck to a hull does only its area blast);
  explosion_small, explosion_large, bullet_vehicle (warthog chaingun 7) and plasma_vehicle (ghost and
  banshee bolts 14) are 1.0. So a BR bullet does 3 to a hull, a sniper round 22.5, a chaingun round 7.
  Vehicle hlmt also give body stun 2 s, recharge time 10 s (scorpion 7 s) and recharge fraction 0.12
  (banshee 0.2); the per-second rate (+0xd4) comes from an undecompiled post-process.
- Remake now: below 50 damage is multiplied by 0.25, 50 or more counts in full, and blasts are raised
  to at least 50 (vehicles.rs:14-16, 657-672, 734). So the chaingun does 1.75, ghost and banshee bolts
  3.5, a BR bullet 1.5, a sniper round 11.25, and the edge of any blast 50. Vehicles never regenerate.
- Change: give each vehicle its armor names (matg materials +0x150, 0xb4 each; +0x10 general, +0x14
  specific) from the hlmt indirect material (damage info +0xce) and, for direct hits, the hit
  region's material, and look up the jpt groups in the damage table as for players. Use the
  blast's own falloff (no floor at 50). Add body regen up to the recharge fraction after the body stun.
- Impact medium. Confidence medium-high (d5bc0 matched; d9020 "todo"; materials re-read from tags).

## 13. Blast distance is to the closest point of the target, and line of sight uses four offset rays

- Decomp: damage.cpp function_d74e0 lines 938-942 (distance from the blast to the closest point of
  the object, function_baff0); function_d6f90 lines 3226-3286 (for units, when the jpt AOE core radius
  +0x18 is above 0: four rays from the blast centre offset sideways and up/down by the core radius,
  then on to that closest point; the target counts as shielded only if all four are blocked);
  function_d6c80 lines 834-917 (up to 64 objects).
- Halo 2: frag core radius 0.75, plasma grenade 0.75, rocket 1.0, needle super 0.5. Falloff uses the
  distance to the nearest point of the victim, so a frag that lands at a player's feet hits at
  distance about 0, not half the body height. A lip or thin post blocks the blast less often.
- Remake now: distance to the body centre at half height and one ray to that centre
  (crates/h2sim/src/game/projectiles.rs:345-357).
- Change: measure from the blast to the closest point on the player's capsule. Treat the player as
  exposed if any of four rays (blast centre offset by +/- the core radius sideways and vertically, then
  to that point) reaches them.
- Impact medium. Confidence medium.

## 14. Game options: the overshields shield type, extra damage, damage resistance, and the Juggernaut's real traits

- Decomp: unknown_157450.cpp function_1588b0 lines 589-650 (MP maximum shield: shield option
  value1c4 0 -> 1, 1 -> 0, 2 -> 3; the Juggernaut gets 3 when its overshield bit (flags22c bit1) is
  set; then x handicap 1/0.75/0.5/0.25). function_15dec0 lines 1951-1997 (damage x handicap; x1.5 for
  trait 2 "extra damage"; x0.5 for trait 3 "damage resistance"; 0 when friendly fire is off and the
  two are not enemies; self-damage 1). unknown_072c70.cpp c_game_engine::v5 lines 424-433 (traits from
  options flags184 bits 2/12/13); juggernaut.cpp v35 lines 415-430 (Juggernaut traits from flags22c
  bits 2/4/6) and v19 lines 329-345 (Juggernaut speed 0.75/1.0/1.5 by the movement option).
- Halo 2: the shield option has three values (normal, none, overshields = 3x that recharges to 3x and
  never drains, because d5de0 only drains above the maximum). Extra damage x1.5 and damage resistance
  x0.5 are variant options. The built-in Juggernaut template (scratch builtin_named.txt line 46):
  jug_extra_damage on, jug_overshield on, jug_damage_resistance off, jug_movement normal (1.0).
- Remake now: Options has shields on or off only (crates/h2sim/src/game/options.rs:22, 91-95). No
  extra damage, resistance or handicap. The Juggernaut takes 0.35 damage and moves 1.3x faster, both
  guesses (crates/h2sim/src/game/juggernaut.rs:9-11).
- Change: add the overshields shield type (max shield 3x) and the extra-damage and resistance
  multipliers. Make the default Juggernaut deal x1.5 damage, have 3x shields that recharge to 3x, take
  normal damage, and move at 1.0. Caveat: as decompiled, d9b60 passes the victim as 15dec0's first
  argument, which would put handicap and extra damage on the victim and resistance on the attacker;
  that only makes sense with the arguments the other way round (both functions are "todo"), so use
  the attacker-first reading and check it before implementing handicap.
- Impact medium (Juggernaut games). Confidence medium.

## 15. Explosion and melee push come from the jpt, with no distance falloff inside the radius

- Decomp: damage.cpp function_d9640 lines 2528-2600: impulse = jpt +0x40 (instantaneous acceleration)
  x the victim's object tag +0x14 (acceleration scale), along the damage direction lifted by +0.15 and
  then +0.3 in z for bipeds; no impulse when the target was outside the radius (flag 0x2000 -> report
  0x200). There is no distance-falloff factor. bipeds.cpp function_de620 line 1299-1304 halves it when
  biped +0x10a bit2 is clear (the bit is not named in the decomp). Melee goes through the same path
  (cfc90 -> d7b80 -> d9640).
- Halo 2: frag 2.25, plasma grenade 2.0, rocket explosion 2.75, needle super 2.25, strike_melee 0.75,
  smash_melee 1.0, all times the biped's acceleration scale.
- Remake now: grenades use GRENADE_PUSH 1.5, a guess (crates/h2sim/src/game.rs:211, 1733).
  Projectile blasts read the jpt value, but every blast scales push by distance falloff
  (game/projectiles.rs:363-370). Melee pushes nothing.
- Change: take grenade push from the detonation jpt (comes with finding 4), apply the melee jpt
  acceleration on a melee hit, and multiply by the biped's acceleration scale (obje +0x14). Whether to
  drop the distance falloff should wait for a play check, since d9640 is "todo". The reader's
  "x falloff" in the formula is not in the decomp.
- Impact low. Confidence medium.

## 16. Each guided round rolls its own turn rate, and most cannot turn back toward a target behind them

- Decomp: projectiles.cpp function_f8200 lines 1274-1282 (turn rate random between proj +0x184 and
  +0x188 per projectile); function_f8eb0 lines 1895-1919 (turns at most rate x dt about the axis to the
  aim point, only when the target is in front unless proj flag bit8; when the target is a player the
  rate is also x difficulty row 19, which is 1.0 in MP on coagulation's matg).
- Halo 2: needles 0.262-0.611 rad/s rolled per needle; rockets 0.785 and the overcharged bolt 1.396
  (fixed). Rockets have bit8 (flags 0x340); needles (0x9) and the overcharged bolt (0x401) do not.
- Remake now: every round uses the maximum (weapon.rs:315; 0.611 for needles) and always steers toward
  the target's body centre, even behind it (game/projectiles.rs:145-156).
- Change: roll the rate uniformly per round between the two bounds. Skip steering when the target is
  behind the round unless proj flag bit8.
- Impact low. Confidence medium.

## 17. Headshots come from the hit model region and the jpt flag, not a height band and a name list

- Decomp: damage.cpp function_d8cb0 lines 1766-1779: any body damage to a region with flag 0x80
  ("head") from a jpt with flags bit1 ("can cause headshots") sets the body to 0, in multiplayer always.
- Halo 2: masterchief_mp and elite_mp have sections body and head (0x80). Flag bit1 is set on magnum
  (0x2), BR (0x2002), carbine (0x2002), sniper (0x6002) and beam rifle (0x6002); not on SMG or shotgun.
- Remake now: the top 0.16 wu of the capsule counts as the head (game.rs:227, 1515) and the weapons are
  a fixed name list (h2viewer main.rs:165-171). The list matches the tag flags today, and the "only
  once shields are down" rule (game.rs:1787) matches d8cb0 running after d9110.
- Change: read jpt flag bit1 instead of the list. Use the head region's collision or node bounds
  (elites' heads sit forward of the capsule top) where available.
- Impact low. Confidence high.

## 18. The armor multiplier is the product of both damage groups

- Decomp: damage.cpp function_d5bc0 lines 429-458 (matched): product over the jpt general (+0x50) and
  specific (+0x54) groups of their modifiers for the material's general (+0x10) and specific (+0x14)
  armor.
- Halo 2: both groups apply. With the stock player armors no jpt has two non-1 groups (sniper:
  bullet_fast 1.0 x sniper 2.0 = 2.0 on shields), so the results match today.
- Remake now: read_damage uses the first group found, specific first (crates/blam-cache/src/weapon.rs:
  570-592).
- Change: multiply both groups' modifiers. Needed once vehicles use materials (finding 12).
- Impact low. Confidence high.

---

## Rejected

- **Needle supercombine "decided at detonation; each needle on its own random fuse"** — the
  proposed change is wrong. projectiles.cpp function_fd560 lines 786-807 (attach) resets the timer
  and arming progress of every needle already stuck in the same object whenever another one sticks,
  so needles wait for the stream to stop, much as the remake's push-back
  (game/projectiles.rs:254-256) does. Giving each needle an independent fuse would break
  supercombines: at the needler's 8 rounds/s, the first needle would pop before the 7th lands. What
  holds: fc330 lines 1566-1630 decides the supercombine when a stuck needle's (reset) fuse ends, not
  when it sticks (so it fires 0.5-0.7 s after the last needle, where the remake fires at once,
  projectiles.rs:265). Its count test is "> super count", which together with fd560's ">= 7 existing"
  marker suggests 8 needles, not 7. Both are "todo" and off-by-one prone, so check a delayed
  supercombine and the 8-needle threshold against Halo 2 footage before changing anything.
- **Multiplayer skips the terminal-velocity falling kill; treat the 14-wu kill as unconfirmed** —
  the decomp reading is right (bipeds.cpp function_e3700 line 3133 returns when game state is 2), but
  there is nothing to act on. The landing (harmful-distance) code is not decompiled, and the remake's
  lerp already does 125 at 10 wu, more than a normal Spartan's 115. The 14 wu cutoff only changes the
  outcome for overshielded players, and the decomp cannot say what Halo 2 does there.
