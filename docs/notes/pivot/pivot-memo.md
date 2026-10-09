# Pivot memo: a "Project Reclaimer" for Halo 2

For John, 9 October 2026. This summarises six research notes, with links at the end. It is not legal advice.

## 1. What "a Reclaimer for Halo 2" means

Project Reclaimer is a free Halo 3 program by anonymous volunteers, written in Rust like ours. You run it instead of MCC:
- It loads Halo 3 from your own Steam copy of MCC and includes no game files.
- It runs the real Halo 3 engine in its own window, with its own menus.
- Each release works with one MCC version only, so MCC updates break it until a new release comes out.
- Its code is private, so we can't reuse any of it.
- It has servers and ranks, but says: "Matchmaking isn't part of Project Reclaimer."

For Halo 2, our program would load `halo2.dll` (the file that holds MCC's Halo 2 engine) and the maps from your MCC folder. The real engine would play the match. Our program would supply everything around the match: menus, the lobby, rank screens, the connection between players, and h2live's matchmaking and 1-50 levels. That fits how MCC works: MCC tells the engine who is playing and what to play, and the engine never searches for games itself.

**Proven:** hobby projects (Opus in 2020, HaloX this year) have run MCC's Halo 2 engine outside MCC for offline play. **Not proven:** nobody has shown an online Halo 2 match done this way. That is the make-or-break question.

Accept two things up front:
- **Feel.** It would play like Halo 2 Classic in MCC: the PC version at 60 frames per second, with MCC's weapon tweaks. That is not exactly the 2004 Xbox game, but it is far closer than our rebuild.
- **Legal.** MCC's licence agreement forbids almost exactly this. It bans users from hosting, providing or developing "matchmaking services for the Game(s)" and from using the game's code "outside of the MCC environment". Reclaimer leaves matchmaking out. I found no public response from Microsoft to Reclaimer, but it is only weeks old. Activision, now in charge of Halo development, has sent cease-and-desist letters to Call of Duty fan projects.

MCC already shows Halo 2's 1-50 numbers in its competitive playlists. What no PC version has is the experience around them: the party leader picking the playlist, levels shown in the lobby and the post-game carnage report, Halo 2's level ranges deciding who you meet, and clans and friends.

## 2. The options

**A. Our Rust engine, switched to read MCC's Halo 2 files**
- **Player gets:** our game, built from files you bought, plus Desolation and Tombstone, which Vista never had.
- **Feel:** only as good as our rebuild, and the feel work never ends.
- **Carries over:** nearly everything.
- **Unknowns:** MCC's new sound format, and whether MCC still has the fonts and menu art our main menu uses.
- **Legal risk:** lowest, because no Microsoft code runs.
- **Effort:** about 1-2 weeks of work on the map reader. But this is the "keep rebuilding" path you want to leave.

**B. Our launcher runs MCC's real Halo 2 engine, with our matchmaking (the Reclaimer model)**
- **Player gets:** the real engine plus Halo 2's original parties, playlists and levels. Nobody offers this on PC.
- **Feel:** real Halo 2 code, in its MCC Classic version.
- **Carries over:**
  - the h2live server (parties, matchmaker, playlists, relay, and level rules that already match Bungie's published ones; about 10,700 lines with tests);
  - the online protocol;
  - the lobby, menu and controller logic.

  Our renderer, HUD, sound and game simulation would mostly stop shipping.
- **Unknowns:**
  - whether two engines can join one online match without parts of MCC's own program;
  - how to get match results out of the engine to award XP;
  - whether `halo2.dll` still has Bungie's matchmaking screens (the Vista version does);
  - the engine's internals move with every MCC update.
- **Legal risk:** the highest of the realistic options.
- **Effort (rough guesses):**
  1. Offline match from our launcher: 1-3 weeks.
  2. Online match between two PCs: unknown. This is the go/no-go.
  3. Matchmaking, parties and results: several weeks.
  4. Halo 2-style lobby, rank screens, carnage report, friends and clans: several weeks.
  5. Then a fix after every MCC update.

  All testing happens on your PC, because our build servers can't hold MCC files.

**C. A mod inside MCC.** Mods need anti-cheat off, and MCC turns off matchmaking when it is off. Matches would have to be forced through MCC's custom games using parts of MCC nobody has documented. Not recommended.

**D. The original Xbox Halo 2 on the xemu emulator.** Insignia already runs the real 2004 matchmaking, parties, clans and playlists, and has since March 2024. xemu needs files copied from a real Xbox. Building our own would only copy Insignia. Worth playing, not building.

**E. Build on Project Cartographer (Halo 2 Vista).** It is open source, runs the real engine, and Vista's engine never changes, so updates never break it. But it uses Vista files, not MCC; Vista is no longer sold; it is C++; and its team would have to accept the work. It is a fallback if B's networking fails.

## 3. Recommendation

Do **B as a time-boxed experiment, with A as the fallback.** B is the only option that gives you what you asked for. The deciding question, one online match between two PCs, can be answered in weeks. If it fails we lose little: h2live and our engine stay intact, and A is a small step from today.

Because of the licence clauses, keep the repository private and the project free (no donations), use a name without "Halo", never include game files, and decide about going public only after the go/no-go.

**Claude, in the next few sessions:**
1. Make h2live work with any game client: its own version number, a small shared protocol library, MCC game types, and a fast relay mode for game traffic. This helps every option.
2. Write a check tool for you to run. It lists your Halo 2 folders and the `halo2.dll` version (file names only, nothing uploaded).
3. Start the launcher: load `halo2.dll` and start an offline match on Lockout.
4. The go/no-go: two launchers, one match, through our relay.

**You:**
1. Confirm you own MCC **on Steam** with Halo 2 installed. Reclaimer mentions only Steam; Game Pass is untested.
2. In Steam, right-click MCC and choose Manage, then Browse local files. Send me that folder path. It is usually `C:\Program Files (x86)\Steam\steamapps\common\Halo The Master Chief Collection`. What matters there is `halo2\halo2.dll` and the maps in `halo2\h2_maps_win64_dx11\`.
3. Run the check tool and paste its output.
4. Test each build. For the go/no-go, find a second PC with MCC on Steam; a friend's works.

## 4. Existing work

- **h2live:** keep it; every option is built around it. Hold off updating the Proxmox server to PROTOCOL 27 until we know which game client ships.
- **main and the five branches** (`controller-wip`, `menu-preview`, `feel-weapons`, `feel-combat`, `feel-bots`): all are pushed to GitHub.
  - Park them, tag main as a checkpoint, and stop merging and feel work until the go/no-go.
  - If B works, menu-preview's lobby, carnage report and menu logic move into the launcher.
  - If B fails, we pick up the PROGRESS.md plan again and add option A.
- **h2sim:** keep it as a stand-in game for h2live's automatic tests.
- **Campaign code** (about 9,300 lines): delete it later; no option uses it.

## Links

- Reclaimer: https://projectreclaimer.dev/ , https://projectreclaimer.dev/host.html , https://projectreclaimer.dev/play.html
- MCC licence: https://store.steampowered.com/eula/976730_eula_0
- Anti-cheat off disables matchmaking: https://support.halowaypoint.com/hc/en-us/articles/360037475251-How-to-Launch-Halo-The-Master-Chief-Collection-with-Easy-Anti-Cheat-EAC-Disabled
- Halo 2 engine run outside MCC: https://github.com/ChimpsAtSea/Blam-Creation-Suite , https://github.com/SpringContingency/HaloX
- MCC Classic feel and weapon tweaks: https://wiki.haloruns.com/Halo_2_MCC , https://c20.reclaimers.net/h2/
- MCC map format: https://github.com/XboxChaos/Assembly
- Bungie's rank rules: https://www.podtacular.com/halo-2-stats-overview/
- MCC's 1-50 rank: https://mp1st.com/news/343-industries-explains-halo-master-chief-collection-ranking-system
- Insignia: https://insignia.live/halo2
- Cartographer: https://github.com/pnill/cartographer , https://www.halo2.online/help/install/
- Activision: https://godisageek.com/2026/09/xbox-halo-activision-268-layoffs/ , https://www.videogameschronicle.com/news/activision-issues-cease-and-desist-to-modern-warfare-remastered-mod-that-shot-it-back-up-the-steam-charts/
- Full notes, in the same folder as this memo: `reclaimer-and-similar-projects.md`, `mcc-engine-hosting.md`, `mcc-vs-vista-map-format.md`, `original-h2-matchmaking.md`, `legal.md`, `reuse-inventory.md`, `check.md`.
