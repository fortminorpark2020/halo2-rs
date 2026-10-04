//! LAN play in the game: every game is open for other PCs on the network to
//! join (System Link in the menus lists the games other PCs host).

use crate::flow::bot_for;
use crate::local::{Keyboard, LocalPlayer};
use crate::{scene, App};
use gilrs::GamepadId;
use h2net::{Client, ClientEvent, Host, HostEvent, LanGame};
use h2sim::Command;

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
            }
        }
    }

    /// Hosting: let PCs join and read their players' controls.
    pub(crate) fn poll_host(&mut self) {
        let Net::Hosting(host) = &mut self.net else {
            return;
        };
        for e in host.poll(&mut self.game, scene::MAX_BODIES) {
            match e {
                HostEvent::Joined { computer, .. } => {
                    self.announce(&format!("{} JOINED", computer.to_uppercase()));
                }
                HostEvent::Added { player } => {
                    self.announce(&format!("PLAYER {} JOINED", player + 1));
                }
                HostEvent::Removed { player } => {
                    // Their Spartan plays on as a bot, as in splitscreen.
                    self.bots.push(bot_for(player));
                    self.announce(&format!("PLAYER {} LEFT", player + 1));
                }
                HostEvent::Left {
                    computer, players, ..
                } => {
                    self.bots.extend(players.into_iter().map(bot_for));
                    self.announce(&format!("{} LEFT", computer.to_uppercase()));
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
                    return;
                }
                ClientEvent::Lost(why) => {
                    println!("lan: lost the host: {why}");
                    self.drop_out("LOST CONNECTION TO THE HOST".into());
                    return;
                }
            }
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
            self.announce(&format!("PLAYER {} JOINED", player + 1));
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
        match Client::connect(game.address, &self.game, &self.map_name, self.locals.len()) {
            Ok(client) => {
                self.announce(&format!("JOINING {}", game.computer.to_uppercase()));
                self.net = Net::Joined {
                    client,
                    computer: game.computer.clone(),
                    seated: false,
                    waiting_pads: Vec::new(),
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
            _ => None,
        }
    }
}
