# Owner-made accounts (LIVE_PROTOCOL 4)

Since this change, accounts on an h2live server are made only by the
server's owner.

- Each account is a gamertag and a password.
- A friend types the gamertag and the password once on each PC. After that
  the PC is remembered, and ONLINE signs in with no typing.
- Nobody can make an account on a closed server unless an admin makes it
  for them.
- The owner decided this for the first version on 2026-10-11 ("a simple
  username and password I create for my test users"). Signing in with
  Steam may come later.

This note says how it works and why it was built this way. LIVE_PROTOCOL 3
is described in live-v3.md. The Rust game's PROTOCOL stays 26.

Built on `launcher-accounts`: stage 1 (16e80d5, h2net and h2live), stage 2
(3323850, the launcher), stage 3 (this note, the docs and the install
script). Not deployed yet: see "Deploy and rollback".

## What the owner does

- **Make an account.** In the launcher, sign in, then ONLINE, the
  playlists' last row "Accounts", then "Create account". Type the gamertag;
  a password is made for you (A on it makes another), then CREATE shows it
  once. Or run `h2live user add <GAMERTAG>` on the server (or have Claude
  run it), which prints the password once.
- **Give a friend their account.** Send them the gamertag, the password and
  the launcher zip.
- **When something goes wrong.** In Accounts, pick the account:
  - "Give a new password" if they forgot it (their PCs stay signed in).
  - "Sign out of all PCs" if a PC was lost or the password leaked. Every PC
    then needs the password again; a leaked password also needs a new one.
  - "Remove account" to delete the account for good.

  Each asks first, and the answer shows on the screen.
- **What only the command line can do.** Make or unmake admins
  (`h2live user admin <GAMERTAG> on|off`), rename
  (`h2live user rename <GAMERTAG> <NEW GAMERTAG>`), make an admin account
  (`h2live user add <GAMERTAG> --admin`), and anything on his own account
  or another admin's (those options are greyed in the launcher, with "Use
  the server's command line for that.").

## What friends do

1. Unzip the launcher and run it. Choose ONLINE.
2. On the sign-in form, type the gamertag and password the owner sent,
   once (GAMERTAG, PASSWORD, SERVER; Tab moves between them). A keyboard is
   needed for this, until the on-screen keyboard arrives in menu Phase 2.
   The password shows as stars.
3. From then on, ONLINE signs in by itself on that PC.

Anyone can change their own password: ONLINE, then the playlists' last
row "My account", then "Change my password". It asks for the current
password and the new one twice. The new one is then known only to them,
and their PCs stay signed in. An account the owner gave no password yet
can't (the row is greyed: "Ask the server's owner for a password first.").

If the launcher says SERVER KEY CHANGED, the server answered with a key
other than the one this PC saw before. Only go on if the owner said the
server changed (A, "Trust the new key"). Otherwise someone may be
pretending to be it: B cancels, and nothing was sent.

## How it works

### Accounts and PCs

An account is still the line in accounts.txt that it always was: an id, a
key, a gamertag, a look, levels and stats. Nothing in that file changed. A
v3 server can still read the folder, and every existing account keeps its
id. The id is what parties, friends, games.log and the engine's XUID
(through the relay) use, so the owner's account keeps its levels, friends
and XUID.

What signs a PC in is still the Ed25519 key the launcher keeps in
live-key.bin. The server now keeps a list of which keys may sign in to
which account. Those keys are the account's "PCs" (devices).

- An account can have up to 5 PCs. A sixth is refused ("THIS ACCOUNT IS ON
  TOO MANY PCS...") until the owner signs it out of all PCs.
- A key belongs to one account at a time.
- Signing in with a password to another account moves that PC's key there
  (and any session of the old account from that key is signed out).
- Each `--instance` of the launcher has its own key, so it counts as its
  own PC. So does a PC whose launcher folder was wiped.
- Each PC is listed under the name the PC gives (its computer name, cut to
  32 characters) and the day it was added.

Accounts made before this change are adopted the first time the new
server opens the folder. Each one's own key becomes its first PC, listed
as "OLD PC", so it keeps signing in as before. It has no password until
the owner gives it one, and it needs one before it can be added to another
PC or its player can change it. The log
says `live: N accounts from an older server sign in from their PCs as
before`.

A new account needs an id that no PC key produces. The server draws 32
random bytes, uses them as the account's "name key" in accounts.txt, and
the id comes from that key as before. Nobody can sign with a name key; only
PCs sign in. Ids 0, 1 and the two highest values are skipped, because the
relay treats them specially, and so is an id already taken.

### Passwords on the server

Passwords are kept only as argon2id hashes (version 19, memory 19 MiB, 2
passes, 1 lane, a random 16-byte salt, written as a standard "PHC" string
that names its own settings), in a new file, logins.txt. Only the file's
owner may read it.

- **Easy to type.** Spaces and dashes are ignored and case doesn't matter
  (`k7mp 4qr9-xw2t` is `K7MP-4QR9-XW2T`). What's left must be 8 to 64
  printable characters (letters, digits or marks).
- **Generated passwords.** The launcher and the command line make
  12-character passwords from 25 letters and digits that can't be confused
  with one another (no 0 or O, 1, I or L, 2 or Z, 5 or S, 8 or B), shown as
  XXXX-XXXX-XXXX (about 56 bits). They are shown once and not kept.
- **Where hashing happens.** Only when a PC is added, an account is made,
  or a password changes, so a few times a week. A hash's time on the
  owner's server is an estimate (tens of milliseconds) until the first
  `h2live user add` there prints "hashing took N ms". In this project's
  Linux container a release build's `user add` took 30 to 36 ms (stage 3's
  check below).
- **Why the main thread is fine.** The hash runs on the server's main
  thread, at most 30 a minute. The launcher's game traffic runs on the
  relay's own thread, so matches aren't touched.
- **Never kept.** A password in clear lives only in memory that is wiped
  when it's dropped. No password, hash or sealed box is ever logged, and no
  password is written anywhere (a made-up one is shown once, on the screen
  or the command line's output, and then forgotten). A hash read from logins.txt or the admin socket
  is refused if its settings are out of bounds (over 256 MiB, 16 passes or
  4 lanes), so a bad line can't make the server work for minutes.

### Passwords on the wire

The control link is a plain WebSocket, so a password is never sent as it
is.

- **The server's key for this connection.** When a launcher signs in, the
  server makes a one-time X25519 key for that connection and signs it with
  its identity key (SEAL_KEY, just before the CHALLENGE). The identity key
  is an Ed25519 key derived from the same secret that signs stat cards
  (secret.txt or H2LIVE_SECRET), under its own label.
- **Sealing.** The launcher seals the password to that key with a key of
  its own made just for that box (X25519, SHA-256 for the box's key, then
  ChaCha20-Poly1305). Each password is padded to the same size (65 bytes),
  so a box is 81 bytes (146 for old and new), and its length says nothing.
- **What the box is tied to.** Each box is bound to the connection's
  challenge, the PC's key, what it is for (signing in, a new account, a
  new password, changing one's own) and who it is for (the gamertag or the
  account). A box copied from one connection, or sent for another account
  or another action, doesn't open.
- **Forward secrecy.** Both one-time keys are gone when the connection (or
  the box) ends. Someone who records the traffic and later steals the
  server's files still can't open old boxes.
- **Only launchers.** The game (h2viewer) is never sent SEAL_KEY and can't
  send a password; a game that sends PROVE_PASSWORD is dropped.

### Recognising the server

The launcher remembers each server's identity key in `live-servers.txt`
in its folder (a `# ` comment line, then one line per server: the address
as typed, in lower case, and the key in 64 hex digits).

- A launcher built with `H2LAUNCH_SERVER_KEY` set to the key (the friends'
  zip) knows the owner's server (`h2.mohnjorrow.com`, the default) from the
  start. A key it learns later wins over the built-in one.
- Otherwise it remembers the key the first time it signs in (after the
  server's WELCOME; the log says `lobby: pinned <server>'s key <code>`).
- On every sign-in it checks SEAL_KEY before sending anything: it must come
  before CHALLENGE and its signature must check, or the launcher hangs up
  ("THE SERVER DIDN'T PROVE WHO IT IS"). If the key differs from the one
  pinned, it hangs up and shows SERVER KEY CHANGED with both short codes,
  and sends nothing until the player says to trust the new one. A then
  pins the new key and signs in again; B goes back to the main menu.
- A short code is the key's first 8 bytes as `AB12-CD34-EF56-7890`, easy to
  read out. `h2live user key` prints both.

### Owner actions from the launcher

Admins send account requests as one message kind, ACCOUNT. Each request is
signed with the PC's key over the connection's challenge and a number that
only goes up on that connection. Someone who takes over the unencrypted
link can't make up a request or replay one. Passwords inside requests are
sealed as above. One request is asked at a time; with no answer in 10
seconds the launcher says "No answer from the server. Check the list." and
asks for the list again.

A launcher admin can:
- list accounts (done each time the Accounts screen opens);
- make non-admin accounts;
- give a new password;
- sign an account out of all PCs;
- remove an account.

A launcher admin can't do any of these to their own account or to another
admin's. The server's command line can do everything, except leave the
server with no admin.

### The launcher's screens

- **Sign in.** GAMERTAG, PASSWORD ("Only on a new PC"), SERVER. The
  password is drawn as stars, never saved (lobby.txt keeps only the
  gamertag and server) or logged (the log says "(with a password)"), sent
  only sealed, and forgotten when the sign-in ends. ONLINE with a saved
  gamertag still signs in at once by the PC's key. After WELCOME the
  gamertag the server gave is saved; if no password was typed and the
  server signed this PC in as someone else, a notice says "This PC is
  signed in as JOHN. To use KAT here, type KAT's password."
- **Refusals** go back to the form where typing helps:

  | Server says | Launcher shows |
  |---|---|
  | TYPE YOUR PASSWORD... | the form, on the password: "This PC isn't signed in to that account. Type your password." |
  | WRONG GAMERTAG OR PASSWORD | the form, password cleared: "Wrong gamertag or password." |
  | TOO MANY WRONG PASSWORDS... | the form: "Too many wrong passwords. Try again in 10 minutes." |
  | THIS ACCOUNT IS ON TOO MANY PCS... | the form: "This account is on too many PCs. Ask the server's owner to sign it out of all PCs." |
  | YOU WERE SIGNED OUT OF THIS PC... | the form, on the password (after the match, if one is playing): "You were signed out of this PC. Type your password." |
  | UPDATE YOUR LAUNCHER | the offline screen: "The server runs a different version from this launcher. Ask its owner for the right one." |
  | anything else (THIS ACCOUNT WAS REMOVED, THE SERVER IS BUSY...) | the offline screen, as before |

  On the start screen or main menu the text is a notice instead.
- **The playlists' last row**, after Settings, once the server has sent
  ACCOUNTS: "Accounts" for admins ("Make accounts for your friends, give
  new passwords, and change your own."), "My account" for everyone else
  ("Change your password.").
- **ACCOUNTS / MY ACCOUNT.** "Change my password" first; for admins then
  "Create account" and every account, each with "n PCs" and the tags ADMIN,
  NO PASSWORD, ONLINE and (YOU). Nine rows show at once and the list
  scrolls; the selection stays on its account when the list changes. The
  right side shows the selected account's PCs (n of 5), password, whether
  it's online, its level and a note. B goes back to the playlists.
- **Popups.** NEW ACCOUNT (gamertag, the made-up password, CREATE), then
  ACCOUNT CREATED with the password ("Give it to them; it won't be shown
  again."); the account options (Give a new password, Sign out of all PCs,
  Remove account), each confirmed first (NEW PASSWORD, SIGN OUT, REMOVE
  ACCOUNT), a new password shown once the same way; CHANGE PASSWORD
  (CURRENT PASSWORD, NEW PASSWORD, NEW PASSWORD AGAIN, all stars, checked
  on the PC first: "Type your current password.", "The new passwords don't
  match.", or the password rules); SERVER KEY CHANGED.
- **`--live`** (the test mode without the lobby) checks the pin, never
  sends a password, and on a PC the account doesn't know stops with "this
  PC isn't signed in to that account: sign in once from the lobby (same
  --instance) with your password".

## Files on the server (in the data folder)

**accounts.txt.** Unchanged. A new account's `a` line reads as any other.

**logins.txt** (new, private: mode 600 on Linux):

```
# h2live sign-ins: who may sign in to the accounts in accounts.txt.
# Private: it holds password hashes. h2live rewrites it; edit it only while h2live is stopped.
u <account id, 16 hex digits> <admin 0|1> <password hash, or ->
d <account id> <PC key, 64 hex digits> <added, Unix time> <PC name>
```

One `u` line per account, then a `d` line per remembered PC. The server
rewrites logins.txt whole. Edit it only while h2live is stopped. A line the
server doesn't understand (a `d` line before its `u` line, an account or a
PC listed twice, a bad hash, any other first letter) stops it from
starting, with the line number. Lines for accounts that accounts.txt
doesn't have are dropped, with a log line. A folder with no accounts gets
no logins.txt.

**admin.sock.** The command line's way in while the server runs. Unix
only, mode 600, removed when the server stops. If the folder's path is too
long for a socket (about 90 characters or more), the server says so in its
log and the command line then works only while the server is stopped.

**h2live.lock.** Held while the server runs. A second server on the same
folder refuses to start ("another h2live is using <folder>"), and the
command line uses this lock to tell whether the server is running.

**Write order**, so a crash never lets anyone in:
- A new account's logins line is written before its accounts.txt line (a
  logins line with no account is dropped at the next start).
- A removed account leaves accounts.txt (and friends.txt) before
  logins.txt (an account without a logins line would be adopted, its name
  key signing in again).

An ordinary sign-in writes nothing new. Back up the whole folder.
logins.txt and secret.txt can't be rebuilt from stat cards.

## Server rules

**When a launcher signs in with only its PC key (PROVE):**
- **The key is one of an account's PCs:** it signs in as that account.
- **The key made an account but was signed out:** "TYPE YOUR PASSWORD"
  (the game gets "YOU WERE SIGNED OUT...").
- **The key is unknown and sign-up is closed:** "TYPE YOUR PASSWORD" (the
  game gets "THIS SERVER IS INVITE ONLY...").
- **The key is unknown and sign-up is open:** a new account, as before
  (or one a stat card brings back), with that key as its first PC.

**When it signs in with a password (PROVE_PASSWORD, the same proof of its
key plus the sealed password)**, checked in this order:
- **More than 30 password checks in the last minute on the whole server:**
  "THE SERVER IS BUSY".
- **10 wrong tries from one address in 10 minutes:** "TOO MANY WRONG
  PASSWORDS" for that address.
- **5 wrong tries for the account in 10 minutes:** the same, with no hash
  and no new count (so the lock isn't made longer). Its remembered PCs
  still sign in, so guessing can't lock a friend out of play.
- **Wrong password or unknown gamertag:** the same answer, "WRONG GAMERTAG
  OR PASSWORD", counted for the account (if there is one) and the address,
  and logged with the gamertag and address.
- **A right password:** the account's wrong count is cleared. From a new
  PC, the PC is added (up to 5) and the log says `live: KAT signed in with
  a password from 1.2.3.4 (new PC "DESKTOP-1", 2 of 5)`.

The existing limit of 10 connections a minute from one address still
applies, and every try is a new connection.

**Names:**
- Only the owner names accounts.
- On a closed server, or for an account with a password, the gamertag a PC
  asks for never renames it (a PROFILE asking for another name is told
  "ASK THE SERVER'S OWNER TO CHANGE YOUR GAMERTAG.").
- Gamertags the owner makes are 1 to 15 capital letters, digits and single
  spaces, starting with a letter. That's stricter than what players could
  pick before, so one never looks like a splitscreen guest's "NAME(1)".
  Names are unique in any case.

**Stat cards:**
- Still kept by PCs.
- A card only raises the record of the account that just signed in.
- On a closed server a card never makes or brings back an account, and a
  card never changes the name of an account the server has.

**Removing an account:**
- signs its PC out ("THIS ACCOUNT WAS REMOVED.");
- takes it out of its party, the search and its matches;
- drops its friendships and friend requests (and refreshes the friends'
  lists);
- frees the gamertag.

The account can't come back: its id is random and never reused, and a
closed server ignores its card. games.log keeps its history.

**The last admin** can't be removed or unmade.

## LIVE_PROTOCOL 4 messages

The launcher's LOGIN is laid out as in version 3, but for the version
number. CHALLENGE, PROVE, WELCOME and REFUSED are unchanged. The game is
never sent the new kinds.

| From | Kind | Name | What it carries |
|---|---|---|---|
| PC | 6 | PROVE_PASSWORD | the usual proof (signature), the PC's name, the sealed password; answers CHALLENGE instead of PROVE |
| PC | 60 | ACCOUNT | a number, an operation, a signature. Operations: list (0), create {gamertag, sealed password} (1), new password {account, sealed password} (2), sign out of all PCs {account} (3), remove {account} (4), change my password {old and new, sealed} (5) |
| server | 119 | SEAL_KEY | the connection's X25519 key, the server's identity key, the signature; just before CHALLENGE |
| server | 120 | ACCOUNTS | admin or not, has a password or not, which request it answers (0 for none), ok and a message, how many accounts there are, and account rows (gamertag, admin, password, online, PCs, best level): all of them for admins (at most 300), only their own for others |

A sealed box is the sender's one-time key (32 bytes) and the box (17 to
146 bytes, with its length first). ACCOUNTS is sent after WELCOME, after
each ACCOUNT, and to every launcher signed in whenever an account changes.
Operations other than 5 from a non-admin, a badly signed or replayed
request, or a box that doesn't open, drop the PC.

New refusal texts (each closes the link):
- TYPE YOUR PASSWORD. THIS PC ISN'T SIGNED IN TO THAT ACCOUNT.
- WRONG GAMERTAG OR PASSWORD
- TOO MANY WRONG PASSWORDS. TRY AGAIN IN 10 MINUTES.
- THE SERVER IS BUSY. TRY AGAIN IN A MINUTE.
- THIS ACCOUNT IS ON TOO MANY PCS. ASK THE SERVER'S OWNER TO SIGN IT OUT
  OF ALL PCS.
- YOU WERE SIGNED OUT OF THIS PC. SIGN IN WITH YOUR PASSWORD.
- THIS ACCOUNT WAS REMOVED.
- THIS SERVER IS INVITE ONLY. ASK ITS OWNER FOR AN ACCOUNT. (the game only)

A version 3 launcher meeting this server is told UPDATE YOUR LAUNCHER, and
so is a version 4 launcher meeting an older server. That is why the server
is updated first.

## The command line

The server's own program, with arguments:

```
h2live user list
h2live user add <GAMERTAG> [--admin] [--password-stdin]
h2live user passwd <GAMERTAG> [--password-stdin]
h2live user revoke <GAMERTAG>          (sign out of all PCs)
h2live user remove <GAMERTAG> --yes    (without --yes: says what would be lost)
h2live user admin <GAMERTAG> on|off
h2live user rename <GAMERTAG> <NEW GAMERTAG>
h2live user key                        (the identity key and its short code)
```

Any of them takes `--data <folder>`; a gamertag with a space goes in
quotes. Exit codes: 0 done, 1 refused or failed, 2 not understood (with the
usage).

- **The data folder** is found as the server finds it: `--data`, else
  `H2LIVE_DATA`, else `data=` in h2live.txt beside the program, else
  `h2live-data`. The command line never makes one.
- **Running server:** the commands go to it through admin.sock (one
  tab-separated request line, answered by `row` and `pc` lines and then
  `ok TEXT` or `error TEXT`), so they are safe while people play. Only the
  server ever writes its files.
- **Stopped server:** the command line takes the lock and opens the folder
  itself, running the same code. On Linux it first checks that it runs as
  the folder's owner, so root can't leave files the server can't read.
  On Windows a running server can't be asked: stop it first.
- **Passwords:** made for you and printed once (`KAT created. Password:
  K7MP-4QR9-XW3T (shown once; it isn't kept anywhere)`), or read from one
  line of standard input with `--password-stdin`. They are never taken as
  an argument, so they don't end up in shell history. The command line does
  the hashing itself (printing "hashing took N ms"), so no plain password
  crosses the socket.
- **`list`** shows each account's gamertag, admin, password, PCs (n/5,
  then each PC's name and date), online, level and the day it was made.
- **On the Proxmox container:** `/usr/local/bin/h2live` runs these commands
  as the h2live user on /var/lib/h2live (`h2live user list` in a root
  shell there, or `pct exec 102 -- h2live user list` on the host).

## The sign-up setting

`signup=open`, `closed` or `auto` goes in h2live.txt beside the program,
or in `H2LIVE_SIGNUP`; the environment wins. Anything else stops the
server with a message.

- **auto** (the default) is closed once any account is an admin, and open
  otherwise. Tests, local servers, h2viewer's own server and Render keep
  working as before.
- **The owner's server** is set to closed in both places by the install
  script.
- **Render** keeps its data in /tmp, which is wiped on restart, so a closed
  server there would lose its logins. It stays on auto, which is open while
  it has no admin.

At start the log says `sign-up: closed (accounts are made with "h2live
user add" or the launcher's ACCOUNTS screen)` (or which kind of open), and
`no admins yet: run "h2live user admin <gamertag> on"` while a closed
server has none. Open set by hand while there are admins logs a warning.

| | open | closed |
|---|---|---|
| unknown PC, launcher | new account (as before) | TYPE YOUR PASSWORD |
| unknown PC, game | new account (as before) | THIS SERVER IS INVITE ONLY |
| a remembered PC | signs in | signs in |
| password sign-in | works | works |
| rename from LOGIN or PROFILE | as before, unless the account has a password | never |
| stat card | as before | only raises an existing account's record |

## Limits

- 5 PCs per account.
- 5 wrong passwords per account, and 10 per address, within 10 minutes.
- 30 password checks or hashes a minute per server.
- Passwords of 8 to 64 characters (not counting spaces and dashes).
- Gamertags the owner makes: 1 to 15 characters.
- PC names: 32 characters.
- At most 300 accounts listed in the launcher.

## Deploy and rollback

The server goes first, and only when the owner says so.

1. **Install.** `deploy/proxmox/install-h2live.sh` keeps the old program as
   h2live.prev and a dated copy of the data folder
   (`/var/lib/h2live.backup-<date>-<time>`). It writes
   `/opt/h2live/h2live.txt` (`data=/var/lib/h2live`, `signup=closed`), adds
   `H2LIVE_SIGNUP=closed` and `UMask=0077` to the unit, installs the
   `/usr/local/bin/h2live` wrapper, and restarts. The new server adopts
   the existing accounts; its log shows the sign-up lines.
2. **Owner's account.** `h2live user list` to find it, `h2live user admin
   <OWNER> on`, then `h2live user passwd <OWNER>`, and give him the
   password. Note the "hashing took N ms" line (over about 150 ms: lower
   the memory, or plan a worker thread).
3. **Launchers.** `h2live user key`, then rebuild the owner's launcher and
   the friends' zip with `H2LAUNCH_SERVER_KEY` set to that key, and rewrite
   the zip's READ ME ("type the gamertag and password John gave you, once
   on each PC"; the READ ME isn't in the repository).
4. **Accounts.** The owner's launcher signs in by its PC and shows
   Accounts; the owner creates the friends' accounts there.

To roll back, `install -m755 /opt/h2live/h2live.prev /opt/h2live/h2live &&
systemctl restart h2live`, and friends go back to the old zip. The version
3 server reads the folder and ignores the new files and the `signup=`
line, but it is open sign-up again. Accounts the owner made can't sign in
under it, though they are kept for later. Rolling forward again adopts any
accounts it made meanwhile, so check `h2live user list`. For an exact
earlier state, restore the dated copy.

## What it doesn't protect

- **Session takeover.** Once signed in, the link is still unencrypted, as
  before. Someone in the middle could take over a session or read parties
  and relay keys, though they can't learn passwords or forge account
  requests. Encrypting every launcher message is a later step.
- **First sign-in without the built-in key.** A launcher without the
  built-in key trusts the server it reaches the first time. The friends'
  zip carries the key.
- **A copied key from the owner's PC.** It gives the ACCOUNTS powers until
  `h2live user revoke` on his account. Asking for his password before
  ACCOUNTS opens is a possible later step.
- **Typed passwords.** A weak typed password could be cracked from a
  stolen logins.txt. Generated ones can't in practice.
- **A stolen secret.txt.** It lets someone pose as the server and forge
  stat cards. Past boxes stay closed.
- **Windows key files.** On Windows, live-key.bin is protected only by the
  user's own folder.
- **Lost logins.txt.** If logins.txt is lost but accounts.txt isn't, the
  next start adopts every account again with its own key, so PCs that had
  been signed out sign in again, and admin flags are lost (sign-up stays
  closed through the explicit setting; `h2live user admin` brings the
  flag back). Hence the backups.

## Choices and why

- **Keys still sign in, and the password is only for adding a PC.** It is
  what "type it once per PC" needs, and normal sign-ins carry nothing
  secret.
- **accounts.txt and cards unchanged; secrets in their own file.** Rollback
  stays possible, and cards never carry secrets.
- **A sealed box instead of TLS.** No certificates are needed behind the
  allowlist, the crates are pure Rust, and one-time keys give forward
  secrecy. A password-authenticated exchange (OPAQUE) was left as the
  upgrade if friends ever sign in from untrusted networks without the zip.
- **Hashing on the main thread, with caps.** It happens rarely, and a
  worker thread would need a waiting state, stable PC numbers and test
  clocks.
- **Admin powers split between launcher and command line.** The launcher
  does day-to-day jobs; making admins and touching admins stays on the
  command line, which is always the way back in.
- **Reset and sign out are separate.** A forgotten password doesn't cost a
  friend their PCs; for a leaked one the owner does both.
- **Auto sign-up by default.** Local servers and tests stay open with no
  setting; the owner's server is closed explicitly, twice, so losing one
  setting doesn't open it.
- **No on-screen keyboard yet.** Phase 2 brings it, and these fields all
  take their typing in one place, ready for it.
- **The Accounts row on the playlists screen.** It sits after Settings,
  because Settings is Halo 2's controller screen with no free row, and the
  row needs the server anyway.
- **`h2live user key`, not `h2live key`.** Every command is under `user`,
  so the program has one word to tell a command from the server.

## Tests

None of the tests need the internet or game files.

- **h2net:** every message round-trips, every kind has its own number,
  bad lengths and too many rows are errors, and a sealed box's Debug shows
  only its length.
- **Sealing:** a fixed known answer (worked out separately when it was
  written), so launcher and server can't drift apart; boxes don't open with any part of the extra data changed, a bit
  flipped or a zero key.
- **Passwords and logins.txt:** the rules, the generator, hashes, a
  round trip of the file, its errors, adoption, mode 600, and a v3 folder
  left byte for byte as it was.
- **Server:** 30 tests in `crates/h2live/src/server/tests/accounts.rs`
  drive the sign-in rules, locks and limits, PCs, admin actions and their
  guards, removal, cards, renames, the game never hearing the new kinds, a
  v3 launcher told to update, and the auto setting, on simulated time.
- **Command line:** `crates/h2live/tests/program.rs` runs the real program
  against a running and a stopped server, a second server on one folder, a
  password given as an argument, a missing folder, and `/admin` still 404.
- **Launcher:** tests sign in against a real server in the same process
  (and one played by the test, to read every byte sent). They check that a
  password is never drawn, logged or sent unsealed, the refusals, that a
  changed server key sends nothing, creating an account and signing in
  with the password shown, new password, sign out, remove, changing a
  password, and that every new screen and popup draws and takes clicks.
- **Stage 3's end-to-end check** (Linux, release builds, 2026-10-11): a
  closed local server, the command line while it ran and while it was
  stopped, and headless lobbies (`lobby.md` has the run):
  - `h2live user add "ADMIN ONE" --admin` and `user add "FRIEND ONE"` over
    the socket, `user list`, `user key`; hashes took 30 to 36 ms.
    logins.txt, secret.txt and admin.sock were mode 600, and a second
    h2live on the folder stopped with "another h2live is using ...".
  - FRIEND ONE typed the gamertag and password (lower case, spaces for the
    dashes) and signed in; started again, it signed in by its PC alone.
  - ADMIN ONE opened Accounts, created KAT, gave FRIEND ONE a new password
    and signed it out of all PCs; FRIEND ONE's lobby, on the playlists, went
    to the form ("You were signed out of this PC. Type your password.") and
    signed in again with the password `user passwd --password-stdin` gave.
    ADMIN ONE's own options were greyed.
  - KAT signed in with the password ACCOUNT CREATED showed, changed it on
    My account, and another instance was refused the old one and let in
    with the new.
  - With the server stopped, `user add`, `user list` and `user remove`
    (without --yes) worked on the folder, and the new account signed in
    once the server was back.
  - With another `H2LIVE_SECRET` the key changed: a pinned lobby with a
    password typed showed SERVER KEY CHANGED and sent nothing (the server
    saw only the connection close); one that trusted the new key signed in
    by its PC.
  - No log or launcher file held a password (in any spelling), a hash or a
    sealed box.
