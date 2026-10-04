# Playing online

Online play needs one server that everyone's game signs in to. It keeps
accounts and levels, finds matches, and passes the games between players.
There are two ways to run it. Option A is the easy one.

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
   - `accounts are kept in ...`: the folder (`h2live-data`) holding everyone's
     accounts. Back it up now and then.
   - Then one of these:
     - `READY: ws://203.0.113.5:47050`: players can reach it. Paste that
       address to Claude.
     - `FORWARD TCP 47050 TO 192.168.1.20`: your router didn't open the way in
       by itself. Open the router's settings page (its address and password are
       usually on a sticker on the router), find **Port Forwarding** (sometimes
       called Virtual Server or NAT), and add one for TCP port 47050 to the
       address at the end of the line. Also give this PC a fixed address there
       (often called DHCP reservation) so the forward keeps pointing at it.
       h2live goes on showing this line afterwards; that's fine. If you get
       stuck, tell Claude the router's make and model. Your address for
       friends is `ws://<your internet address>:47050`: search "what is my
       IP" to find it, and paste that to Claude.
     - `CGNAT: USE THE HOSTED OPTION`: your internet provider shares one
       internet address between several homes, so nobody outside can reach
       this PC. Use option A.
   - `FAILED: port 47050 is in use`: h2live is running already, in another
     window.
   - Anything else is detail, for Claude if something goes wrong.
4. Leave the window open while anyone plays. Ctrl+C or closing the window
   stops it.

Your own game finds a server running on the same PC by itself. Your home's
internet address can change now and then: if friends suddenly can't sign in,
check the READY line again and send Claude the new address.

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
