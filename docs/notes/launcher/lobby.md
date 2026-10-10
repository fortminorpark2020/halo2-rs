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
  our own. It signs in to h2live and stays signed in between matches.
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

- The window is drawn on the CPU into a pixel buffer (`softbuffer`), with
  text from a font already on the PC (Bahnschrift, then Segoe UI or Arial
  on Windows; DejaVu Sans or Liberation Sans on Linux) through `ab_glyph`.
  No font or art is shipped or read from MCC. A screen is cheap to draw,
  and the same code writes PNGs for tests without a GPU.
- Layout is in units of 1/720 of the window's height, so it scales.
- Keyboard (arrows, Enter for A, Esc for B, the X and Y keys), mouse (click
  a row to select it, again to pick it; click a button hint) and Xbox
  controllers (gilrs: d-pad or left stick, A, B, X, Y; Start is A and Back
  is B).
- Files in the launcher's folder (`%LOCALAPPDATA%\h2launch`, or its
  `--instance` folder): `lobby.txt`, `lobby.log` (the lobby's log, with the
  engine's lines as `engine:`), `live-key.bin` and `live-card.txt` (the same
  account as `--live`), and `match-session.txt` (the last match's session).

## Screens

1. Sign in: type a gamertag the first time; kept in `lobby.txt` in the
   launcher's folder with the server's address.
2. Xbox Live: the playlists (name, your level in it, people searching and
   playing) and the party (members and their levels in the chosen
   playlist). A searches (party leader only), X opens the players list, Y
   the party screen, B quits.
3. Players: everyone signed in. A invites them, X joins their open party,
   B goes back. An invitation pops up over the party screens: A accepts, B
   declines.
4. Searching: the playlist, the stage the server reports and the level
   range; B cancels.
5. Pregame: the map, the game type and the players with their teams and
   levels for a few seconds, then the engine starts.
6. In game: the lobby window waits while the engine's window is up, saying
   whether the engine is starting, loading or playing. B leaves the game.
7. Carnage report: each player's place, team, score and deaths (kills
   once they are found in the results), and the level changes MATCH_OVER
   gives. A goes back to the party (the leader of a custom game goes back
   to the custom game screen, to pick the next).

8. Custom game: the row after the playlists, for the party leader. Left
   and right pick the game type (one of each kind the launcher playlists
   play, `names::CUSTOM_GAMES`) and the map (those on this PC), A starts
   it. The server (LAUNCHER_CUSTOM, `LIVE_PROTOCOL` 2) checks the game is
   one of its launcher playlists' and the map one every member has that a
   launcher playlist plays, then makes an unranked match for the party
   with the leader hosting and teams alternating down the party; from
   there it goes as a playlist's match does. It works alone too.
9. Party: the members, each one's level and part (leader, member, you,
   guests) and whether anyone can join. The leader picks another member
   and makes them leader (A, PROMOTE) or removes them (X, asked first;
   KICK, after which they come back only if invited), and Y switches
   between open and invite only (PRIVACY). Anyone picks their own row to
   leave (A, LEAVE_PARTY). It goes back to the playlists while the party
   searches.
10. Offline: the server couldn't be reached or was lost. A tries again, Y
   goes back to the sign-in screen, B quits. B on the party screen asks
   before quitting, and offers Y to sign out.

## Testing without MCC

`--fake-engine` stands in for the engine on any platform: it reads the
session, says running and maploaded, waits a few seconds
(`H2LOBBY_FAKE_SECONDS`, default 5) and ends the game with made-up results
that are the same on every PC, so the server counts them
(`H2LOBBY_FAKE_QUIT=1` quits halfway instead). On Linux the lobby always
uses it; `H2LOBBY_FAKE=1` makes it do so on Windows too.

`H2LOBBY_HEADLESS=1` runs the lobby without a window, driven by
`H2LOBBY_SCRIPT`, and `H2LOBBY_SHOTS=<folder>` saves a PNG each time the
screen changes. `H2LOBBY_ENGINE_ARGS` adds flags to the engine's command
line (`--set-option 0x354=i32:60 --pad none`, say). A script line is
`<seconds> <command>`, the seconds counted from the step before:

```
0 type ALPHA          # typed into the focused field
0.3 a                 # a key: up down left right a b x y tab back
0 wait live 20        # until the screen is "live" (at most 20 s)
1 down
0.5 a
0 wait pregame 60
0 wait carnage* 60    # a * matches the start: carnage-waiting too
1 shot report         # a screenshot now
3 quit
```

The screens are `signin`, `connecting`, `live`, `searching`, `players`,
`custom`,
`pregame`, `ingame`, `carnage-waiting`, `carnage` and `failed`, and the
popups `invite`, `quit` and `leave`.

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
