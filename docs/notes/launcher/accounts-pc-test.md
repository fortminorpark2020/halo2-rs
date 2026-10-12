# Owner-made accounts: test plan for the owner's PC

This plan is for the Remote Control session on the owner's Windows PC. It tests
`launcher-accounts` at `SRC`: 74a01701355d, or a newer tip if the branch
moves. That covers h2live `LIVE_PROTOCOL` 4 with its command line, and the
launcher's sign-in form, ACCOUNTS screen and key pin. Everything runs against
a **local** h2live on 127.0.0.1:47270 with its own data folder. Nothing here
talks to the Proxmox server (192.168.8.102) or to h2.mohnjorrow.com:

- every lobby gets `--live 127.0.0.1:47270` and an `--instance acct-...`;
- both servers bind 127.0.0.1, so h2live doesn't ask the router for ports.

The session can build, run programs and post to the thread. It can't push,
and doesn't need to.

## Rules for the whole run

- **No password ever goes into a message, a pasted log line or a command
  line.**
  - Passwords the CLI makes up go from its output straight into
    `C:\h2work\acct-test\secrets\<KEY>.pw`.
  - Passwords made up here go there too (`new_pw`).
  - Lobby scripts get them from those files at run time (`render`), and the
    filled-in copy is deleted after each run.
  - Never `cat`, `type`, `head` or `Get-Content` these:
    - a `.pw` or `run-*.txt` file;
    - `data\logins.txt`;
    - anything else in `secrets`.

  What goes back to the thread:
  - pass or fail for each step, and the timings;
  - log lines that hold no password or hash (gamertags, short key codes,
    counts and the hex identity key are fine).
- Don't touch the owner's own launcher files. They are in
  `%LOCALAPPDATA%\h2launch` (`lobby.txt`, `live-key.bin`, `lobby.log`) and in
  any of his instance folders. Only `acct-*` instance folders are made and
  deleted.
  - Check first: `ls "$(cygpath -m "$LOCALAPPDATA")/h2launch"`.
  - If any `acct-*` folder already exists, stop and ask.
- Screenshots: none are needed. The engine may save some in its instance
  folder (P10). They stay on the PC and are deleted in P11.
- Commands are for the session's Bash tool (Git Bash). Paths are written
  `C:/...`, which Git Bash and the Windows programs both accept.
- **Pacing.** h2live allows 10 sign-ins a minute from one address. Every
  connection a lobby makes counts, refused ones too, and every lobby here is
  127.0.0.1. Restarting h2live clears that count, and the wrong-password
  counts with it. The steps are spaced to stay under the limit. If a lobby
  log still says `TOO MANY SIGN-INS, WAIT A MINUTE`:
  - that isn't a fail;
  - restart h2live if the step allows it, or wait a minute;
  - then run the step again.
- A step **passes** only when every item under "Pass" holds. When one fails:
  - keep the logs and write down what was seen;
  - carry on, unless the step says "stop".

## P0. Build and set up

### P0.1 The version 4 programs

`C:\h2work\accounts` should hold the branch. If it doesn't exist, make it
from the clone the PC already has, `C:\h2work\launch`.

```
[ -d C:/h2work/accounts ] || { git -C C:/h2work/launch fetch origin && git -C C:/h2work/launch worktree add --detach C:/h2work/accounts origin/launcher-accounts; }
cd C:/h2work/accounts && git fetch origin && git checkout --detach origin/launcher-accounts
git rev-parse HEAD                      # write down as SRC (74a01701355d... or newer)
git status --short                      # must print nothing
echo "${H2LAUNCH_SERVER_KEY-unset}"     # must print unset: the test launcher has no built-in key
cargo build --release -p h2live -p h2launch
./target/release/h2launch.exe --version # SRC's first 12 digits, without "-dirty"
cargo test --release -p h2live          # optional; a failure is reported, not a stop
```

### P0.2 The version 3 programs

The Proxmox server runs `launcher-friends` af23128. The owner's desktop
launcher and the friends' zip are built from `launcher-menu` 18279ad. Both
use live protocol 3.

```
[ -d C:/h2work/accounts-v3 ] || git -C C:/h2work/accounts worktree add --detach C:/h2work/accounts-v3 af23128
cd C:/h2work/accounts-v3 && git checkout --detach af23128
CARGO_TARGET_DIR=C:/h2work/target-v3 cargo build --release -p h2live
mkdir -p C:/h2work/acct-test/bin3 && cp C:/h2work/target-v3/release/h2live.exe C:/h2work/acct-test/bin3/
git checkout --detach 18279ad
CARGO_TARGET_DIR=C:/h2work/target-v3 cargo build --release -p h2launch
```

### P0.3 The test folder, the settings and the helpers

```
T=C:/h2work/acct-test
mkdir -p $T/bin $T/bin3 $T/data $T/logs $T/scripts $T/secrets
cp C:/h2work/accounts/target/release/h2live.exe $T/bin/
printf 'port=47270\nbind=127.0.0.1\ndata=C:/h2work/acct-test/data\n' > $T/bin3/h2live.txt
printf 'port=47270\nbind=127.0.0.1\ndata=C:/h2work/acct-test/data\nsignup=closed\n' > $T/bin/h2live.txt
netstat -ano | grep -c ':47270 '     # must be 0
```

Settings go in an `h2live.txt` beside each program, as on the Proxmox
server. Don't use `PORT=`, which tells h2live it runs behind a hosting proxy.

Write `C:/h2work/acct-test/env.sh` as below. Source it at the start of every
call with `. C:/h2work/acct-test/env.sh`.

```bash
T=C:/h2work/acct-test; S=$T/secrets; SLOG=$T/logs/h2live.log
H=$T/bin/h2live.exe
L=C:/h2work/accounts/target/release/h2launch.exe     # version 4
L3=C:/h2work/target-v3/release/h2launch.exe          # version 3 (18279ad)
LAD=$(cygpath -m "$LOCALAPPDATA")/h2launch
SERVER=127.0.0.1:47270
unset H2LIVE_DATA H2LIVE_SIGNUP H2LIVE_SECRET H2LIVE_BIND H2LIVE_RELAY PORT H2LAUNCH_SERVER_KEY
export H2LIVE_UPNP=0
mark()    { wc -l < "$SLOG" 2>/dev/null || echo 0; }          # the server log's length now
since()   { tail -n +"$(( $1 + 1 ))" "$SLOG"; }               # server log lines after a mark
started() { for i in $(seq 1 60); do since "$1" | grep -qF "$2" && return 0; sleep 0.5; done; echo "NOT STARTED"; return 1; }
stop_live() { powershell -NoProfile -Command "Get-Process h2live -ErrorAction SilentlyContinue | Where-Object { \$_.Path -like 'C:\h2work\acct-test\*' } | Stop-Process -Force"; }
# need <file|-> <text>...: every text must be there; none <file|-> <text>...: none may be
need() { local d; if [ "$1" = - ]; then d=$(cat); else d=$(cat "$1" 2>/dev/null); fi; shift
  for t in "$@"; do grep -qF -- "$t" <<<"$d" && echo "ok: $t" || echo "MISSING: $t"; done; }
none() { local d; if [ "$1" = - ]; then d=$(cat); else d=$(cat "$1" 2>/dev/null); fi; shift
  for t in "$@"; do grep -qF -- "$t" <<<"$d" && echo "UNEXPECTED: $t" || echo "ok, absent: $t"; done; }
# h2live's command line on the test folder; prints the exit code
cli()    { "$H" "$@" < /dev/null; echo "[exit $?]"; }
cli_in() { local f=$1; shift; "$H" "$@" < "$f"; echo "[exit $?]"; }    # --password-stdin from a .pw file
# a made-up password: kept in $S/<KEY>.pw, and the output shown without it
cli_pw() { local k=$1; shift; "$H" "$@" < /dev/null > "$S/$k.out"; local rc=$?
  sed -n 's/.*Password: \([A-Z0-9-]*\) (shown once.*/\1/p' "$S/$k.out" > "$S/$k.pw"
  sed "s/Password: [A-Z0-9-]*/Password: (kept in $k.pw)/" "$S/$k.out"; rm -f "$S/$k.out"
  echo "[exit $rc] $k.pw has $(wc -c < "$S/$k.pw") bytes (15 expected)"; }
# a password made up here (12 characters of h2live's alphabet), for --password-stdin and scripts
new_pw() { head -c 4096 /dev/urandom | LC_ALL=C tr -dc 'ACDEFGHJKMNPQRTUVWXY34679' | head -c 12 > "$S/$1.pw"; echo >> "$S/$1.pw"; }
# fill in a script template: {GT}, {K} and {EXPECT} from the environment, then
# {PW:KEY} and {PWLOW:KEY} (lower case, dashes as spaces) from $S/KEY.pw
render() { local t f n p; t=$(cat "$1"); t=${t//"{GT}"/${GT:-}}; t=${t//"{K}"/${K:-}}; t=${t//"{EXPECT}"/${EXPECT:-}}
  for f in "$S"/*.pw; do [ -e "$f" ] || continue; n=$(basename "$f" .pw); p=$(head -n1 "$f")
    t=${t//"{PW:$n}"/$p}; t=${t//"{PWLOW:$n}"/$(printf %s "$p" | tr 'A-Z-' 'a-z ')}; done
  printf '%s\n' "$t" > "$2"; }
# a headless lobby: lobby <instance> <template> <tag>. Set REAL=1 for MCC's engine,
# EARGS for the engine's flags, and LX for another launcher (the version 3 one).
# Its log goes to $T/logs/<instance>-<tag>.log.
lobby() { local inst=$1 tag=$3 run="$S/run-$1-$3.txt"; render "$2" "$run"
  local fake=(H2LOBBY_FAKE=1); [ "${REAL:-0}" = 1 ] && fake=()
  env -u H2LOBBY_FAKE H2LOBBY_HEADLESS=1 H2LOBBY_MENU=off H2LOBBY_RANKS=off H2LOBBY_H2FONTS=off "${fake[@]}" \
    H2LOBBY_SCRIPT="$run" H2LOBBY_ENGINE_ARGS="${EARGS:-}" \
    "${LX:-$L}" --lobby --live "$SERVER" --instance "$inst" > "$T/logs/$inst-$tag.log" 2>&1 < /dev/null
  local rc=$?; rm -f "$run"; echo "[lobby $inst $tag: exit $rc]"; return $rc; }
LOG() { echo "$T/logs/$1-$2.log"; }    # a lobby run's log: LOG <instance> <tag>
```

**Starting a server.** Always start it with the Bash tool's
`run_in_background`, so it outlives the call. Its input is empty, so a
FAILED start doesn't sit waiting for Enter.

- Version 4: `. C:/h2work/acct-test/env.sh && cd $T/bin && ./h2live.exe < /dev/null >> $SLOG 2>&1`
- Version 3: the same, from `$T/bin3`.
- Settings from the environment go in front, for example
  `H2LIVE_SIGNUP=open ./h2live.exe ...`.

Take `M=$(mark)` before each start, then wait for the relay line, which
h2live logs last: `started $M 'relay on UDP port 47270'`. Stop the server
with `stop_live`.

### P0.4 Script templates

Write these once into `$T/scripts`. They hold placeholders, never passwords.
A line starting `#` is a comment. Each line is `<seconds> <command>`, with
the seconds counted from the step before; a `wait` or `see` holds the script
until it is met.

`dev.txt` signs in by this PC's key, with the gamertag saved on the instance:
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait live 20
2 quit
```
`form.txt` is a first sign-in with a password:
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait signin 10
0 type {GT}
0 tab
0 type {PW:{K}}
0.3 a
0 wait live 20
2 quit
```
- `form-low.txt` is `form.txt` with `{PWLOW:{K}}` in place of `{PW:{K}}`.
- `newpc.txt` is `form.txt` without its `0 tab` and `0 type {PW:{K}}` lines:
  a gamertag and no password, expecting to get in.

`nopw.txt` is a gamertag and no password, expecting to be refused with
`{EXPECT}`:
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait signin 10
0 type {GT}
0.3 a
0 see {EXPECT} 20
1 quit
```
`devpw.txt` has a saved gamertag that the server refuses with `{EXPECT}`,
then types the password:
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait signin 20
0 see {EXPECT} 5
0 type {PW:{K}}
0.3 a
0 wait live 20
2 quit
```
`v3seed.txt` is for the version 3 launcher (18279ad): start screen, ONLINE,
then its sign-in form, which has no password field:
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait signin 10
0 type {GT}
0.3 a
0 wait live 20
2 quit
```
`v3old.txt` is `v3seed.txt` with its last two lines replaced by
`0 see UPDATE YOUR LAUNCHER 20` and `1 quit`.

The other templates are given in their steps.

### P0.5 Optional: a malformed built-in key fails the build

This is cheap: it is a check, not a build, and it doesn't touch the exe
from P0.1.

```
cd C:/h2work/accounts
H2LAUNCH_SERVER_KEY=1234-ABCD CARGO_TARGET_DIR=C:/h2work/target-keycheck cargo check --release -p h2launch 2>&1 | grep -c 'H2LAUNCH_SERVER_KEY must be the 64 hex digits'
H2LAUNCH_SERVER_KEY=$(printf '%064d' 0) CARGO_TARGET_DIR=C:/h2work/target-keycheck cargo check --release -p h2launch; echo "exit $?"
rm -rf C:/h2work/target-keycheck
```

Pass:
- the first command counts 1 or more, and its check failed;
- the second, with 64 hex digits (a dummy key), exits 0.

## P1. A version 3 data folder (the Proxmox server as it is now)

1. Start the version 3 server: `M=$(mark)`, start it from `bin3`, then
   `started $M 'relay on UDP port 47270'`.
2. Make three version 3 accounts. Each is made by its PC signing in with the
   desktop or zip launcher:
   ```
   GT=V3OWN LX=$L3 lobby acct-v3own $T/scripts/v3seed.txt seed
   GT=V3FRIEND LX=$L3 lobby acct-v3friend $T/scripts/v3seed.txt seed
   GT=V3SPARE LX=$L3 lobby acct-v3spare $T/scripts/v3seed.txt seed
   ```
   Pass:
   - all three exit 0;
   - `since $M | grep -c 'signed in from'` is 3.
3. A version 4 launcher on the version 3 server:
   `GT=V4EARLY EXPECT='different version' lobby acct-v4early $T/scripts/nopw.txt v3srv`

   Pass:
   - it exits 0;
   - `need $(LOG acct-v4early v3srv) "lobby: The server runs a different version from this launcher. Ask its owner for the right one."`
   - `ls $LAD/acct-v4early/live-servers.txt` finds nothing.
4. `stop_live`. Then keep a copy to start over from:
   ```
   cp -a $T/data $T/data-v3-seed
   mkdir -p $T/inst-v3-seed && cp -a $LAD/acct-v3own $LAD/acct-v3friend $LAD/acct-v3spare $T/inst-v3-seed/
   sha256sum $T/data/accounts.txt $T/data/secret.txt > $T/logs/v3-sums.txt
   ```
   To redo anything from P2 on:
   - stop h2live;
   - put `data-v3-seed` back as `data`, and the three instance folders back;
   - delete the other `acct-*` folders.

## P2. The first version 4 start: adoption, closed sign-up, refusals

`bin/h2live.txt` says `signup=closed` (from P0).

1. Start version 4: `M=$(mark)`, start it, then
   `started $M 'relay on UDP port 47270'`. Pass:
   ```
   since $M | need - 'h2live, protocol 26 (launchers: live protocol 4)' 'settings from C:' \
     'live: 3 accounts from an older server sign in from their PCs as before' 'accounts are kept in C:' \
     'sign-up: closed (accounts are made with "h2live user add" or the launcher'"'"'s ACCOUNTS screen)' \
     'no admins yet: run "h2live user admin <gamertag> on"' 'relay on UDP port 47270'
   since $M | none - FAILED ignored warning
   ```
2. The files. Pass:
   - `sha256sum -c $T/logs/v3-sums.txt` says OK for both files, so
     accounts.txt and secret.txt are untouched;
   - `grep -c '^u ' $T/data/logins.txt` is 3;
   - `grep -c ' OLD PC$' $T/data/logins.txt` is 3;
   - `grep -c argon2 $T/data/logins.txt` is 0.

   These are counts only. Never print the file.
3. The web side. Pass:
   - `curl -s http://127.0.0.1:47270/health` prints `OK`;
   - `curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:47270/admin`
     prints 404 (no admin over HTTP).
4. One server per folder:
   `cd $T/bin && timeout 20 ./h2live.exe < /dev/null; echo "exit $?"`

   Pass:
   - it prints `FAILED: another h2live is using` and the data folder, then
     exits 1;
   - `curl -s http://127.0.0.1:47270/health` still prints `OK`.
5. The command line while the server runs: `cli user list`.

   Pass: it prints `h2live is running; stop it and try again` and
   `[exit 1]`. On Windows the command line works only while the server is
   stopped; Linux uses the admin socket instead.
6. The version 3 launcher on the version 4 server:
   `GT=V3LATE LX=$L3 lobby acct-v3late $T/scripts/v3old.txt v4srv`

   Pass:
   - it exits 0, having seen `The server turned us away: UPDATE YOUR LAUNCHER`;
   - `since $M | need - 'dropped: UPDATE YOUR LAUNCHER'`
   - `since $M | none - V3LATE`
7. An adopted account signs in from its old PC, and has no password yet.
   `myacct.txt` is these lines:
   ```
   0 wait start 10
   1 a
   0 see ONLINE 5
   0.3 a
   0 wait live 20
   1 see My account 10
   ```
   then twelve lines of `0.1 down`, then:
   ```
   0.3 a
   0 wait accounts 10
   0.5 a
   0 see Ask the server's owner for a password first 5
   1 b
   0 wait live 5
   1 quit
   ```
   Run `lobby acct-v3friend $T/scripts/myacct.txt adopted`. Pass:
   - it exits 0;
   - `since $M | need - 'live: V3FRIEND signed in from 127.0.0.1 (launcher'`
   - `since $M | none - 'with a password'`
   - `need $(LOG acct-v3friend adopted) 'lobby: signing in to ws://127.0.0.1:47270/live' "lobby: pinned 127.0.0.1:47270's key" 'lobby: signed in to 127.0.0.1:47270 as V3FRIEND'`
   - `none $(LOG acct-v3friend adopted) 'with a password'`
   - write down the code in the `pinned` line as **K1code**. It is the form
     `XXXX-XXXX-XXXX-XXXX`, a public key, not a secret.
8. A new PC with no password, on a closed server:
   `GT=NEWPC EXPECT="isn't signed in to that account" lobby acct-newpc $T/scripts/nopw.txt closed`

   Pass:
   - it exits 0;
   - `since $M | need - "dropped: TYPE YOUR PASSWORD. THIS PC ISN'T SIGNED IN TO THAT ACCOUNT."`
   - `since $M | none - 'NEWPC signed in'`
9. `stop_live`.
10. Auto sign-up, on a throwaway copy, with the data folder taken from the
    environment:
    ```
    cp -a $T/data-v3-seed $T/data-auto
    ```
    1. Start it: `M=$(mark)`, start version 4 with
       `H2LIVE_DATA=C:/h2work/acct-test/data-auto H2LIVE_SIGNUP=auto` in front
       of `./h2live.exe`, then `started $M 'relay on UDP port 47270'`.

       Pass:
       - `since $M | grep 'accounts are kept in'` names `data-auto`:
         H2LIVE_DATA wins over h2live.txt's `data=`;
       - `since $M | need - 'sign-up: open (every new PC gets an account; it closes once there'"'"'s an admin)'`
       - `since $M | none - 'no admins yet'`
    2. A new PC gets an account:
       `GT=AUTOPC lobby acct-autopc $T/scripts/newpc.txt auto`

       Pass:
       - it exits 0;
       - `since $M | need - 'live: AUTOPC signed in from 127.0.0.1'`
    3. Make an admin with the server stopped, then start it again with the
       same environment:
       ```
       stop_live
       cli user admin V3OWN on --data C:/h2work/acct-test/data-auto
       ```
       Then `M=$(mark)`, start as in step 1, and wait for the relay line.

       Pass:
       - the CLI prints `V3OWN IS AN ADMIN NOW. V3OWN HAS NO PASSWORD YET.`
         and `[exit 0]`;
       - `since $M | need - 'sign-up: closed (accounts are made with'`
       - then
         `GT=AUTOPC2 EXPECT="isn't signed in to that account" lobby acct-autopc2 $T/scripts/nopw.txt auto`
         exits 0, and `since $M | none - 'AUTOPC2 signed in'`.
    4. Clean up:
       `stop_live; rm -rf $T/data-auto $LAD/acct-autopc $LAD/acct-autopc2`.

## P3. The command line, with the server stopped

Each made-up password lands in a `.pw` file. Every hash prints
`hashing took N ms` on the error output: write each N down.

| # | Command | Pass |
|---|---|---|
| 1 | `cli user list` | Header `GAMERTAG  ADMIN  PASSWORD  PCS  ONLINE  LEVEL  CREATED`. 3 rows, each with ADMIN `no`, PASSWORD `no`, PCS `1/5` and a `PC "OLD PC", added ...` line. Then `3 accounts`, `[exit 0]` |
| 2 | `cli user key` | `identity key: <64 hex>` and `short code:   <code>`. The code equals K1code and is the key's first 16 hex digits in capitals, grouped by 4. Write down **K1** (the hex) |
| 3 | `cli user admin V3OWN on` | `V3OWN IS AN ADMIN NOW. V3OWN HAS NO PASSWORD YET.`, `[exit 0]` |
| 4 | `cli user admin V3OWN off` | `h2live: MAKE SOMEONE ELSE AN ADMIN FIRST.`, `[exit 1]` |
| 5 | `cli user remove V3OWN --yes` | The same text, `[exit 1]` |
| 6 | `cli_pw V3OWN user passwd V3OWN` | `V3OWN HAS A NEW PASSWORD. THEIR PCS STAY SIGNED IN.`, `[exit 0]`, 15 bytes |
| 7 | `cli_pw FR1 user add FR1` | `FR1 created. Password: (kept in FR1.pw) ...`, `[exit 0]`, 15 bytes |
| 8 | `new_pw FR2; cli_in $S/FR2.pw user add FR2 --password-stdin` | `FR2 created.`, `[exit 0]` |
| 9 | `cli_pw TEMP user add "TEMP ONE"` | `TEMP ONE created. ...`, `[exit 0]`, 15 bytes |
| 10 | `cli_pw ADM2 user add ADM2 --admin` | `ADM2 is an admin.`, `[exit 0]`, 15 bytes |
| 11 | `cli user add FR9 NOTAPASSWORD9` | `too many words (a password is never an argument: it's made up, or read with --password-stdin)`, then the usage, `[exit 2]` |
| 12 | `cli user add 'BAD*NAME'` | `GAMERTAGS ARE 1 TO 15 LETTERS, NUMBERS AND SINGLE SPACES, STARTING WITH A LETTER`, then the usage, `[exit 2]` |
| 13 | `cli user list --data C:/h2work/acct-test/nope` | `no h2live data in C:/h2work/acct-test/nope; pass --data or set H2LIVE_DATA`, `[exit 1]` |
| 14 | `cli user remove FR1` | `Removing FR1 loses it for good: ...`, then `h2live: not removed: run it again with --yes to remove it`, `[exit 1]` |
| 15 | `cli user rename V3SPARE "SPARE ONE"` | `V3SPARE IS SPARE ONE NOW`, `[exit 0]` |
| 16 | `cli user list` | `7 accounts`. V3OWN and ADM2 are admins. V3OWN, FR1, FR2, TEMP ONE and ADM2 have PASSWORD `yes`. The new ones are `0/5`. SPARE ONE is listed and V3SPARE isn't. `[exit 0]` |

Hash timing passes when every `hashing took N ms` is under 150. Tens of ms
are expected on this PC. Over 150 is a flag for the report, not a stop.

## P4. The sign-up setting, and a friend's first sign-in

1. Take `signup=` out, which leaves the default, auto:
   `printf 'port=47270\nbind=127.0.0.1\ndata=C:/h2work/acct-test/data\n' > $T/bin/h2live.txt`
2. Check that open wins over the file, with a warning. Don't run any lobby
   while sign-up is open. `M=$(mark)`, start version 4 with
   `H2LIVE_SIGNUP=open` in front, and wait for the relay line. Pass:
   ```
   since $M | need - 'sign-up: open (every new PC gets an account)' \
     'warning: sign-up is open (signup=open or H2LIVE_SIGNUP=open) though there are 2 admins'
   ```
   Then `stop_live`.
3. `M=$(mark)`, start version 4 plainly, and wait for the relay line.
   - Pass: `since $M | need - 'sign-up: closed (accounts are made with'`.
     Auto mode is closed once there are admins.
   - Pass: `since $M | none - 'no admins yet' warning`.
4. FR1's first sign-in on a new instance, typed in lower case with spaces for
   the dashes: `GT=FR1 K=FR1 lobby acct-fr1 $T/scripts/form-low.txt first`

   Pass:
   - it exits 0;
   - in `since $M`, in this order:
     - `live: FR1 signed in with a password from 127.0.0.1 (new PC "<this PC's name>", 1 of 5)`
     - `live: FR1 signed in from 127.0.0.1 (launcher ...`
   - `need $(LOG acct-fr1 first) 'lobby: signing in to ws://127.0.0.1:47270/live with a password' "lobby: pinned 127.0.0.1:47270's key <K1code>" 'lobby: signed in to 127.0.0.1:47270 as FR1'`
   - `grep -c "ws://127.0.0.1:47270/live <K1>" $LAD/acct-fr1/live-servers.txt`
     is 1, using K1 in lower case as the file has it;
   - timing: the seconds between the `signing in to` line and the
     `signed in to` line are under 1.0. The lobby log's first column is
     seconds since it started. Write the time down as "password sign-in".
5. FR1 again, by its PC: `lobby acct-fr1 $T/scripts/dev.txt second`

   Pass:
   - it exits 0;
   - a new `live: FR1 signed in from` line, and no new `with a password`
     line;
   - the lobby log's `signing in to` line doesn't end in `with a password`.
6. TEMP ONE's first sign-in, typed as printed (capitals and dashes):
   `GT="TEMP ONE" K=TEMP lobby acct-temp $T/scripts/form.txt first`

   Pass:
   - it exits 0;
   - `since $M | need - 'live: TEMP ONE signed in with a password from 127.0.0.1 (new PC'`

FR2 doesn't sign in yet: P5 uses its first sign-in.

## P5. Wrong passwords and lockouts

1. Clear the counters: `stop_live`, `M=$(mark)`, start version 4, and wait
   for the relay line.
2. Five wrong passwords for FR1, then the right one. `lock.txt`:
   ```
   0 wait start 10
   1 a
   0 see ONLINE 5
   0.3 a
   0 wait signin 10
   0 type FR1
   0 tab
   0 type WRONGPASS1
   0.3 a
   0 see Wrong gamertag or password 15
   6 type WRONGPASS2
   0.3 a
   6 type WRONGPASS3
   0.3 a
   6 type WRONGPASS4
   0.3 a
   6 type WRONGPASS5
   0.3 a
   6 type {PW:FR1}
   0.3 a
   0 see Too many wrong passwords 15
   1 quit
   ```
   Run `lobby acct-guess $T/scripts/lock.txt fr1`. Pass:
   - it exits 0;
   - `since $M` has
     `live: wrong password for FR1 from 127.0.0.1 (1 in 10 minutes)` and
     every count up to `(5 in 10 minutes)`;
   - `since $M | none - '(6 in 10' 'FR1 signed in'`
   - `since $M | need - 'dropped: TOO MANY WRONG PASSWORDS. TRY AGAIN IN 10 MINUTES.'`
   - `need $(LOG acct-guess fr1) 'lobby: Too many wrong passwords. Try again in 10 minutes.'`
   - timing: for try 1, the seconds from the first `signing in to` line to
     the first `lobby: Wrong gamertag or password.` line. Write it down as
     "known gamertag, wrong".
3. FR1's own PC isn't locked out: `lobby acct-fr1 $T/scripts/dev.txt locked`

   Pass: it exits 0, and there is a new `live: FR1 signed in from` line. A
   PC the account knows needs no password, so no lock stops it.
4. Fill this address's count with a gamertag nobody has. `ip.txt` waits a
   minute first, so P5.2's tries no longer count against the 10 sign-ins a
   minute:
   ```
   0 wait start 10
   65 a
   0 see ONLINE 5
   0.3 a
   0 wait signin 10
   0 type NOBODY
   0 tab
   0 type WRONGPASS1
   0.3 a
   0 see Wrong gamertag or password 15
   6 type WRONGPASS2
   0.3 a
   6 type WRONGPASS3
   0.3 a
   6 type WRONGPASS4
   0.3 a
   6 type WRONGPASS5
   0.3 a
   6 quit
   ```
   Run `lobby acct-nobody $T/scripts/ip.txt nobody`. Pass:
   - it exits 0;
   - `since $M` has
     `live: wrong password for a gamertag no account has, from 127.0.0.1 (6 in 10 minutes)`
     and every count up to `(10 in 10 minutes)`. The count is the address's,
     which already holds P5.2's five;
   - `since $M | none - NOBODY` (the server log never names a gamertag no
     account has);
   - timing: try 1's seconds, as in P5.2. Write it down as "unknown
     gamertag". Both cases check one hash: the stand-in one for an unknown
     gamertag. They should be within about 50 ms of each other; more than
     that is a flag for the report.
5. The address is locked now, so even a right password is refused.
   `iplock.txt` is `form.txt` with its last two lines replaced by
   `0 see Too many wrong passwords 15` and `1 quit`. Run
   `GT=FR2 K=FR2 lobby acct-fr2a $T/scripts/iplock.txt locked`. Pass:
   - it exits 0;
   - `since $M | none - 'wrong password for FR2' 'FR2 signed in'`
6. A restart clears it: `stop_live`, `M=$(mark)`, start version 4 and wait
   for the relay line. Then
   `GT=FR2 K=FR2 lobby acct-fr2a $T/scripts/form.txt first`. Pass:
   - it exits 0;
   - `since $M | need - 'live: FR2 signed in with a password from 127.0.0.1 (new PC'`
     with `1 of 5` on that line.

## P6. The admin's ACCOUNTS screen

The server keeps running from P5.6. Three lobbies run at once:
- FR2 and TEMP ONE sign in by their PCs and wait;
- V3OWN works. It has been an admin since P3, and its old PC signs it in.

`out.txt` (FR2):
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait live 20
0 wait signin 180
0 see You were signed out of this PC 5
1 quit
```
`removed.txt` (TEMP ONE):
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait live 20
0 wait failed 180
0 see THIS ACCOUNT WAS REMOVED 5
1 quit
```
`admin.txt` (V3OWN):
```
0 wait start 10
5 a
0 see ONLINE 5
0.3 a
0 wait live 20
1 see Accounts 10
```
then twelve lines of `0.1 down`, then:
```
0.3 a
0 wait accounts 10
# Create account: type the gamertag, A goes to CREATE, A creates
1 down
0.3 a
0 wait newaccount 5
0 type KAT
0.3 a
0.3 a
0 wait newpassword 15
# NEW PASSWORD ignores A and B for its first second
1.5 a
0 wait accounts 5
# your own row and another admin's are greyed
0 pick V3OWN
0.3 a
0 wait accountoptions 5
0.3 a
0 see Use the server's command line for that 5
0.3 b
0 wait accounts 5
0 pick ADM2
0.3 a
0 wait accountoptions 5
0.3 a
0 see Use the server's command line for that 5
0.3 b
0 wait accounts 5
# Give a new password
0 pick V3FRIEND
0.3 a
0 wait accountoptions 5
0.3 a
0 wait confirmaccount 5
0.3 a
0 wait newpassword 15
1.5 a
0 wait accounts 5
# Sign out of all PCs, while FR2 is online
0 pick FR2
0.3 a
0 wait accountoptions 5
0.1 down
0.3 a
0 wait confirmaccount 5
0.3 a
0 see IS SIGNED OUT OF ALL PCS 15
0 wait accounts 10
# Remove account, while TEMP ONE is online
0 pick TEMP ONE
0.3 a
0 wait accountoptions 5
0.1 down
0.1 down
0.3 a
0 wait confirmaccount 5
0.3 a
0 see REMOVED TEMP ONE 15
2 quit
```
Run all three in one call, with a timeout of 600000:
```
. C:/h2work/acct-test/env.sh; M=$(mark)
lobby acct-fr2a $T/scripts/out.txt p6 &
lobby acct-temp $T/scripts/removed.txt p6 &
lobby acct-v3own $T/scripts/admin.txt p6
wait
```
Pass:
- all three exit 0;
- the server log:
  ```
  since $M | need - 'live: V3OWN: made the account KAT' 'live: V3OWN gave V3FRIEND a new password' \
    'live: V3OWN signed FR2 out of all PCs (1 PC)' \
    'live: FR2 signed out: YOU WERE SIGNED OUT OF THIS PC. SIGN IN WITH YOUR PASSWORD.' \
    'live: V3OWN removed TEMP ONE' 'live: TEMP ONE signed out: THIS ACCOUNT WAS REMOVED.'
  since $M | none - 'V3OWN gave V3OWN' 'removed ADM2' 'signed ADM2' 'signed V3OWN'
  ```
  The greyed rows send nothing;
- V3OWN's lobby log:
  ```
  need $(LOG acct-v3own p6) 'lobby: asked to create account KAT' 'lobby: account KAT created' \
    'lobby: asked to give V3FRIEND a new password' 'lobby: V3FRIEND has a new password' \
    'lobby: asked to sign FR2 out of all PCs' 'lobby: asked to remove account TEMP ONE' 'ok true)'
  none $(LOG acct-v3own p6) 'ok false' 'No answer from the server yet'
  ```
- timing: the seconds from `asked to create account KAT` to
  `account KAT created` are under 1.0. That includes one hash on the server;
- `need $(LOG acct-fr2a p6) 'lobby: You were signed out of this PC. Type your password.'`
- `need $(LOG acct-temp p6) 'lobby: The server turned us away: THIS ACCOUNT WAS REMOVED.'`

The passwords that NEW PASSWORD showed for KAT and V3FRIEND are never
written anywhere. P7 gives KAT one in a file. The optional step at the end
covers reading one off the screen.

## P7. The command line again, with the server stopped

```
stop_live
new_pw KAT; cli_in $S/KAT.pw user passwd KAT --password-stdin   # KAT HAS A NEW PASSWORD. THEIR PCS STAY SIGNED IN.  [exit 0]
cli user revoke FR1                                              # FR1 IS SIGNED OUT OF ALL PCS.  [exit 0]
cli user remove ADM2 --yes                                       # REMOVED ADM2  [exit 0]  (V3OWN is still an admin)
cli user list                                                    # [exit 0]
```
Pass:
- each command prints what its comment says;
- the list says `6 accounts`:

  | Account | Admin | Password | PCs |
  |---|---|---|---|
  | V3OWN | yes | yes | 1/5 |
  | V3FRIEND | no | yes | 1/5 (a new password signs no PC out) |
  | SPARE ONE | no | no | 1/5 |
  | FR1 | no | yes | 0/5 |
  | FR2 | no | yes | 0/5 |
  | KAT | no | yes | 0/5 |

- there is no TEMP ONE and no ADM2;
- write down KAT's `hashing took`.

## P8. Sign-ins after the changes, and Change my password

Start the server: `M=$(mark)`, start version 4, and wait for the relay line.

1. KAT's first sign-in:
   `GT=KAT K=KAT lobby acct-kat $T/scripts/form.txt first`

   Pass:
   - it exits 0;
   - `since $M | need - 'live: KAT signed in with a password from 127.0.0.1 (new PC'`
     with `1 of 5` on that line;
   - write down the password sign-in time, as in P4.4.
2. FR1 after the revoke, with its gamertag saved:
   `K=FR1 EXPECT="isn't signed in to that account" lobby acct-fr1 $T/scripts/devpw.txt revoked`

   Pass:
   - it exits 0;
   - `need $(LOG acct-fr1 revoked) "lobby: This PC isn't signed in to that account. Type your password."`
   - `since $M | need - 'live: FR1 signed in with a password from 127.0.0.1 (new PC'`
     with `1 of 5` on that line.
3. The renamed account, by its old PC:
   `lobby acct-v3spare $T/scripts/dev.txt renamed`

   Pass:
   - it exits 0;
   - `since $M | need - 'live: SPARE ONE signed in from 127.0.0.1'`
   - `need $(LOG acct-v3spare renamed) 'lobby: the server calls us SPARE ONE' 'lobby: signed in to 127.0.0.1:47270 as SPARE ONE'`
   - `grep -c 'gamertag = SPARE ONE' $LAD/acct-v3spare/lobby.txt` is 1.
4. A new password doesn't sign PCs out:
   `lobby acct-v3friend $T/scripts/dev.txt newpw`

   Pass: it exits 0, and there is a new `live: V3FRIEND signed in from` line.
5. Change my password, on KAT's PC. Run `new_pw KAT2` first, then use
   `change.txt`:
   ```
   0 wait start 10
   1 a
   0 see ONLINE 5
   0.3 a
   0 wait live 20
   1 see My account 10
   ```
   then twelve lines of `0.1 down`, then:
   ```
   0.3 a
   0 wait accounts 10
   0.3 a
   0 wait changepassword 5
   0 type WRONGCURRENT9
   0.3 a
   0 type {PW:KAT2}
   0.3 a
   0 type {PW:KAT2}Z
   0.3 a
   0 see The new passwords don't match 5
   0 back
   0.3 a
   0 see wrong password 10
   0 type {PW:KAT}
   0.3 a
   0 see Password changed 10
   1 b
   1 quit
   ```
   The steps of that script:
   - A moves CURRENT, then NEW, then NEW AGAIN;
   - the mismatch is caught in the lobby, and nothing is sent;
   - Backspace takes off the Z;
   - the server answers WRONG PASSWORD, which clears CURRENT and puts the
     cursor there.

   Run `lobby acct-kat $T/scripts/change.txt change`. Pass:
   - it exits 0;
   - the server log has, in this order:
     - `live: wrong password for KAT from 127.0.0.1 changing it (1 in 10 minutes)`
     - `live: KAT changed their password`
   - `grep -c 'lobby: asked to change our password' $(LOG acct-kat change)`
     is 2;
   - `none $(LOG acct-kat change) 'No answer from the server yet'`
6. The old password is refused and the new one taken, on another PC.
   `kat2.txt`:
   ```
   0 wait start 10
   1 a
   0 see ONLINE 5
   0.3 a
   0 wait signin 10
   0 type KAT
   0 tab
   0 type {PW:KAT}
   0.3 a
   0 see Wrong gamertag or password 15
   6 type {PW:KAT2}
   0.3 a
   0 wait live 20
   2 quit
   ```
   Run `lobby acct-kat-b $T/scripts/kat2.txt swap`. Pass:
   - it exits 0;
   - the server log has, in this order:
     - `live: wrong password for KAT from 127.0.0.1 (1 in 10 minutes)`. The
       count starts again after the change;
     - `live: KAT signed in with a password from 127.0.0.1 (new PC`, with
       `2 of 5` on that line.
7. Optional: `--live` (the engine started without the lobby) refuses a PC
   the account doesn't know. It needs MCC, and it plays no game:
   ```
   "$L" --live 127.0.0.1:47270 --name FR2 --instance acct-livecheck --live-wait 5 < /dev/null > $T/logs/acct-livecheck.log 2>&1; echo "exit $?"
   ```
   Pass:
   - the exit code isn't 0;
   - `need $LAD/acct-livecheck/h2launch.log "live: this PC isn't signed in to that account: sign in once from the lobby (same --instance) with your password"`
   - `since $M | none - 'FR2 signed in'`
8. Optional: TOO MANY PCS. KAT is on 2 PCs. Bring it to 5, then try a sixth.
   - `kat-pc.txt` is `form.txt` with its second line `1 a` changed to
     `65 a`. The wait keeps under the 10 sign-ins a minute.
   - Run, one after the other:
     ```
     GT=KAT K=KAT2 lobby acct-kat-c $T/scripts/kat-pc.txt pc
     GT=KAT K=KAT2 lobby acct-kat-d $T/scripts/form.txt pc
     GT=KAT K=KAT2 lobby acct-kat-e $T/scripts/form.txt pc
     ```
     These make 3, 4 and 5 of 5.
   - Then the sixth:
     `GT=KAT K=KAT2 EXPECT='too many PCs' lobby acct-kat-f $T/scripts/kat-pcs.txt pc`.
     `kat-pcs.txt` is `iplock.txt` with `{EXPECT}` in place of
     `Too many wrong passwords`.

   Pass:
   - the first three exit 0, with `3 of 5`, `4 of 5` and `5 of 5` in the
     server log;
   - the last exits 0, and
     `need $(LOG acct-kat-f pc) 'lobby: This account is on too many PCs. Ask the server'"'"'s owner to sign it out of all PCs.'`

## P9. The server's key: pin, and SERVER KEY CHANGED

1. Another key, from a test value of H2LIVE_SECRET (it is only a test
   value):
   ```
   stop_live
   H2LIVE_SECRET=pc-test-other-secret cli user key     # write down K2code (the short code)
   ```
   Then `M=$(mark)`, start version 4 with
   `H2LIVE_SECRET=pc-test-other-secret` in front of `./h2live.exe`, and wait
   for the relay line.

   Pass: K2code isn't K1code.
2. A pinned PC that doesn't trust the new key. `keyb.txt`:
   ```
   0 wait start 10
   1 a
   0 see ONLINE 5
   0.3 a
   0 wait keychanged 15
   2 b
   0 wait main 5
   1 quit
   ```
   Run `sha256sum $LAD/acct-fr1/live-servers.txt` before, then
   `lobby acct-fr1 $T/scripts/keyb.txt k2`. Pass:
   - it exits 0;
   - `need $(LOG acct-fr1 k2) "lobby: 127.0.0.1:47270 answered with key <K2code> instead of <K1code>: only the sign-in request was sent (no password, no proof of this PC's key)" "lobby: the server's new key wasn't trusted"`
   - the file's sha256 is unchanged;
   - `since $M | none - 'FR1 signed in'`
3. A password typed, then the new key trusted, on a new instance that has
   only the pins:
   `mkdir -p $LAD/acct-pin && cp $LAD/acct-fr1/live-servers.txt $LAD/acct-pin/`.
   `keya.txt`:
   ```
   0 wait start 10
   1 a
   0 see ONLINE 5
   0.3 a
   0 wait signin 10
   0 type FR2
   0 tab
   0 type {PW:FR2}
   0.3 a
   0 wait keychanged 15
   # too soon: KEY CHANGED ignores A for its first second
   0.2 a
   0 wait keychanged 2
   1.5 a
   0 wait live 20
   2 quit
   ```
   Run `M2=$(mark)` and then `lobby acct-pin $T/scripts/keya.txt trust`.
   Pass:
   - it exits 0;
   - in the lobby log, in this order:
     - `lobby: signing in to ws://127.0.0.1:47270/live with a password`
     - the `answered with key <K2code> instead of <K1code>` line
     - `lobby: trusting 127.0.0.1:47270's new key <K2code>`
     - a second `signing in to ... with a password`
     - `lobby: signed in to 127.0.0.1:47270 as FR2`
   - the A at 0.2 s did nothing: the run didn't fail at `wait keychanged 2`;
   - `since $M2 | grep -c 'FR2 signed in with a password'` is 1, and that
     line says `1 of 5`, since P6 signed FR2 out;
   - `since $M2 | none - 'wrong password for FR2'`. The password went only
     to the trusted key.
4. Back to the real key: `stop_live`, `M=$(mark)`, start version 4
   plainly, and wait for the relay line. `keyback.txt` is `keyb.txt` with
   `1.5 a` in place of `2 b`, and `0 wait live 20` in place of
   `0 wait main 5`. Run `lobby acct-pin $T/scripts/keyback.txt back`.

   Pass:
   - it exits 0;
   - `need $(LOG acct-pin back) "lobby: trusting 127.0.0.1:47270's new key <K1code>"`
   - `grep -c "<K1>" $LAD/acct-pin/live-servers.txt` is 1;
   - `since $M | need - 'live: FR2 signed in from 127.0.0.1'`
   - `since $M | none - 'with a password'`. It signed in by its PC.

   The stat cards that the test key signed are ignored by the real server.
   That is harmless: logins.txt, not a card, says which account a PC is.

## P10. A ranked match on MCC's engine, with the in-game GAME MENU

FR1 (`acct-fr1`) and KAT (`acct-kat`) are signed in by their PCs (P8), and
the server runs from P9.4. MCC must be installed: this is the real
halo2.dll (`REAL=1`, no `H2LOBBY_FAKE`). Two Halo 2 windows open for about
three minutes; the owner shouldn't need the PC meanwhile.

`gm.txt` is the engine's input script for FR1. Its times count from when the
map has loaded:
```
base state1
5 press start
7 press start
```
`hh.txt` is for both lobbies (Head to Head is the playlists' row 1);
`hh2.txt` is the same with `3 a` in place of `1 a`:
```
0 wait start 10
1 a
0 see ONLINE 5
0.3 a
0 wait live 20
1 down
0.5 a
0 wait pregame 150
0 wait ingame 120
0 wait carnage* 300
3 quit
```
Run both in one call, with a timeout of 600000:
```
. C:/h2work/acct-test/env.sh; M=$(mark)
REAL=1 EARGS="--set-option 0x354=i32:60 --input-script C:/h2work/acct-test/scripts/gm.txt" lobby acct-fr1 $T/scripts/hh.txt p10 &
REAL=1 EARGS="--set-option 0x354=i32:60 --pad none" lobby acct-kat $T/scripts/hh2.txt p10
wait
```
Pass:
- both exit 0: they met, played and reached the carnage report;
- the server log has `live: match <id> started with 2 PCs`, and then
  `live: match <id> is over, counted`. If it says it didn't count, write
  down the reason;
- in `$LAD/acct-fr1/h2launch.log`:
  - `game menu: ready (fonts:` (write down which fonts it names);
  - `game menu: opened (`;
  - `game menu: closed`;
- `$LAD/acct-kat/h2launch.log` has `game menu: ready`;
- both engines closed by themselves after the 60 s limit.

If they don't meet within 150 s, write it down and stop this step. The custom
game route (party invite, then Custom Game) is lobby.md's two-lobby run.

## P11. Timing, a secret check, clean-up, and the report

1. The timing summary:
   - every CLI `hashing took N ms` (P3, P7);
   - the password sign-in round trips (P4.4, P8.1);
   - the create round trip (P6);
   - the wrong-password round trips for a known and an unknown gamertag
     (P5.2, P5.4), and their difference.

   Pass:
   - hashes are under 150 ms;
   - round trips are under 1 s;
   - the known and unknown cases are within about 50 ms of each other.
2. No password anywhere but `secrets`, and only hashes in `logins.txt`. The
   patterns are read from a file, so no password goes on a command line:
   ```
   . C:/h2work/acct-test/env.sh
   for f in $S/*.pw; do
     { head -n1 "$f"; head -n1 "$f" | tr 'A-Z-' 'a-z '; head -n1 "$f" | tr -d '-'; } > "$S/pat.tmp"
     n=$(grep -rlaF -f "$S/pat.tmp" $T/logs $T/data $T/scripts $LAD/acct-* 2>/dev/null | wc -l)
     echo "$(basename "$f" .pw): $n files"; done; rm -f "$S/pat.tmp"
   grep -rlaF '$argon2' $T/logs $T/scripts $LAD/acct-* 2>/dev/null | wc -l
   grep -c 'argon2id' $T/data/logins.txt
   ls $S/run-*.txt 2>/dev/null | wc -l
   ```
   Pass:
   - every count is `0 files`;
   - the first lone number is 0;
   - the second is 5, the accounts with a password: V3OWN, V3FRIEND, FR1,
     FR2 and KAT;
   - the last is 0: no filled-in script was left behind.
3. Clean up:
   ```
   stop_live
   rm -rf $S
   [ -n "$LOCALAPPDATA" ] && rm -rf $LAD/acct-*
   rm -rf $T/data $T/data-v3-seed $T/inst-v3-seed
   ```
   The data folders hold hashes of the test passwords. Keep `$T/logs` for
   the report. `C:/h2work/accounts-v3` and `C:/h2work/target-v3` can stay;
   the deploy doesn't need them.
4. Report to the thread:
   - SRC;
   - one pass or fail line for each step;
   - the timings, K1code and K2code;
   - anything unexpected.

   Never post passwords, hashes, `logins.txt` lines or screenshots.

## Optional, with the owner at the PC (the real screens)

This runs windowed, so the owner reads the password himself.

1. Start version 4 (P8's state). Run
   `"$L" --lobby --live 127.0.0.1:47270 --instance acct-v3own`, without
   `H2LOBBY_HEADLESS`.
2. He opens Accounts, creates OWNERTEST, and reads the password on ACCOUNT
   CREATED.
3. In a second window (`--instance acct-owner2`), he types OWNERTEST and
   that password with the keyboard.

Pass:
- it reaches the playlists;
- the server log has
  `live: OWNERTEST signed in with a password from 127.0.0.1 (new PC`.

He doesn't post the password. Afterwards he removes OWNERTEST from his
Accounts screen. Run this before P11's clean-up.

## Not covered here (unit tests cover them)

`cargo test -p h2live` covers these, in
`crates/h2live/src/server/tests/accounts.rs` and the CLI's tests:
- the server-wide limit of 30 hashes a minute (SERVER BUSY);
- 10 password checks per account and 3 password changes per account in 10
  minutes;
- the "couldn't save its files" answers;
- the admin socket on Linux.

The deploy runbook checks the admin socket on the real server.
