# Fact-check and completeness pass on the pivot research (2026-10-09)

Method: I re-opened the cited web pages (WebFetch; curl to projectreclaimer.dev is
blocked by the proxy, WebFetch works) and re-read the cloned reference repos in
`scratchpad/pivot/repos` (libmcc d203900 2025-11-21, HaloX e61750d 2026-05-01,
Blam Creation Suite twist84 2c07b99 2020-09-03 and ChimpsAtSea 535e3ce
2020-12-19, Cartographer 107ea5c 2026-10-03, Assembly, c20). Nothing in
/home/claude/halo2-rs was changed.

Verdicts: CONFIRMED, CORRECTED (with the correction), UNVERIFIED.

## 1. Project Reclaimer (Halo 3)

| Claim | Verdict | Notes / source |
|---|---|---|
| Standalone launcher for Halo 3 from MCC; own window, own menus; gameplay "run by Halo 3's own engine"; loads from the user's own Steam install and doesn't change it | CONFIRMED | https://projectreclaimer.dev/ |
| Written in Rust (edition 2024); source private, "goal is to eventually publish it as open source" | CONFIRMED | https://projectreclaimer.dev/download.html |
| Hosts copy `halo3/halo3.dll` + `halo3/maps/shared.map` (+ campaign.map, 040_voi.map for Forge) into `game/`; Docker image with Wine, "never game files" | CONFIRMED | https://projectreclaimer.dev/host.html |
| "Each release is built for one version of Halo 3's engine file"; after a Steam update "Wait for a Project Reclaimer release that supports the new version" | CONFIRMED | https://projectreclaimer.dev/ , https://projectreclaimer.dev/play.html |
| "Matchmaking isn't part of Project Reclaimer: multiplayer is custom games and Forge on community servers." | CONFIRMED (verbatim). Nuance: Host Game lists MCC's Halo 3 matchmaking-playlist *variants*, marked MATCHMAKING; that is variants, not matchmaking. | https://projectreclaimer.dev/ , https://projectreclaimer.dev/play.html |
| Stats service; Halo 3 rank "from Recruit up to General" with EXP and a skill meter; arrived v0.8.6 (27 Sep) | CONFIRMED | https://projectreclaimer.dev/play.html , https://github.com/ProjectReclaimer/project-reclaimer-releases/releases |
| Account = nickname + key on the PC; no email/password | CONFIRMED | https://projectreclaimer.dev/play.html |
| Up to 128 players (127 on a dedicated server), 16-player co-op | CONFIRMED | https://projectreclaimer.dev/ |
| Current version 0.9.11 | CORRECTED (minor): the site is inconsistent. Download page and Docker example say 0.9.11; home page banner says "New in 0.9.10" and FAQ says "0.9.9 at the moment". | download.html, host.html, / |
| Inference: it loads halo3.dll in-process like MCC's shell | UNVERIFIED (still inference; the site never says DLL). Supported by the file list and by Opus/HaloX doing exactly that. | host.html; Opus/HaloX code |
| Press coverage 3-5 Oct 2026; no Microsoft response | CORRECTED (minor): coverage continues to at least 8 Oct (Windows Central, Adam Hales). Still no Microsoft / Halo Studios / Activision statement found. | https://www.windowscentral.com/gaming/halo/halo-3-gets-128-player-multiplayer-project-reclaimer |
| Halo 2 support planned | No mention anywhere on the site or in coverage. | / , play.html, search |
| NEW: Steam need not be running to play | ADDED. "Steam has to be running, signed in with an account that owns MCC" is only in the "Through Steam Workshop" section; the guide says "You don't need to start the game through Steam." So Reclaimer apparently does not pass through a Steam/Xbox ownership gate in halo3.dll (inference), which matters for DMCA 1201 risk. | https://projectreclaimer.dev/play.html |

## 2. MCC engine DLLs run outside MCC

| Claim | Verdict | Notes / source |
|---|---|---|
| Engine DLLs export CreateGameEngine / CreateDataAccess / SetLibrarySettings; Opus loads `Halo2\halo2.dll` | CONFIRMED | bcs-twist84 `Game/Halo2Lib/halo2_game_host.cpp:49,81` (`c_game_runtime(_engine_type_halo2, "halo2", "Halo2\\halo2.dll")`) https://github.com/twist84/Blam-Creation-Suite |
| Opus network send/recv handlers were empty stubs | CONFIRMED | `opus_game_engine_host.cpp:720-735` return 0 |
| Opus kept per-build halo2.dll offsets for 11 builds 1477-1955 | CONFIRMED (ChimpsAtSea 535e3ce, 2020-12-19, `halo2_game_host.mainmenu.inl`: 1477,1499,1520,1570,1619,1698,1716,1829,1864,1871,1955). That is 11 halo2.dll builds within roughly one year of MCC PC patches. | https://github.com/ChimpsAtSea/Blam-Creation-Suite |
| Opus patched the map table to boot `scenarios\ui\mainmenu\mainmenu` | CONFIRMED as code; whether it actually booted is UNVERIFIED (the code is in a testing/mainmenu .inl, no result recorded). | same |
| libmcc host vtable has network_sendto_unreliable/reliable/recvfrom at 0x148/0x150/0x158; s_game_result 0x5D138; s_game_options 0x2BF30 with 17 peer addresses and 16 players (XUID, address) | CONFIRMED (counted back from the `// 0x198` marker in `game_manager.h`; static_asserts in `game_results.h`, `game_options.h`). | https://github.com/SpringContingency/libmcc |
| Flags include multiplayer, local-multiplayer and listen-server bits | CONFIRMED from HaloX `game_options_layout.h` (bits 3, 4, 6; "(flags & 0x48)==0x48 => online MP host"); libmcc itself only names bits 3 and 9. | https://github.com/SpringContingency/HaloX |
| HaloX loads halo2.dll + groundhog.dll; network send/recv return false | CONFIRMED (`game_instance_manager.launch.cpp:164-194`, `game_manager.network.cpp`). HaloX is a multi-title host (halo1/2/3/4/reach/odst). | HaloX |
| Engine game traffic goes through the host's send/recv slots, so a launcher can carry it | CORRECTED (weaker than stated): HaloX found this for **Reach only**, and its own notes say the slots are read only when peer type == 8 and "neither happens in halox's current launch path" (never exercised). HaloX's other note says MCC's session layer lives in MCC-Win64-Shipping.exe + halonetworklayer_ship.dll (`NetworkSession_GetCurrent` singleton), and its bridge to that is a "research scaffold" that would crash if called. So for Halo 2: plausible but unproven, and session setup may depend on MCC-side objects. | HaloX `src/network/engine_context_shim.h`, `src/network/mcc_network_bridge.h` |
| libmcc / HaloX have no licence file; BCS, Assembly, Gravemind Reclaimer are GPL-3; halo2-rs is MIT | CONFIRMED | repo checkouts |
| No public project runs networked halo2.dll outside MCC | CONFIRMED as far as searches go (no hits for a Halo 2 MCC standalone launcher; halo2mac.com is a macOS wrapper for Vista/Cartographer, not MCC). | searches; https://www.halo2mac.com/ |
| MCC still patched (17 Sep 2025 cross-play fix) | CONFIRMED, and there were more: Feb 2026 Exchange update, 9 Apr 2026 EAC backend update ("stealth", no download). Whether halo2.dll changed is unknown. | https://www.purexbox.com/news/2025/09/halo-the-master-chief-collection-gets-surprise-update-on-xbox-and-pc , https://www.purexbox.com/news/2026/02/halo-the-master-chief-collection-gets-surprise-update-on-xbox-and-pc , https://www.purexbox.com/news/2026/04/halo-master-chief-collection-deploys-significant-ban-wave-in-latest-update |
| MCC: EAC off => matchmaking disabled; matchmaking on 343 dedicated servers | CONFIRMED, plus: PCGamingWiki says self-hosted (custom) lobbies are peer-to-peer, i.e. halo2.dll already supports a player acting as host. | https://www.pcgamingwiki.com/wiki/Halo_2:_Anniversary |

## 3. MCC Halo 2 map format

| Claim | Verdict | Source |
|---|---|---|
| Version 10 header 0x1000; version 13 "Halo 2 MCC Update 1" header 0x380; empty build string; little-endian; module halo2; exe MCC-Win64-Shipping / MCCWinStore-Win64-Shipping | CONFIRMED | Assembly `src/Blamite/Formats/Engines.xml` lines 1207-1239; `Formats/Halo2MCC/LayoutsU1/H2MCC_LayoutsU1_Core.xml` |
| Compression in 0x40000 chunks; v13 chunk fields at 0x308-0x314, flags at 0x1C; v10 data from 0x3000 | CONFIRMED | Assembly `SecondGenSaberZLib.cs`, `H2MCC_LayoutsU1_Core.xml` |
| H2A MP (groundhog) is third-generation with Halo 4 fallback plugins | CONFIRMED | Engines.xml 1241+ |
| MCC hard-coded MP tag patches (BR error angle 0.1, melee, grenades, magnum 5.5, SMG 4.625, plasma rifle dual 0.7, AP turret) | CONFIRMED (c20 table). Note c20 places it under the MCC heading but says only "if the game executable uses cache files"; whether Xbox/Vista had them is not stated. | https://c20.reclaimers.net/h2/ |
| Halo 2 shared maps supported by MCC mods since the 19 Sep 2023 update; mod maps need .map and .dat files in `halo2\h2_maps_win64_dx11` | ADDED | https://github.com/Pepper-Man/mcc-build-scenarios-shared |
| HaloX: MCC Halo 2 text fonts are pre-rasterised in `halo2\h2_fonts\` | CONFIRMED as HaloX's loader path (`font_cache.cpp:131-138`); folder contents not seen. | HaloX |
| halo2_tag_test.exe "doesn't include network functionality and won't load multiplayer levels" | CORRECTED: c20 says it "doesn't include network functionality and it intended for testing single-player maps", "The UI works is largely functional", and its own load example is the multiplayer map `scenarios\multi\halo\coagulation\coagulation`. Nothing says MP maps won't load. | https://c20.reclaimers.net/h2/tools/h2-ek/h2-standalone-build/ |

## 4. Original Halo 2 matchmaking, Insignia, MCC, Cartographer

| Claim | Verdict | Source |
|---|---|---|
| Bungie FAQ rules (L50 at 10,250 XP, Maximum Match ranges, party rule L12+L1 => 6, loss/win factors, midpoint retention, big-party table) | CONFIRMED | https://www.podtacular.com/halo-2-stats-overview/ |
| h2live levels.rs matches (min_xp(50)=10250, level_range, effective_level) | CONFIRMED | crates/h2live/src/levels.rs:87,108,307 |
| MCC's rank "identical to the leveling system from the original Halo 2", "same exact XP requirements", per playlist | CONFIRMED (29 Oct 2014) | https://mp1st.com/news/343-industries-explains-halo-master-chief-collection-ranking-system |
| Insignia Halo 2 public beta; 12 playlists; ranked ones = Double Team, Team Slayer, Team Skirmish, Team Snipers, Team Hardcore, H2 Challenge, Head to Head; TU5 + all 4 map packs; no web carnage reports | CONFIRMED | https://insignia.live/halo2 |
| "unclear whether in-game level matching works on Insignia" | CORRECTED (my reading, still not proven): each playlist is labelled "Players Ranked: YES/NO", the page says "All in-game functionality is working", and the disclaimer "This leaderboard is not ranked in-game. While the levels shown are sent by Halo 2, the game does not take this level into account when performing matchmaking..." is about the *website* leaderboard's level. So in-game per-playlist levels most likely work; no player report found either way. | https://insignia.live/halo2 |
| Insignia page "doesn't discuss xemu" | Note: the Halo 2 page itself doesn't; xemu support is from Wikipedia/players. | https://en.wikipedia.org/wiki/Insignia_(Xbox) |
| Cartographer CartoStat ranks "only on registered dedicated servers" | CORRECTED: Cartographer v0.6 did ship a per-playlist ranking system "based on" Halo 2's, on invite-only, manually activated ranked servers ("Player hosted matches will not have stat tracking at this time"). In today's code (107ea5c, 3 Oct 2026) `StatsHandler::Initialize()` returns at once ("TODO FIXME disabled for now"), so ranks are off in current builds. | https://www.halo2.online/threads/project-cartographer-v0-6-update-and-changelog.4211/ ; `xlive/H2MOD/Modules/Stats/StatsHandler.cpp:23-26` |
| Halo 2 Vista retail kept no rankings; dedicated servers; parties where the leader picks maps; servers off July 2015 | CONFIRMED (Halopedia; "quick match for Gold" from GameSpot not re-read) | https://www.halopedia.org/Halo_2_Vista |
| Halo 2 decompilation 28.61% matched, no port | CONFIRMED (796,701 of 2,784,283 bytes; matched code includes "network sessions, message codecs and the bitstream") | https://github.com/kirklandsig/halo2-decompiled |

## 5. Legal

| Claim | Verdict | Source |
|---|---|---|
| MCC EULA "Competing Service": Code Use ("...outside of the MCC environment"), Matchmaking ("Host, provide or develop matchmaking services for the Game(s), or intercept, emulate or redirect the communication protocols used by our Game..."), Unauthorized Connections; One Major Rule; no distribution of Modded Versions; Xbox Live account clause; no revision date | CONFIRMED verbatim | https://store.steampowered.com/eula/976730_eula_0 |
| Waypoint EULA FAQ: anything in retail MCC may be used for modding; no selling; take-down rather than blanket ban; points to Competing Service | CONFIRMED | https://www.halowaypoint.com/news/mccs-eula-the-faq |
| ElDewrito 2018: "built upon Microsoft-owned assets that were never lawfully released..." | CONFIRMED. The team said "There was no Cease and Desist, no DMCA" in 2018 (DMCA notices were in 2015). | https://kotaku.com/halo-online-fan-project-goes-on-hold-after-microsoft-in-1825540894 |
| Installation 01 allowed (June 2017) if non-commercial, no money | CONFIRMED | https://www.gamespot.com/articles/fan-made-halo-game-is-legal-and-development-can-co/1100-6451273 |
| bnetd: "a reverse-engineered matchmaking server broke the DMCA" | CORRECTED: the 8th Cir. (2005) affirmed that the EULA's anti-reverse-engineering clause was enforceable (not preempted) and that bnetd violated the DMCA because its Battle.net emulator skipped Blizzard's CD-key authentication, letting unauthorised copies play; the interoperability exception failed. It is about circumventing an access control, not about matchmaking as such. | https://en.wikipedia.org/wiki/Bnetd |
| Activision took over Halo development on 22 Sep 2026 | CONFIRMED (Game File report 22 Sep; PC Gamer 23 Sep: "Halo Studios is no longer making Halo"). Windows Central (7 Oct) reports ~30 staff left, citing a leaker (unconfirmed). | https://en.wikipedia.org/wiki/Halo_Studios , https://www.pcgamer.com/games/halo/halo-needed-to-change-but-activision-faces-a-huge-challenge-to-fix-it/ , https://www.windowscentral.com/gaming/halo/halo-studios-has-one-final-job |
| Activision C&D to H2M (15 Aug 2024; needed MWR base game) | CONFIRMED | https://www.videogameschronicle.com/news/activision-issues-cease-and-desist-to-modern-warfare-remastered-mod-that-shot-it-back-up-the-steam-charts/ |
| Cartographer installer includes the full game (low confidence) | UPGRADED to medium: the official help says "The entire install process, including game download, is covered here" and its repair steps say "Mount the Halo 2 ISO". | https://www.halo2.online/help/install/ |
| AlphaRing archived 6 May 2026 | CONFIRMED | https://github.com/WinterSquire/AlphaRing |
| Not re-checked (low impact): SM2/X Labs 2023, re3 settlement, MDY, Sega, 1201(f), EU Art. 8, 37 CFR 201.40, TrueSkill patent expiry, Project Misriah, Spartan Survivors, OpenCE, Halo CE browser port, ElDewrito 0.7. | UNVERIFIED by this pass | as cited by researchers |

## 6. Reuse inventory

| Claim | Verdict |
|---|---|
| Line counts (81,446 total; per crate) | CONFIRMED within 0.2% (this checkout: 81,323; blam-cache 8,178, h2sim 18,325, h2net 7,905, h2live 10,714, h2viewer 32,094, h2tool 1,916, wma 2,191) |
| PROTOCOL = 26 on main | CONFIRMED (`crates/h2net/src/lib.rs:37`) |
| h2live uses h2sim only for Look, Reader/Writer, GameType, clean_name, Game (client results) | CONFIRMED (grep of crates/h2live/src) |

## 7. Added findings (questions that decide the pivot)

### A. Halo 2's own OptiMatch / matchmaking code survives in the PC code line
Cartographer's reverse-engineered headers for Halo 2 Vista name, in the shipped game code:
- `e_session_protocol`: splitscreen/system-link/xbox_live coop and custom, and `_session_protocol_xbox_live_optimatch` (`xlive/Blam/Engine/interface/user_interface_networking.h:5-14`);
- screens `_screen_optimatch_hoppers_fullscreen`, `_screen_optimatch_hoppers_lobby` (`user_interface_widget_window.h:243,261`) and a pregame-lobby `_pregame_pane_optimatch`;
- life cycle `_life_cycle_state_matchmaking` and `c_game_life_cycle_handler_matchmaking` (`networking/network_game_definitions.h:16`, `networking/logic/life_cycle_manager.h:67`);
- UI errors such as confirm party leader leave matchmaking, matchmaking failure missing content / membership changed / no games / timeout (`interface/user_interface_errors.h:147-273`);
- squad settings items change hopper, switch to optimatch, party management (`interface/screens/screen_squad_settings.cpp:192-227`).
Source: https://github.com/pnill/cartographer (commit 107ea5c). MCC's classic Halo 2 is a port of the Vista version (https://c20.reclaimers.net/h2/ ; https://wiki.haloruns.com/Halo_2_MCC), so the same code very probably sits in MCC's halo2.dll, possibly dormant. UNVERIFIED for halo2.dll.
Why it matters: the original Halo 2 matchmaking screens (playlists, pregame lobby with levels, party management) may not need to be redrawn; a launcher or a Cartographer-style shim could feed them from h2live. This is what Insignia does on the Xbox, from the server side.

### B. A pivot option nobody evaluated: build on Project Cartographer (Halo 2 Vista)
- It is open source (GPL-3, C/C++), active (last commit 3 Oct 2026), runs the real Halo 2 engine, and already has accounts, an online server list, dedicated servers and .hpl playlists. Sources: https://github.com/pnill/cartographer ; https://github.com/TheUncutFighter/Halo2PlaylistCreator
- It once had per-playlist Halo 2-style ranks (v0.6), now switched off.
- The Vista exe is frozen (last patch 1.00.00.11122, c20), so offsets never move; MCC's halo2.dll changed 11 times in ~2020 alone (Opus tables).
- Against it: it uses Vista files, not MCC (John asked for MCC); Vista is no longer sold and Cartographer's install "includes game download", so file provenance is weaker than "your own Steam MCC"; C++ only; Cartographer's contribution rules require a dev_preview account, 2-developer approval for big changes and 16-player tests (README).
- It also reaches h2live directly: h2live could become the matchmaking backend behind Cartographer's emulated XLive session layer.

### C. A halo2.dll host plays like MCC Halo 2 Classic, not Xbox Halo 2
- MCC classic is the Vista port at 60 fps. Only Vista's superbounces exist; double shots are harder because of the frame rate; sword flying was missing at release and restored in 2018 (https://wiki.haloruns.com/Halo_2_MCC).
- Button combos (BXR, BXB, double shot, quad shot) work in Classic, not Anniversary (https://steamcommunity.com/sharedfiles/filedetails/?id=1930409469).
- Hard-coded MP tag patches apply at load (c20).
- So "feel exactly like Xbox Halo 2" (PROGRESS.md goal) is not what the real-engine route delivers by default; it delivers MCC/Vista feel. John should accept that trade.

### D. Halo 2 in MCC already supports a player-hosted game
MCC custom-game lobbies are peer-to-peer (PCGamingWiki), so halo2.dll can act as listen host and client. Original Halo 2 Live matches were console-hosted too, and h2live already picks a host. A dedicated server is optional, not required.

### E. No Steam/Xbox sign-in gate seen in Reclaimer's model
Reclaimer runs without Steam running (Workshop excepted) and with no Microsoft account (play.html). If halo2.dll behaves like halo3.dll, a Halo 2 launcher would not need to bypass an ownership check. That is the line bnetd and MDY v. Blizzard drew under DMCA 1201. Inference.

### F. Patch churn is real but may slow
MCC got client or backend updates in Sep 2025, Feb 2026 and Apr 2026. Halo is now with Activision, and Halo Studios is reportedly down to about 30 people. That could mean fewer MCC patches and more stable offsets; this is speculation and unconfirmed.

### G. Cheapest feasibility probes (all need John's PC, none ship game files)
1. Confirm he owns MCC on Steam, with Halo 2 installed. Reclaimer supports Steam only; nothing confirms Store or Game Pass copies work.
2. List `...\steamapps\common\Halo The Master Chief Collection\halo2\` and `h2_maps_win64_dx11`, and record the halo2.dll version. This settles the "is there a mainmenu.map / h2_fonts" disagreement.
3. Once blam-cache reads v13 maps: run an `h2tool ui` listing on MCC's Halo 2 mainmenu.map (or shared.map) and look for optimatch hopper / pregame lobby screen widgets. This checks finding A against MCC data without disassembly.
4. Go/no-go proof of concept: launch two networked halo2.dll instances, using libmcc/HaloX only as reference.

## 8. Remaining unknowns
- Whether MCC's halo2.dll still contains, and can run, the OptiMatch / matchmaking life cycle without XLive. Cartographer proves it for Vista only.
- Whether halo2.dll's networked session can be set up purely from s_game_options plus host send/recv, or needs MCC-side session objects (HaloX's Reach notes point both ways).
- How Reclaimer does networking, sessions and ranks for halo3.dll. It is closed source, and it has no stated Halo 2 plans.
- The current halo2.dll build, its host-vtable layout and the s_game_result layout.
- Microsoft's or Activision's stance on Reclaimer, or on any MCC-DLL launcher with its own matchmaking (the EULA's Matchmaking and Code Use clauses apply on their face).
- Whether Insignia's in-game levels move and drive matching. Likely yes from the page wording; unconfirmed by players.
- Whether Store/Game Pass installs can be used.
- Whether the MCC hard-coded tag patches match Xbox title-update values.
- Whether Cartographer's team would accept a matchmaking feature, or whether h2live could sit behind its XLive emulation.
- John's own MCC ownership and which store he bought it from.
