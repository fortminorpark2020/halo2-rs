Online play: decomp (kirklandsig/halo2-decompiled, Xbox retail) vs halo2-rs main
===============================================================================

Read only. D = decomp path under /home/claude/kirklandsig/halo2-decompiled. R = remake path under /home/claude/halo2-rs.
Status from D/config/functions.csv (matched = byte-exact; todo = draft that may be wrong).

Caveat that applies to almost everything below
----------------------------------------------
- Ranking and matchmaking tuning lives in g_network_configuration (retail 0x4ce040, 0x1730 bytes).
  network_configuration_set_defaults (0x66330, TODO draft, D/src/unknown_0662e0.cpp) fills it, and
  Xbox Live replaced it with a downloaded network_configuration.dat. The hoppers (playlists, 16 x 0x614
  records at 0x551ae8; decomp misnames the type s_surface_description) were also downloaded. What the
  decomp gives is the retail fallback/default, not necessarily what Live served.
- Vista (H2V/GFWL) has its own binary and its own Live config. Tag data checked: mainmenu.map wigl
  'ui\ui_shared_globals' block at +0x158 (50 skill ranges, identity 0..48, last 49..127) matches Xbox.

1. XP and levels (ranked hoppers)
---------------------------------
Per-opponent XP: D/src/unknown_072db0.cpp function_746b0 @0x746b0 (MATCHED, line 346).
  index = my_skill - opp_skill + 127 (skill = level-1).
  If my place < opp place: value378[index] * value9f4[my_skill] / 100 + valueb74[my_skill]
  If opp place < mine:      value576[index] * value974[my_skill] / 100 + valuea74[my_skill]
  Equal place or NONE: 0. Integer (truncating) arithmetic.
Defaults (D/src/unknown_0662e0.cpp 649-758):
  win  value378: d<=-8 -> 180; d=-7..7 -> 100-10d; d>=8 -> 20       (d = my level - opp level)
  loss value576: d<=-8 -> -20; d=-7..7 -> -(100+10d); d>=8 -> -180
  offsets valueb74/valuea74 all 0.
  loss factor value974 (by my level): L1 0, L2 5, L3 10, L4 20, L5 35, L6 45, L7 55, L8 60, L9 65,
    L10 70, L11 75, L12 80, L13 85, L14 90, L15 95, L16+ 100 (%).
  win factor value9f4: 100 to L41, L42 95, L43 90, L44 85, L45 80, L46 70, L47 65, L48 60, L49 55, L50 50.
  per-game clamp valuec78 200 / valuec7c -200.
Averaging: function_74800 @0x74800 (TODO draft, line 530): sum over valid players in same game
  (active, team != NONE, !(flags & 3), skill != NONE), on another team (FFA: each player own team),
  team mode uses team rank for both sides; total /= count (truncating); clamp so XP stays in
  0..0x3fffffff; clamp result to [-200, 200]. function_74720 (TODO) is the team/clan variant.
Level from XP: function_74970 @0x74970 (MATCHED, line 878):
  skill = number of thresholds value774[1..] that XP reaches; if skill == previous-1 and
  XP > (value774[skill] + value774[skill+1]) / 2, keep previous. One-level hysteresis only.
Thresholds value774 (lines 697-726), level: min XP
  L1 0, L2 100, L3 200, L4 400, L5 600, L6 900, L7 1200, L8 1600, L9 2000, then +500 per level
  (L10 2500 ... L40 17500), L41 18100, L42 18900, L43 20000, L44 21500, L45 23500, L46 26100,
  L47 29400, L48 33500, L49 38500, L50 44500; above 50 unreachable (0x3fffffff).
Post-game: function_741f0 @0x741f0 (TODO): per hopper writes XP (add), level (set), highest level
  (max), and three counters to Live stats leaderboards (id, id+32, id+64); clan hoppers rate the
  clan via function_74720. Clamps g_46722c. 0x74aa0 schedules stat reads/writes.

Remake R/crates/h2live/src/levels.rs:
  min_xp 40-47: 100(n-1) to L12, +200 to L16, +250 to L50 (L50 = 10250).
  EXPECTED/UPSET 18-24: curved, gap capped at 15 (50..150).
  LOSS_FACTOR 27-30: 2.5% steps to L29, 100% from L30. WIN_FACTOR 33: 95..55 for L42..L50.
  xp_changes 213-237: factor chosen by sign of the summed total, then round half away from zero.
  Rank::after 75-82: multi-level hysteresis (walks down while below each midpoint).
  No +/-200 clamp (irrelevant with default tables: max avg is 180).
Differences: thresholds, curve, loss/win factors (L46-50 one step lower in decomp, L16+ full loss),
  factor per opponent vs per total, truncation vs rounding, single- vs multi-level hysteresis.

2. Skill match range (k table)
------------------------------
Built-in per-hopper table field_51c[128] (D/src/unknown_1936e0.cpp 180-191, inside 0x1936e0 TODO):
  upper bound for skill s: L1-6 +5, L7-16 +6, L17-26 +7, L27-31 +8, L32-36 +9, L37+ +10 (cap 127).
function_1931a0 (MATCHED, unknown_1932c0.cpp ~245) = lowest skill whose upper bound reaches a skill.
  So with defaults: L12 meets L7..L18; L1 meets L1..L6; L50 meets L40..L50.
Search window (online_session_search.cpp ~880, draft): [lowest that reaches party max,
  upper bound of max(that, party min)]; widens one level per search round.
Party rule (unknown_07a9a0.cpp ~1150 inside 0x7e210 TODO): members at or below the lowest skill that
  can meet the party's highest are adjusted using the same table (exact replacement unclear in draft).
Remake level_range 87-96 / can_match 100: symmetric, L1 10..L6 5, 7-16 6, 17-26 7, 27-29 8, 30 9, 31 10,
  32-35 l-21, 36+ 50-l; max of both players' k; widening +3 every 15 s, any level after 60 s
  (matchmaker.rs 27-29, 291-294). Every hopper on Live carried its own 128-entry table, so the
  Bungie FAQ example in the remake test (L12 meets 6+) may reflect Live data; the binary default gives 7+.

3. Built-in hoppers (fallback when Live list missing) - 0x193c70 (TODO) in unknown_1936e0.cpp 246+
-----------------------------------------------------------------------------------------------
  0 Rumble Pit     ranked FFA 4-8 players, start-delay scale 30 s, give-up 150 s
  1 Team Skirmish  ranked 2 teams x 3-4, imbalance 0, party 1-4, scale 30 s, give-up 150 s
  2 Head to Head   ranked FFA 2-2, give-up 120 s
  3 Big Team Battle RANKED 2 teams x 5-8, imbalance 1, party 1-5, scale 45 s, give-up 300 s
  4 Minor Clanmatch 2 x 3-4; 5 Major Clanmatch 2 x 6-8; 6 Team Training unranked 2 x 2-4, imbalance 1
Hopper types: 1 unranked FFA, 2 ranked FFA, 3 unranked team, 4 ranked team, 5 clan (unknown_1932c0.cpp).
Remake playlists.txt: Rumble Pit 3-8; Team Skirmish 4-8 party 2; BTB UNRANKED 2-16 party 8 bots fill 12;
  Team Training 2-8 party 4; Double Team, Team Slayer, Team Snipers, Team Hardcore are later Live lists.
Map choice: 0x192eb0 (MATCHED): weighted random, weights clamped 1..1000, only maps everyone owns,
  no previous-map exclusion. Remake form() matchmaker.rs 789-805: uniform random, excludes previous map.

4. Gathering / start delay / give up
------------------------------------
0x193560 (MATCHED): FFA delay = scale*(max-count)/(max-min) (0 at max);
  teams: (maxPerTeam - count/teams)*scale/(maxPerTeam-minPerTeam).
0x71730 host gathering (TODO): countdown measured from last player-count increase; give-up after
  hopper time (0x59c); then the session searches again.
value198 = 240 s: searching client gives up searching and hosts its own session.
value190 = 10 s join timeout. value354 5000 ms.
Remake: COUNTDOWN 20 at min, +5 per arrival up to 40, start at once at max, GIVE_UP 600
  (matchmaker.rs 33-37, 584-660).

5. Search scoring (online_session_search.cpp, unknown_0662e0.cpp 583-647)
-------------------------------------------------------------------------
  ping score per 100 ms {100,80,50,0,-100,-200...} (value1c0..); language matrix value1fc
  (JP/KO/ZH: same +100 else -100; others same 0, different -50/-100); skill closeness
  100-15*|diff| up to 4 (value1f0 4, value1f4 100, value1f8 25); same hopper +50 (value340);
  player count x2 capped 180 (value344/348); min score 100 (value1ec); 150 results (value1b8).
Remake: server picks; no ping/language scoring for grouping (host order by RTT only).

6. Team balancing (team_balancing.cpp, mixed matched/todo)
---------------------------------------------------------
Greedy: largest party first onto the smallest team, then pairwise party swaps (same flag) minimizing
player-count spread, then cost 7*total_skill_spread + 3*rank_spread; deterministic; up to 4 teams.
Unranked: if balancing with parties kept fails, retries without parties (0x71730 draft).
Remake split() matchmaker.rs 384-405: brute force 2 teams, count then summed level, random tie.

7. Countdowns and lobby
-----------------------
Matchmade: c_session_state_start_match::update 0x72700 (TODO, unknown_058dd0.cpp ~1795): 10 s
  countdown once every member is ready; restarts if someone is not. Strings ":%02d".
  Remake: LINK_WAIT 20 s (server/matches.rs 43, 404) and viewer PREGAME 8 (online/matches.rs 29).
Custom game lobby: function_19a78e (MATCHED, unknown_19987f.cpp 2037): leader start -> countdown
  (caller passes 10); pressing again while running -1 s down to minimum (3); function_19a7e9 (MATCHED)
  leader cancels, member re-starts it (delay, name shown via name_of_last_person_to_delay_countdown).
  Remake menu.rs 1409: START GAME starts at once, no countdown; lan.rs LAN_WAIT is a load wait.

8. Host selection / host loss
-----------------------------
function_619b0 (TODO, unknown_059ad0.cpp 3668): candidate A beats B if: more peers reachable
  (bit count) > fewer local players > score = (bw_A - bw_B)*1.0172526e-5 (bw capped value84[players])
  + signed ((ping_B - ping_A)*0.0125)^2 > 0.3. function_61ac0 (TODO) host choice; host handoff
  messages (unknown_0acc20.cpp). network_session_host_lost 0x62ab0 (TODO) -> election state 9;
  timeouts value1474 3000 (proposal), 1478 10000, 148c 4000, 1490 6000, 1494 1000 (election).
In-game: simulation failure -> host back to lobby, clients leave (0x72700 area, TODO).
Whether a matchmade game counts when host leaves: NOT determined (0x741f0 TODO; arbitration not read).
Remake: host chosen by server, fastest RTT with 10 ms tie -> bigger party, 3-match penalty after
  failing to host (matchmaker.rs 40-46, 408-425); host leaving voids the game, host quit = loss
  (server/matches.rs 62-63, 707, 867); no migration.

9. Ranks shown
--------------
function_149ead (unknown_1490ec.cpp): rank icon = index of the wigl+0x158 range containing skill;
  unranked hopper -> no icon. Vista wigl matches. Remake rank.rs: level k -> icon k-1, same result.

10. Voice (network_voice.cpp)
-----------------------------
voice_update_mode 0x544e0; team rule 0x57b60; juggernaut/dead rules 0x57270/0x57c20/0x577d0;
proximity 0x57080 (TODO): within motion sensor range (game option, default 8.0) + 1.5 wu.
Dead players hear/talk only to dead (per mode). Voice host chosen by quality (0x53e30).
Remake: no voice (input.rs 188). Vista: GFWL voice, same rules presumably; PC mic handling differs.

Not decompiled / not determined
-------------------------------
- Real Live values (network_configuration.dat, hopper list): not in binary.
- 0x66330 defaults draft (TODO), 0x74800 averaging (TODO), 0x741f0 post-game (TODO), 0x71730 gathering
  (TODO), 0x7e210 party summary (TODO), 0x193c70/0x1936e0 hopper builders (TODO), 0x619b0/0x61ac0/
  0x62ab0 host choice/loss (TODO), 0x72700 start-match (TODO), 0x57080 proximity voice (TODO).
- Arbitration (result agreement) and quit penalties: not located.
- Leaver placement in FFA/team results: not located.
