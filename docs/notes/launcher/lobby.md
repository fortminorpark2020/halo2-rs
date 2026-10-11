# The launcher's lobby (milestone 4)

Built on `launcher-lobby` (`crates/h2launch/src/lobby/`). Tested on Linux
with a local h2live and two lobbies on the stand-in engine, and on the
owner's PC (2026-10-10 07:45) with two headless lobbies playing a real
match on halo2.dll through the Proxmox h2live: Head to Head, Swords on
Gemini, one hosting and one joining, a 60 s time limit
(`H2LOBBY_ENGINE_ARGS=--set-option 0x354=i32:60`). Both engines said
running, maploaded and ended (the same results on both), the server counted
the game, both engines closed by themselves about 7 s after the end (the
engine's own restart_game(0)), and both lobbies showed the carnage report.
The sign-in window opened and drew with no errors.

What a player sees around a match, in the spirit of Halo 2's Xbox Live
screens: sign in, a party, the playlists with their levels, searching,
the pregame lobby, and the carnage report after the game. Halo 2's own
menus inside halo2.dll aren't used: driving them would need patches, which
the launcher doesn't do.

## Shape

- `h2launch` with no arguments (or `--lobby`) opens the lobby: a window of
  our own, on Halo 2's start screen. It signs in to h2live when the player
  picks ONLINE on the main menu, and stays signed in between matches
  (and on the main menu).
- For each match it starts a second copy of itself for the engine:
  `h2launch --session <file> --me <n> --map <m> --variant <v> --events`.
  The session file is written from LAUNCHER_MATCH (`live::session_from`).
  The child joins the relay room and plays the match exactly as `--live`
  does, but tells the lobby what the engine does on its standard output
  (`H2EVENT running`, `maploaded`, `ended <players>`, `closing`) instead of
  talking to the server. The lobby turns those into HOSTING, JOINED,
  LAUNCHER_RESULT and LEFT_MATCH with the same rules `--live` uses
  (`live::Told`), and shows the carnage report when MATCH_OVER comes.
- A fresh engine process per match: halo2.dll is loaded once per process,
  as MCC does, and a crash in a match doesn't take the lobby with it.
- The lobby writes `quit` on the child's standard input to close the
  engine as closing its window would: when the player leaves the game (B,
  then A), when the server ends the match before the game ended here, when
  the lobby closes, and 20 s after the game ended if the engine is still up
  (an estimate of its own postgame). A child that hasn't closed 10 s after
  being asked is stopped. Closing before the end says LEFT_MATCH.
- Double-clicked, the launcher lets go of the console window Windows gave
  it (its log is in `lobby.log`); started from a terminal it keeps that
  one. The engine's copy starts with no console (its output goes to the
  lobby), and closes when the lobby's pipe to it closes.
- A match that comes again with another host (the one asked didn't start
  hosting) stops the engine and starts it again on the new session.
- `--live` stays as it is, for tests. `--offline` starts an offline match
  on the engine, which used to be what no flags did.

## Drawing and input

- The window is drawn on the CPU into a pixel buffer (`softbuffer`). Text
  is in Halo 2's own fonts, read at start from the owner's MCC
  (`halo2\h2_fonts`, the same format as Halo 2 Vista's `maps\fonts`, read
  by `blam_cache::font`; checked on the owner's PC 2026-10-10): by size,
  conduit-9 for small print, Handel Gothic 11 for column headings,
  conduit-13 for text, Handel Gothic 13 (the main menu's font) for names
  and rows, and Handel Gothic 24 for titles; digits always from conduit,
  as Handel Gothic's 1 is a bare stroke like its I. The glyphs are scaled
  so their capitals are 0.65 of the text's size, as the system font's are
  (their ascent and descent leave different room above the capitals).
  Each glyph's image goes its origin x right of the pen, and the next one
  its advance past that (from the pen, L's foot runs into a following I);
  kerning pairs are code points, in the file's pixels. Characters
  they lack, and everything when MCC isn't found, come from a font
  already on the PC (Bahnschrift, then Segoe UI or Arial on Windows;
  DejaVu Sans or Liberation Sans on Linux) through `ab_glyph`.
  `H2LOBBY_H2FONTS` names another folder of them, or `off` for the system
  font alone. No font or art is shipped. A screen is cheap to draw, and
  the same code writes PNGs for tests without a GPU (with Halo 2's fonts
  those PNGs hold MCC's glyphs: keep them on the owner's PC).
- Layout is in units of 1/720 of the window's height, so it scales.
- Notices (the server's NOTICEs and the lobby's own) stay up 6 s, at most
  three at once, stacked above the button hints, newest lowest. They are
  drawn last and below every popup's box (y 524 to 652), so a popup
  neither hides nor dims them.
- Keyboard (arrows, Enter or Space for A, Esc or Backspace for B, the X
  and Y keys, Q or Page Up for LB and E or Page Down for RB), mouse (click
  a row to select it, again to pick it; click a button hint or a `<` `>`
  arrow) and Xbox controllers (gilrs: d-pad or left stick, both ways, A,
  B, X, Y, the bumpers LB and RB; Start is A and Back is B). Where the
  screens below say so, LB opens a service record (yours, or the selected
  player's) and RB the friends list. Where a gamertag is typed (the
  sign-in screen, the add friend popup) letters are typed, not buttons,
  and Backspace rubs one out. W A S D don't move the selection: A is a
  button.
- While the engine plays (the in-game screen), the lobby ignores the
  controller, the letter keys, and Enter, Esc and Backspace (A and B) on
  that screen and in its Leave Game popup: its window may still be in
  front if the engine's didn't come forward, and B then A (melee, jump),
  Esc (Halo 2's pause) then Enter, or Esc then Space would otherwise leave
  the game. Leaving takes a click on the B hint and then on the popup's
  A. The window's keys come through `App::key`, which does this; clicks,
  the controller and the headless script's `press` go to `App::input`.
- The engine's window should take the foreground when it opens (not yet
  seen on a PC): the lobby, being in front, passes the right to it
  (`AllowSetForegroundWindow` with the child's process id, at the start
  and again when the engine says running and maploaded), and the window
  asks for the foreground once a second for 30 s until it has had it,
  restoring itself first if it is minimised, and stopping if some other
  window than the one in front at the start (the lobby) is brought to the
  front. Keys and the mouse reach only the window in front; the
  controller works either way.
- The left stick moves the selection by its stronger axis only, so a
  sloppy diagonal push doesn't also change the value on a row.
- The start screen and the main menu are Halo 2's own (menu.md, phase 1),
  drawn by `crates/h2ui` into the same pixels in place of the lobby's
  chrome (no gradient, title, header or keyboard line): the still
  background (the navy and the framing; drawn once for a window size and
  copied in under each frame, `cpu::Kept`), then the screen. Their
  layouts and pictures come from the first `mainmenu.map` that gives
  pictures, looked for where the rank icons are (MCC's first, then the
  Halo 2 Vista folders, then `maps\mainmenu.map` beside the launcher);
  `H2LOBBY_MENU` names one map or its folder instead, or `off` for the
  flat look (plain shapes in Halo 2's colours, which is what Linux and
  the tests get). Their text is in Halo 2's fonts from the same folder
  as the lobby's, else in `h2ui`'s built-in font. The log says where each
  piece came from (lines starting `menus:`) and, once per window size and
  screen, how long a frame takes on the CPU averaged over its first 30
  frames there: `menus: 1920x1080, start screen 3.4 ms a frame on the CPU
  (background 20 ms once)`. Popups and notices are drawn over them as on
  every screen.
- Levels are drawn with Halo 2's own level icons (the 50 rank icons, big
  and small) from a `mainmenu.map` (`lobby/ranks.rs`). MCC's own comes
  first: `halo2\h2_maps_win64_dx11\mainmenu.map` in the MCC folder the
  launcher finds (as for `h2_fonts`), with its pixels in the
  `textures.dat` beside it (cache format 13, read by `blam_cache::mcc`;
  see `mcc-maps.md`). Then the three Halo 2 Vista folders the old h2viewer
  used, then `maps\mainmenu.map` beside the launcher. The first that reads
  is used, and the log says which (`lobby: rank icons from <path> (MCC's,
  cache format 13)` or `(Halo 2 Vista's)`) and why any before it failed.
  `H2LOBBY_RANKS` names one map or its folder instead (either format: the
  version word tells them apart), or `off`. Without icons, levels are
  numbers, as before. The icons are the game's: screenshots with them stay
  on the PC.
- Files in the launcher's folder (`%LOCALAPPDATA%\h2launch`, or its
  `--instance` folder): `lobby.txt` (the server, the gamertag and the
  controller settings: `button_layout`, `thumbstick_layout`,
  `look_sensitivity`, `look_inversion`, `auto_look_centering`, `vibration`,
  `mouse_sensitivity` and `mouse_inversion`; a file from before them reads
  with Halo 2's defaults, and unknown lines are skipped), `lobby.log` (the lobby's log, with the
  engine's lines as `engine:`), `live-key.bin` and `live-card.txt` (the same
  account as `--live`), `match-session.txt` (the last match's session),
  and `results\` (the engine's results blocks of the last 30 games,
  `H2LAUNCH_RESULT_DUMP` for the engine's copy, so the rest of the block's
  layout can be worked out from real games. Kills, assists and betrayals
  are read at offsets inferred from the order Halo games keep them in and
  not yet seen above zero; in a Slayer game the engine's log says whether
  each player's kills are their score plus their suicides, as they must
  be (`result: kills: ...`). Until they are seen right
  (`results::COUNTS_SEEN`), the server is sent kills only from a Slayer
  game that bears them out, and assists and betrayals as 0, so a wrong
  guess never goes into anyone's tally. They are the engine's data: never
  commit or upload them).

## Screens

0. Start and main menu. The lobby opens on the start screen: the logo
   and the pulsing PRESS START. A (Enter, Space, the controller's A or
   Start) or a click anywhere opens the main menu; B asks before quitting
   (Y there signs out, while signed in). The main menu has ONLINE and
   SETTINGS (the original Xbox wording; split screen and system link
   aren't offered), up and down (or the mouse wheel) move the focus,
   which fades as Halo 2's did, and a click focuses a row, or picks it if
   it had the focus (but not the second click of a double-click that
   opened the main menu: a double-click on PRESS START, which lies over
   ONLINE, stops on the main menu). The row picked last keeps the focus when the player
   comes back. The gamertag signed in (else the one saved) is the small
   print at the bottom right. ONLINE goes to the playlists when signed
   in, to the signing-in screen while that goes on, else signs in with
   the gamertag and server last signed in with (not what was typed in the
   form and left with B), or opens the sign-in form when there is none.
   SETTINGS opens the controller settings. B goes back to the start
   screen. If the server is lost while the player is on either screen, or
   in the settings opened from the main menu, they stay there and the
   reason is a notice; ONLINE signs in again. An invitation pops up
   over the main menu (A accepts and shows the playlists), and a match the
   party is put in opens the pregame lobby from either, as from any
   screen.
1. Sign in: type a gamertag the first time; kept in `lobby.txt` in the
   launcher's folder with the server's address. B goes back to the main
   menu, and so does B while signing in (which calls it off).
2. Xbox Live: the playlists (name, your level in it, people searching and
   playing) and the party (members and their levels in the playlist
   selected, or the one searched while searching; their highest on the
   custom game row, the custom game screen or an unranked playlist). PARTY gives the others'
   levels only in the playlist the party last searched or played, so for
   another playlist the lobby asks for the members' service records and
   reads the level there (level 1 in one they haven't played; "-" until
   the record comes). A searches (party leader only), X opens the players list, Y
   the party screen, RB the friends list ("Friends (2)" while two friend
   requests wait for an answer), LB your own service record, B goes back
   to the main menu (still signed in, the party kept).
3. Players: everyone signed in. A invites them, X joins their open party,
   Y sends them a friend request (unless they are a friend or a request is
   waiting either way), LB opens their service record, RB the friends
   list, B goes back. An invitation pops up over the party screens (the
   main menu, playlists, players, party, friends and service record): A
   accepts (from the main menu it then shows the playlists), B declines.
4. Searching: the playlist, the stage the server reports and the level
   range; B cancels. LB and RB still open the service record and the
   friends list, and the search goes on; B there comes back to it.
5. Pregame: the map, the game type and the players with their teams and
   levels for a few seconds, then the engine starts.
6. In game: the lobby window waits while the engine's window is up, saying
   whether the engine is starting, loading or playing. B leaves the game
   (while the engine plays, only a click on the B hint and then on A).
7. Carnage report: each player's place, team, score, kills, assists,
   deaths and level, in Halo 2 Xbox's columns (PLACE, PLAYER, SCORE,
   KILLS, ASSISTS, DEATHS, LEVEL), sorted by place, then team, then score,
   and the level changes MATCH_OVER gives. Until `results::COUNTS_SEEN`,
   a KILLS or ASSISTS column that is 0 for everyone says "-", as it may
   only mean the numbers weren't sent. Up and down select a player
   (starting on yours): LB opens their service record, Y sends them a
   friend request. A (or B) goes back to the party (the leader of a custom
   game goes back to the custom game screen, to pick the next).

8. Custom game: the row after the playlists, for the party leader. Left
   and right pick the game type (one of each kind the launcher playlists
   play, `names::CUSTOM_GAMES`) and the map (those on this PC), A starts
   it. The server (LAUNCHER_CUSTOM, `LIVE_PROTOCOL` 2) checks the game is
   one of its launcher playlists' and the map one every member has that a
   launcher playlist plays, then makes an unranked match for the party
   with the leader hosting and teams alternating down the party; from
   there it goes as a playlist's match does. It works alone too.
9. Party: the members, each one's level (as the party panel shows it: in
   the playlist selected on the playlists screen) and role (leader,
   member, you, guests) and whether anyone can join. The leader picks another member
   and makes them leader (A, PROMOTE) or removes them (X, asked first;
   KICK, after which they come back only if invited), and Y switches
   between open and invite only (PRIVACY). Anyone picks their own row to
   leave (A, LEAVE_PARTY), and LB opens the selected member's service
   record. It goes back to the playlists while the party searches.
10. Offline: the server couldn't be reached or was lost. A tries again, Y
   goes back to the sign-in screen, X opens the controller settings, B
   goes back to the main menu.
11. Friends (RB; `LIVE_PROTOCOL` 3, `docs/notes/launcher/live-v3.md`):
   requests to you first, then friends on the launcher (a green dot),
   friends on the game (a grey dot), friends offline, then the requests
   you sent, by gamertag within each, with "12 of 100" (friends and sent
   requests; Xbox Live 1.0 kept 100). Each friend's status says what they
   do ("In a lobby", "Searching Team Slayer", "Head to Head: Swords on
   Gemini", "Custom game: ...", "Online (h2viewer)", "Offline"), and the
   right side whether their party is yours, open to you, or invite only.
   On a request to you A accepts and X declines; on a request you sent X
   takes it back; on a friend X joins their party when it's open to you
   (otherwise it says why, as a notice: "DAN'S PARTY IS INVITE ONLY",
   "... IS FULL", "... IS IN A MATCH", "DAN IS OFFLINE" and so on; a join
   the server still turns down, on a flag a second old, gets the server's
   own notice), and A opens the options: Invite to party, Join party, Service record,
   Remove friend (only those that apply; removing asks first, and they
   aren't told). Y types a gamertag to ask (the keyboard is needed for
   that; with a controller alone, Y on the players list or the carnage
   report asks the player selected). LB opens the selected player's
   service record, B goes back to the playlists. A list comes with the
   notice of a friend action (the server sends it at once), and otherwise
   at most every second; the selection, and a popup naming a friend, stay on that
   player (a popup closes if they leave the list). The answers come as
   notices.
12. Service record (LB): the player's gamertag, highest level (over the
   launcher's ranked playlists: levels from h2viewer's don't count), totals over
   their ranked playlists (games, wins, kills, deaths, assists, K/D) and
   when they signed up, and a row per ranked playlist they've played (its
   level, games, wins, kills, deaths, assists and K/D; up and down scroll).
   The lobby keeps the last 32 it got and asks the server again only for
   one older than 60 s, showing the old one meanwhile; it waits 5 s, then
   says the server didn't answer (A tries again). The end of a match drops
   the kept records of everyone in it. B goes back.

13. Controller settings (SETTINGS on the main menu; "Settings", the row
   after Custom Game, for anyone; or the controller's X on the sign-in and
   offline screens, and the hint there can be clicked): CONTROLLER, with
   the rows of Halo 2's CONTROLLER screen (in a profile's settings) in its
   order and words:
   Thumbstick Layout (Default, Southpaw, Legacy, Legacy Southpaw), Button
   Layout (Default, Southpaw, Boxer, Green Thumb, Bumper Jumper, Recon),
   Look Sensitivity (1 to 10, "3 (Default)"), Look Inversion (the
   thumbstick's), Automatic Look Centering, Controller Vibration; then the
   mouse's Mouse Sensitivity (0.1 to 10 in tenths, default 1.6) and Mouse
   Inversion (its own, off by default, as MCC keeps it); and Halo 2's last
   row, Restore Defaults. Up and down pick a row, left and right (or the
   arrows) change it, A moves it on (and numbers go round), A on Restore
   Defaults puts them all back. The right side lists what each button does
   in the layout picked, in Halo 2's words ("Melee attack", "Zoom view",
   "Multiplayer score", Start "Pause game"), and what each stick does (on
   the mouse's rows, the keyboard and mouse keys instead: halo2.dll's own
   plainly, MCC's marked `?`), and the line under the rows says what the
   row is for. Every change is saved to `lobby.txt` at once and passed to
   every engine the lobby starts (`--layout ... --sticks ...
   --look-sensitivity ...`); `--offline` reads `lobby.txt` too. B goes
   back. See "Controls" in `README.md` for what the engine is given and
   what is an estimate.

## Testing without MCC

`--fake-engine` stands in for the engine on any platform: it reads the
session, says running and maploaded, waits a few seconds
(`H2LOBBY_FAKE_SECONDS`, default 5) and ends the game with made-up results
that are the same on every PC, so the server counts them
(`H2LOBBY_FAKE_QUIT=1` quits halfway instead). On Linux the lobby always
uses it; `H2LOBBY_FAKE=1` makes it do so on Windows too.

`--fake-engine`'s results are the same on every PC and plausible: player
`i` of `n` scores `n - i`, the last one killed themselves once, each one's
kills are their score plus their suicides, and each one's kills are the
next one's deaths (alone, no one is killed).

`H2LOBBY_HEADLESS=1` runs the lobby without a window, driven by
`H2LOBBY_SCRIPT`, and `H2LOBBY_SHOTS=<folder>` saves a PNG each time the
screen changes (on the start screen and main menu once they have faded
in, about 1.5 s and 0.3 s, with the script held until then).
`H2LOBBY_ENGINE_ARGS` adds flags to the engine's command
line (`--set-option 0x354=i32:60 --pad none`, say). A script line is
`<seconds> <command>`, the seconds counted from the step before:

```
0 wait start 5        # the lobby opens on the start screen
0.3 a                 # PRESS START: the main menu
0 see ONLINE 5
0.3 a                 # ONLINE: the sign-in form (no gamertag saved yet)
0 wait signin 5
0 type ALPHA          # typed into the focused field
0.3 a                 # a key: up down left right a b x y lb rb tab back
0 wait live 20        # until the screen is "live" (at most 20 s)
1 down
0.5 a
0 wait pregame 60
0 wait carnage* 60    # a * matches the start: carnage-waiting too
1 shot report         # a screenshot now
0 pick CHARLIE        # select their row (players, friends, party, carnage)
0.3 lb
0 see Head to Head 10 # until that text is drawn (any case; at most 10 s)
0 set button_layout bumper_jumper   # a controller setting, by its lobby.txt name
3 quit
```

`see` takes its last word as the seconds when it is a number (so
`see 12 of 100 120` waits for "12 of 100"; the start screen's and main
menu's text counts once it has faded in), and fails the run as `wait`
does; `pick` fails it when there is no such row, and `set` when the
setting or its value is wrong.

With a gamertag saved, ONLINE signs in at once (`wait connecting`,
then `wait live`), without the form.

The screens are `start`, `main`, `signin`, `connecting`, `live`,
`searching`, `players`, `party`, `custom`, `pregame`, `ingame`,
`carnage-waiting`, `carnage`, `failed`, `friends`, `record` and
`settings`, and the popups `invite`, `quit`, `leave`, `remove` (a party
member), `addfriend`, `friend` (a friend's options) and `unfriend`.

Two lobbies on one machine against a local server:

```
PORT=47250 H2LIVE_BIND=127.0.0.1 H2LIVE_DATA=/tmp/live target/release/h2live &
H2LOBBY_HEADLESS=1 H2LOBBY_SCRIPT=a.txt H2LOBBY_SHOTS=shots-a \
  target/release/h2launch --lobby --live 127.0.0.1:47250 --instance la &
H2LOBBY_HEADLESS=1 H2LOBBY_SCRIPT=b.txt H2LOBBY_SHOTS=shots-b \
  target/release/h2launch --lobby --live 127.0.0.1:47250 --instance lb
```

Run that way (2026-10-10), both searched Head to Head, met in a match,
played it on the stand-in engine (one hosting, one joining), and saw the
carnage report with the server's verdict (counted, level 1 to 2 for the
winner); then one invited the other into its party, which it accepted and
left again. A second run had the joining player leave mid-game: it was
back in the party screen with "You left the game", and the host's game
ended and counted.

A custom game the same way (2026-10-10): ALPHA invited BRAVO, went down to
Custom Game, picked Team Slayer on Lockout and started it; both lobbies got
the match (ALPHA hosting, red against blue), played it on the stand-in
engine and showed the carnage report with "Unranked: levels don't change",
and ALPHA was back on the custom game screen after it. The server's side
is also covered by `a_launcher_party_plays_custom_games` in
`crates/h2live/src/server/tests.rs`.

The party screen the same way (2026-10-10), with three lobbies: ALPHA
invited BRAVO and CHARLIE, made the party invite only, removed BRAVO (who
got "YOU WERE REMOVED FROM THE PARTY" and a party of their own), made
CHARLIE leader, and left; CHARLIE was then leader of an invite-only party
of one.

On the owner's PC (2026-10-10 07:42 to 07:45), the same with the real
engine against a local h2live (`127.0.0.1:47260`), with a 60 s time limit:
the party played Team Slayer on Lockout (red against blue, the leader
hosting, about 1.3 MB of game traffic each way through the relay), both
engines ended the game together and closed by themselves, both carnage
reports said "Unranked: levels don't change", and the leader was back on
the custom game screen. Then, alone, the leader played a second custom
game: one PC hosting, the map loaded in 6 s, the game ended at 60 s and the
engine closed by itself.

Friends and service records with three lobbies (2026-10-10, `LIVE_PROTOCOL`
3, the run in `live-v3.md`): ALPHA's requests to NOBODY, to itself and a
second one to BRAVO got NO SUCH PLAYER, YOU CAN'T ADD YOURSELF and YOU
ALREADY SENT BRAVO A FRIEND REQUEST; BRAVO accepted ALPHA's ("In a
lobby"); ALPHA declined CHARLIE's, and CHARLIE's list went empty with no
notice. ALPHA and CHARLIE played Head to Head (Swords on Gemini) on the
stand-in engine while BRAVO's friends list said "Head to Head: Swords on
Gemini"; the carnage report had kills, assists and deaths, and both
ALPHA (CHARLIE's) and BRAVO (ALPHA's) opened service records with the
game in them. The server was stopped and started again on the same data
folder; the lobbies signed in again and the friendship was still there.
ALPHA invited BRAVO from the options popup (BRAVO joined), then removed
BRAVO, whose list went empty. Afterwards `accounts.txt` had 11-word `x`
lines, `games.log` 14-field entries and `friends.txt` nothing.
