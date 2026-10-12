# Playing online

Online play needs one server that everyone's game signs in to. It keeps
accounts and levels, finds matches, and passes the games between players.
There are three ways to run it. Option A is the easy one.

The server uses one port number twice: TCP 47050 for signing in, and UDP
47050 for the relay that carries the launcher's games between PCs (the
game's own packets go over UDP so that one lost packet doesn't hold up
everyone). Friends outside your home network need both. The relay only
lets in the players of matches the server set up, so opening the port
doesn't give strangers a free relay. (Until the launcher's matches are
set up through the server, launcher tests use the test relay described at
the end of "Moving or turning off the relay".)

## Option A: a free server on Render (recommended)

Nothing to install, no router settings, and it works for every friend on any
network.

1. Before you start, tell Claude roughly where you and your friends live.
   The server goes in Ohio (middle of the USA) unless Claude moves it, and it
   can't be moved afterwards.
2. Go to [render.com](https://render.com), choose **Get Started**, and sign in
   with GitHub. When GitHub asks which repositories Render may see, include
   **halo2-rs**.
3. In Render, choose **New** → **Blueprint**, pick **halo2-rs**, and choose
   **Apply**. (If it asks for a name for the Blueprint, any name will do.) It
   shows one service, **h2live**, on the Free plan.
4. Wait until h2live says **Live**. The first time takes about 5 to 10
   minutes.
5. Open h2live in Render. Its address is near the top, something like
   `https://h2live-abcd.onrender.com`. Open that address in your browser: it
   should say `H2LIVE` and `0 PLAYERS ONLINE`. Paste the address to Claude,
   who puts it into the game. From the next build on, ONLINE signs in there.

Good to know:

- **Never delete the h2live service.** It holds the secret that vouches for
  everyone's levels. A new service would mean starting over.
- A free server goes to sleep after about 15 minutes with nobody on. The first
  person to sign in after that waits up to a minute while it wakes up.
- It forgets everything when it restarts, but nothing is lost: each PC keeps a
  signed copy of its own levels and hands it back when it signs in.
- So leave its sign-up as it is (auto, which stays open while no account
  is an admin; see "Who gets an account" below). A closed server needs a
  data folder that survives restarts, which Render's free plan doesn't
  have: its data is in /tmp, so the accounts made by hand and their
  passwords would be gone at the next restart.
- When Claude changes the server, Render updates it by itself a few minutes
  later. A game being played right then ends.
- Render's free plan only carries TCP, so the launcher's games (which go
  through the UDP relay) can't be played through it. For those, use option
  B or C.

## Option B: the server on your own PC

Only if option A doesn't work out. The PC has to stay on whenever anyone plays,
your router may need a setting changed, and players can see your home's
internet address.

1. `h2live.exe` comes in the same zip as the game. Put it in a folder of its
   own and double-click it.
2. If Windows says "Windows protected your PC", choose **More info** → **Run
   anyway**. When Windows Firewall asks, tick both **Private** and **Public**
   networks and choose **Allow access**.
3. Read what it says. Most lines start with the date and time.
   - `listening on port 47050`: it's running.
   - `relay on UDP port 47050`: the relay for the launcher's games is
     running beside it.
   - `accounts are kept in ...`: the folder (`h2live-data`) holding everyone's
     accounts. Back up the whole folder now and then, and before every
     update. Besides `accounts.txt` (accounts,
     levels, and kills, assists, deaths, betrayals and suicides in each
     ranked playlist) it holds `friends.txt` (launcher players' friends and
     friend requests), `games.log` (every match played), `logins.txt`
     (password hashes and the PCs each account signs in from; private: on
     Linux only h2live's own user may read it, and on Windows it's only as
     private as its folder, so keep that inside your own user folder) and
     `secret.txt` (the key that signs
     players' stat cards; launchers also check the server by a key made
     from it, so a new one makes every launcher say SERVER KEY CHANGED).
     Neither of the last two can be made again from anything else. A
     server from before friends opens the folder as it is; once a newer
     server has written to it, an older one can't read it, so keep a copy
     before updating.
   - `sign-up: open ...` or `sign-up: closed ...`: who gets an account (see
     "Who gets an account" below).
   - Then one of these:
     - `READY: ws://203.0.113.5:47050`: players can reach it. Paste that
       address to Claude. (The router opened UDP 47050 for the relay too,
       unless a `FORWARD UDP 47050` line follows: then forward just that one
       as below.)
     - `FORWARD TCP AND UDP 47050 TO 192.168.1.20`: your router didn't open
       the way in by itself. Open the router's settings page (its address
       and password are usually on a sticker on the router), find **Port
       Forwarding** (sometimes called Virtual Server or NAT), and forward port
       47050 for both TCP and UDP to the address at the end of the line. Many
       routers have a **Both** or **TCP/UDP** choice that does it in one
       rule; otherwise add two rules, one for each. Also give this PC a fixed
       address there (often called DHCP reservation) so the forwards keep
       pointing at it. h2live goes on showing this line afterwards; that's
       fine. If you get stuck, tell Claude the router's make and model. Your
       address for friends is `ws://<your internet address>:47050`: search
       "what is my IP" to find it, and paste that to Claude.
     - `CGNAT: USE THE HOSTED OPTION`: your internet provider shares one
       internet address between several homes, so nobody outside can reach
       this PC. Use option A.
   - `FAILED: port 47050 is in use`: h2live is running already, in another
     window.
   - `relay off: can't listen on UDP port 47050`: something else on this PC
     has that UDP port. Signing in still works, but launcher games can't go
     through this server until that's closed (or the relay is moved, below).
   - Anything else is detail, for Claude if something goes wrong.
4. Leave the window open while anyone plays. Ctrl+C or closing the window
   stops it.

Your own game finds a server running on the same PC by itself. Your home's
internet address can change now and then: if friends suddenly can't sign in,
check the READY line again and send Claude the new address.

## Option C: a container on a home server (Proxmox)

This is how it runs now: in a Debian container (LXC) on a Proxmox host at
home, which starts it whenever the host boots. Claude sets it up and updates
it from your PC.

- `deploy/proxmox/install-h2live.sh`, run as root inside the container next
  to a Linux build of `h2live`, installs it as the `h2live` service (program
  in `/opt/h2live`, accounts in `/var/lib/h2live`). Running it again updates
  the program and keeps every account. Each time, it stops the service,
  keeps the program it replaces as `/opt/h2live/h2live.prev` (only the one
  before the latest install) and copies the data folder to
  `/var/lib/h2live.backup-<date>-<time>` (old copies pile up; delete them
  by hand), then starts the new one. The last program from before accounts
  (live protocol 3) is also kept, for good, as
  `/opt/h2live/h2live.before-accounts`. If the new server isn't running
  after the start, the script shows its log and ends with an error. The new program goes in
  last, after the settings, the unit and the wrapper, so if a step fails
  and the script starts the service again, it runs the old program or the
  new one with sign-up closed, never the new one with sign-up open.
- It sets sign-up closed twice (`signup=closed` in `/opt/h2live/h2live.txt`
  and `H2LIVE_SIGNUP=closed` in the unit), so only the server's owner makes
  accounts there, and `UMask=0077`, so the files the server writes
  (`logins.txt`, the admin socket) are its own alone.
- It installs `/usr/local/bin/h2live`, which runs the server's command line
  as the h2live user: `h2live user list`, `h2live user add <gamertag>` and
  the rest (below), in a root shell in the container (over ssh), or from
  the Proxmox host with `pct exec 102 -- h2live user list`. On its own,
  `h2live` only says how to use it: the server is the service.
- To go back to the program before the latest update: `install -m755
  /opt/h2live/h2live.prev /opt/h2live/h2live && systemctl restart h2live`.
  To go back to the server from before accounts, whatever was installed
  since, use `/opt/h2live/h2live.before-accounts` in place of
  `h2live.prev`. For the data as it was too, first stop the service and
  put that update's backup in place of the folder, not over it (files the
  backup doesn't have, such as `logins.txt` in the one from before
  accounts, must go too): `mv /var/lib/h2live
  /var/lib/h2live.replaced-$(date +%Y%m%d-%H%M%S)`, then `cp -a
  /var/lib/h2live.backup-<date>-<time> /var/lib/h2live`. A server from
  before accounts (live protocol 3) ignores `logins.txt` and `signup=`, so it is
  open sign-up again, PCs signed out with `h2live user revoke` (or Sign out
  of all PCs) get back in to accounts their own keys made, removed accounts
  of that kind come back when their PCs sign in, and accounts made by hand
  can't sign in to it until the newer one is back. So don't go back to it
  while a lost or stolen PC matters (see `docs/notes/launcher/accounts.md`,
  "Deploy and rollback"). It has no command line either: until then,
  `h2live user ...` says so and does nothing.
- `journalctl -u h2live` shows its log, with the `relay on UDP port 47050`
  line and the READY, FORWARD or CGNAT line described in option B. FORWARD
  means the router needs port forwards for TCP 47050 and UDP 47050 to the
  container's address, and the container's address should be reserved in
  the router so it doesn't change. The install script also says whether the
  relay is listening.
- The container has no firewall of its own unless one was added. If the
  Proxmox firewall is turned on for it (Datacenter, the node, or the
  container's own Firewall tab), add rules there letting in TCP 47050 and
  UDP 47050.
- On the same network, games reach it at `ws://<container address>:47050`
  (a `server=` line in `profile.txt`, below), and launchers send their game
  traffic to UDP 47050 at the same address.

## Who gets an account

`signup=` in `h2live.txt` (or `H2LIVE_SIGNUP`, which wins) decides:

- `open`: every new PC that signs in gets an account of its own, named by
  the gamertag it asks for. That's how it always was.
- `closed`: only the server's owner makes accounts, each a gamertag and a
  password he hands out. A launcher types the password once on each PC (up
  to 5 PCs an account), and that PC is remembered. A launcher on a PC no
  account knows is asked for a password; a game (h2viewer) whose
  `identity.key` the server doesn't know is told THIS SERVER IS INVITE
  ONLY.
- `auto`, the default: open until some account is an admin, then closed.
  Local servers, tests and Render stay open with no setting.

The owner's accounts are made with h2live's command line on the server's
machine, or from an admin's launcher (Accounts, the playlists' last row):

```
h2live user list                       every account, its PCs, admin or not
h2live user add <GAMERTAG> [--admin]   a new account; its made-up password is shown once
h2live user passwd <GAMERTAG>          a new password (its PCs stay signed in)
h2live user revoke <GAMERTAG>          sign it out of all PCs (a leaked password: passwd first)
h2live user remove <GAMERTAG> --yes    remove it (for good while sign-up is closed)
h2live user admin <GAMERTAG> on|off    make it an admin, or not
h2live user rename <GAMERTAG> <NEW>    give it another gamertag
h2live user key                        the server's key, for H2LAUNCH_SERVER_KEY
```

They find the data folder as the server does (or `--data <folder>`), work
while it's stopped and, on Linux (the Proxmox container), while it runs
too (on Windows, close h2live first), and never take a password as an
argument (`--password-stdin` reads one instead of making one up). A
gamertag starting with `-` goes after `--` (`h2live user admin -- -KAT-
on`). With sign-up open, a removed account that a PC made by signing in
comes back when that PC signs in again, its levels too. Accounts
from before (live protocol 3) keep signing in from the PC that made them,
with no password until the owner gives them one.

If the server can't write its files (a full disk, say), a command or the
launcher's Accounts screen says so:
- "THE SERVER COULDN'T SAVE ITS FILES" means nothing changed.
- What was done followed by "BUT THE SERVER COULDN'T SAVE ITS FILES, SO
  IT'S UNDONE WHEN THE SERVER NEXT STARTS" (a new password, a sign-out,
  unmaking an admin) holds only until h2live restarts.

Either way, free the space and do it again. The whole design is in
`docs/notes/launcher/accounts.md`.

## Moving or turning off the relay

The relay listens on UDP at the same number as the sign-in port unless told
otherwise. A `relay=47051` line in `h2live.txt` next to the program (or the
setting `H2LIVE_RELAY=47051`, for the container) moves it to another port;
`relay=off` turns it off. Forward whichever UDP port it uses.

For tests on one PC or your home network, `h2relay.exe` runs a relay on
its own, without the rest of the server: `h2relay` listens on UDP 47050,
and `h2relay 47051` or `h2relay 127.0.0.1:47051` somewhere else. Unlike
h2live's, it lets anyone in who knows a match's number, so never forward
its port in the router. (On the server container, run it on another port,
such as 47051, beside h2live.)

## What friends do

1. Get the game's zip (the same one you use), unzip it and run the game. If
   Windows warns about it, choose **More info** → **Run anyway**.
2. Choose **ONLINE**. That's all, once the server's address is in the game.

To use a server the game doesn't know yet, open the profile folder (press
Windows+R, type `%APPDATA%\halo2-rs` and press Enter), open `profile.txt` in
Notepad (make it if it isn't there) and add a line with the server's address,
like `server=https://h2live-abcd.onrender.com`.

The file `identity.key` in that same folder is your online account. Keep a
copy: without it, your gamertag and levels can't be signed in to again. On a
new PC, copy it into the same folder.

On a server whose owner makes the accounts (sign-up closed, like the
Proxmox one), `identity.key` is one remembered PC of its account: an
account the game made before keeps signing in with it (copied to a new PC
too), but a game with a new `identity.key` is told THIS SERVER IS INVITE
ONLY, as the game can't type a password.
