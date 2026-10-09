# Angle: running MCC's Halo 2 engine (halo2.dll) standalone with our own matchmaking

Researched 2026-10-09. Repos cloned read-only under `scratchpad/pivot/repos/`
(bcs, bcs-full, alpharing, libmcc, halox, assembly).

## 1. How MCC runs Halo 2 on PC

- MCC's main program is an Unreal Engine 4 shell (`MCC-Win64-Shipping.exe`, Store build
  `MCCWinStore-Win64-Shipping.exe`) that does menus, input and customization; each game is a
  separate engine DLL. Sources: PCGamingWiki H2A page (UE4 for main menu/customization UX,
  Scaleform in-game) https://www.pcgamingwiki.com/wiki/Halo_2:_Anniversary ; c20 H2 page
  ("Unreal Engine serves as a menu and input layer") https://c20.reclaimers.net/h2/ ;
  Assembly Engines.xml `<executable>MCC-Win64-Shipping</executable>` `<module>halo2</module>`
  https://github.com/XboxChaos/Assembly (src/Blamite/Formats/Engines.xml).
- Halo 2 in MCC is two engines: `halo2\halo2.dll` (classic Halo 2, renderer + HUD + game
  thread, derived from the Vista port) and `groundhog\groundhog.dll` (Halo 2 Anniversary
  multiplayer and the Anniversary visuals, a Halo 4-generation engine). MCC initializes both
  when launching Halo 2 classic so the graphics toggle works.
  Sources: HaloX `src/game/game_instance_manager.launch.cpp` comments
  https://github.com/SpringContingency/HaloX ; PCGamingWiki ("Halo 2: Anniversary MP is stored
  in a separate game engine folder named groundhog"); c20 ("adds the secondary Saber3D engine");
  Assembly Engines.xml ("Halo 2 Anniversary MCC" uses module `groundhog`, fallback plugins Halo4).
- Engine DLL exports: `CreateGameEngine`, `CreateDataAccess`, `SetLibrarySettings`.
  Source: Opus `Framework/GameFramework/MCC/game_runtime.cpp` (BCS commit 535e3ce, Dec 2020)
  https://github.com/ChimpsAtSea/Blam-Creation-Suite ; HaloX `game_instance_manager.launch.cpp`.
- Engine interface (i_game_engine / IGameEngine): initialize_graphics(ID3D11Device, context,
  swapchain), initialize_game(host, game_options) -> returns the engine's game-thread HANDLE,
  post_message (pause, resume, quit, resize, boot player, team change), preload_common_begin,
  preload_level_begin(map), post_command ("HS: " script commands).
  Sources: libmcc `include/libmcc/game/game_engine.h` https://github.com/SpringContingency/libmcc ;
  Opus `opus_legacy/IGameEngine.h`.
- Host interface the shell must implement (i_game_manager / IGameEngineHost), ~60+ virtuals:
  begin_frame/end_frame(swapchain), set_game_state, save_game, **set_game_result**
  (opaque 0x5D138-byte results buffer), pause, game event manager (telemetry events incl.
  PlayerGameResults, BroadcastingKill/Death/MatchEnd, RankedStatsUpdate, MatchmakingHopper),
  variant save, get_map_info, video/audio settings, get_player_profile(XUID), input state
  (keyboard[256], mouse, XInput pad) and rumble, **network_sendto_unreliable /
  network_sendto_reliable / network_recvfrom(network_id, buf, len, port)**, folder paths,
  string tables, font glyph rasterization, emblem/skin drawing.
  Sources: libmcc `game/game_manager.h`, `game/game_results.h`, `game/game_event_manager.h`;
  Opus `IGameEngineHost.h` (older layout with session_membership_update_handler,
  session_info_get, game_results_submission_handler, network_sendto/recvfrom_handler).
- Launch struct (s_game_options, 0x2BF30 bytes): flags (multiplayer bit, listen-server bit,
  local MP bit), max players (1..16), team count, game mode (campaign/MP/ui shell/...),
  map id, game variant + map variant blobs, host secure address, **peer list (17 addresses)
  and player list (16 × XUID + address + team)**, local network id, saved-film path.
  MCC builds this; every engine copies it. The engine does not do matchmaking itself: it is
  told who is in the game. Sources: libmcc `game/game_options.h`; HaloX
  `src/game/game_options_layout.h`; Opus `game_options.h`.
- The vtable layout of the host interface moves between MCC builds (Opus tracked insertions
  "added in 1377/1629/1658/1896", relocations "after 1350"); Opus kept per-build offset tables
  for 11 halo2.dll builds (1477…1955). Source: Opus `IGameEngineHost.cpp`, Halo2Lib
  `halo2_game_host.fixes.inl`.
- MCC's own networking lives in `halonetworklayer_ship.dll` + `simplenetworklibrary-x64-release.dll`
  under the UE4 exe. Source: HaloX `src/network/mcc_network_bridge.h`.
- Files: classic H2 maps live in `halo2\h2_maps_win64_dx11\` (Steam guide
  https://steamcommunity.com/sharedfiles/filedetails/?id=2673977984 ; H2EK docs put built maps in
  `H2EK\h2_maps_win64_dx11\` https://learn.microsoft.com/en-us/halo-master-chief-collection/h2/guides/guidebuildcache ).
  `halo2\hopper_game_variants\*.bin` and `hopper_map_variants\*.mvar` exist (HaloX
  `game_instance_manager.local.cpp`). Fonts `<game>\maps\fonts\font_package*.bin` (HaloX
  `text/font_cache.cpp`). Audio for classic H2 is Miles (`mss64.dll`) per HaloX
  `game/halox_audio.cpp`. MCC H2 map format: Assembly engine "Halo 2 MCC" version 10 and
  "Update 1" version 13, little endian, zlib-compressed in 0x40000-byte chunks
  (`SecondGenSaberZLib.cs`), its own tag layouts (Formats/Halo2MCC). Our blam-cache reads H2V
  maps and would need these changes. NOT CONFIRMED: exact presence/names of shared.map,
  single_player_shared.map, mainmenu.map in MCC's halo2 folder (no source found).
- Map ids in the classic engine include every H2 MP map incl. Vista-era District/Uplift and
  Desolation/Tombstone (libmcc `scenario/scenario_map_id.h`).

## 2. Has anyone loaded halo2.dll outside MCC?

- **Opus (Chimps at Sea / Blam Creation Suite, 2020, GPLv3)**: standalone host for MCC engine
  DLLs. Had `Halo2Lib/halo2_game_host.cpp` that loads `Halo2\halo2.dll`, a load-crash fix and a
  patch to boot Halo 2's original `scenarios\ui\mainmenu\mainmenu`. Its network handlers were
  stubs (`return 0`). Removed in the Oct 2021 "Mega Refactor". Source: BCS history
  (commits 535e3ce, 6c4ae58) https://github.com/ChimpsAtSea/Blam-Creation-Suite
- **HaloX (SpringContingency, active to May 2026, NO LICENSE file)**: standalone host built on
  libmcc. Loads all MCC DLLs incl. halo2 + groundhog from the user's MCC folder, launches
  campaign and local MP with hopper variants, has keyboard remap hooks into halo2.dll's input
  tables, ImGui menus. Networking is an unfinished research scaffold (Reach only); its
  `network_sendto/recvfrom` are stubs. Source: https://github.com/SpringContingency/HaloX
- **Project Reclaimer (Halo 3, 2026, closed source)**: "A standalone launcher for Halo 3 from
  Halo: The Master Chief Collection", loads Halo 3 from the user's Steam install, 128-player
  custom games, dedicated servers on Windows or Linux (Docker + Wine) that copy
  `halo3/halo3.dll` + `halo3/maps/shared.map` + maps, master servers, a stats service where
  "players' ranks count" finished games, each release pinned to one engine-file version,
  "Matchmaking isn't part of Project Reclaimer". Sources: https://projectreclaimer.dev/ ,
  https://projectreclaimer.dev/host.html , https://projectreclaimer.dev/play.html ,
  https://projectreclaimer.dev/legal.html , Windows Central 2026-10-08
  https://www.windowscentral.com/gaming/halo/halo-3-gets-128-player-multiplayer-project-reclaimer ,
  WindowsForum https://windowsforum.com/news/project-reclaimer-adds-64-player-halo-3-custom-games-on-windows.447125/
- **AlphaRing (WinterSquire, archived May 2026)**: not standalone; a DLL proxy
  (`wtsapi32.dll`) inside MCC with patches for halo2 (e.g. `game_options_verify` patched) and
  halo3 (`can_accept_any_join_request`). https://github.com/WinterSquire/AlphaRing
- No public project found that runs **networked** Halo 2 (halo2.dll) multiplayer outside MCC.

## 3. Inference: where our matchmaking plugs in

(My reading of libmcc + HaloX, not stated by either.) libmcc's i_game_manager slots 41/42/43
are network_sendto_unreliable / reliable / network_recvfrom = byte offsets 0x148/0x150/0x158.
HaloX's Reach RE found the engine sends/receives through `g_engineContext` vtable +0x148/+0x158,
i.e. the host's own network slots. So the engine's whole game traffic goes through the host:
a standalone host can carry it over our own UDP/relay (h2live relay) using the peer ids it put
in s_game_options. Matchmaking, parties, playlists, levels stay entirely in our launcher +
h2live; the engine only runs the match and reports results/events back. HaloX notes the
remaining hard part is getting the engine into its MP session state with a real peer table
(Reach), which is what Project Reclaimer has solved for Halo 3 privately.

## 4. Constraints / risks

- MCC still gets patches (Sept 17 2025 cross-play matchmaking fix)
  https://www.purexbox.com/news/2025/09/halo-the-master-chief-collection-gets-surprise-update-on-xbox-and-pc
  -> pin to one halo2.dll build, detect version, update offsets per patch (Project Reclaimer does this).
- EAC: MCC with EAC off disables matchmaking (PCGamingWiki) — so "mod inside MCC" cannot give
  matchmaking; a standalone host is not under EAC at all.
- Halo 2 classic's own pregame lobby / Xbox Live matchmaking UI: NOT CONFIRMED to exist in MCC's
  halo2.dll; MCC uses UE4 for menus. Plan on building lobby/playlist/rank UI in the launcher.
- Results: s_game_result is an opaque 0x5D138-byte buffer in libmcc; layout undocumented.
  Event interface (PlayerGameResults, Broadcasting* events) may be easier; which events halo2.dll
  emits is unverified.
- Legal: 343 MCC EULA FAQ — "Anything contained within MCC is allowed to be used for modding",
  no selling; refers to the EULA's "Competing Service" and "Ownership" sections
  https://www.halowaypoint.com/news/mccs-eula-the-faq . Precedent: Microsoft action against
  ElDewrito distribution of Halo Online files (2018) https://elaztek.com/news/eldewrito-faces-legal-action-r14/
  Project Reclaimer ships no game files and requires the user's own Steam copy.
- Licensing for reuse: BCS/Opus GPLv3 (reusable with GPL); HaloX and libmcc have no license
  file (read for reference only, don't copy).
- Host code is native Windows x64 implementing C++ vtables (all reference code is C++). Rust can
  do it (repr(C) vtables, libloading) but it's more friction; Linux servers would need Wine.

## 5. Feasibility

Yes, technically feasible, and partly proven: halo2.dll has been loaded and run outside MCC
(Opus 2020, HaloX 2026, local play only), and a team has made the sibling halo3.dll fully
networked with dedicated servers and ranks outside MCC (Project Reclaimer, closed source).
Networked Halo 2 outside MCC has not been shown publicly; that is the main research risk
(months of reverse engineering of the host interface, session setup, results, per-build offsets).
