//! LAN play in the game: every lobby and game is open for other PCs on the
//! network to join (System Link in the menus lists the games other PCs
//! host). Joined PCs wait in the host's lobby between games and follow it
//! into each game it starts.

use crate::flow::bot_for;
use crate::local::{Keyboard, LocalPlayer};
use crate::menu::{self, Screen, SeatInfo};
use crate::options::GameOptions;
use crate::{scene, App, Mode, Then};
use gilrs::GamepadId;
use h2net::{Client, ClientEvent, Host, HostEvent, LanGame, Lobby, LobbyPlayer};
use h2sim::game::guest_name;
use h2sim::Command;

/// Longest a new game waits for PCs from the lobby to load its map.
pub const LAN_WAIT: f32 = 20.0;

pub enum Net {
    /// Can't host (no network); playing alone.
    Offline,
    /// Running the game; other PCs may join.
    Hosting(Host),
    /// Playing in another PC's game.
    Joined {
        client: Client,
        /// The host's name, for messages.
        computer: String,
        /// Our players are in the host's game (otherwise still joining).
        seated: bool,
        /// Controllers waiting for the host to add their player.
        waiting_pads: Vec<Option<GamepadId>>,
        /// The host's lobby, while waiting there for its next game.
        lobby: Option<Lobby>,
        /// Who plays here and the teams they'd like, as last told to the
        /// host.
        teams_sent: Vec<u8>,
    },
}

impl App {
    pub(crate) fn joined(&self) -> bool {
        matches!(self.net, Net::Joined { .. })
    }

    /// Open this game to the network.
    pub(crate) fn start_hosting(&mut self) {
        match Host::new(&self.map_name, self.session) {
            Ok(host) => {
                let ip = h2net::local_ip().map_or("this PC".into(), |ip| ip.to_string());
                println!("lan: open to other PCs at {ip}:{}", host.port());
                self.net = Net::Hosting(host);
            }
            Err(e) => {
                println!("lan: can't host: {e}");
                self.net = Net::Offline;
                self.can_host = false;
            }
        }
    }

    /// The menus' LAN work: hosting the lobby (other PCs can join it while
    /// it's up), or waiting in the lobby of the PC we joined.
    pub(crate) fn update_lobby_net(&mut self) {
        if self.joined() {
            let Net::Joined { client, .. } = &mut self.net else {
                return;
            };
            let (status, _) = client.poll(&mut self.game);
            if !self.client_status(status) || self.loading.is_some() {
                return;
            }
            // Splitscreen players coming and going here, and their teams,
            // show in the host's lobby.
            let wanted = self.wanted_teams();
            if let Net::Joined {
                client,
                lobby: Some(_),
                teams_sent,
                ..
            } = &mut self.net
            {
                if *teams_sent != wanted {
                    client.rejoin(&self.game, &self.map_name, &wanted);
                    *teams_sent = wanted;
                }
            }
            return;
        }
        let in_lobby = matches!(self.menu.screen, Screen::Lobby | Screen::Options);
        if !in_lobby {
            // Leaving the lobby closes it.
            self.net = Net::Offline;
            return;
        }
        if matches!(self.net, Net::Offline) && self.can_host {
            self.start_hosting();
        }
        let lobby = self.lobby_info();
        let Net::Hosting(host) = &mut self.net else {
            return;
        };
        host.set_lobby(lobby);
        for e in host.poll(&mut self.game, scene::MAX_BODIES) {
            let sound = match e {
                HostEvent::Arrived { .. } => menu::Sound::Advance,
                HostEvent::Left { .. } => menu::Sound::Back,
                _ => continue,
            };
            self.sound.play_ui(&self.scene, sound);
        }
    }

    /// Tell the PCs we play with that we're still here, while the game
    /// isn't running.
    pub(crate) fn keep_alive(&mut self) {
        match &mut self.net {
            Net::Hosting(host) => host.keep_alive(),
            Net::Joined { client, .. } => client.keep_alive(),
            Net::Offline => {}
        }
    }

    /// Players on PCs that joined our lobby.
    fn lan_members(&self) -> Vec<SeatInfo> {
        let Net::Hosting(host) = &self.net else {
            return Vec::new();
        };
        let mut seats = Vec::new();
        for (name, look, teams) in host.members() {
            for (k, team) in teams.into_iter().enumerate() {
                seats.push(SeatInfo {
                    name: guest_name(&name, k),
                    look: look.guest(k),
                    how: "SYSTEM LINK",
                    team,
                    // Listed after the people here.
                    level: crate::rank::test_level(self.seats.len() + seats.len()),
                });
            }
        }
        seats
    }

    /// Everyone in the lobby, as the menus list them.
    pub(crate) fn lobby_seats(&self) -> Vec<SeatInfo> {
        if let Net::Joined {
            lobby: Some(lobby), ..
        } = &self.net
        {
            return lobby
                .players
                .iter()
                .enumerate()
                .map(|(k, p)| SeatInfo {
                    name: p.name.clone(),
                    look: p.look,
                    how: if p.remote { "SYSTEM LINK" } else { "HOST" },
                    team: p.team,
                    level: crate::rank::test_level(k),
                })
                .collect();
        }
        let mut seats = self.seat_infos();
        seats.extend(self.lan_members());
        seats
    }

    /// Our lobby, for PCs that joined it.
    fn lobby_info(&self) -> Lobby {
        let s = &self.menu.settings;
        let players = self
            .lobby_seats()
            .into_iter()
            .map(|seat| LobbyPlayer {
                remote: seat.how == "SYSTEM LINK",
                name: seat.name,
                look: seat.look,
                team: seat.team,
            })
            .collect();
        Lobby {
            map: self
                .maps
                .get(s.map)
                .map_or(String::new(), |m| m.name.clone()),
            game_type: menu::GAME_TYPES[s.game_type.min(menu::GAME_TYPES.len() - 1)]
                .1
                .into(),
            score: menu::score_label(s),
            options: menu::options_label(&s.options),
            teams: s.game_type().teams(),
            players,
            bots: s.bots.min(255) as u8,
        }
    }

    /// Hosting: how many play on PCs that joined the lobby.
    pub(crate) fn lan_players(&self) -> usize {
        match &self.net {
            Net::Hosting(host) => host.members().iter().map(|m| m.2.len()).sum(),
            _ => 0,
        }
    }

    /// Hosting a game just started: hold it while PCs from the lobby load
    /// the map (for a while at most).
    pub(crate) fn waiting_for_lan(&mut self, dt: f32) -> bool {
        let Net::Hosting(host) = &self.net else {
            return false;
        };
        if self.lan_wait >= LAN_WAIT {
            return false;
        }
        if host.joining() == 0 {
            self.lan_wait = LAN_WAIT;
            return false;
        }
        self.lan_wait += dt;
        self.pending = 0.0;
        true
    }

    /// Hosting: let PCs join and read their players' controls.
    pub(crate) fn poll_host(&mut self) {
        let Net::Hosting(host) = &mut self.net else {
            return;
        };
        for e in host.poll(&mut self.game, scene::MAX_BODIES) {
            match e {
                HostEvent::Arrived { .. } => {}
                HostEvent::Joined { players, .. } => {
                    for p in players {
                        self.announce(&format!("{} JOINED", self.game.name(p)));
                    }
                }
                HostEvent::Added { player } => {
                    self.announce(&format!("{} JOINED", self.game.name(player)));
                }
                HostEvent::Removed { player } => {
                    // Their Spartan plays on as a bot, as in splitscreen.
                    self.bots.push(bot_for(player));
                    self.announce(&format!("{} LEFT", self.game.name(player)));
                }
                HostEvent::Left { players, .. } => {
                    for &p in &players {
                        self.announce(&format!("{} LEFT", self.game.name(p)));
                    }
                    self.bots.extend(players.into_iter().map(bot_for));
                }
            }
        }
    }

    /// Hosting: controls of players on joined PCs, for this tick.
    pub(crate) fn remote_commands(&mut self, commands: &mut [Command]) {
        if let Net::Hosting(host) = &mut self.net {
            for (i, c) in commands.iter_mut().enumerate() {
                if let Some(cmd) = host.command(i) {
                    *c = cmd;
                }
            }
        }
    }

    /// Hosting: send joined PCs the game after this frame's ticks.
    pub(crate) fn send_to_joined(&mut self, ticked: bool) {
        let events = std::mem::take(&mut self.frame_events);
        if let Net::Hosting(host) = &mut self.net {
            host.send(&self.game, &events, ticked);
        }
    }

    /// Joined: take on the host's game and send our controls.
    pub(crate) fn update_joined(&mut self, dt: f32) {
        let Net::Joined { client, .. } = &mut self.net else {
            return;
        };
        let before = client.snapshots;
        let (status, events) = client.poll(&mut self.game);
        let fresh = client.snapshots != before;
        if !self.client_status(status) || self.mode != Mode::Playing || self.loading.is_some() {
            return;
        }
        if let Some(players) = self.welcome.take() {
            if players.iter().all(|&p| p < self.game.players.len()) {
                self.take_seats(&players);
            } else {
                self.welcome = Some(players);
            }
        }
        if !matches!(self.net, Net::Joined { seated: true, .. }) {
            return;
        }
        // Follow our eyes from one game state to the next.
        if fresh {
            self.pending = 0.0;
            for l in &mut self.locals {
                if l.player < self.game.players.len() {
                    l.eyes = (l.eyes.1, crate::local::view_point(&self.game, l.player));
                }
            }
        } else {
            self.pending += dt;
        }
        self.game.events = events;
        let keyboard = Keyboard {
            keys: &self.keys,
            captured: self.captured,
            fire_held: self.fire_held,
            zoom_held: self.zoom_held,
        };
        let commands: Vec<(usize, Command)> = self
            .locals
            .iter()
            .map(|l| {
                if self.menu_open {
                    return (l.player, l.command(None, None));
                }
                let pad = l.pad.and_then(|id| self.pads.state(id));
                (l.player, l.command(l.keyboard.then_some(&keyboard), pad))
            })
            .collect();
        if let Net::Joined { client, .. } = &mut self.net {
            client.send_commands(&commands);
        }
        for l in &mut self.locals {
            l.taps = Default::default();
        }
        self.handle_events();
    }

    /// Act on what the host said. False once we're no longer joined.
    fn client_status(&mut self, status: Vec<ClientEvent>) -> bool {
        for s in status {
            match s {
                ClientEvent::Welcomed { computer, players } => {
                    if let Net::Joined { computer: name, .. } = &mut self.net {
                        *name = computer;
                    }
                    self.welcome = Some(players);
                }
                ClientEvent::Added(player) => self.seat_added(player),
                ClientEvent::Refused(why) => {
                    self.drop_out(format!("COULDN'T JOIN: {why}"));
                    return false;
                }
                ClientEvent::Lost(why) => {
                    println!("lan: lost the host: {why}");
                    self.drop_out("LOST CONNECTION TO THE HOST".into());
                    return false;
                }
                ClientEvent::Lobby(lobby) => self.wait_in_lobby(lobby),
                ClientEvent::Start(map) => {
                    self.follow_host(&map);
                    if !self.joined() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// The host is in its lobby: wait there with it.
    fn wait_in_lobby(&mut self, lobby: Lobby) {
        // Its map's picture.
        if let Some(k) = self.maps.iter().position(|m| m.name == lobby.map) {
            self.menu.settings.map = k;
        }
        if let Net::Joined {
            lobby: waiting,
            seated,
            ..
        } = &mut self.net
        {
            *waiting = Some(lobby);
            *seated = false;
        }
        if self.mode == Mode::Playing {
            self.back_to_lobby();
        }
    }

    /// The host started a game: load its map, then join it.
    fn follow_host(&mut self, map: &str) {
        if let Net::Joined { lobby, .. } = &mut self.net {
            *lobby = None;
        }
        if map.eq_ignore_ascii_case(&self.map_name) {
            self.rejoin_host();
            return;
        }
        match self.map_named(map) {
            Some(path) => self.begin_load(path, Then::Rejoin),
            None => self.drop_out(format!("YOU DON'T HAVE {}", menu::map_title(map))),
        }
    }

    /// The host's game is on the map loaded here: join it.
    pub(crate) fn rejoin_host(&mut self) {
        if !self.joined() {
            return;
        }
        // The host's options arrive with its game.
        self.seat_players(0, &GameOptions::default());
        let mut wanted = self.wanted_teams();
        wanted.truncate(self.locals.len());
        if let Net::Joined {
            client, teams_sent, ..
        } = &mut self.net
        {
            client.rejoin(&self.game, &self.map_name, &wanted);
            *teams_sent = wanted;
        }
    }

    /// Leave the PC we joined, for the list of games on the network.
    pub(crate) fn leave_game(&mut self) {
        self.net = Net::Offline;
        self.back_to_lobby();
        self.menu.show(Screen::SystemLink);
    }

    /// The host has our players: play them instead of our own game.
    fn take_seats(&mut self, players: &[usize]) {
        let Net::Joined {
            computer, seated, ..
        } = &mut self.net
        else {
            return;
        };
        *seated = true;
        let text = format!("JOINED {}", computer.to_uppercase());
        self.bots.clear();
        self.bodies.clear();
        self.body_poses.clear();
        self.body_actions.clear();
        let old = std::mem::take(&mut self.locals);
        for (l, &p) in old.into_iter().zip(players) {
            let mut seat = LocalPlayer::new(p, &self.game);
            seat.keyboard = l.keyboard;
            seat.pad = l.pad;
            seat.messages = l.messages;
            self.locals.push(seat);
        }
        self.announce(&text);
    }

    fn seat_added(&mut self, player: usize) {
        let Net::Joined { waiting_pads, .. } = &mut self.net else {
            return;
        };
        let pad = if waiting_pads.is_empty() {
            None
        } else {
            waiting_pads.remove(0)
        };
        if player < self.game.players.len() {
            let mut l = LocalPlayer::new(player, &self.game);
            l.pad = pad;
            self.locals.push(l);
            let name = crate::local::player_name(&self.game, usize::MAX, player);
            self.announce(&format!("{name} JOINED"));
        }
    }

    /// Joined: another person here wants to play (controller Start).
    pub(crate) fn request_local(&mut self, pad: Option<GamepadId>) {
        if let Net::Joined {
            client,
            waiting_pads,
            ..
        } = &mut self.net
        {
            if self.locals.len() + waiting_pads.len() < crate::MAX_LOCAL {
                waiting_pads.push(pad);
                client.add_local();
            }
        }
    }

    /// Join a game at a known address on the loaded map (H2_JOIN).
    pub(crate) fn join_address(&mut self, address: &str) {
        let Ok(address) = address.parse() else {
            println!("lan: bad address {address}");
            return;
        };
        let game = LanGame::at(address, &self.map_name);
        self.begin_join(&game);
    }

    pub(crate) fn connect(&mut self, game: &LanGame) {
        // Stop hosting first so no one joins us meanwhile.
        self.net = Net::Offline;
        let me = (self.menu.profile.name.as_str(), self.menu.profile.look);
        let mut wanted = self.wanted_teams();
        wanted.truncate(self.locals.len());
        match Client::connect(game.address, &self.game, &self.map_name, &wanted, me) {
            Ok(client) => {
                self.announce(&format!("JOINING {}", game.computer.to_uppercase()));
                self.net = Net::Joined {
                    client,
                    computer: game.computer.clone(),
                    seated: false,
                    waiting_pads: Vec::new(),
                    lobby: None,
                    teams_sent: wanted,
                };
            }
            Err(e) => {
                println!("lan: couldn't reach {}: {e}", game.address);
                self.drop_out(format!("COULDN'T REACH {}", game.computer.to_uppercase()));
            }
        }
    }

    /// The standing LAN line for the person at the keyboard.
    pub(crate) fn lan_notice(&self) -> Option<String> {
        match &self.net {
            Net::Joined {
                seated: false,
                computer,
                ..
            } => Some(format!("JOINING {}...", computer.to_uppercase())),
            Net::Hosting(host) if self.lan_wait < LAN_WAIT && host.joining() > 0 => {
                Some("WAITING FOR PLAYERS...".into())
            }
            _ => None,
        }
    }
}
