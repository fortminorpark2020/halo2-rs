//! Custom games online, as the party plays them: its leader opens one
//! (CUSTOM GAME) and hosts it, with the usual lobby and game options but
//! only the maps everyone in the party has. The others come into the
//! leader's lobby through relay legs, wait there as in a LAN host's, and
//! follow it into each game it starts; so do members who join the party
//! later. Nothing is ranked. When the leader backs out it's over, and
//! everyone is back in the party lobby.

use super::{online_screen, recent};
use crate::lan::Net;
use crate::menu::{Screen, Sound};
use crate::{App, Mode};
use h2net::live::{End, LinkInfo, PartyMember, ToServer};
use h2net::{Client, Connection, Host, Verified};

/// The party's custom game we're in, from CUSTOM_OPEN until we're back in
/// the party lobby.
pub(super) struct Custom {
    /// The party, and its leader, who hosts it.
    party: u64,
    leader: u64,
    /// The map its lobby is on, as the service last heard.
    map: String,
    /// The members we've played the game going on with (by account): our
    /// recent players now.
    met: Vec<u64>,
}

impl App {
    /// The party's custom game is open (CUSTOM_OPEN): as its leader, host
    /// its lobby; otherwise wait to be linked to the leader. It comes again
    /// when the leader moves it to another map.
    pub(super) fn custom_open(&mut self, party: u64, leader: u64, map: String) {
        if self.online.in_match() {
            return;
        }
        let open = Custom {
            party,
            leader,
            map: map.clone(),
            met: Vec::new(),
        };
        if self.online.custom.replace(open).is_some() {
            return;
        }
        println!("live: in the party's custom game on {map}");
        if self.mode == Mode::Playing {
            self.back_to_lobby();
        }
        if self.online.me().is_some_and(|(me, _)| me == leader) {
            self.net = Net::Hosting(Host::online(&map));
            let ours = self
                .maps
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(&map));
            if let Some(k) = ours {
                self.menu.settings.map = k;
            }
            self.menu.show(Screen::Lobby);
            self.menu.notice = None;
        } else {
            self.net = Net::Offline;
            let view = self.online.view(self.online.now());
            let leader = view.and_then(|v| v.leader().map(str::to_string));
            let leader = leader.unwrap_or_else(|| "THE PARTY LEADER".into());
            self.menu.notice = Some(format!("JOINING {leader}..."));
        }
        self.sound.play_ui(&self.scene, Sound::Advance);
    }

    /// A relay leg's other end came: as the leader, a member coming into
    /// our lobby (or the service, on our fan-out leg); otherwise the
    /// leader, whose lobby we wait in.
    pub(super) fn custom_linked(&mut self, link: LinkInfo, conn: Connection) {
        if !self
            .online
            .custom
            .as_ref()
            .is_some_and(|c| c.party == link.id)
        {
            return;
        }
        if link.end == End::Fanout {
            if let Net::Hosting(host) = &mut self.net {
                println!("live: the fan-out leg is open");
                host.set_fanout(conn);
            }
            return;
        }
        println!("live: linked to {}", link.gamertag);
        if link.end == End::Host {
            if let Net::Hosting(host) = &mut self.net {
                let verified = Verified {
                    account: link.peer,
                    gamertag: link.gamertag,
                    level: link.level,
                    team: link.team,
                };
                host.add_connection(conn, verified);
            }
            return;
        }
        let Some(gamertag) = self.online.me().map(|(_, g)| g.to_string()) else {
            return;
        };
        // We pick our own team, as in a LAN lobby.
        let teams = self.wanted_teams();
        let me = (gamertag.as_str(), self.menu.profile.look);
        let client = Box::new(Client::over(conn, &self.game, &self.map_name, &teams, me));
        self.net = Net::Joined {
            client,
            computer: link.gamertag,
            seated: false,
            waiting_pads: Vec::new(),
            lobby: None,
            teams_sent: teams,
        };
    }

    /// Keep the party's custom game going, every frame: hosting it, tell
    /// the service which map its lobby is on.
    pub(super) fn update_custom(&mut self) {
        let me = self.online.me().map(|(a, _)| a);
        let map = self
            .maps
            .get(self.menu.settings.map)
            .map(|m| m.name.clone());
        let lobby = self.mode == Mode::Menu && self.loading.is_none();
        let Some(c) = &mut self.online.custom else {
            return;
        };
        let moved =
            map.filter(|m| lobby && Some(c.leader) == me && !m.eq_ignore_ascii_case(&c.map));
        if let Some(map) = moved {
            c.map = map.clone();
            self.online.send(ToServer::CustomMap(map));
        }
        self.meet_custom_players();
    }

    /// The party's members in the custom game's game with us are our recent
    /// players, from when each is first seen in it.
    fn meet_custom_players(&mut self) {
        let members = self.online.party_members();
        let playing = self.mode == Mode::Playing;
        let Some(c) = &mut self.online.custom else {
            return;
        };
        if !playing {
            c.met.clear();
            return;
        }
        let game = &self.game;
        let in_game = |name: &str| (0..game.players.len()).any(|i| game.name(i) == name);
        let new: Vec<PartyMember> = members
            .into_iter()
            .filter(|m| !c.met.contains(&m.account) && in_game(&m.gamertag))
            .collect();
        c.met.extend(new.iter().map(|m| m.account));
        let players = new.iter().map(|m| (m.account, m.gamertag.as_str(), m.best));
        self.online.met(players, recent::CUSTOM, &self.map_name);
    }

    /// Hosting the party's custom game (whose lobby stays up while a map
    /// loads).
    pub(crate) fn hosting_custom(&self) -> bool {
        self.online.in_custom() && matches!(self.net, Net::Hosting(_))
    }

    /// Back to the party lobby from its custom game, which is over for
    /// everyone if we host it.
    pub(crate) fn leave_custom(&mut self) {
        self.online.send(ToServer::Back);
        self.custom_over();
    }

    /// Out of the party's custom game: back in the party lobby.
    pub(crate) fn custom_over(&mut self) {
        self.online.custom = None;
        self.online.legs.clear();
        self.net = Net::Offline;
        if self.mode == Mode::Playing {
            self.back_to_lobby();
        }
        if !online_screen(self.menu.screen) {
            self.menu.show(Screen::Live);
        }
        println!("live: back in the party");
    }
}
