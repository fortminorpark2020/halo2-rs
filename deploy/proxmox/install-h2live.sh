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
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ "$(id -u)" = 0 ] || { echo "run this as root"; exit 1; }
[ -f "$here/h2live" ] || { echo "h2live isn't next to this script"; exit 1; }

id h2live >/dev/null 2>&1 ||
    useradd --system --home-dir /var/lib/h2live --shell /usr/sbin/nologin h2live
install -d -o h2live -g h2live -m 750 /var/lib/h2live
install -d -m 755 /opt/h2live
install -m 755 "$here/h2live" /opt/h2live/h2live.new
mv /opt/h2live/h2live.new /opt/h2live/h2live

cat >/etc/systemd/system/h2live.service <<'UNIT'
[Unit]
Description=h2live, the Halo 2 remake's online server
Wants=network-online.target
After=network-online.target

[Service]
User=h2live
Group=h2live
Environment=H2LIVE_DATA=/var/lib/h2live
ExecStart=/opt/h2live/h2live
Restart=always
RestartSec=3
NoNewPrivileges=yes

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable h2live >/dev/null
systemctl restart h2live
sleep 6
systemctl --no-pager --lines=0 status h2live | head -3
journalctl -u h2live --no-pager -n 20 -o cat
curl -fsS http://127.0.0.1:47050/health >/dev/null 2>&1 && echo "health: ok" ||
    echo "health: no answer yet (curl missing or still starting)"
if command -v ss >/dev/null; then
    if [ -n "$(ss -Hlun 'sport = :47050')" ]; then
        echo "relay: listening on UDP 47050"
    else
        echo "relay: not on UDP 47050 (see the log above)"
    fi
fi
