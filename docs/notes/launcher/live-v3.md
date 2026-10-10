# Live v3: kills, service records, friends and rank icons

The design for the next batch on `launcher-friends` (based on
`launcher-lobby`): four things a Halo 2 Xbox Live player had that the
lobby lacks.

1. Kills, assists and betrayals from the engine's results, shown on the
   carnage report and kept per playlist on the server.
2. A service record for any player.
3. A friends list like Xbox Live 1.0's.
4. Halo 2's own level icons beside every level.

Not in this batch: clans, rank leaderboards, voice, recent players.

All of it ships together at `LIVE_PROTOCOL` 3, with one Proxmox deploy
at the end. The game's `PROTOCOL` stays 26, and the game (h2viewer) keeps
signing in and playing against the new server as it does now.

The work goes in three parts:

- **Part A**, first: `crates/h2net/src/live.rs` and `crates/h2live` (the
  messages, the server, its files, the client in `h2live::client`, and
  their tests).
- **Part C**, any time before B needs it: `crates/h2launch/src/lobby/ranks.rs`,
  a module on its own that finds and decodes the rank icons, with its own
  unit tests.
- **Part B**, after A: the rest of `crates/h2launch` (results block, event
  lines, fake engine, lobby screens, rank icons drawn), `lobby.md` and the
  README.

## Rules for every part

- `LIVE_PROTOCOL` goes from 2 to 3 in `crates/h2net/src/live.rs`. Its doc
  comment gets "3: kills, assists, betrayals and suicides in
  LAUNCHER_RESULT; RECORD and SERVICE_RECORD; the friends messages."
  `PROTOCOL` in `crates/h2net/src/lib.rs` doesn't change. A launcher at
  version 2 is turned away with UPDATE YOUR LAUNCHER, as
  `Server::login_refusal` already does for any other version.
- **The game must never be sent a new kind.** Builds of h2viewer already
  out there read a message kind they don't know as "bad data from the
  server" (`LiveClient::take` gets an error from `ToPc::read`) and drop the
  link. So the server sends SERVICE_RECORD, FRIENDS and every friend notice
  only to PCs whose `Pc::client` is `ClientKind::Launcher`. The new
  PC-to-server kinds, sent from the game, are unexpected messages: the PC
  is dropped, as for any message it shouldn't send.
- The game's own messages (RESULT, ONLINE, PARTY and the rest) don't
  change.
- A data folder written by today's server loads as it is. `accounts.txt`
  keeps its lines and gains optional fields (see 1.5), and `friends.txt` is
  a new file that may be missing. Going back to the old server after v3 has
  written its files would fail on the longer `x` lines, so the deploy keeps
  a copy of the data folder first (see "Deploy").
- No game or MCC files, and no bytes from them, are committed: no maps, no
  results blocks, no icons, and no screenshots that show Halo 2's fonts or
  icons. Tests build synthetic bytes.

## Message kinds

New and changed kinds, chosen clear of every kind in use (PC to server 1-5,
10-18, 20-23, 30-36, 40, 42; server to PC 41, 101-116):

| Kind | Name | Way | Body |
|---|---|---|---|
| 35 | LAUNCHER_RESULT | PC to server | changed: see 1.2 |
| 37 | RECORD | PC to server | `account u64` |
| 50 | FRIEND_REQUEST | PC to server | `gamertag str` (at most `MAX_NAME` bytes) |
| 51 | FRIEND_ACCEPT | PC to server | `account u64` (who asked) |
| 52 | FRIEND_DECLINE | PC to server | `account u64` (who asked) |
| 53 | FRIEND_REMOVE | PC to server | `account u64` (a friend, or someone we asked) |
| 117 | SERVICE_RECORD | server to PC | see 2.1 |
| 118 | FRIENDS | server to PC | see 3.2 |

Strings, counts and bools are written as they are now: `str` is
`Writer::str`, a bool is a 0 or 1 byte (`read_bool`), and a count is a byte
read with `read_count` against its limit. `live.rs` gets a test that lists
every constant in `mod kind` and checks that no two PC-to-server kinds, and
no two server-to-PC kinds, share a number.

New limits in `live.rs`:

```rust
/// Most friends a player has, the friend requests they sent counted in
/// (Xbox Live 1.0 kept 100).
pub const MAX_FRIENDS: usize = 100;
/// Most friend requests waiting for a player's answer.
pub const MAX_FRIEND_REQUESTS: usize = 100;
/// Most entries a FRIENDS list holds: friends and requests either way.
pub const MAX_FRIEND_ENTRIES: usize = MAX_FRIENDS + MAX_FRIEND_REQUESTS;
/// Most playlists a SERVICE_RECORD lists.
pub const MAX_RECORD_PLAYLISTS: usize = 64;
```

A FRIENDS list at its largest is about 20 KB (100 friends with a map and a
variant name each, and 100 requests), well inside a message.

## 1. Kills, assists and betrayals

### 1.1 The engine's results block (`crates/h2launch/src/results.rs`, part B)

Seen on the owner's PC (2026-10-10), in each player's statistics block (at
`STATS + STATS_STRIDE * i`, 0x1CE0 + 0x5448 * i):

- +0x00 and +0x04 were 1 in every game.
- The lone winner had +0x0C = 1 and +0x18 = 1.
- +0x2C deaths and +0x34 suicides, equal in games where every death was a
  suicide.
- +0x3C was 59 in a 60 s game: seconds alive.
- Everything else was 0 in games with no kills.

That fits the order Halo games keep a player's results statistics in:
kills, assists, deaths, betrayals, suicides, most kills in a row, seconds
alive. So results.rs reads:

| Offset | Field | Type | Status |
|---|---|---|---|
| +0x24 | kills | u32 | inferred |
| +0x28 | assists | u32 | inferred |
| +0x2C | deaths | u32 | seen |
| +0x30 | betrayals | u32 | inferred |
| +0x34 | suicides | u32 | seen |
| +0x38 | most kills in a row | u32 | inferred |
| +0x3C | seconds alive | u32 | seen |

The module comment says which offsets are inferred and that none of them has
been seen above zero yet, in the file's tone ("not known yet" becomes
"inferred from the order ..., not yet seen above zero"). Values that are
inferred say so on their constants too.

`PlayerResult` gains `kills`, `assists`, `betrayals`, `in_a_row` and
`alive` (all u32). `describe` adds them to each player's line:
`... deaths 2 suicides 2 kills 0 assists 0 betrayals 0 in a row 0 alive 59 s played 60.0 s`.

`for_server` fills `kills`, `assists`, `deaths`, `betrayals` and
`suicides`, each clamped to `u16::MAX` as deaths is now.

**The kills check.** A new function confirms or refutes the offsets in the
first real game:

```rust
/// One line for the log on whether each player's kills (+0x24) are their
/// score plus their suicides (plus their betrayals, where the variant
/// takes a point off for those), as they must be in Slayer. It only
/// checks; it changes nothing.
pub fn check_kills(players: &[PlayerResult]) -> String
```

It returns one of:

- `kills: nothing to check (no one scored, killed themselves or betrayed)`
  when `score + suicides + betrayals` is 0 for every player (a 0-kill game
  proves nothing);
- `kills: every player's kills are their score plus suicides: the offsets hold`
  when, for every player, `kills == score + suicides` or
  `kills == score + suicides + betrayals`;
- otherwise
  `kills: player <i> has kills <k> but score <s> + suicides <u> (+ betrayals <b>): the kills offset (+0x24) may be wrong`,
  naming the first player that doesn't fit.

`win/host.rs` `game_result` logs it (as `result: <line>`) after
`describe`'s lines when the game's variant is a Slayer one. Part B makes
the variant reachable there (it is on the engine's command line,
`--variant`; keep it in a `OnceLock` at start if nothing holds it yet) and
adds `names::slayer(variant) -> bool`: true for the variants whose game type
in h2live's `playlists.txt` is `slayer` or `team_slayer` (15 files today),
from a table in `names.rs`, with a test that reads `playlists.txt` (as the
`CUSTOM_GAMES` test does) and checks the table against every `mcc_variant`
line both ways. The check is a log line only: results are never changed by
it. Windows-only code can't be built on Linux CI, so B runs
`cargo check --target x86_64-pc-windows-gnu -p h2launch` when the target is
installed, and otherwise says so in the commit.

### 1.2 LAUNCHER_RESULT at v3 (part A)

`LauncherPlayerResult` becomes, in this field order (the struct's and the
wire's):

```
relay_id  u64
team      u8
place     u8
score     i32 (as u32)
kills     u16
assists   u16   (new)
deaths    u16
betrayals u16   (new)
suicides  u16   (new)
left      bool
```

The rest of LAUNCHER_RESULT (`id`, `finished`, `team_scores`, the player
count) is as it is. RESULT (the game's `PlayerResult`) doesn't change.

Part A fills the new fields with 0 wherever h2launch builds a
`LauncherPlayerResult` today (results.rs, live.rs, child.rs and their
tests), so the workspace builds and the gate passes; part B fills them
properly.

### 1.3 The engine's `ended` line (part B)

The `--events` line between the engine's copy and the lobby gets the new
fields, in the wire's order:

```
H2EVENT ended <relay id hex>,<team>,<place>,<score>,<kills>,<assists>,<deaths>,<betrayals>,<suicides>,<left>
```

Both ends are the same build, so `parse_event_line` takes only the 10-field
form, and a 7-field one is not an event (`None`).

### 1.4 The stand-in engine (`lobby/child.rs`, part B)

`fake_results` stays a function of the session alone (the same on every
PC, so the server counts the game), with plausible non-zero numbers. For
`n` players in session order, player `i`:

- `score = n - i` (as now), `place` as now;
- `suicides = 1` for the last player when `n >= 2`, else 0; `betrayals = 0`;
- `kills = score + suicides` (so the kills check above would pass);
- `assists = (n - i) / 2`;
- `deaths[j] = kills[(j + n - 1) % n] + suicides[j]`: each player's kills
  are the next player's deaths. With one player, deaths are 0.

So the deaths add up to the kills plus the suicides. A test checks that sum,
that kills, assists and deaths aren't all zero for two or more players, and
that the results are the same on every call.

### 1.5 What the server keeps (part A)

`store.rs` gets:

```rust
/// What a player did in the games that counted for them in a playlist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub kills: u32,
    pub assists: u32,
    pub deaths: u32,
    pub betrayals: u32,
    pub suicides: u32,
}
```

with `add(&mut self, other: Tally)` (saturating), and `Stats` gains
`tally: Tally`.

**accounts.txt.** The `x` line gets five optional numbers at its end:

```text
x <id> <playlist key> <xp> <level> <games> <wins> [<kills> <assists> <deaths> <betrayals> <suicides>]
```

`stats_line` takes 6 words (a tally of zeros, as every file and stat card
written today has) or 11, and nothing else. `Account::lines` writes the
short form when the tally is all zeros, so an account v3 hasn't touched is
written byte for byte as before. Stat cards are `Account::lines` signed, so
they carry the tally too, and old cards still verify and read.

**When it adds.** In `Server::end` (server/matches.rs), in the loop that
adds a game and maybe a win to each rated player: when the match counted
for everyone (`Counted::Yes`), each rated player's tally from the host's
result is added to their stats in that playlist. A host's loss for quitting
(`Counted::HostLoss`) adds a game but no tally. Unranked games keep
nothing, as now.

Where the host's tallies come from:

- Launcher matches: `Launcher::reports` becomes a list of
  `Report { from: u64, finished: bool, team_scores: Vec<i32>, tallies: Vec<(u64, i32, Tally)> }`,
  where each player is named by account (looked up from the relay id as
  `launcher_result` does now) with their score. `end` takes the host's.
- The game's matches: the host's RESULT gives kills and deaths (assists,
  betrayals and suicides 0), so the game's playlists keep kills and deaths
  too, at no cost.

**games.log.** Each player's entry gets six more fields when the host's
result has them:

```text
<id>:<team>:<place>:<left>:<old xp>:<new xp>:<old level>:<new level>[:<score>:<kills>:<assists>:<deaths>:<betrayals>:<suicides>]
```

A player the host's result doesn't name, or every player of a match with
no result from its host, keeps the 8-field form. `RecordedPlayer` gains
`result: Option<(i32, Tally)>`. The module comment in store.rs describes
both forms. Nothing reads games.log back today; the 8-field entries stay
valid.

### 1.6 The carnage report (part B)

Columns, Halo 2 Xbox's order (layout units, headings at y 228 as now):

| Heading | x | Align |
|---|---|---|
| PLACE | 100 | left |
| PLAYER | 200 | left (names fit in 440) |
| SCORE | 700 | centre |
| KILLS | 790 | centre |
| ASSISTS | 880 | centre |
| DEATHS | 970 | centre |
| LEVEL | 1090 | centre (rank icon, see 4) |

Rows are sorted by `(place, team, -score)`, so a team game lists each team
together in its finishing order, under its colour bar as now. The `-`
fallback for kills goes: the numbers are drawn as they come.

The report gets a selected row (`csel`, starting on this PC's player, lit
with `Pen::row` as other lists are). Up and down move it. LB opens the
selected player's service record, and Y sends them a friend request (shown
when they aren't you, a friend, or someone a request is waiting with). A
continues, as now.

## 2. Service record

### 2.1 Messages (part A)

RECORD (37, launcher to server): `account u64`.

SERVICE_RECORD (117, server to launcher):

```
account   u64
found     bool
-- only when found:
gamertag  str
look      8 bytes (Look::write)
best      u8      (highest level in any ranked playlist, 1 if none)
created   u64     (Unix time the account was made)
count     u8      (at most MAX_RECORD_PLAYLISTS)
each playlist:
  playlist  u8   (its id, as PLAYLISTS names it)
  level     u8
  games     u32
  wins      u32
  kills     u32
  assists   u32
  deaths    u32
  betrayals u32
  suicides  u32
```

In h2net:

```rust
pub struct ServiceRecord { pub account: u64, pub found: Option<Record> }
pub struct Record {
    pub gamertag: String,
    pub look: Look,
    pub best: u8,
    pub created: u64,
    pub playlists: Vec<PlaylistRecord>,
}
pub struct PlaylistRecord {
    pub playlist: u8,
    pub level: u8,
    pub games: u32,
    pub wins: u32,
    pub tally: Tally,   // h2net's own copy of the five counts
}
```

(`h2net` can't use `h2live::store::Tally`, so `live.rs` gets its own
`Tally` with the same five u32 fields; h2live converts.)

`found: false` with anything after it is malformed, as is a count above the
limit.

### 2.2 The server (part A, `server/social.rs`)

- A RECORD is answered only for launcher PCs. Each PC gets at most 20
  answers in any 10 seconds (`RECORDS_PER_10S`); more are ignored without
  a reply (the lobby caches, so a player never gets there).
- An account the server doesn't have gets `found: None`. Nothing else is
  said about it.
- The playlists listed are the asker's program's ranked playlists that the
  account has played (`games > 0`), in the server's playlist order. So a
  launcher sees only launcher playlists.
- `best` is `Account::best_level`, as ONLINE gives it.

### 2.3 The client (part A, `h2live::client`)

`LiveEvent::ServiceRecord(ServiceRecord)` when one comes. The View doesn't
keep them; the lobby does.

### 2.4 The screen (part B)

Screen `Record`, label `record`, title "SERVICE RECORD". The App keeps the
account shown, the screen to go back to, and the list's first row.

Opening one:

- the Live screen: LB opens your own;
- the party screen, players online, friends and carnage report: LB opens
  the selected player's.

**Cache.** The App keeps the last 32 records it got, with when. Opening a
record that came less than 60 s ago (`RECORD_FRESH`) shows it and asks for
nothing. Otherwise it asks once (RECORD), shows the cached one meanwhile if
there is one, and waits 5 s (`RECORD_WAIT`). Asking again for an account
while waiting for it sends nothing. MATCH_OVER drops the cached records of
everyone in that match, so the next look shows the new numbers.

Layout (1280x720 layout units, title at (60, 58) as on every screen):

- Left panel at (60, 110), 380 by 530:
  - gamertag at (84, 170), 32 pt, fit to 340;
  - the highest level's big rank icon at (84, 196), 84 by 78 units (three
    times its size); "HIGHEST LEVEL" 16 pt HEAD at (184, 222) and the
    number 36 pt at (184, 262). With no icons, the number alone sits there;
  - totals over the listed playlists, one per line from y 330, 36 apart, label
    at x 84 (20 pt, DIM) and value right-aligned at x 416 (20 pt, TEXT):
    Ranked games, Wins, Kills, Deaths, Assists, K/D;
  - "Member since 2026-10-10" (from `created`, UTC date) 16 pt DIM at
    (84, 610).
- Right panel at (460, 110), 760 by 530. Headings 16 pt HEAD at y 142:
  PLAYLIST at x 484 (left), then centred: LEVEL 740, GAMES 815, WINS 880,
  KILLS 945, DEATHS 1015, ASSISTS 1085, K/D 1160. Rows from y 156, 50
  apart, 46 high, 9 shown; up and down scroll them (`first_row`). Playlist
  names come from PLAYLISTS by id ("Playlist <id>" if the lobby doesn't
  have it), fit to 220; the level is a small rank icon.
- K/D is `kills / max(deaths, 1)` with two decimals, or "-" with no kills
  and no deaths.
- While waiting with nothing cached: "Loading the service record." 22 pt at
  (840, 330) and the spinner at (840, 420). Not found: "No such player."
  No rows: "No ranked games yet." No answer within 5 s: "The server didn't
  answer." and A tries again.
- Hints: B Back (and A Try again after no answer).

The look isn't shown: the launcher signs in with the default look, so it
would be the same for everyone. It goes on the wire for a later profile
screen.

## 3. Friends

### 3.1 Rules

- A launcher player sends a friend request by gamertag. The server looks
  the gamertag up as sign-in does, after `clean_name` and ignoring case,
  among all accounts (online or not).
- Friends and the requests a player sent count together towards
  `MAX_FRIENDS` (100), as Xbox Live 1.0 kept 100 friends. Counting the
  sent requests in is our assumption; the research notes give only the
  cap. A player has at most `MAX_FRIEND_REQUESTS` (100) requests waiting
  for their answer.
- Asking someone who already asked you makes you friends at once.
- Accepting is only possible for a request that is waiting. When it is
  accepted, the asker's sent request becomes a friend (their count doesn't
  change); the one accepting needs room for one more.
- Declining removes the request. The asker isn't told; the request just
  goes from their list.
- Removing ends a friendship, or takes back a request you sent. The other
  player isn't told; their list changes.
- Requests don't expire.
- A friend's status (online, what they're doing, their party) is shown
  only once the request is accepted. A request shows only the gamertag and
  highest level.
- Inviting a friend uses INVITE, and joining their party JOIN_PARTY, with
  the rules and privacy they have now (invite only, removed members, full,
  in a match, another program).
- Friend actions (the four PC-to-server kinds above) are limited to 30 a
  minute per PC. More are ignored, with one notice (SLOW_DOWN) per minute.
- The game (h2viewer) can't send or answer requests, and never hears of
  them. A launcher player can still ask a game player's account (it
  exists); the request waits unanswered until it is taken back. A friend
  signed in on the game shows as online there, without details, and can't
  be invited or joined (OTHER_PROGRAM, as now).

What players are told (NOTICE, launchers only; `{X}` is a gamertag as the
server has it):

| Constant | Text | To |
|---|---|---|
| `FRIEND_ASKED` | `FRIEND REQUEST SENT TO {X}` | the asker |
| `NO_SUCH_PLAYER` | `NO SUCH PLAYER` | the asker |
| `NOT_YOURSELF` | `YOU CAN'T ADD YOURSELF` | the asker |
| `ALREADY_FRIENDS` | `{X} IS ALREADY YOUR FRIEND` | the asker |
| `ALREADY_ASKED` | `YOU ALREADY SENT {X} A FRIEND REQUEST` | the asker |
| `LIST_FULL` | `YOUR FRIENDS LIST IS FULL` | the asker, or the one accepting |
| `TOO_MANY_REQUESTS` | `{X} HAS TOO MANY FRIEND REQUESTS` | the asker |
| `ASKED_YOU` | `{X} SENT YOU A FRIEND REQUEST` | the one asked, if online |
| `ACCEPTED` | `{X} ACCEPTED YOUR FRIEND REQUEST` | the asker, if online |
| `NOW_FRIENDS` | `YOU AND {X} ARE NOW FRIENDS` | both, when requests crossed |
| `SLOW_DOWN` | `SLOW DOWN. TRY AGAIN IN A MINUTE.` | the PC over the limit |

For a gamertag no account has, the only answer is NO SUCH PLAYER.

### 3.2 FRIENDS (118, server to launcher)

The whole list, which replaces the one the PC had:

```
count u8 (at most MAX_FRIEND_ENTRIES)
each entry:
  account   u64
  gamertag  str
  relation  u8   0 friend, 1 asked us, 2 we asked them
  best      u8   their highest level
  online    u8   0 offline, 1 on the launcher, 2 on the game (h2viewer)
  -- meaningful only for a friend on the launcher; 0 and empty otherwise:
  activity  u8   Activity
  playlist  u8   the playlist they search or play (QUICKMATCH while
                 searching quickmatch, CUSTOM_GAME in a custom game)
  map       str  in a match: its map (MCC map name)
  variant   str  in a match: its MCC game variant
  party     u64
  joinable  bool their party is open, has room, isn't in a match, and
                 isn't ours
```

In h2net:

```rust
pub enum Relation { Friend = 0, AskedUs = 1, WeAsked = 2 }
pub enum Online { Offline = 0, Launcher = 1, Game = 2 }
pub struct Friend {
    pub account: u64,
    pub gamertag: String,
    pub relation: Relation,
    pub best: u8,
    pub online: Online,
    pub activity: Activity,
    pub playlist: u8,
    pub map: String,
    pub variant: String,
    pub party: u64,
    pub joinable: bool,
}
```

Out-of-range `relation`, `online` or `activity` bytes are malformed.

The server sends requests to us first (oldest first), then friends (by
gamertag, ignoring case), then requests we sent (oldest first). Accounts
the server doesn't have (a lost data folder whose `friends.txt` survived)
are left out until they come back.

The client (`h2live::client`) keeps the list in `View::friends` and says
`LiveEvent::Friends` each time one comes.

### 3.3 Server state and `friends.txt` (part A)

A new module `crates/h2live/src/friends.rs` holds the friendships and the
rules, with no I/O beyond loading and saving, so it is unit tested on its
own:

```rust
pub struct Friends { /* pairs: BTreeSet<(u64, u64)> (smaller id first), requests: Vec<Request> */ }
pub struct Request { pub from: u64, pub to: u64, pub unix: u64 }
pub enum Asked { Sent, NowFriends }
pub enum Refusal { Yourself, AlreadyFriends, AlreadyAsked, ListFull, TheirRequestsFull }

impl Friends {
    pub fn parse(text: &str) -> Result<Friends, String>;
    pub fn text(&self) -> String;
    /// None if there's no such file.
    pub fn load(path: &Path) -> Result<Friends, String>;
    pub fn save(&self, path: &Path) -> std::io::Result<()>;  // store::replace
    pub fn are_friends(&self, a: u64, b: u64) -> bool;
    pub fn friends_of(&self, a: u64) -> Vec<u64>;
    pub fn asked_by(&self, a: u64) -> Vec<u64>;  // requests to `a`, oldest first
    pub fn asking(&self, a: u64) -> Vec<u64>;    // requests from `a`, oldest first
    /// Friends plus requests sent: what MAX_FRIENDS limits.
    pub fn count(&self, a: u64) -> usize;
    pub fn ask(&mut self, from: u64, to: u64, unix: u64) -> Result<Asked, Refusal>;
    pub fn accept(&mut self, me: u64, from: u64) -> Result<bool, Refusal>; // false: no such request
    pub fn decline(&mut self, me: u64, from: u64) -> bool;
    pub fn remove(&mut self, me: u64, other: u64) -> bool;
}
```

`friends.txt`, in the data folder beside `accounts.txt`, written whole
(`store::replace`) after every change that changes something:

```text
f <id> <id>
r <from id> <to id> <unix time>
```

Ids are 16 hex digits, as in `accounts.txt`. An `f` line is a friendship,
written smaller id first; an `r` line is a request waiting for an answer,
with when it was sent. Empty lines are allowed. A line that doesn't read
(another letter, bad hex, an id paired with itself, a pair or request listed
twice, or a request between friends) is an error that stops the server
starting, as a broken `accounts.txt` does: dropping it quietly would lose it
at the next save. Ids that `accounts.txt` doesn't have are kept. No file at
all is an empty list, so a server that never saw friends starts as before.

The limits are checked when players act, not when loading.

Friends aren't on the stat card: a card is one account, and a friendship
is two.

`Server` gains `friends: Friends`, loaded in `Server::open`, and the
handlers go in a new `crates/h2live/src/server/social.rs` (friends and
service records), dispatched from `Server::handle` for launcher PCs only.
`Pc` gains the times of its recent friend actions and record requests (for
the two limits).

### 3.4 Status updates, without flooding

- Each second at most (`FRIENDS_EVERY`, 1.0 s, as ONLINE goes), the server
  works out the presence of every signed-in account: online (and on which
  program), gamertag, best level, activity, playlist, map and variant, party,
  and whether that party is open and how many more fit. It keeps the last
  one per account, and an account that signed out becomes offline. One
  account-to-PC map is built per tick, so this costs one pass over the
  signed-in PCs.
- An account whose presence changed marks each of its friends that is
  signed in on a launcher as needing a new list.
- A change to a list (a request, an accept, a decline, a removal) marks both
  players. Signing in marks the player.
- Then each marked player gets one FRIENDS, with the joinable flag worked
  out for them, and the marks are cleared. So a player hears at most one
  list a second, however busy their friends are.
- Map and variant come from the match the friend is in (its `info.map`
  and the launcher's `variant`).

### 3.5 Screens (part B)

**Friends** (screen `Friends`, label `friends`, title "FRIENDS"). RB opens
it from the Live screen and from Players online; B goes back to the Live
screen.

Panel at (60, 110), 1160 by 530. Headings 16 pt HEAD at y 142: GAMERTAG at
x 84 (left), LEVEL at 560 (centre), STATUS at 660 (left); "12 of 100"
(friends and sent requests) 16 pt DIM right-aligned at 1196.

Rows from y 156, 50 apart, 46 high, 9 shown (`first_row`). The lobby
orders them: requests to us, friends on the launcher, friends on the game,
friends offline, then requests we sent; by gamertag within each. Each row:

- a dot at (90, row middle), radius 6: GOOD on the launcher, DIM on the
  game, none offline or for requests;
- the gamertag at x 108, 24 pt, fit to 400;
- the level as a small rank icon at 560;
- the status at x 660, 20 pt (DIM, GOLD for a request to us):
  - "In a lobby";
  - "Searching <playlist>", or "Searching (quickmatch)";
  - "<playlist>: <game> on <map>" in a playlist's match (game and map by
    `names::variant` and `names::map`);
  - "Custom game: <game> on <map>";
  - "Online (h2viewer)";
  - "Offline";
  - "Wants to be your friend";
  - "Friend request sent";
- right-aligned at 1196, 20 pt DIM: "Your party", "Open party" when
  joinable, "Invite only" when their party isn't open, else nothing.

Empty: "No friends yet. Press Y to send a friend request." 22 pt DIM at
(640, 320).

Keys, by the selected row:

| Row | A | X | LB | Y | B |
|---|---|---|---|---|---|
| request to us | Accept (FRIEND_ACCEPT) | Decline (FRIEND_DECLINE) | Service record | Add friend | Back |
| friend | Options popup | Join party (when joinable) | Service record | Add friend | Back |
| request we sent | - | Cancel request (FRIEND_REMOVE) | Service record | Add friend | Back |
| (empty list) | - | - | - | Add friend | Back |

Hints show only what the row allows.

**Options popup** (label `friend`), for a friend: a box at (320, 200), 640
by 320, colours as `Pen::popup`. The friend's gamertag 30 pt centred at y
250, then options from y 280, 44 apart, as rows 560 wide from x 360, 22 pt
centred, lit when selected:

1. Invite to party (online on the launcher and not in your party): INVITE.
2. Join party (joinable): JOIN_PARTY with their party, then back to the
   Live screen.
3. Service record.
4. Remove friend: the confirm popup below.

Only those that apply are listed, in that order, so Remove friend is always
last. Up and down move the selection (up from the first goes to the last),
A picks, B closes. Hints A Select, B Back at y 492. Clicks on a row select
it, then pick it, as in lists.

**Add friend popup** (label `addfriend`): `Pen::popup`'s box with the
title "SEND FRIEND REQUEST", "Type their gamertag." and a field at
(380, 330), 520 by 52, drawn like the sign-in screen's gamertag field. It
takes letters, digits and spaces, upper-cased, up to `GAMERTAG_LEN`, and
Backspace. While it is up, letters are typed, not buttons (`App::input`'s
`typing` covers it). A sends FRIEND_REQUEST with `settings::clean_gamertag`
of it (nothing if empty) and closes the popup; B closes it. The answer comes
as a notice (a toast). Y on the Players screen and the carnage report sends
the selected player's gamertag directly, with no popup.

**Remove friend popup** (label `unfriend`): `Pen::popup("REMOVE FRIEND",
"Remove {X} from your friends? They won't be told.", [A Remove, B Cancel])`,
as the party screen's REMOVE PLAYER.

**Players online** gains: Y Add friend (shown unless they're a friend or a
request is waiting either way), LB Service record, RB Friends.

**Party screen** gains LB Service record (the selected member).

**Live screen** hints become: A Search (or Custom game), X Players, Y
Party, RB Friends, LB Service record, B Quit.

## 4. Rank icons

Halo 2's level icons are two bitmap tags in Halo 2 Vista's `mainmenu.map`,
50 images each, in level order (image index = level - 1), A8R8G8B8 with a
real alpha cut-out, one zlib stream each, no sequences:

- `ui\global_bitmaps\rank_icons`: 28 by 26;
- `ui\global_bitmaps\rank_icons_sm`: 17 by 17.

`shared.map` has the tag headers only, with the pixels pointing back into
`mainmenu.map`, so `mainmenu.map` is what's read. MCC's own maps are cache
format 13, which blam-cache doesn't read yet (`Header::parse` refuses any
version but 8), so for now the icons come from a Halo 2 Vista install.
Reading them from MCC's format-13 `mainmenu.map` is a later step, noted in
lobby.md. Without a Vista `mainmenu.map` the lobby draws level numbers
exactly as now.

### 4.1 `crates/h2launch/src/lobby/ranks.rs` (part C)

A module on its own, with no drawing and no lobby state. Its public API:

```rust
//! Halo 2's level icons, read from a Halo 2 Vista mainmenu.map ...

use blam_cache::bitmap::Image;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The environment variable that names the map (a file, or a folder holding
/// mainmenu.map), or `off`.
pub const ENV: &str = "H2LOBBY_RANKS";
/// Levels, and so icons in each tag.
pub const LEVELS: u8 = 50;
pub const BIG_TAG: &str = r"ui\global_bitmaps\rank_icons";
pub const SMALL_TAG: &str = r"ui\global_bitmaps\rank_icons_sm";
/// Where Halo 2 Vista keeps its maps, as the old h2viewer looked.
pub const MAP_DIRS: [&str; 3] = [
    r"C:\Games\Halo 2 Project Cartographer\maps",
    r"C:\Program Files (x86)\Microsoft Games\Halo 2\maps",
    r"C:\Program Files\Microsoft Games\Halo 2\maps",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// rank_icons_sm, 17 by 17: lists.
    Small,
    /// rank_icons, 28 by 26: the carnage report, pregame, service record.
    Big,
}

/// One level's icon: RGBA8 with straight (not premultiplied) alpha, the top
/// row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Both sets of icons, and the map they came from.
#[derive(Clone, Debug)]
pub struct RankIcons { /* big: Vec<Icon>, small: Vec<Icon>, from: PathBuf */ }

impl RankIcons {
    /// From decoded images in level order: exactly LEVELS of each, every
    /// one at least 1 by 1 with `width * height * 4` bytes. Sizes other
    /// than 28 by 26 and 17 by 17 are taken (a mod may change them).
    pub fn new(big: Vec<Image>, small: Vec<Image>, from: PathBuf) -> Result<RankIcons, String>;
    /// The icon for `level` (1 to 50); None for 0 or above 50.
    pub fn icon(&self, level: u8, size: Size) -> Option<&Icon>;
    /// The map they were read from.
    pub fn from(&self) -> &Path;
}

/// The maps to try, in order, or None when `H2LOBBY_RANKS` is `off`.
/// With `env` set: only that (a folder means its mainmenu.map). Without:
/// each of MAP_DIRS' mainmenu.map, then `maps\mainmenu.map` beside the
/// launcher (`exe_dir`).
pub fn candidates(env: Option<&OsStr>, exe_dir: Option<&Path>) -> Option<Vec<PathBuf>>;

/// The first candidate that is a file.
pub fn find(candidates: &[PathBuf]) -> Option<PathBuf>;

/// Both tags' 50 images from a Halo 2 Vista mainmenu.map. An error names
/// the map and why: not a map, MCC's cache format 13 ("MCC's format
/// (cache version 13) isn't read yet"), a tag missing, an image that
/// won't decode.
pub fn read(map: &Path) -> Result<RankIcons, String>;

/// What the lobby calls at start: `candidates` from the environment and
/// the running launcher's folder, `find`, `read`, and one log line saying
/// what came of it. None means levels are drawn as numbers.
pub fn load(log: &dyn Fn(&str)) -> Option<RankIcons>;
```

How `read` decodes: `MapSet::open(map)`, then for each tag
`set.map.find_tag(GroupTag::parse("bitm")?, name)?.datum` and
`blam_cache::bitmap::read_bitmap_at(&mut set, datum, i)` for `i` in 0..50,
then `RankIcons::new`. This is the path the h2viewer's emblems use
(`crates/h2viewer/src/emblem.rs`, `pictures`), with no tinting or mask
split: the icons are full colour. blam-cache's errors are turned into
strings; nothing panics on a bad file.

No atlas: the lobby draws on the CPU (`Canvas`), where a list of small
images is as quick to draw from as one big one. The 100 images are about
200 KB decoded.

`load`'s log lines:

- `lobby: rank icons from <path>`
- `lobby: rank icons off (H2LOBBY_RANKS=off); levels show as numbers`
- `lobby: no Halo 2 Vista mainmenu.map found; levels show as numbers`
- `lobby: rank icons: <error>; levels show as numbers`

### 4.2 Drawing them (part B)

- `Canvas::image(x, y, w, h, width, height, rgba)` in canvas.rs: draws an
  RGBA image scaled into a rectangle of window pixels, sampling bilinearly
  in straight alpha and blending over what's there. It takes plain sizes
  and bytes, so canvas.rs doesn't depend on ranks.rs.
- `run` in lobby/mod.rs calls `ranks::load` once, before `App::new`, and
  `Config` gains `ranks: Option<Arc<RankIcons>>`. `Pen` gets the icons
  when `App::draw` makes it.
- `Pen::level(x, base, size, col, align, level, Size)` draws a level where
  the code draws its number now, taking the same arguments: with icons, the
  icon `size * 1.25` units high for Small (square) or `size * 1.45` high for
  Big (28:26 wide), centred on `base - size * 0.35` (the middle of the
  capitals, which is the row's middle the way rows place text), with `x` its
  left edge, centre or right edge by `align`. Without icons, or for a level
  outside 1 to 50, it draws the number with `Pen::text` as now. Level 0
  draws "-".
- Where:
  - Small: playlists (LEVEL), the party panel, the party screen, players
    online, friends, the service record's rows, and the header (left of
    the gamertag, which is right-aligned at 1220);
  - Big: pregame, the carnage report, and the service record's highest level
    (at three times its size there).
- Levels 44 to 50 are pictures without a number on them (moons, sun, comet,
  the Halo); Halo 2 showed them as they are, and so does the lobby.
- Linux tests and CI have no map, so their screens show numbers. A screenshot
  taken where the icons load holds them: keep it on that PC.

## Keys and buttons

`Input` gains `Lb` and `Rb`: the controller's bumpers (gilrs
`Button::LeftTrigger` and `Button::RightTrigger`), Page Up and Page Down,
and Q and E where letters are buttons. `Pen::button` draws them as
rounded faces 34 by 22 units with "LB" or "RB" in 13 pt. The keyboard line
at the bottom becomes "Keyboard: Enter = A, Esc = B, X, Y, Q = LB, E = RB,
arrows". LB is always the service record, RB always the friends list.

| Screen | A | B | X | Y | LB | RB |
|---|---|---|---|---|---|---|
| Live | Search / Custom game | Quit | Players | Party | Your service record | Friends |
| Players | Invite | Back | Join party | Add friend | Service record | Friends |
| Party | Make leader / Leave | Back | Remove | Privacy | Service record | - |
| Friends | see 3.5 | Back | see 3.5 | Add friend | Service record | - |
| Service record | Try again (no answer) | Back | - | - | - | - |
| Carnage | Continue | Continue | - | Add friend | Service record | - |

## Headless testing (part B)

New screen labels: `friends`, `record`. New popup labels: `addfriend`,
`friend` (the options), `unfriend`.

New `H2LOBBY_SCRIPT` commands:

- `lb`, `rb`: the bumpers.
- `see <text> [<seconds>]`: waits until text containing `<text>` (ignoring
  case) is drawn on the screen, at most `<seconds>` (120 if not given),
  and fails the run as `wait` does. The `Pen` keeps the strings it drew in
  the last frame (`App::drawn`).
- `pick <gamertag>`: selects that player's row on the players, friends,
  party or carnage screen, ignoring case; the run fails if there is none.
  Scripts then don't depend on list order.

`parse_script`'s test gains these.

### The three-lobby run

A local h2live with a fresh data folder, and three headless lobbies on the
stand-in engine (`--instance la`, `lb`, `lc`), as in lobby.md:

1. ALPHA: `rb`, `wait friends`, `y`, `type NOBODY`, `a`: the log has the
   notice NO SUCH PLAYER. `y`, `type ALPHA`, `a`: YOU CAN'T ADD YOURSELF.
   `y`, `type bravo`, `a`: FRIEND REQUEST SENT TO BRAVO, and
   `see Friend request sent`. `y`, `type BRAVO`, `a` again: YOU ALREADY
   SENT BRAVO A FRIEND REQUEST.
2. BRAVO: `rb`, `see Wants to be your friend 20`, `pick ALPHA`, `a`.
   ALPHA: `see In a lobby 20` (BRAVO is now a friend and online).
3. CHARLIE asks ALPHA; ALPHA `see CHARLIE`, `pick CHARLIE`, `x`; CHARLIE
   gets no notice and `see No friends yet`.
4. ALPHA and CHARLIE each search Head to Head (from the Live screen; it is
   the second launcher playlist today, `1 down`). BRAVO, on the friends
   screen: `see Head to Head: 60` while ALPHA plays.
5. ALPHA: `wait carnage 60`, `see ASSISTS`, `shot carnage`, `pick CHARLIE`,
   `lb`, `wait record`, `see Head to Head 10`, `shot record`, `b`,
   `wait carnage`, `a`.
6. BRAVO: `pick ALPHA`, `lb`, `wait record`, `see Head to Head 10`: one
   game, with kills, assists and deaths from the stand-in's results.
7. Stop the server and start it again on the same data folder. Each lobby
   `wait failed 30`, `a`, `wait live 20`. ALPHA: `rb`, `see BRAVO 10`
   (friends outlived the restart).
8. ALPHA: `pick BRAVO`, `a`, `wait friend`, `a` (Invite to party). BRAVO:
   `wait invite 10`, `a`. Then ALPHA: `rb`, `pick BRAVO`, `a`, `up`
   (Remove friend), `a`, `wait unfriend`, `a`. BRAVO: `rb`,
   `see No friends yet 10`.

Afterwards: `accounts.txt` has 11-word `x` lines for ALPHA and CHARLIE in
`mcc_head_to_head`, `games.log` has 14-field entries, and `friends.txt` is
empty again (no `f` or `r` lines). Screenshots on Linux use the system
font and numbers, so they may be kept with the test notes.

## Test plan

### Part A

`crates/h2net/src/live.rs`:

- `every_message_reads_back_as_written` gains RECORD, the four friend
  kinds, SERVICE_RECORD (found, with two playlists, and not found), FRIENDS
  (each relation and online value, one entry with a map and a variant) and
  the v3 LAUNCHER_RESULT with non-zero new fields.
- Out of range: FRIENDS with 201 entries, SERVICE_RECORD with 65
  playlists, relation 3, online 3, activity 4, and a not-found record with
  bytes after it, are all `Malformed`.
- The kind-numbers test above, and `LIVE_PROTOCOL == 3`.
- A versioned launcher LOGIN at 2 is refused with UPDATE YOUR LAUNCHER
  (beside the existing `LIVE_PROTOCOL + 1` case).

`crates/h2live/src/store.rs`:

- Accounts with tallies read back as written; a zero tally writes the
  6-word `x` line, byte for byte as today's server writes it (a literal
  line in the test).
- Today's 6-word lines read, with zero tallies. `x` lines of 7 to 10 words,
  or 12, are errors, as are tally numbers that aren't u32s.
- games.log: a player with a result gets the 14-field entry; one without
  keeps 8 fields.

`crates/h2live/src/card.rs`: a card with tallies verifies and reads back;
an old 6-word card still verifies.

`crates/h2live/src/friends.rs`:

- Reads back as written. No file is an empty list. Each kind of broken line
  is an error.
- `ask`: yourself, twice, already friends, crossed requests (NowFriends),
  the 100 cap with friends and sent requests counted together, and the 100
  waiting requests cap.
- `accept` needs a waiting request and room; it doesn't change the asker's
  count. `decline` and `remove` (a friend, and a sent request) report
  whether anything changed.
- `asked_by` and `asking` are oldest first.

Server integration tests, in `crates/h2live/src/server/tests.rs` style,
with `sign_in_launcher` and a relay where matches are played:

1. `launchers_make_friends_by_gamertag`: the notices for NO SUCH PLAYER,
   yourself and a second request; asking "bravo" in lower case finds BRAVO;
   BRAVO's list shows ALPHA as AskedUs, offline, with no party; after
   FRIEND_ACCEPT both lists show Friend, online on the launcher, in a
   lobby, and ALPHA heard ACCEPTED.
2. `friends_see_what_friends_are_doing`: ALPHA's party searching, then
   playing a custom game (`LauncherCustom`), shows to BRAVO with the
   playlist, map and variant; joinable follows PRIVACY; JOIN_PARTY with
   the entry's party works.
3. `declining_and_removing_are_quiet`: a decline and a removal leave the
   other side's list changed and no notice.
4. `friends_outlive_a_restart`: after `World::restart` the friendship and a
   waiting request are still there. A data folder holding a literal
   today's-format `accounts.txt` and no `friends.txt` opens, and its
   accounts sign in.
5. `friend_lists_come_at_most_once_a_second`: BRAVO toggles PRIVACY ten
   times within a second; ALPHA gets at most two `LiveEvent::Friends` in
   that second, and the last list is right.
6. `the_game_never_hears_of_friends`: a game PC (a raw connection with the
   game's LOGIN, as the tests already make) whose account a launcher asks;
   every kind the raw connection receives is one the game knows (no 117 or
   118, and no friend notice); a FRIEND_REQUEST sent from a game PC drops
   it.
7. `service_records_and_tallies`: two launchers play a ranked Head to Head
   match (playlist 11) through the relay with LAUNCHER_RESULTs carrying
   kills, assists, deaths, betrayals and suicides; the winner's RECORD shows
   the playlist with 1 game, 1 win and the host's numbers; accounts.txt has
   the 11-word `x` lines and games.log the 14-field entries; RECORD for an
   unknown id is `found: None`; 25 RECORDs in one step get at most 20
   answers. A disputed game (results that don't agree) adds nothing.
8. In `server/tests/matches.rs`, after a counted Double Team, each player's
   tally has the kills and deaths of the host's RESULT.

### Part C

`ranks.rs`, on synthetic data only:

- `RankIcons::new` takes 50 and 50 images; refuses 49 or 51 of either, a
  0 by 0 image, and pixels that don't match the size; `icon` gives level 1
  as image 0 and level 50 as image 49, None for 0 and 51, and the right set
  for each `Size`.
- `candidates`: `off`; a file; a folder (its mainmenu.map); unset (the three
  Vista folders, then `maps\mainmenu.map` beside the launcher, in that
  order).
- `find` picks the first that exists (files made in a temporary folder).
- `read` on a missing file, a file of zeros, and a 0x800-byte file with
  `head` and `foot` and version 13 gives errors, the last one saying
  version 13; none panics.
- One `#[ignore]` test reads `mainmenu.map` from `H2_MAPS` and checks 50
  icons of 28 by 26 and 50 of 17 by 17.

### Part B

- results.rs: a synthetic block with kills, assists, betrayals, in a row and
  alive at their offsets reads back; `for_server` carries them (clamped);
  `check_kills` gives each of its three lines.
- names.rs: `slayer` against `playlists.txt`.
- live.rs (h2launch): the 10-field `ended` line round trip; the 7-field form
  is None.
- child.rs: the stand-in's results as in 1.4.
- canvas.rs: a 2 by 2 image with an opaque, a half and a clear pixel, drawn
  4 by 4 over a known background, gives the blended colours and leaves the
  clear corner alone.
- app.rs, with a `LiveClient` on one end of `Connection::pair` and its
  `view` filled in by the test, reading what the App sends from the other
  end:
  - the friends screen orders rows as 3.5 says, and A on a request sends
    FRIEND_ACCEPT, X FRIEND_DECLINE;
  - the add friend popup takes typed letters (not buttons) and sends
    FRIEND_REQUEST once;
  - the options popup lists only what applies, with Remove friend last, and
    up from the top selects it;
  - LB on the carnage report sends RECORD for the selected player, a second
    LB within 60 s sends nothing, and a not-found answer draws "No such
    player.";
  - the record screen draws with and without rank icons (a synthetic
    `RankIcons`).
- mod.rs: `parse_script` reads `lb`, `rb`, `see` and `pick`.
- The three-lobby run above, with its screenshots looked at.

## Work split

**Part A** (h2net and h2live):

- `crates/h2net/src/live.rs`: `LIVE_PROTOCOL` 3, the kinds, limits, types
  and their read and write, and the tests.
- `crates/h2live/src/store.rs`: `Tally`, `Stats::tally`, the `x` line,
  games.log, and the module comment.
- `crates/h2live/src/friends.rs` (new) and `lib.rs`.
- `crates/h2live/src/server.rs` (fields, `open`, `handle`, `Pc`),
  `server/social.rs` (new), `server/matches.rs` (reports and tallies),
  `server/party.rs` if the presence pass sits beside `send_online`.
- `crates/h2live/src/client.rs`: `View::friends`, `LiveEvent::Friends` and
  `LiveEvent::ServiceRecord`.
- The server's module comment and `docs/ONLINE.md` (friends.txt, the new
  `x` fields).
- Just enough in h2launch and h2viewer to build (zeros in the new
  `LauncherPlayerResult` fields; new `LiveEvent`s ignored).

**Part C**: `crates/h2launch/src/lobby/ranks.rs` and its `pub mod` line,
nothing else.

**Part B** (h2launch and docs): results.rs, `win/host.rs` (the kills check),
live.rs (event line), `lobby/child.rs`, `lobby/names.rs`, `lobby/canvas.rs`,
`lobby/window.rs` (bumpers and keys), `lobby/app.rs` (screens, popups,
carnage, record cache, `Pen::level`), `lobby/mod.rs` (ranks at start,
script commands), `docs/notes/launcher/lobby.md` (screens, keys, labels,
the icons and `H2LOBBY_RANKS`, and that MCC's format-13 `mainmenu.map` is a
later step), and README.md in plain words: the carnage report's columns,
friends, service records, and the level icons when a Halo 2 Vista install
is there.

**Deploy** (after A and B are merged on this branch): copy the Proxmox
server's data folder aside, deploy h2live, and give the owner the new
launcher at the same time (a launcher at 2 is told UPDATE YOUR LAUNCHER).
The game's players need nothing. PROGRESS.md says what was deployed.
