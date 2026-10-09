# Original Halo 2 (Xbox Live 2004-2010) matchmaking and ranks, who has them today, and h2live

Angle: what the original matchmaking and 1-50 experience was, which revivals/ports have it, and
how far h2live (crates/h2live) already is. Researched 2026-10-09. web.archive.org was blocked by
the session's proxy, so Bungie's own archived pages were read through mirrors (noted).

## 1. What the original had

### Playlists, Quickmatch, OptiMatch
- Halo 2 manual p.14: "Quickmatch picks a random game type, map, and the fastest service";
  "OptiMatch: Choose a matchmaking playlist and you'll get matched with other players who select
  that same playlist"; "As you get better at a playlist, your level increases"; "A Custom Game has
  no effect on any of your Xbox Live Halo 2 levels."
  https://www.manualslib.com/manual/378770/Microsoft-Game-Studios-Halo-2.html?page=14
- Halopedia describes Quickmatch differently ("immediately put the player in any available match
  from the most recent matchmaking playlist the player selected"). Conflict, unresolved.
  https://www.halopedia.org/Matchmaking
- Retail built-in (fallback) hoppers in the decompiled Xbox build: Rumble Pit (ranked FFA 4-8),
  Team Skirmish (ranked 2x3-4), Head to Head (ranked 1v1), Big Team Battle (ranked 2x5-8), Minor
  Clanmatch, Major Clanmatch, Team Training (unranked). Live replaced the list by download.
  /home/claude/halo2-rs/docs/notes/decomp/online-unverified.md (section 3; draft code, unverified)
- Dec 1 2004 update added ranked Team Slayer (parties up to 4) and unranked Rumble Training
  (parties up to 8, guests allowed); match times extended; different-sized parties allowed in BTB
  and Major Clan Match. https://www.gamespot.com/articles/second-halo-2-update-expands-matchmaking/1100-6114238/
- Double Team (ranked) added May 2005. https://halopedia.org/Double_Team
- Rumble Pit ranked from 2004 "until early 2007"; minimum raised to 6 on Aug 22 2006.
  https://www.halopedia.org/Rumble_Pit
- Playlists changed many times; there is no single "original" list. Pick an era.

### Parties, friends, recent players, clans (manual p.15)
https://www.manualslib.com/manual/378770/Microsoft-Game-Studios-Halo-2.html?page=15
- Friends list: up to 100; see friends, clan details, recent opponents; text or voice messages.
- Party: "a temporary group of friends, or other players you've just met", ends when everyone
  logs out. The party leader "decides where the group goes". Invites sent from the Pregame Lobby.
- Players list: current players plus up to 100 recent opponents/teammates; view profile, send
  message, give feedback on behaviour, send friend request.
- Clans: "semi-permanent", up to 100 members, one clan per player.
- Clan ranks Peon, Member, Staff, Overlord. https://www.halopedia.org/Clan_(social_group)
- Voice: proximity ("the louder the players' voices, the closer they are to you on a map"), manual
  p.13 https://www.manualslib.com/manual/378770/Microsoft-Game-Studios-Halo-2.html?page=13
- Xbox Live's 100-friend limit was reportedly kept because of Halo 2 (Aug 10 2009).
  https://www.shacknews.com/article/59918/report-halo-2-blamed-for

### The 1-50 levels (Bungie's "Halo 2 Stats Overview", mirrored)
Primary: https://www.podtacular.com/halo-2-stats-overview/ (mirror of
halo.bungie.net/stats/content.aspx?link=h2statoverview); cross-check https://halopedia.org/Rank_(Halo_2)
- One level per ranked playlist; none until played; 1-50; start at 1, most near 10.
- Friends, Clan and Recent Players lists show HIGHEST level; Pregame Lobby, Matchmaking screen and
  Postgame Carnage Report show the CURRENT PLAYLIST's level; clan playlists show the clan's level.
- Only final placement counts. XP is worked out against each opposing player and averaged; FFA =
  everyone their own team. Table (diff 0..15+): higher-level win 100,92,85,79,74,70,66,63,60,58,56,
  54,53,52,51,50; lower-level win 100,108,...,150 (mirror).
- Loss factor (low levels lose less): L1 0%, L5 10%, L10 40%, L20 77.5%, L29+ 100%.
  Win factor: L42 95, 43 90, 44 85, 45 80, 46 70, 47 65, 48 60, 49 55, L50 50%.
- Thresholds: L1 1-99, L2 100, ... L10 900, L17 2000, then +250 per level, L49 10000, L50 10250.
- A level is kept until XP drops below the midpoint of the previous level's requirement.
- Matching table ("Maximum Match"): L1 +10, L2 +9, L3 +8, L4 +7, L5 +6, L6 +5, L7-16 +6, L17-26 +7,
  L27-29 +8, L30 +9, L31 +10, L32 +11, L33 +12, L34 +13, L35 +14, L36+ up to 50. "Stretches these
  limits only when needed" (no published widening schedule).
- Parties beyond the limit: the lower player counts as the lowest acceptable level for matching and
  XP (L12 + L1 friend: friend counts as L6).
- Party balance table: Team Skirmish big party 3-4, imbalance 0; BTB 3-5, imbalance 2; Minor Clan
  3-4, 0; Major Clan 6-8, 2; Double Team 2-2, 0. Party size does not change XP.
- Drops: teams with at least one finisher rank above teams where everyone dropped; a dropper shares
  the team result; no bonus for winning short-handed. Guests cannot play ranked.
- Web stats were pulled one way from Live and could lag; trust in-game.
- Design: Max Hoberman, from Myth's 25-rank system, Elo-inspired; 25 ranks doubled to 50 in Aug 2004;
  top 7 ranks have special icons (49 a comet). https://halopedia.org/Rank_(Halo_2)

### TrueSkill
- Halo 2 did NOT use TrueSkill on Live per Bungie's tables above. Microsoft Research built TrueSkill
  using Halo 2 BETA data. https://papers.nips.cc/paper_files/paper/2006/file/f44ee263952e65b3610b8ba51229d1f9-Paper.pdf
  https://en.wikipedia.org/wiki/TrueSkill . Halopedia: "Halo 3 was the first Halo game to use
  TrueSkill in matchmaking." https://www.halopedia.org/Matchmaking
- Shacknews says Halo 2 used TrueSkill (contradicts the above; treat as wrong).
  https://shacknews.com/article/87755/halo-evolving-to-guardians?page=3

### Host, cheating, updates
- Original client computed XP/levels and wrote XP/level/highest to Live stats (decomp draft
  0x741f0, unverified); Insignia: "the levels shown are sent by Halo 2" (https://insignia.live/halo2).
- Cheating: standbying/lag killing, BXR; automated banning. https://en.wikipedia.org/wiki/Halo_2
  Updates: Apr 2005 standby fix, host-abuse limits, clan match changes, tie fix; Jul 20 2005 mod
  cheating bans; Apr 12 2007 TU5 (1.5) Blastacular + Banhammer.
  https://www.halopedia.org/Halo_2_(Xbox)_Auto-Updates
- Leaderboards (Top 1000) removed Oct 18 2005: "a shrine for cheaters and hackers".
  https://www.bungie.net/en/Forums/Post/214957
- Host migration: pre-release FAQ (Sep 2004) said another Xbox takes over if host drops
  (https://rampancy.net/halo2faq/hosting). Decomp shows lobby host election; in-game behaviour not
  confirmed. UNKNOWN.
- Original Xbox Live shut Apr 15 2010; last player online until May 10 2010.
  https://www.generationamiga.com/2026/08/09/how-one-halo-2-player-outsmarted-microsoft-for-nearly-a-month/
- Bungie.net Halo 2 detailed game stats purged June 2018; halo.bungie.net closed 2021-02-09; Archive
  Team kept samples (first/last 1M games, random 2M). https://wiki.archiveteam.org/index.php/Halo
  HaloArchive holds 801M Halo 2 matches (invite-only). https://windowscentral.com/gaming/halo/diving-back-into-bungie-era-halo-stats-with-halo-archive

## 2. Who has it today

| | Original matchmaking flow | 1-50 per playlist | Clans | Notes |
|---|---|---|---|---|
| Insignia (orig. Xbox / xemu) | Yes, original game code | Recorded; in-game use unclear | Yes | Public beta since 2024-03-15 |
| MCC Halo 2 | MCC's own shell, dedicated servers | Yes, Halo 2-identical, competitive playlists only, over hidden TrueSkill | No | |
| Cartographer (H2V) | No: server browser + dedicated servers | Optional per-playlist rank on registered dedis (code) | No | XLive over System Link |
| Halo 2 Vista retail (GFWL) | Server browser; Gold quick match | "doesn't appear to keep track of any player rankings" | No | Offline 2013-2015 |
| XLink Kai | System Link tunnel only | No | No | |

Sources:
- Insignia: https://insignia.live/halo2 ("All in-game functionality is working, with the web features
  expected as a future release"; Custom Games, Clans, Parties work; TU5 + all 4 map packs required;
  ranked/unranked playlists listed; scoreboard: "This leaderboard is not ranked in-game", "the game
  does not take this level into account when performing matchmaking ... or for your level shown on
  the Friends List"; no PCR web yet). Launch 2024-03-15: https://shacknews.com/article/139112/halo-2-online-matchmaking-public-beta
  Built by "closed-source reverse engineering of the original Live server software"; works with xemu:
  https://en.wikipedia.org/wiki/Insignia_(Xbox) , https://www.gfinityesports.com/news/og-halo-2-multiplayer-playable-via-insignia-xbox-live/
  xemu needs BIOS/MCPX dumped from your own Xbox: https://xemu.app/docs/required-files/
- MCC: 343 (Oct 29 2014): per-playlist, "identical to the leveling system from the original Halo 2",
  "same exact XP requirements". https://mp1st.com/news/343-industries-explains-halo-master-chief-collection-ranking-system
  Halopedia: used at launch, "now employed only in the competitive playlists"; Season ranks since Dec
  2019. https://www.halopedia.org/Rank_(Halo:_The_Master_Chief_Collection)
  "visual only ... sitting on top of an invisible Trueskill" (FyreWulff, community, 2014):
  http://carnage.bungie.org/haloforum/halo.forum.pl?read=1199030
  Halo 2 MCC population thin (Steam thread, Mar 2024): https://steamcommunity.com/app/976730/discussions/0/7056650139338405296
- Cartographer: xlive.dll replacement, own accounts/backend/server list
  (https://deepwiki.com/pnill/cartographer ; README "simulate XLive libraries over System Link",
  GPLv3, github.com/pnill/cartographer). StatsHandler.cpp calls halo2pc.com CartoStat API
  (PlaylistRanks, ServerRegistrationCheck, StatsEnabled) and sends a "rank_change" message that sets
  player_displayed_skill: ranks exist only on registered dedicated servers. Whether active now: unknown.
  .hpl = dedicated-server rotation playlist: https://github.com/TheUncutFighter/Halo2PlaylistCreator
- H2V: https://www.gamespot.com/reviews/halo-2-review/1900-6171591/ (May 25 2007);
  https://www.halopedia.org/Halo_2_Vista (servers off July 2015).
- XLink Kai: LAN tunnel, arenas: https://consolemods.org/wiki/XLink_Kai ; Halo 2 played "pretty much
  24-7" for 22 years: https://hitsave.org/keeping-the-dream-alive-twenty-two-years-of-free-online-gaming-with-xlink-kai/

## 3. h2live versus the original

Already the same as Bungie's published rules (crates/h2live/src/levels.rs):
- XP table EXPECTED/UPSET, thresholds min_xp (L50 = 10250), loss factor to L29, win factor L42-50,
  midpoint hysteresis, per-playlist XP and level, highest level, level_range = Bungie's table,
  effective_level = party rule, no guests in ranked, dropped players share their team's result.
- Parties (invite, open/invite-only, kick, promote, leader migrates), leader searches, Quickmatch,
  playlists from a text file, gathering countdown, team split by count then summed level, host by
  RTT, relay (no IPs exposed), result agreement, server-side XP (safer than the original, whose
  client wrote its own levels), signed stat cards, recent players (client file, 50), custom games.

Missing or different:
1. Friends list (100, with presence, invites, messages) - h2live shows everyone online instead.
2. Clans: 100 members, Peon/Member/Staff/Overlord, clan playlists with clan level.
3. Messages and player feedback.
4. Voice (proximity, team, dead rules) - none.
5. Big-party balance per playlist (min/max big party, imbalance) - not implemented.
6. Team where everyone dropped ranks last - places() ignores it.
7. Recent players 100 (manual) vs 50.
8. Quickmatch semantics differ (h2live: busiest playlist).
9. Level-range widening: h2live +3/15 s, any after 60 s; Bungie unpublished; decomp +1 per round.
10. Web stats (game history, carnage reports, per-playlist levels), as Bungie.net had.
11. Playlist eras: built-in list mixes launch and later lists.
12. Host migration (unknown in original), host choice (original used reachability/bandwidth/ping).
13. Hidden "network_configuration" values from Live are lost; FAQ values are the best record.

## 4. What this means for the pivot
- h2live's rules are the reusable heart of "original Halo 2 matchmaking and ranks" whatever game
  files the client reads. Keep it as the service; extend with friends, clans, voice, stats site.
- MCC already shows Halo 2's 1-50 numbers, so the selling point is Halo 2's own flow (party leader,
  OptiMatch playlists, Halo 2 lobby/PCR showing levels, clans, highest level on lists) and Halo 2's
  level-range matching instead of hidden TrueSkill, not the numbers alone.
- Insignia already gives the real thing on original Xbox/xemu; a PC-native standalone is the gap.
