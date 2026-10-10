# Playing online

Online play needs one server that everyone's game signs in to. It keeps
accounts and levels, finds matches, and passes the games between players.
There are three ways to run it. Option A is the easy one.

The server uses one port number twice: TCP 47050 for signing in, and UDP
47050 for the relay that carries the launcher's games between PCs (the
game's own packets go over UDP so that one lost packet doesn't hold up
everyone). Friends outside your home network need both.

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
     accounts. Back it up now and then.
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
  the program and keeps every account.
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

## Moving or turning off the relay

The relay listens on UDP at the same number as the sign-in port unless told
otherwise. A `relay=47051` line in `h2live.txt` next to the program (or the
setting `H2LIVE_RELAY=47051`, for the container) moves it to another port;
`relay=off` turns it off. Forward whichever UDP port it uses.

For tests on one PC, `h2relay.exe` runs the relay on its own, without the
rest of the server: `h2relay` listens on UDP 47050, and `h2relay 47051` or
`h2relay 127.0.0.1:47051` somewhere else.

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
