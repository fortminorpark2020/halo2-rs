# Game engines: verified findings (Halo 2 Xbox decompilation vs halo2-rs main)

Area: the multiplayer game engines and their rules. That covers scoring and flow for Slayer, CTF, Oddball, King of the Hill, Juggernaut, Territories and Assault, plus respawn, spawn choice, rounds, sudden death, announcer and HUD events, variant settings and team balancing.

How this was checked:
- I re-read every cited decompiled function myself in /home/claude/kirklandsig/halo2-decompiled.
- I took each function's status from config/functions.csv. "matched" means byte-exact. "todo" means the source is readable but not byte-verified. "near" means almost matched. "not decompiled" means there is no source file.
- I re-read the remake code. main is now 59a10ba. Since fcec0f4 only docs/ has changed, so the game code is the same.
- I checked the tag constants directly in the owner's Vista maps:
  - I parsed the goof built-ins and templates myself from the raw hex.
  - I hex-dumped the mulg runtime and constants blocks and the assault bomb jpt! with the existing target/release/h2tool binary. I did not build anything.
  - I dumped the netgame flags and equipment of 23 multiplayer maps.
- The worktree branches do not touch any of these rules.

Xbox vs Vista: the code is the retail Xbox build. Vista's game engine code is assumed to be the same, but title updates could have changed details. Constants that live in tags were read from the Vista maps and should be read from the tags at runtime. Where the Xbox code default and the Vista tag disagree, the Vista tag is quoted.

Halo 2 runs 30 ticks per second; the remake runs 60 (game.rs:42). Tick counts below are converted to seconds.

---

## Confirmed findings, ranked by how much a player would notice

### 1. Juggernaut: who becomes the Juggernaut, and what scores (high)
- **Decomp:**
  - src/juggernaut.cpp:282 `c_juggernaut_engine::v18` (0x192530, todo).
  - :367 `v30` (0x1922a0, todo).
  - :196 `function_192a30` (0x192a30, todo).
- **Halo 2:**
  - **Start and replacement:** v18 runs every update. Whenever there is no Juggernaut, 0x192a30 picks a random living player who is not waiting to spawn. So a Juggernaut exists from the start of the game.
  - **On each death (v30):**
    - The Juggernaut is killed by another player: the killer becomes Juggernaut and gets **no point** (statistic 0x26 only, event 8/2).
    - The Juggernaut gets a kill: +1 (statistic 0x27).
    - betrayal_point_loss is on (variant+0xf0 bit3, Vista template ON): a non-Juggernaut who kills another non-Juggernaut gets -1.
    - The Juggernaut kills themself: a random other player becomes Juggernaut.
    - The Juggernaut dies with no killer, or leaves: the role is cleared and v18 picks a random living player.
- **Remake now** (crates/h2sim/src/game/juggernaut.rs:22-41):
  - There is no Juggernaut until the first kill. That kill makes the killer Juggernaut and scores +1.
  - Killing the Juggernaut scores +1.
  - A non-Juggernaut killing another does nothing.
  - After a Juggernaut suicide there is no Juggernaut until the next kill.
- **Change:**
  - Pick a random Juggernaut at the start, and whenever there is none (suicide, fall, leaving).
  - Give no point for killing the Juggernaut; only the Juggernaut's kills score.
  - Take 1 point for non-Juggernaut on non-Juggernaut kills while betrayal_point_loss is on.
- **Evidence:** I read v18, v30 and 0x192a30 line by line. The goof juggernaut template has setting 68 (betrayal_point_loss) = 1 (my own parse of goof.hex).

### 2. Juggernaut traits: overshield, 1.5x damage, infinite ammo, normal speed (high)
- **Decomp:**
  - juggernaut.cpp:414 v35 (0x1926c0, matched): camo is bit2, extra damage bit4, damage resistance bit6.
  - unknown_157450.cpp:588 0x1588b0 (todo): shields x3 when bit1 is on.
  - :994 0x15fef0 (matched): motion sensor, bit0.
  - :954 0x159dd0 (matched): infinite ammo, bit5.
  - juggernaut.cpp:328 v19 (0x1924c0, matched): speed from jug_movement (variant+0xf4): 0 = 0.75, 1 = 1.0, 2 = 1.5.
  - unknown_157450.cpp:2895 0x15c000 (todo): the speed scale drops to its target at once and rises toward it at 0.15 per second.
  - :1951 0x15dec0 (todo): extra damage is x1.5 on all damage dealt; resistance is x0.5 on damage taken.
- **Halo 2:** the Vista goof juggernaut template has:
  - overshield on (shields x3)
  - extra damage on (x1.5 dealt)
  - infinite ammo on
  - motion sensor on
  - camo off
  - damage resistance off
  - movement normal (1.0)

  Dreadnaut uses resistance and the fast setting. The game-wide overshields shield type does not stack on the Juggernaut (0x1588b0).
  
  Vista note: the Xbox code default for a new custom Juggernaut variant (0x19d220, matched) clears bit5, so it has no infinite ammo. The Vista template turns it on, so follow the tag.
- **Remake now** (juggernaut.rs:8-11, 14-20, 51-60): the Juggernaut takes 0.35x damage and runs 1.3x faster. There is no overshield, no damage bonus and no infinite ammo, and the speed change is immediate.
- **Change:**
  - Apply the variant's Juggernaut traits: shield x3, damage dealt x1.5, infinite ammo, speed 1.0 by default.
  - Apply 0.5x damage taken only if jug_damage_resistance is set.
  - Change speed through a scale that drops at once and recovers at 0.15 per second.

### 3. King of the Hill: a contested hill still scores by default (high)
- **Decomp:** unknown_2bd960.cpp:1790 `function_2beee0` (0x2beee0, todo).
- **Halo 2:**
  - uncontested_hill is variant+0xf0 bit0. It is OFF in the Vista king template and ON only in Team King, Phantom King and Team Crazy King.
  - When it is ON, nobody scores unless exactly one player (FFA) or one team is inside.
  - When it is OFF, everyone inside scores even with enemies present.
  - In FFA each player inside gets +1 whenever their own continuous time inside reaches a whole second (`time_inside % ticks_per_second == 0`).
- **Remake now** (crates/h2sim/src/game/zones.rs:157-215): only `HillControl::Held` scores. A contested hill scores nothing in every variant, and there is no uncontested_hill setting.
- **Change:** add uncontested_hill (default off). While it is off, everyone in the hill earns points. While it is on, keep today's rule that the hill must be held alone.
- **Evidence:** I read 0x2beee0. Goof parse: king template (38,0); Team King, Phantom King and Team Crazy King each have (38,1).

### 4. King of the Hill: the hill stays put by default; when it moves, the next hill is random (high)
- **Decomp:**
  - unknown_2bd0b0.cpp:327 v40 (0x2bd680, matched).
  - :65 0x2bd0b0 (todo): the random pick.
  - :210 v25 (0x2bd200, matched): the first hill.
- **Halo 2:**
  - The hill moves only when moving_hill (variant+0xf4) is non-zero. The setting allows off, 30, 60, 120, 180 or 300 s.
  - The Vista king template has moving_hill off. Crazy King and Team Crazy King use 60 s.
  - The game always starts on hill 0.
  - On a move, the game picks a random hill group other than the current one. It takes a random slot in the list of hill groups and, if that is the current hill, the next one. It skips groups with fewer than 4 flags and raises event 6/4 ("hill moved").
  - Vista note: the Xbox code default for a new custom King variant (0x19d220, matched) sets variant+0xf4 to 60. The Vista built-in "King of the Hill" (template) has it off.
- **Remake now:**
  - `hill_move_time` is 60 s for every King game (crates/h2sim/src/game.rs:482). The lobby has no Crazy King and no moving-hill option.
  - Hills advance in order: `self.hill + 1` (zones.rs:162-176).
- **Change:** default to no movement and add the moving_hill setting (or a Crazy King type with 60 s). On a move, pick a random valid hill other than the current one.
- **Evidence:** I read v25, v40 and 0x2bd0b0. Goof parse: king template (40,0); crazy_king (40,60); team_crazy_king (40,60).

### 5. Territories: per-player contest and control timers, not a 6 s team capture (high)
- **Decomp** (all in unknown_2bd960.cpp):
  - :771 v23 (0x2bf310, todo).
  - :1021 territories_update_players (0x2bfc40, todo).
  - :1452 territories_update_holders (0x2bff80, todo).
  - :1364 territory_set_holder (0x2c0740, todo).
  - :1424 territories_update_scores (0x2c0a90, matched).
- **Halo 2:**
  - **Durations:** contest time is variant+0xf2 and control time is variant+0xf4. Both default to 5 s; the options are 3, 5, 10, 15, 20 and 30 s; the minimum is 1 s.
  - **Who holds:** a territory is held by one *player*.
  - **Contest (held territory):** an enemy of the holder standing inside builds up contest time. That progress is reset every tick that the *holder player himself* is inside the same territory. The holder's teammates do not block it. When the contest time fills, the territory becomes neutral (event 9/2 or 9/5, "lost").
  - **Control (neutral territory):** any player inside builds up control time on their own, and other players present do not block it. The first to fill it becomes the holder (event 9/1 or 9/4).
  - **Progress after leaving:** progress drains at 3 ticks per tick, so it drains 3x as fast as it builds. Progress is kept if the player returns to the same territory. Everyone's progress in a territory resets when it changes hands.
  - **Scoring:** the holder gets +1 every whole game second (0x2c0a90).
- **Remake now** (zones.rs:217-260; game.rs:483):
  - A team alone in a territory for `territory_capture_time` (6 s) takes it straight from the other team, with no neutral stage.
  - Any second team present pauses the take.
  - Leaving clears the progress.
- **Change:**
  - Replace the team capture with per-player contest and control timers using the variant's times (default 5 and 5 s).
  - Add the neutral stage and the rule that only the holder blocks contesting.
  - Add the 3x drain and the per-player memory of the last territory.
  - Fix the percentage display in objective.rs:581 to match.
- **Evidence:** I read all five functions. engine `p27` returns "are enemies": 0x15dec0 zeroes damage when friendly fire is off and `!p27`, and juggernaut v27 returns false for the same team. So the `opposing` branch in 0x2bff80 really is enemies. Goof territories template: (98,5), (99,5).

### 6. Flag, bomb and ball carriers move at 0.75x by default (high)
- **Decomp:**
  - unknown_2420a0.cpp:1021 0x240e30 v19 (matched): CTF and Assault.
  - unknown_072c70.cpp:704 0x2bc3d0 v41 (matched): Oddball.
  - Applied through 0x15c000 (todo): the drop is instant and the recovery is 0.15 per second.
- **Halo 2:**
  - CTF slow flag and Assault bomb speed share variant+0xf8: 0 is "on" (the default) and gives 0.75; 1 is "off" and gives 1.0. Blast Resort turns bomb speed off.
  - Oddball ball_speed uses the same field: slow 0.75 (the default), normal 1.0, fast 1.25.
  - Speed comes back gradually after letting go.
- **Remake now:** carrying the flag, bomb or ball does not change movement. take_flag (crates/h2sim/src/game/ctf.rs:312-339) only swaps the weapon. The only speed change is the Juggernaut's, in juggernaut.rs:51-60.
- **Change:** scale carrier movement by the variant's carrier speed (default 0.75). Use the same ramped speed scale as the Juggernaut.
- **Evidence:** both functions are matched. Goof templates: ctf (55,0), assault (64,0), oddball (45,0).

### 7. Spawn point choice: tag-weighted influences, not "farthest from enemies" (high)
- **Decomp:**
  - unknown_14b560.cpp:1531 0x14ef00 (todo).
  - unknown_23b500.cpp:
    - 0x23aea0
    - 0x23b060 (:148, matched)
    - 0x23ba90 (todo)
    - 0x23b170 (todo)
    - 0x23b500 (todo)
    - 0x23b8e0 (todo)
    - 0x23bb70 (matched)
    - 0x23bc40 (matched)
  - Engine extras:
    - King: 0x2bdb00 (matched).
    - Oddball: 0x2bcd50 (matched).
    - Territories: 0x2bf740 (todo).
- **Halo 2:**
  - **Candidates and tie-break:** every player starting location is scored, with no team or game-type filter. The highest-scoring spot where the biped fits wins; the fit test is done at position z+0.0701 (0x23bb70). A random 0 to 1.0 is added to each score (mulg constants +0x0, "max random spawn bias").
  - **Influence shape:** each influence has full weight inside its inner radius and falls linearly to 0 at its outer radius, measured on the ground plane. It only counts within 3 m above and 3 m below (mulg runtime+0x150 and +0x154 = 3.0 and 3.0).
  - **Weights** (mulg runtime+0x180+k*0x1c, inner / outer / weight, verified in hex on lockout.map):
    - enemy player: 6 / 12 / -0.75
    - teammate (same team value): 8 / 10 / +0.10
    - vehicle with an enemy in it: 3 / 12 / -0.75
    - vehicle with friends: 4 / 12 / +0.05
    - empty vehicle: weight 0
    - moving vehicle (speed > 1.0): -5 at its position 0.75 s ahead, with inner radius = speed (capped at 4.0) and outer = 1.2 x inner
    - live projectile whose tag has the two danger fields: -5 at its position 0.75 s ahead, 3.5 / 4.0 m
    - King hill centre: -1 at 4->8 m and +0.05 at 16->20 m
    - each ball: -1 at 2->6 m and +0.05 at 16->20 m
    - a territory held by you or your team: +0.25 at 3->6 m
    - death spot (type 10): 3 / 6 / -0.75
  - **Death spots:** they count for dead players who are **not enemies** of the spawning player (themselves and teammates). In FFA (teams off) every dead player's spot counts.
  - **Scenario spawn data** (0x23b170, 0x23aea0): its respawn zones (and separate initial-spawn zones) add influences, filtered by team and game-type masks. It can also override the vertical window and the weights per influence type.
- **Remake now** (game.rs:1189-1234): the score is the distance to the nearest living enemy capped at 50, plus a CTF side term clamped to ±30, plus random*4, minus 1000 if anyone is within 1.5 m. The scenario's spawn data is not read anywhere.
- **Change:**
  - Build the influence list from the mulg weights and the scenario spawn data, including its overrides and zones.
  - Score every starting location, add the random bias, and take the best one where the biped fits.
- **Corrections to the original finding:**
  - Death spots are teammates' and your own (everyone's in FFA), not "non-friendly" players'.
  - The scenario can override the mulg weights.

### 8. Respawn delay: suicide and betrayal penalties, 1 s minimum, inheritance and cycling, spawn staggering (high)
- **Decomp:**
  - unknown_157450.cpp:3607 0x15ce70 (todo).
  - :2750 0x15dd40 (todo).
  - :1999 0x15d6c0 (matched).
- **Halo 2:**
  - **Penalties:** a suicide (attacker == victim) adds suicide_penalty (variant+0x84) to the victim. Templates: 5 s for Slayer, King, Oddball and Juggernaut; 10 s for CTF, Assault and Territories. Killing a non-enemy adds betrayal_penalty (variant+0xac, 10 s) to the killer.
  - **Delay:** delay = max(penalty + respawn_time (variant+0x80), 1 s).
  - **Modifiers:** respawn_time_modifier (variant+0xa8) applies **only in team games**.
    - Inheritance (0): respawn together with the soonest teammate whose countdown is at least your own minimum (max(penalty, 1 s)), otherwise your full delay.
    - Cycling (1): respawn on the next multiple of the respawn time since the round started.
    - None (2): the template default. Team Ball, Team King, 3 Plots, Land Grab, Gold Rush and Contention use inheritance.
  - **Staggering:** after the first 3 ticks of a round, a player can only spawn on a tick where `player_index % 32 == elapsed_ticks % 32`. This adds up to about 1.07 s.
  - **Countdown sounds:** events 0/20 at 3, 2 and 1 s, and 0/21 on the last tick (countdown_for_respawn and player_respawn).
- **Remake now** (game.rs:1805-1824): respawn_in is always rules.respawn_time (corpse time for bots), with no penalties, minimum, modifiers or staggering.
- **Change:** add the penalties, the 1 s minimum, the two team modifiers and the staggering (32 Halo ticks is 32/30 s).

### 9. CTF and Assault sudden death holds the clock at 5 s (high)
- **Decomp:**
  - unknown_2420a0.cpp:1210 v25 (0x241320, matched).
  - :1628 0x2431a0 (matched).
  - :1762 0x244300 (todo).
  - unknown_1600f0.cpp:536 0x162470 (todo).
- **Halo 2:**
  - Sudden death is variant+0xf0 bit1, ON in the CTF and Assault templates.
  - **When the clock holds:** once 5 s or less remain, the displayed clock is held at 5 s while any flag or bomb is carried, dropped, armed, or away from home with a player who could take it within 6.0 m. 6.0 m is the mulg constants field +0xd8, verified in hex.
  - **When it stops:** the last 5 s count down, and they are restored to 5 s if play becomes live again.
  - **Announcement:** the first hold raises event 0/37, general\misc\sudden_death.
- **Remake now:** the game ends the moment time_left reaches 0 (game.rs:1256-1258, time_up at 1909-1937).
- **Change:** in CTF and Assault, hold the remaining time at 5 s while play around an objective is live, and announce sudden death once.

### 10. Respawn time choices and per-game-type defaults (medium)
- **Source:** sily players__respawn_time (variant+0x80). The hex shows values 3, 5, 10, 15, 20 and 30. The goof templates, from my parse, also apply.
- **Halo 2:**
  - There is no instant respawn: the minimum value is 3 s, and code enforces 1 s.
  - The default is 5 s for Slayer, King, Oddball and Juggernaut, and 10 s for CTF, Assault and Territories.
- **Remake now:** RESPAWN_TIMES is [5, 0, 3, 10, 15, 30] (crates/h2viewer/src/options.rs:29), and 5 s is used for every type (options.rs:170, 200).
- **Change:** use Halo 2's list (drop 0, add 20) and the per-type defaults.

### 11. Territories: shape, and only the variant's number of territories (medium)
- **Decomp:** unknown_2bd960.cpp:953 v34 (0x2bf5b0, todo); :1021 0x2bfc40, the inside test.
- **Halo 2:**
  - **Which flags:** territory i is the type-10 flags with identifier i, for i < number_of_territories (variant+0xf0 short). Template 3; Land Grab 5; Gold Rush 4; Control Issues 2; Contention 1.
  - **Centre and radius:** the centre is the first such flag. The radius is the larger of 1.0 m and the horizontal distance to the farthest same-identifier flag.
  - **Height:** a player is inside if they are less than max(0.1, drop to the lowest flag) below the centre and less than max(0.9, rise to the highest flag + 0.8) above it.
- **Remake now:**
  - Every identifier on the map becomes a territory (crates/h2viewer/src/objective.rs:83-94).
  - `contains` accepts anything within 2.5 m of any of its flags and within ±1.5 m vertically (zones.rs:11-13, 115-119).
- **Example (Lockout, verified):**
  - The map has identifiers 0 to 4, so the remake plays 5 territories where the template plays 3.
  - Territory 0 has two flags 2.32 m apart. Halo 2's circle is 2.32 m around the first flag; the remake reaches about 4.8 m from it.
- **Change:**
  - Use the centre, radius and height window above.
  - Limit the territories to the variant's count.
  - Update the drawn rings in the viewer to match.
- **Correction:** Lockout's territory 4 has two flags, not one, and it is not used at all with the default count of 3.

### 12. King of the Hill: hill outline and height window (medium)
- **Decomp** (unknown_2bd960.cpp):
  - :481 hill_set (0x2bdc70, near).
  - :376 hill_build_polygon (0x2be050, todo).
  - :298-370 spline helpers 0x2bdd20 (matched), 0x2bdd70 and 0x2bde90 (todo).
  - :723 inside test 0x2be880 (todo).
  - Marker query: unknown_19ec40.cpp; its keys are type, team and identifier.
- **Halo 2:**
  - **Outline flags:** the outline is the type-(11+n) flags with **identifier 0**, in scenario order, at most 16. There must be at least 4, and an odd count drops the last flag.
  - **Curve:** each pair of flags makes a cubic curve through the midpoints. The curve is sampled into 32 vertices.
  - **Height markers:** flags with identifier 1 (up to 8) only widen the height range. Flags with identifier 2 or higher are ignored.
  - **Inside test:** the player is inside the 32-vertex polygon and within its circle, between the lowest marker z - 0.1 and the highest marker z + 0.8. "Marker" here means the centroid of the outline flags plus the identifier-1 flags.
- **Remake now:**
  - hills() takes every KingHill(n) flag regardless of identifier, needs at least 3, and sorts them by angle around the centroid (objective.rs:68-80, zones.rs:24-32).
  - `contains` uses that straight-edged polygon from 1.0 m below the lowest flag to 2.5 m above the highest (zones.rs:8-10, 38-64).
- **Effect on real maps** (all 23 multiplayer maps checked):
  - Many hills have identifier-1 height markers. Examples: backwash, containment, deltatap, dune, elongation, gemini, triplicate, turf and warlock (2 per hill); ascension, beavercreek, foundation and midship.
  - Some have identifier-2 markers: coagulation, colossus and waterworks.
  - These sit inside the outline, so the remake cuts notches into the hill. For example, backwash KingHill(0) has two identifier-1 markers at (1.33, 2.14).
  - The height is also wrong. On backwash, Halo 2's top is 1.8 m above the floor; the remake's is about 3.5 m. On coagulation KingHill(0), Halo 2's top is +0.8 m; the remake's is +3.4 m.
  - Odd groups that drop their last flag: deltatap KingHill(2) has 7 flags, and needle KingHill(2), (5) and (6) have 7, 7 and 5.
- **Change:** build the outline from the identifier-0 flags in scenario order, curve it to 32 vertices, and use the identifier-1 flags plus -0.1 / +0.8 for the height window.
- **Correction:** Lockout's KingHill(2) and KingHill(3) each have 8 identifier-0 flags, not 1. Lockout is unaffected by the identifier rule.

### 13. King of the Hill: per-player hill time and the team point rule (medium)
- **Decomp:** unknown_2bd0b0.cpp:220 v37 (0x2bd2c0, matched); unknown_2bd960.cpp:1790 0x2beee0 (todo).
- **Halo 2:**
  - Each player's time inside counts up every tick they are in the hill and resets to 0 the moment they are not (or are dead). In FFA you must stay a full second to earn each point.
  - In team games the team's counter runs while any member is inside and resets when none is.
  - On each whole second, the point goes to the member who has been inside longest. On equal time it goes to the **higher** player index.
  - That member's time is then set back to 1, so the next point tends to go to another member.
  - team_time_multiplier (variant+0xf0 bit1, default off) gives the point to every member inside instead.
- **Remake now:**
  - hold_point keeps a per-player accumulator `p.hold` that is never reset on leaving the hill (zones.rs:144-155).
  - In team games every point goes to the first player who entered, as long as they stay in (zones.rs:182-189).
- **Change:**
  - Reset the player's hill time when they leave.
  - Add the team counter and the longest-inside rule with the reset to 1 (or all members when team_time_multiplier is on).
- **Correction:** ties go to the higher index, not the lower, and the original missed the reset to 1.

### 14. Slayer scoring: deaths with no killer, suicide_point_loss, death_point_loss, bonus points (medium)
- **Decomp:** unknown_072c70.cpp:629 `c_game_engine_derived::v0` (0x2bc000, todo).
- **Halo 2:**
  - A kill is +1.
  - A betrayal is -1 to the killer.
  - A suicide is -1 only if suicide_point_loss is on (variant+0xf0 bit1). It is ON in the Slayer template and OFF in Rockets, Elimination and Phantom Elimination.
  - A death with no killer changes no score, unless death_point_loss (bit2, default off) is on; that takes 1 point from the victim on every death.
  - bonus_points (bit0, off) adds +1 for a kill that earns a medal (award 0 to 14) and +1 if the victim was the leader.
- **Remake now** (game.rs:1855-1884): killer None or killer == victim always costs the victim a point.
- **Change:**
  - Stop taking a point for deaths with no killer.
  - Add suicide_point_loss (off for Rockets), death_point_loss and bonus_points.
- **Caveat:** the caller that passes the killer (0x1e9fa0) is not decompiled, so which deaths reach the engine with no killer is not certain. The kill handler 0x15ce70 does distinguish no-killer deaths (subtype 0xd "killed by unknown", 0x22 fall) from self-kills, and mulg has a "gen_killed_by_unknown" response. The Rockets setting is certain from the goof tag, settings (20,0) and (29,0).

### 15. Built-in variant defaults (medium)
- **Source:** goof multiplayer\game_variant_settings\multiplayer_game_settings (Vista). I re-parsed the 39 built-ins and 7 templates from goof.hex. The Xbox code defaults are in unknown_19d220.cpp:194 (0x19d220, matched).
- **Halo 2 (Vista):**
  - Team Slayer: score 50, teams on.
  - Juggernaut template: score 15.
  - King template: 120 s. Team King, Phantom King and Team Crazy King: 60 s with uncontested hill on.
  - Rockets: motion sensor off, suicide point loss off, weapons_on_map rockets.
  - Snipers: score 15, motion sensor off, sniper and magnum starts, weapons_on_map "sniping", no vehicles.
  - Swords: weapons_on_map none, no vehicles.
  - CTF and Assault templates: respawn 10 s.
- **Remake now:**
  - Team Slayer and Slayer both default to 25 points (crates/h2viewer/src/menu.rs:197, 221-229).
  - Juggernaut defaults to 10 (menu.rs:203).
  - Team King defaults to 120 s like King.
  - The ROCKETS and SNIPERS presets keep the motion sensor on.
  - SWORDS puts a sword on every weapon spot (options.rs:113-160).
- **Change:** take the per-type defaults and the preset contents from the goof tag.
- **Note:** SNIPERS keeping the radar was an explicit earlier decision ("keep SNIPERS as on main"). Raise it with John; do not change it silently.

### 16. Carrier traits: 1.5x damage for flag, bomb and ball carriers; resistance and camo options (medium)
- **Decomp:**
  - unknown_2420a0.cpp:1371 0x242010 v35 (matched).
  - unknown_2bc1b0.cpp:69 0x2bcc90 (matched).
  - Applied through 0x15dec0 (todo).
- **Halo 2:**
  - A flag or bomb carrier has the extra-damage trait (x1.5 on everything they deal) when flag or bomb damage is "massive" (variant+0xfc == 0, the default).
  - Damage resistance (bit6, x0.5 taken) and active camo (bit7) are options.
  - A ball carrier gets extra damage when ball_hit_damage is massive (variant+0xf6 == 0, the default), camo with ball_camo (bit1), and resistance with ball_toughness (bit2, on in Swordball).
- **Remake now:** a carrier only has the carried weapon's own tag melee damage (ctf.rs:312-339; melee in game.rs around 1585-1620). There are no carrier traits.
- **Change:** apply these traits through the same damage multiplier stage as finding 24.

### 17. Time-remaining warnings (medium)
- **Decomp:** unknown_157450.cpp:4349 0x15be20 (todo).
- **Halo 2:** with a round time limit set (variant+0x54), the game raises general events at 30 min, 15 min, 5 min, 1 min, 30 s and 10 s left (subtypes 14, 15, 16, 17, 32 and 33). The sounds are general\countdown\thirty_mins_remaining ... ten_secs_remaining, verified in the mulg event dump. At 0 it raises event 0/18 and ends the round with the winner from 0x15b330.
- **Remake now:** no warnings (time_left at game.rs:1112-1115; end at 1256-1258).
- **Change:** raise these events from the game clock and play the mulg responses.

### 18. Announcer and HUD text from the mulg event table (medium)
- **Decomp** (unknown_19de80.cpp, all matched):
  - :238 0x19de80
  - :145 0x19df10
  - 0x19e770
  - 0x19e890
- **Halo 2:**
  - Every game event (type, subtype, cause and effect player and team) is matched per player against the mulg response blocks. The first match is taken in this audience order: cause player, effect player, cause team, effect team, everyone. Each response has its own required and excluded rules.
  - A response carries text with #cause_player and other tokens, a plural form and timer text, a sound with weighted random alternatives, and a delay.
- **Remake now:** about 30 sound names are hard-coded (crates/h2viewer/src/scene.rs:2034-2093), and kill text is hard-coded (local.rs:286-301).
- **Missing, all present in the Vista mulg table:**
  - the time warnings
  - 1 min / 30 s / 10 s to win (events 7-10, 47, 48)
  - sudden death (37)
  - round over (31)
  - team change (23)
  - offense and defense (3/6)
  - slayer new target (2/1)
  - player joined and rejoined (12, 24: secondary_message)
- **Change:** load the response blocks (h2tool `events` already decodes them) and drive both text and sound from them.

### 19. Multikill and spree announcements: one per kill, and the 25+ spree repeats (medium for audio, low for rules)
- **Decomp:** unknown_157450.cpp:3607 0x15ce70, lines 3743-3786 (todo).
- **Halo 2:**
  - Each kill gives at most one flavor message, and a multikill wins over a spree.
  - Multikill chains of 2, 3, 4, 5, 6 and 7+ give double, triple, killtacular, killing frenzy, killtrocity and killimanjaro; killimanjaro repeats.
  - Sprees of 5, 10, 15 and 20 give killing spree, running riot, in the zone and untouchable. un_frikin_believable plays at every further multiple of 5 (25, 30, 35 ...).
  - If neither applies and the victim had a streak of 5 or more, "broke killing spree" shows as text only; the mulg response has no sound.
  - Betrayals get none of these.
  - Both medals are still recorded in the statistics.
- **Remake now:** a multikill medal and a spree medal can both fire on the same kill, and spree medals stop at 25 (game.rs:229-233, 1866-1884).
- **Change:** play one message per kill, multikill first. Repeat the 25-kill line every 5 kills after 20. Add the broke-spree text.

### 20. Assault bomb: explosion from the tag, 5 s arm time (medium)
- **Source:**
  - mulg constants +0xe4 "bomb explode damage effect" points to jpt! objects\weapons\multiplayer\assault_bomb\damage_effects\bomb_explosion (hex on lockout.map and shared.map): radius 5.0-8.0 m, core 3.0 m, damage 100-250, group explosion_large.
  - The goof assault template has arm time 5 s (variant+0x108; Neutral Bomb and Blast Resort use 10) and sticky arming on.
  - Arming, defusing and detonation (0x240820, 0x242660) are **not decompiled**.
- **Remake now:**
  - BOMB_RADIUS is 4.0 m and BOMB_DAMAGE is 500 (crates/h2sim/src/game/ctf.rs:29-30).
  - bomb_arm_time is 3 s plus a separate 4 s fuse (game.rs:480-481).
  - Defusing takes the arm time (ctf.rs:230-310).
- **Change:**
  - Apply the bomb_explosion jpt! from mulg as a normal area damage effect.
  - Default the arm time to 5 s.
  - Leave fuse and defuse until 0x240820 and 0x242660 are decompiled.

### 21. Lead announcements: no "tied the leader" in Oddball, King or Territories (low)
- **Decomp:** unknown_23f260.cpp:451 0x23fdd0 (matched); :529 0x23f6e0 (matched).
- **Halo 2:**
  - Leads are checked after a score change.
  - "Gained the lead": a side becomes sole first coming from behind or from a tie.
  - "Lost the lead": a side drops out of first.
  - "Tied the leader": only when a side moves from behind to tied first, and never in engine types 3, 4 and 8 (Oddball, King, Territories).
- **Remake now:** lead_changes announces Tied in every game type (game.rs:1954-1987), including the point-per-second modes.
- **Change:** suppress Tied for Oddball, King and Territories.

### 22. Rounds, lives, max active players, force even teams (medium; needed for several built-ins)
- **Decomp:**
  - unknown_157450.cpp:2319 0x15b3a0 (todo): rounds.
  - 0x15ce70: lives are decremented on death.
  - unknown_15d770.cpp:50 0x15db30 (matched): out of lives.
  - :65 0x15d770 (todo): max active players.
  - :130 0x15d920 (todo): force even teams.
- **Halo 2:**
  - number_of_rounds (variant+0x4c): 1, 2, 4 or 6 rounds, or best of 3, 5 or 7; never more than 31 rounds.
  - rounds_reset_map defaults on.
  - lives_per_round (variant+0x7c): a player with no lives left waits out the round.
  - max_active_players (variant+0x78) and force_even_teams (variant+0x48 bit14) hold players back from spawning.
  - Used by Elimination, 1 Flag CTF, Single Bomb, Neutral Bomb, Blast Resort, Swordball, Gold Rush, Phantom Fodder, 2 on 1 and 3 on 1.
- **Remake now:** one game per match, with no lives, active-player cap or even-team rule (game.rs:1897-1937).
- **Change:** add round flow (round winner, map reset, end by count or best-of), lives, the active-player cap and force even teams.

### 23. CTF and Assault flag types: single (attack/defend rounds) and neutral (medium)
- **Decomp:** unknown_2420a0.cpp:
  - :1338 v36 (0x241fc0, matched)
  - 0x2432e0 (matched)
  - 0x243250 (matched)
  - 0x243a80 (matched)
- **Halo 2:**
  - ctf_flag_type and assault_bomb_type (variant+0x104): 0 multi, 1 single, 2 neutral (one flag or bomb at the slot-8 neutral marker).
  - With single: in CTF only the defenders' flag exists; in Assault only the attackers' bomb exists.
  - The defending team changes each round, and event 3/6 or 10/6 plays offense or defense.
- **Remake now:** always one flag or bomb per team (ctf.rs:109-130).
- **Change:** add the single and neutral types together with rounds (finding 22).
- **Correction:** the Vista Assault template is multi-bomb ("all_bombs", setting 59 = 0), not single. Single Bomb, Single Bomb Fast, Blast Resort, 1 Flag CTF and 1 Flag CTF Fast use single; Neutral Bomb uses neutral.

### 24. Shield type, handicap, extra damage and damage resistance multipliers (medium)
- **Decomp:** unknown_157450.cpp:588 0x1588b0 (todo); :1951 0x15dec0 (todo).
- **Halo 2:**
  - **Shields:**
    - Shield multiplier from shield_type (variant+0x88): normal x1, none x0, overshields x3.
    - The per-player handicap (player+0xc1, a profile setting) scales shields and damage dealt by 1, 0.75, 0.5 or 0.25.
  - **Damage dealt to another player**, in this order:
    - x handicap
    - x1.5 if the attacker has extra damage (variant+0x48 bit12, or an engine trait)
    - x0.5 if the victim has damage resistance (bit13, or an engine trait)
    - x0.1 if the victim's +0x194 timer is running (its setter is not found)
    - x0 between non-enemies when friendly fire is off
  - Self-damage is never scaled.
- **Remake now:** Options only has shields on or off (crates/h2sim/src/game/options.rs:19-35, 96-102). There is no overshield shield type, handicap, extra damage or resistance.
- **Change:** add one damage and shield multiplier stage in this order. Findings 2 and 16 plug into it.

### 25. HUD status line (low)
- **Decomp:** unknown_157450.cpp:3226 0x15ee30 (near).
- **Halo 2:** the status codes come from the mulg notice list (0x15ebd0):
  - 1 no team; 2 respawning; 3 waiting; 4 out of lives.
  - 22-24 lives left, for 3 s after spawning when lives are limited.
  - 18 round start, for the first 3 s.
  - 17 final seconds, under 5 s left.
  - 5, 6 and 7 leading, tied and behind (25, 26 and 27 with no score limit).
  - 8, 9 and 10 won, tied and lost after the game.

  Engines can override the line: Juggernaut v37 (0x192750, juggernaut.cpp:437) returns 15, and Oddball 0x2bcd20 returns 14 for the ball carrier.
- **Remake now:** the only status shown is "RESPAWN IN n" (crates/h2viewer/src/local.rs:1290).
- **Change:** add the status codes, with text from mulg.
- **Correction:** 0x2bd2c0, which the original cited as a KOTH override, is the per-tick hill timer, not a status override.

### 26. Starting grenades: at most 2 of each, 1 with 9+ players; grenade, camo and overshield pickups toggles (low)
- **Decomp:**
  - unknown_157450.cpp:4252 0x15e970 (todo).
  - 0x158e20: sets flag 8 when there are 9 or more players.
  - :1236 0x15a5b0 (todo).
- **Halo 2:**
  - **Counts:** starting grenades come from the scenario's starting profile, capped at 2 of each, or 1 of each with 9 or more players.
  - **Toggles and conversion:**
    - starting_grenades (variant+0x48 bit11) off means none.
    - weapons_on_map plasma or covenant turns frags into plasmas; human turns plasmas into frags.
    - grenades_on_map (bit10), invis (bit9) and overshields (bit8) remove those pickups from the map. Overshield pickups are also removed when shield_type is "no shields".
- **Remake now:** every multiplayer player starts with 2 frags and 0 plasmas (Rules default, crates/h2sim/src/game.rs:449-450), or none if grenades are off (crates/h2viewer/src/options.rs:198-199). There is no 9+ player cap and no pickup toggles.
- **Change:** take the counts from the starting profile with the cap. Add the conversion and the three pickup toggles.

### 27. Matchmaking team balancing (low, optional)
- **Decomp:** src/team_balancing.cpp (mostly matched):
  - :304 balance_teams_greedy (0x91a30, todo)
  - :329 try_swap (0x91bd0, todo)
  - :249 rank_spread (0x918c0, matched)
  - :393 balance_teams (0x91fc0, todo)
- **Halo 2:**
  - **Greedy pass:** parties stay whole. The largest party goes first to the team with the fewest players; ties go to the lower skill total.
  - **Swaps:** pairs of parties are then swapped between teams, flagged parties only with flagged parties, as long as the swap does not widen the size gap.
  - **Ranking a swap:** a smaller size spread wins. Then a lower cost = 7 x (skill-total spread) + 3 x (rank spread) wins, where rank spread compares each team's k-th best player.
  - Up to 4 teams.
- **Remake now:** h2live split() tries every two-team split and keeps the closest size, then the closest summed level, choosing randomly among equals (crates/h2live/src/matchmaker.rs:383-405).
- **Change (optional):** add the rank-spread term (7:3) so teams match player by player.

---

## Rejected

- **"CTF: dropped-flag reset timer pauses near enemies":** the reset timer lives in 0x240820, which is **not decompiled**. The only support is the mulg field name "flag reset stop distance" (6.0 m, constants +0xd8) and the fact that 0x240820 calls 0x244300, the same proximity test that sudden death uses. That is suggestive but not a readable rule. Revisit when 0x240820 is decompiled.
- **"Item respawn fallback" (the item-respawn half of the original grenade finding):** the claim that "an entry with spawn time 0 comes back after 1 s" is wrong. The viewer already turns an equipment spawn time of 0 into 30 s (crates/h2viewer/src/scene.rs:832-835) before item_timers' `.max(1.0)` sees it.

  Halo 2's middle step, the itmc or vehc collection's own spawn time (0x15ac50), does exist in the Vista tags: rocket_launcher itmc is 120 s and several powerup itmc are 60 s. But on coagulation, zanzibar, ascension, headlong and lockout, every netgame equipment entry that points at those collections has its own non-zero spawn time. Every entry with 0 points at a collection that is also 0. So no player-visible difference was found. The starting-grenade half is kept as finding 26.
