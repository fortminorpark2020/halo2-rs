#!/bin/bash
# Installs h2live (the Halo 2 remake's online server) as a service on this
# Debian/Ubuntu machine, from the h2live program next to this script. Run
# it as root; running it again updates the program and keeps every
# account. It listens on TCP 47050 (sign-ins) and UDP 47050 (the relay
# that carries the launcher's games) and asks the router to open both
# (UPnP); `journalctl -u h2live` says READY, FORWARD or CGNAT. If a
# firewall guards this container (the Proxmox firewall, say), it must let
# both in. To move or turn off the relay, add for example
# `Environment=H2LIVE_RELAY=off` to the unit below.
#
# Accounts on this server are made by its owner only (sign-up closed), each
# a gamertag and a password: see docs/notes/launcher/accounts.md. So this
# script also:
# - stops the service, then keeps the program it replaces as
#   /opt/h2live/h2live.prev (only the one before the latest install) and a
#   copy of the data folder as /var/lib/h2live.backup-<date>-<time>
#   (logins.txt and secret.txt there can't be rebuilt from anything else);
#   the last program from before accounts (live protocol 3, no command
#   line) is also kept, for good, as /opt/h2live/h2live.before-accounts. If
#   a step fails after the stop (a full disk, say), it starts the service
#   again before it gives up. The new program goes in last, once the
#   settings, the unit and the wrapper are written, so that start runs the
#   program from before, or the new one with sign-up closed;
# - writes /opt/h2live/h2live.txt (the data folder and sign-up closed),
#   which the server and its command line both read, and sets sign-up
#   closed in the unit too, so losing one of them doesn't open it;
# - makes the files the server writes private to it (UMask=0077);
# - installs /usr/local/bin/h2live, which runs the server's command line
#   (`h2live user list`, `h2live user add <gamertag>` and the rest) as the
#   h2live user.
#
# To go back to the program before the latest install (which may be one
# with accounts too): as root,
#   install -m755 /opt/h2live/h2live.prev /opt/h2live/h2live && systemctl restart h2live
# To go back to the server from before accounts (live protocol 3, for the
# old launcher zip), whatever was installed since:
#   install -m755 /opt/h2live/h2live.before-accounts /opt/h2live/h2live && systemctl restart h2live
# For the exact data from before an update too, stop h2live and copy that
# update's backup folder back over /var/lib/h2live first. A program from
# before accounts has no command line: until the newer one is back,
# "h2live user ..." says so and does nothing.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ "$(id -u)" = 0 ] || { echo "run this as root"; exit 1; }
[ -f "$here/h2live" ] || { echo "h2live isn't next to this script"; exit 1; }

id h2live >/dev/null 2>&1 ||
    useradd --system --home-dir /var/lib/h2live --shell /usr/sbin/nologin h2live
install -d -o h2live -g h2live -m 750 /var/lib/h2live
install -d -m 755 /opt/h2live

# Stopped first, so the backup is of files at rest (it starts again below).
# Should a step before then fail, the server is started again on the way
# out rather than left stopped (Restart=always doesn't restart a unit
# stopped by hand).
if systemctl is-active --quiet h2live; then
    trap 'echo "the update failed: starting h2live again" >&2; systemctl start h2live || true' EXIT
    systemctl stop h2live
fi
# The program it replaces, unless it's the same one (so running this again
# keeps the one before).
if [ -f /opt/h2live/h2live ] && ! cmp -s "$here/h2live" /opt/h2live/h2live; then
    cp -p /opt/h2live/h2live /opt/h2live/h2live.prev
    echo "kept the program before as /opt/h2live/h2live.prev"
    # The program from before accounts (it has no command line: the same
    # test as the wrapper's below) is also kept by its own name, so later
    # updates, which replace h2live.prev, keep the way back to it.
    if ! grep -qaF 'usage: h2live user list' /opt/h2live/h2live; then
        cp -p /opt/h2live/h2live /opt/h2live/h2live.before-accounts
        echo "kept the program from before accounts as /opt/h2live/h2live.before-accounts"
    fi
fi
# A copy of the data folder each time this runs. Old copies pile up: delete
# them by hand (rm -r /var/lib/h2live.backup-...) once they're not needed.
if [ -n "$(ls -A /var/lib/h2live)" ]; then
    backup=/var/lib/h2live.backup-$(date +%Y%m%d-%H%M%S)
    # (A copy cut short, by a full disk say, isn't left to take up room.)
    cp -a /var/lib/h2live "$backup" || {
        rm -rf "$backup"
        false
    }
    echo "copied the data folder to $backup"
fi

# The settings beside the program, read by the server and by "h2live user".
# Lines other than data= and signup= that someone added are kept.
settings=/opt/h2live/h2live.txt
note1="# h2live's settings, written by install-h2live.sh. The server and"
note2="# its command line (h2live user ...) read them; the unit's Environment= wins."
{
    echo "$note1"
    echo "$note2"
    echo "data=/var/lib/h2live"
    echo "signup=closed"
    if [ -f "$settings" ]; then
        grep -vxF -e "$note1" -e "$note2" "$settings" |
            grep -viE '^[[:space:]]*(data|signup)[[:space:]]*=' || true
    fi
} >"$settings.new"
chmod 644 "$settings.new"
mv "$settings.new" "$settings"

cat >/etc/systemd/system/h2live.service <<'UNIT'
[Unit]
Description=h2live, the Halo 2 remake's online server
Wants=network-online.target
After=network-online.target

[Service]
User=h2live
Group=h2live
Environment=H2LIVE_DATA=/var/lib/h2live
Environment=H2LIVE_SIGNUP=closed
ExecStart=/opt/h2live/h2live
Restart=always
RestartSec=3
NoNewPrivileges=yes
UMask=0077

[Install]
WantedBy=multi-user.target
UNIT

# The command line, as the h2live user. Without arguments it would start a
# second server, which the service already is, so the wrapper refuses.
cat >/usr/local/bin/h2live.new <<'WRAPPER'
#!/bin/sh
# h2live's command line on this machine: "h2live user ..." makes and
# changes the server's accounts, as the h2live user, on the data folder
# that /opt/h2live/h2live.txt names (/var/lib/h2live). While the service
# runs, it asks the server; while it's stopped, it opens the folder itself.
# (The server keeps its key in secret.txt there. If the unit is ever given
# H2LIVE_SECRET instead, set the same here, or "h2live user key" run while
# the service is stopped shows another key.)
if [ "$#" -eq 0 ]; then
    echo "usage: h2live user list|add|passwd|revoke|remove|admin|rename|key ... (h2live help)" >&2
    echo "The server itself runs as the h2live service (systemctl status h2live)." >&2
    exit 2
fi
# A program from before accounts (put back to go back to it) has no command
# line: it would start a second server whatever it was asked. (Its usage's
# first line is how a program with one is told.)
if ! grep -qaF 'usage: h2live user list' /opt/h2live/h2live; then
    echo "This h2live is from before accounts and has no command line." >&2
    echo "The server runs as the h2live service (systemctl status h2live)." >&2
    exit 1
fi
exec runuser -u h2live -- /opt/h2live/h2live "$@"
WRAPPER
chmod 755 /usr/local/bin/h2live.new
mv /usr/local/bin/h2live.new /usr/local/bin/h2live

systemctl daemon-reload
# The new program last: from here on, whatever starts it (the trap, should
# a step fail) finds sign-up closed in h2live.txt and in the unit.
install -m 755 "$here/h2live" /opt/h2live/h2live.new
mv /opt/h2live/h2live.new /opt/h2live/h2live
systemctl enable h2live >/dev/null
trap - EXIT
systemctl restart h2live
sleep 6
# (status exits non-zero while the service isn't running, auto-restarting
# included: that mustn't end the script before the log below says why.)
{ systemctl --no-pager --lines=0 status h2live || true; } | head -3
running=yes
systemctl is-active --quiet h2live || running=no
# This start's lines: the sign-up setting, and how many accounts from an
# older server now sign in from their PCs as before.
journalctl -u h2live --no-pager -n 25 -o cat
curl -fsS http://127.0.0.1:47050/health >/dev/null 2>&1 && echo "health: ok" ||
    echo "health: no answer yet (curl missing or still starting)"
if command -v ss >/dev/null; then
    if [ -n "$(ss -Hlun 'sport = :47050')" ]; then
        echo "relay: listening on UDP 47050"
    else
        echo "relay: not on UDP 47050 (see the log above)"
    fi
fi
cat <<'HINT'

Accounts (as root here, or "pct exec <container> -- h2live ..." on the host):
  h2live user list                   every account, its PCs and whether it's an admin
  h2live user admin <gamertag> on    make an account an admin (its launcher shows Accounts)
  h2live user passwd <gamertag>      give it a new password (shown once)
  h2live user add <gamertag>         make an account (its password is shown once)
  h2live user key                    the server's key, for H2LAUNCH_SERVER_KEY
HINT
if [ "$running" = no ]; then
    echo
    echo "h2live isn't running: its log above says why" >&2
    exit 1
fi
