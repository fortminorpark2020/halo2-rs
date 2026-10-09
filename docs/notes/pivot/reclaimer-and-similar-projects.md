# Angle: what "Project Reclaimer" (Halo 3) is, and similar projects

Researched 2026-10-09. Every claim has a source URL. "Inference" marks my own reading.

## 1. Which "Reclaimer"?

Several unrelated things share the name:

| Name | What it is | Source |
|---|---|---|
| **Project Reclaimer** (projectreclaimer.dev) | The Halo 3 standalone launcher John means. Fan project, first public builds late Sept 2026 | https://projectreclaimer.dev/ |
| Reclaimer by Gravemind2401 | C#/.NET asset viewer/exporter for Halo .map/.module files (GPL-3.0). A modding tool, not a game | https://github.com/Gravemind2401/Reclaimer |
| `reclaimer` Python library (Sigmmma / MEK) | Tag-definition library for Halo 1 tools (GPL-3.0) | https://www.pypi.org/project/reclaimer/2.9.3/ |
| The Reclaimers Library (c20) | Halo modding wiki | https://c20.reclaimers.net/h2/tools/h2-ek/ |
| "Project Reclaimer" on IndieDB | Deleted Unreal Engine Halo 4 fan remake by GigenMeister; unrelated | https://www.indiedb.com/games/project-reclaimer |

## 2. Project Reclaimer (Halo 3)

**Who runs it.** No names anywhere. The site and legal page say "volunteers" and an "independent fan project"; contact is admin@projectreclaimer.dev; the GitHub org "ProjectReclaimer" has "no public members". News coverage (Dexerto, PPE.pl) names no developer and only cites a promoting post by Mint Blitz (@MintBlitz).
- https://projectreclaimer.dev/legal.html
- https://github.com/ProjectReclaimer
- https://www.dexerto.com/halo/halo-3-mod-quadruples-multiplayer-matches-with-64-player-battles-3415552/
- https://www.ppe.pl/news/429275/halo-3-wraca-w-wielkim-stylu-totalny-chaos-na-serwerach-w-najlepszym-wydaniu.html

**What it is.** "A standalone launcher for Halo 3 from Halo: The Master Chief Collection." It "loads Halo 3 from your own Steam installation while it runs", never changes the install, runs Halo 3 "in its own window with its own menus" (you don't go through MCC's menus or matchmaking), and gameplay is "run by Halo 3's own engine". It says it is "not a faithful restoration". https://projectreclaimer.dev/

**How it is built (what is stated):**
- Written in **Rust (edition 2024)**. https://projectreclaimer.dev/download.html
- "Each release is built for one version of Halo 3's engine file"; after a Steam update that changes the engine file, the client reports "game version isn't supported". https://projectreclaimer.dev/ , https://projectreclaimer.dev/play.html
- The dedicated server is the same exe (`project-reclaimer.exe dedicated`); hosts copy from their own MCC install into `game/`: `halo3/halo3.dll`, `halo3/maps/shared.map`, `campaign.map`, `040_voi.map` and the maps they host. Linux hosting is Docker running the Windows server under Wine (+ xvfb). https://projectreclaimer.dev/host.html
- Its own menus "need OpenGL 3.0 or later"; a `RECLAIMER_UI_COMPOSITION=window` option exists for Proton/Wine. https://projectreclaimer.dev/play.html
- **Inference (not stated by the project):** it loads MCC's `halo3.dll` engine module in its own process (the way MCC's Unreal shell does) and draws its own OpenGL UI over it. Evidence: the server needs `halo3.dll` copied, releases are pinned to one engine-file version, and gameplay is "Halo 3's own engine". A public precedent for exactly this (Opus, below) exists. It is NOT a reimplementation and NOT a leaked build.

**What users must own / what is distributed.** A Steam copy of MCC with Halo 3 Campaign and Multiplayer; every player and every server host needs their own copy. "Project Reclaimer includes no game files"; releases contain only the client exe (and a server-only exe, RCON client, Docker image). Microsoft Store/Game Pass MCC is not mentioned. Free, non-commercial, not affiliated with Microsoft/Halo Studios/Bungie; names used in plain text only, no logos. https://projectreclaimer.dev/ , https://projectreclaimer.dev/legal.html , https://projectreclaimer.dev/play.html

**Online features.** Up to 128-player games (dedicated servers hold 127), 4-player online split screen, 16-player campaign co-op, dedicated servers, server browser over federated master servers (TCP 49175, signed listings, NAT help), host-set playlists with lobby voting (3 random choices + "None of the above"), JSON/Megalo scripted modes, Forge with AI, Steam Workshop mods, own anti-cheat and bans, voice/text chat. https://projectreclaimer.dev/ , https://projectreclaimer.dev/host.html , https://projectreclaimer.dev/masters.html
- **No matchmaking:** "Matchmaking isn't part of Project Reclaimer: multiplayer is custom games and Forge on community servers." https://projectreclaimer.dev/
- **Ranks:** games on servers that report to the project's public stats service count toward a Halo 3 rank "from Recruit up to General" with EXP and a skill meter (added v0.8.6). https://projectreclaimer.dev/play.html , https://github.com/ProjectReclaimer/project-reclaimer-releases/releases
- **Accounts:** no email/password; "a nickname plus a cryptographic key stored on your PC". https://projectreclaimer.dev/play.html

**Repositories and status.** Source is private ("Releases only; the source isn't published"), with a stated goal to open-source it, no date. Releases: https://github.com/ProjectReclaimer/project-reclaimer-releases (v0.8.4 on 27 Sep 2026 → v0.8.13 on 30 Sep; site now says 0.9.11; v0.9.0 on 3 Oct 2026 per WindowsForum). RCON client: https://github.com/ProjectReclaimer/project-reclaimer-rcon-client . Server image: ghcr.io/projectreclaimer/project-reclaimer-dedicated. Early development, "Expect bugs". https://projectreclaimer.dev/download.html , https://windowsforum.com/news/project-reclaimer-adds-64-player-halo-3-custom-games-on-windows.447125/

## 3. Public precedent for hosting an MCC engine DLL: Opus (Blam Creation Suite)

- Old BCS README: "**Opus** is a launcher framework for replacing the Unreal Engine loader for Master Chief Collection and allowing greater programming control over each of the game engines." GPL-3.0. https://github.com/twist84/Blam-Creation-Suite
- Code (fork, last commit 2020-09-03, supports MCC builds up to 1.1792): `game_runtime.cpp` does `LoadLibraryA(...)` then `GetProcAddress(m_game_module, "CreateGameEngine")`, `"CreateDataAccess"`, `"SetLibrarySettings"`. https://github.com/twist84/Blam-Creation-Suite/blob/2c07b999951562dd29ff61519174d9d19c2beaec/Framework/GameFramework/MCC/game_runtime.cpp#L42-L53
- `IGameEngine` gets a D3D11 device/swap chain (`InitGraphics`) and an `IGameEngineHost` (`InitThread`). https://github.com/twist84/Blam-Creation-Suite/blob/2c07b999951562dd29ff61519174d9d19c2beaec/Framework/GameFramework/MCC/opus_legacy/IGameEngine.h
- `IGameEngineHost` callbacks the engine calls on the host include `game_results_submission_handler`, `session_info_get`, `session_membership_update_handler`, `input_update_handler`, `network_sendto_handler`, `network_recvfrom_handler`: i.e. in MCC the engine DLL hands its network packets and session membership to the host. Vtable layout changes between MCC builds. https://github.com/twist84/Blam-Creation-Suite/blob/2c07b999951562dd29ff61519174d9d19c2beaec/Framework/GameFramework/MCC/IGameEngineHost.h#L219-L252
- There is a `c_halo2_game_host` for MCC's Halo 2 engine. https://github.com/twist84/Blam-Creation-Suite/blob/2c07b999951562dd29ff61519174d9d19c2beaec/Game/Halo2Lib/halo2_game_host.h
- The current BCS (ChimpsAtSea, Nov 2025) no longer has Opus in its README but still maps engines to `halo1.dll`, `halo2.dll`, `halo3.dll`, … https://github.com/ChimpsAtSea/Blam-Creation-Suite/blob/5b0acba63860dfd00aaf827fcac57fabad8bade1/framework/platform/shared/engine_platform_build.cpp#L695-L701

## 4. Similar projects for other Halo games and how each is built

| Project | Game | How it is built | Online | Users supply / distributed | Source |
|---|---|---|---|---|---|
| **Project Cartographer** | Halo 2 Vista (PC, 2007) | Replacement `xlive.dll` (exports the GFWL XLive ordinals) that "simulate[s] XLive libraries over System Link", plus hooks into the game; C/C++, GPL-3.0, active (last commit 3 Oct 2026) | Accounts, server list ("NETWORK"), dedicated servers with playlists; a stats/ranks API (CartoStat at halo2pc.com) exists in code but `StatsHandler::Initialize()` returns immediately ("TODO FIXME disabled for now") | Own Halo 2 Vista | https://github.com/pnill/cartographer , https://github.com/pnill/cartographer/blob/107ea5c0f7dc2260600f871555a18236be051ffd/xlive/xlive_exports.def , https://github.com/pnill/cartographer/blob/107ea5c0f7dc2260600f871555a18236be051ffd/xlive/H2MOD/Modules/Stats/StatsHandler.cpp#L23-L26 , https://www.halo2.online/help/install/ , https://cartographer.online/ |
| **Insignia** | Halo 2 on original Xbox (and other OG Xbox games) | Closed-source reimplementation of the Xbox Live **server** side by reverse engineering; the original, unmodified game client talks to it. Runs on real consoles and xemu | Halo 2 public beta since 15 Mar 2024: original matchmaking playlists (Team Slayer, Team Skirmish, Double Team, Team Snipers, Team Hardcore, H2 Challenge, Head to Head, BTB, Rumble Pit…), parties, clans, custom games; leaderboard of levels "sent by Halo 2"; PCRs captured but not yet shown on web | Own Xbox + Halo 2; TU5 and map packs served by Insignia | https://insignia.live/halo2 , https://en.wikipedia.org/wiki/Insignia_(Xbox) , https://kotaku.com/halo-2-multiplayer-servers-xbox-insignia-1851340292 |
| **ElDewrito** | Halo Online (cancelled 2015 Halo 3-engine F2P) | `mtndew.dll` injected into a patched `eldorado.exe`; hooks/patches; C++ | Server browser via ElDewrito-MasterServer, dedicated servers, stats/ranks in 0.7 | 2018: 343/Microsoft asked them to pause and to remove Halo Online data (no formal DMCA per Halopedia); 0.7 by a new team on 20 Apr 2024, launcher installs via P2P; 0.7 source status not stated | https://github.com/ElDewrito/ElDorito , https://www.halopedia.org/ElDewrito , https://www.gamespark.jp/article/2018/04/26/80340.html , https://github.com/eldewrito2/ElDewritoLauncher , https://cal1.lr.ggtyler.dev/r/HaloOnline/comments/1cuwpnf/eldewrito_071_release |
| **OpenCE** | Halo: CE (Xbox) | Native port built from a decompilation (bnunu/halo-1, fork of punpckhdq/halo) of Xbox build 2342; Windows/Linux/Android | System link up to 128 players, internet via invite link, no project server needed; no matchmaking | Own Xbox disc image | https://github.com/OpenCommunityEdition/OpenCE , https://fangamesdb.com/games/opence-halo-combat-evolved.html , https://www.techspot.com/news/114042-you-can-now-play-original-halo-browser-128.html |
| **Halo 2 decompilations** | Halo 2 (Xbox retail) | Matching decompilation, 28.61% of bytes matched; native port "not yet started"; CC0. Also BirchWoodGod/halo2-decomp (functional recovery) | Network session/bitstream code partly matched; no Live/matchmaking mention | Own XBE | https://github.com/kirklandsig/halo2-decompiled |
| **Halo 3 Delta Recomp** | Halo 3 pre-release "delta" Xbox 360 build (8 Mar 2007) | Recompilation of `halo3_cache_release.xex`; BSD-3-Clause; source only | — | Own files | https://github.com/twist84/halo3_cache_release_recomp , https://fangamesdb.com/games/halo-3-delta-recomp.html |
| **Halo 5 Reforged** | Halo 5 | "project-authored PC runtime built around Halo 5: Forge" using user-supplied Halo 5 files; no public build | Internet co-op in dev; no matchmaking described | Own files, never bundled | https://h5reforged.dev/ |
| **AlphaRing** | MCC (all titles) | C++ mod injected into MCC, adds local split screen; needs EAC off; builds tied to MCC versions (e.g. tag 1.3528.0.0); archived 6 May 2026 | MCC custom games only, no matchmaking | Own MCC | https://github.com/WinterSquire/AlphaRing , https://steamcommunity.com/app/976730/discussions/0/720116202450494672 |
| **Halomods Launcher** | MCC | Replaces `mcclauncher.exe` to start MCC with or without EAC | — | Own MCC | https://github.com/Halo-Mods/Launcher |
| **H2 standalone build (official)** | Halo 2 MCC Editing Kit | `halo2_tag_test.exe`, a tag build; "doesn't include network functionality"; won't load MP levels | none | Needs H2A on Steam | https://c20.reclaimers.net/h2/tools/h2-ek/h2-standalone-build/ , https://www.halowaypoint.com/news/halo-2-and-halo-3-mod-tools-release |

MCC itself: with EAC off "the public matchmaking features will be unavailable"; modded content is allowed in Custom Games and campaign. https://support.halowaypoint.com/hc/en-us/articles/360037475251-How-to-Launch-Halo-The-Master-Chief-Collection-with-Easy-Anti-Cheat-EAC-Disabled

## 5. What this means for a "Reclaimer for Halo 2"

- The Reclaimer model = own exe + MCC's engine DLL from the user's Steam install + own menus, servers, master list and stats service. For Halo 2 that is MCC's `halo2.dll` (Halo 2 Classic engine) + `halo2/maps/*.map` (inference; the H2 equivalent of what Reclaimer copies).
- Opus shows the MCC engine DLL interface already routes packets and session membership through the host (`network_sendto_handler`, `network_recvfrom_handler`, `session_membership_update_handler`) and submits results (`game_results_submission_handler`). That is the hook point where an h2live-style matchmaker, party system and 1-50 levels could sit. Interface details are from 2020 builds and are unverified for current MCC.
- No public project gives Halo 2 on PC its original matchmaking + 1-50 ranks. Reclaimer has none for Halo 3. Cartographer's ranks are switched off in code. Insignia has it, but only on the original Xbox / xemu.
- Costs of the MCC-DLL route: every MCC patch can break it (Reclaimer and AlphaRing pin to one engine version); Steam-only so far; closed reverse-engineering of undocumented vtables; no source from Reclaimer to reuse.
- What carries over from halo2-rs: h2live (matchmaking, parties, playlists, levels 1-50, host choice, relay), the master/accounts ideas, Rust toolchain (same as Reclaimer), blam-cache for reading MCC H2 maps (format differences between H2 Vista and MCC H2 maps are unverified here).

## Unknowns

- Who develops Project Reclaimer; whether it really hosts `halo3.dll` the way Opus did (strongly implied, not stated); how it renders menus over the engine (OpenGL 3.0 overlay is inferred).
- Whether Reclaimer will ever support Halo 2 (nothing found).
- Whether MCC's `halo2.dll` still contains Halo 2's original Xbox Live matchmaking UI/flow, or whether MCC's Unreal shell replaced it (not researched here).
- Current `IGameEngine` / `IGameEngineHost` layout for MCC builds after 2020.
- Microsoft's stance on Reclaimer specifically: no statement found.
