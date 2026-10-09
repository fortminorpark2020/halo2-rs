//! The game's flow: menus, loading maps in the background, starting and
//! ending games, and the menu music.

use crate::campaign;
use crate::input::PadPress;
use crate::lan::Net;
use crate::local::{player_colors, LocalPlayer, TEAM_COLORS, TEAM_NAMES};
use crate::menu::{self, Action, Input, MapChoice, Menu, ScoreLine, Screen, SeatInfo};
use crate::options::GameOptions;
use crate::{
    level_focus, load_level, new_game, scene, App, Level, Loading, Mode, Then, MUSIC_VOLUME,
};
use gilrs::GamepadId;
use h2net::LanGame;
use h2sim::bot::{bot_look, bot_name};
use h2sim::game::{guest_name, Medal};
use h2sim::{game::TEAMS, Bot, Game};
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};

/// How far out and how high (in level radii) the overview camera is.
const OVERVIEW: (f32, f32) = (1.1, 0.55);

/// Looking over the level from above and to one side, `angle` radians
/// around it.
fn overview((focus, radius): (glam::Vec3, f32), angle: f32) -> crate::camera::FlyCamera {
    let (r, up) = OVERVIEW;
    let r = radius * r;
    let from = focus + glam::vec3(angle.cos() * r, angle.sin() * r, radius * up);
    crate::camera::FlyCamera::looking_at(from, focus)
}

/// Someone playing at this PC: their controller (none for the keyboard)
/// and their team in the lobby.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Seat {
    pub pad: Option<GamepadId>,
    pub team: u8,
    /// They chose the team (in another PC's lobby; otherwise its host
    /// chooses).
    pub picked: bool,
}

/// A controller press, as the menus see it.
enum Press {
    Menu(Input),
    Join,
    Leave,
    Team,
    None,
}

fn press(p: PadPress) -> Press {
    match p {
        PadPress::Up => Press::Menu(Input::Up),
        PadPress::Down => Press::Menu(Input::Down),
        PadPress::Left => Press::Menu(Input::Left),
        PadPress::Right => Press::Menu(Input::Right),
        PadPress::Claim => Press::Menu(Input::Select),
        PadPress::Melee => Press::Menu(Input::Back),
        PadPress::Join => Press::Join,
        PadPress::Leave => Press::Leave,
        PadPress::Reload => Press::Team,
        _ => Press::None,
    }
}

/// The profile colour of the Master Chief's armour.
const CHIEF_OLIVE: u8 = 5;

pub fn bot_for(player: usize) -> (usize, Bot) {
    (player, Bot::new(player as u32 * 7919 + 13))
}

/// The teams of `bots` bots in a team game with `people` on the teams they
/// want (`ANY_TEAM`: the smaller, as the host puts them): each on the
/// smaller team in turn.
fn bot_teams(people: &[u8], bots: usize) -> Vec<u8> {
    let mut count = [0usize; TEAMS as usize];
    let smaller = |count: &[usize]| (0..count.len()).min_by_key(|&t| count[t]).unwrap_or(0);
    for &t in people {
        let t = match t {
            h2net::ANY_TEAM => smaller(&count),
            t => (t as usize).min(count.len() - 1),
        };
        count[t] += 1;
    }
    (0..bots)
        .map(|_| {
            let t = smaller(&count);
            count[t] += 1;
            t as u8
        })
        .collect()
}

impl App {
    /// The menus are showing (over the map, or over a game).
    pub(crate) fn in_menu(&self) -> bool {
        self.mode == Mode::Menu || self.menu_open
    }

    /// The menu over the game is up for local player `k`: their own pause
    /// menu, or one for everyone (the carnage report).
    pub(crate) fn menu_for(&self, k: usize) -> bool {
        let owner = self.menu_owner.filter(|&o| o < self.locals.len());
        self.menu_open && owner.is_none_or(|o| o == k)
    }

    /// The keyboard and mouse work the menus: any of them, but over a game
    /// only one that's up for the player at the keyboard.
    pub(crate) fn keyboard_in_menu(&self) -> bool {
        match self.locals.iter().position(|l| l.keyboard) {
            Some(k) if self.mode == Mode::Playing => self.menu_for(k),
            _ => self.in_menu(),
        }
    }

    /// Controller `id` works the menus: any of them, but over a game only
    /// one that's up for its player (or for everyone).
    pub(crate) fn pad_in_menu(&self, id: GamepadId) -> bool {
        if self.mode != Mode::Playing {
            return self.in_menu();
        }
        let k = self.locals.iter().position(|l| l.pad == Some(id));
        self.menu_for(k.unwrap_or(usize::MAX))
    }

    /// The game stands still: someone here paused it, and no one on another
    /// PC plays in it (Halo 2 pauses a game at one console, but not one
    /// over System Link).
    pub(crate) fn paused(&self) -> bool {
        let alone = match &self.net {
            Net::Offline | Net::Connecting { .. } => true,
            Net::Hosting(host) => {
                host.joined() == 0 && !self.online.in_match() && !self.online.in_custom()
            }
            Net::Joined { .. } => false,
        };
        let pause = self.menu_open && self.menu.pausing();
        self.mode == Mode::Playing && pause && alone
    }

    /// Where the menus show: the whole window, or over a game the view of
    /// the player whose pause menu it is.
    pub(crate) fn menu_area(&self) -> [u32; 4] {
        let (w, h) = self.window_size();
        let (w, h) = (w as u32, h as u32);
        let window = [0, 0, w, h];
        let owner = self
            .menu_owner
            .filter(|_| self.menu_open && self.mode == Mode::Playing);
        owner
            .and_then(|k| {
                crate::local::viewports(self.locals.len(), w, h)
                    .get(k)
                    .copied()
            })
            .unwrap_or(window)
    }

    /// Run `f` with the menu and what it shows.
    pub(crate) fn with_menu<R>(&mut self, f: impl FnOnce(&mut Menu, &menu::Context) -> R) -> R {
        let scores = self.score_lines();
        let outcome = self.outcome();
        let seats = self.lobby_seats();
        let joined = self.joined();
        // An online match's pregame shows its lobby (the host's, once
        // there).
        let match_lobby = self.match_lobby();
        let host_lobby = match &self.net {
            Net::Joined { lobby, .. } => lobby.as_ref(),
            _ => None,
        };
        let host_lobby = host_lobby.or(match_lobby.as_ref());
        let objectives = self
            .mission
            .as_ref()
            .map(|m| m.objectives(&self.scene))
            .unwrap_or_default();
        let online = self.online.view(self.online.now());
        let ctx = menu::Context {
            maps: &self.maps,
            missions: &self.missions,
            lan: &self.lan_games,
            seats: &seats,
            local: self.seats.len(),
            scores: &scores,
            outcome: outcome.as_deref(),
            joined,
            host_lobby,
            objectives: &objectives,
            online: online.as_ref(),
        };
        f(&mut self.menu, &ctx)
    }

    pub(crate) fn menu_input(&mut self, input: Input) {
        let action = self.with_menu(|m, ctx| m.input(input, ctx));
        self.after_menu(action);
    }

    /// The mouse in the menus' area: where it is there, and the area's size.
    fn menu_mouse(&self) -> ([f32; 2], f32, f32) {
        let [x, y, w, h] = self.menu_area().map(|v| v as f32);
        ([self.mouse[0] - x, self.mouse[1] - y], w, h)
    }

    pub(crate) fn menu_click(&mut self) {
        self.menu.controller = false;
        let (pos, w, h) = self.menu_mouse();
        let action = self.with_menu(|m, ctx| m.click(pos, w, h, ctx));
        self.after_menu(action);
    }

    pub(crate) fn menu_hover(&mut self) {
        let (pos, w, h) = self.menu_mouse();
        self.with_menu(|m, ctx| m.hover(pos, w, h, ctx));
        self.after_menu(Action::None);
    }

    pub(crate) fn menu_wheel(&mut self, up: bool) {
        self.with_menu(|m, ctx| m.wheel(up, ctx));
        self.after_menu(Action::None);
    }

    pub(crate) fn window_size(&self) -> (f32, f32) {
        self.gpu.as_ref().map_or((1280.0, 720.0), |g| g.size())
    }

    pub(crate) fn after_menu(&mut self, action: Action) {
        if let Some(s) = self.menu.sound.take() {
            self.sound.play_ui(&self.scene, s);
        }
        match action {
            Action::None => {}
            Action::Start => self.start_selected(),
            Action::Mission(i) => self.start_mission(i),
            Action::Join(game) => self.join_game(&game),
            Action::SaveProfile => self.menu.profile.save(),
            Action::Resume => {
                if self.keyboard_in_menu() {
                    self.set_capture(true);
                }
                self.menu_open = false;
            }
            Action::EndGame => self.end_game(),
            Action::Leave => self.leave_game(),
            Action::Quit => self.quit = true,
            Action::GoOnline => self.go_online(),
            Action::SignOut => self.sign_out(),
            Action::Search(_)
            | Action::CancelSearch
            | Action::Custom
            | Action::Invite(_)
            | Action::JoinParty(_)
            | Action::Accept(_)
            | Action::LeaveParty
            | Action::Kick(_)
            | Action::Promote(_)
            | Action::Privacy(_) => self.ask_live(action),
        }
    }

    /// A controller press while the menus are up.
    pub(crate) fn menu_pad(&mut self, id: GamepadId, pad_press: PadPress) {
        self.menu.controller = true;
        let seated = self.seats.iter().position(|s| s.pad == Some(id));
        let lobby = self.mode == Mode::Menu && self.menu.screen == Screen::Lobby;
        match press(pad_press) {
            // A guest's B takes only them out of the lobby.
            Press::Menu(Input::Back) if lobby && seated.is_some_and(|k| k > 0) => {
                self.seats.retain(|s| s.pad != Some(id));
                self.sound.play_ui(&self.scene, menu::Sound::Back);
            }
            Press::Menu(input) => {
                // A on a new controller takes over player one (the keyboard
                // and mouse still work for them too).
                let menus = self.mode == Mode::Menu;
                if input == Input::Select
                    && menus
                    && seated.is_none()
                    && self.seats[0].pad.is_none()
                {
                    self.seats[0].pad = Some(id);
                    if self.keyboard_used {
                        // Taking over, not choosing what's highlighted.
                        self.sound.play_ui(&self.scene, menu::Sound::Forward);
                        return;
                    }
                }
                self.menu_input(input);
            }
            Press::Join if lobby && seated.is_none() => {
                if self.seat_free() {
                    self.seats[0].pad = Some(id);
                } else if self.seats.len() < crate::MAX_LOCAL {
                    self.seats.push(Seat {
                        pad: Some(id),
                        ..Seat::default()
                    });
                }
                self.sound.play_ui(&self.scene, menu::Sound::Forward);
            }
            Press::Join if self.menu.pausing() => self.menu_input(Input::Back),
            Press::Join => self.menu_input(Input::Select),
            Press::Leave if lobby => match seated {
                Some(0) => self.seats[0].pad = None,
                Some(k) => {
                    self.seats.remove(k);
                    self.sound.play_ui(&self.scene, menu::Sound::Back);
                }
                None => {}
            },
            Press::Leave => self.menu_input(Input::Back),
            Press::Team if lobby => {
                if seated.is_none() && self.seat_free() {
                    self.seats[0].pad = Some(id);
                }
                if let Some(k) = self.seats.iter().position(|s| s.pad == Some(id)) {
                    self.change_team(k);
                }
            }
            Press::Team | Press::None => {}
        }
    }

    /// Player one is free for a controller: no one has used the keyboard or
    /// mouse, and no controller has it yet.
    fn seat_free(&self) -> bool {
        !self.keyboard_used && self.seats[0].pad.is_none()
    }

    /// Put someone at this PC on the other team, in team games.
    pub(crate) fn change_team(&mut self, seat: usize) {
        let teams = match &self.net {
            Net::Joined { lobby, .. } => lobby.as_ref().is_some_and(|l| l.teams),
            _ => self.menu.settings.game_type().teams(),
        };
        if !teams {
            return;
        }
        let joined = self.joined();
        if let Some(s) = self.seats.get_mut(seat) {
            // Joined, the first press keeps the seat's team, now chosen.
            if s.picked || !joined {
                s.team = (s.team + 1) % TEAMS;
            }
            s.picked = true;
            self.sound.play_ui(&self.scene, menu::Sound::Cursor);
        }
    }

    /// The teams the people here would like, for the host: `ANY_TEAM`
    /// where they didn't pick one.
    pub(crate) fn wanted_teams(&self) -> Vec<u8> {
        self.seats
            .iter()
            .map(|s| if s.picked { s.team } else { h2net::ANY_TEAM })
            .collect()
    }

    /// How each person at this PC plays, for the lobby.
    pub(crate) fn seat_infos(&self) -> Vec<SeatInfo> {
        let profile = &self.menu.profile;
        self.seats
            .iter()
            .enumerate()
            .map(|(k, s)| SeatInfo {
                name: guest_name(&profile.name, k),
                look: profile.look.guest(k),
                how: if s.pad.is_some() {
                    "CONTROLLER"
                } else {
                    menu::KEYBOARD
                },
                team: s.team,
                level: crate::rank::test_level(k),
            })
            .collect()
    }

    /// Everyone's score, best first; in team games, each team (best first)
    /// heads its players.
    pub(crate) fn score_lines(&self) -> Vec<ScoreLine> {
        let game = &self.game;
        let text = &self.scene.text;
        // Lives still going were cut short when the game ended.
        let now = self.ended_at.unwrap_or(game.time);
        let line = |i: usize| {
            let p = &game.players[i];
            let s = &p.stats;
            // Time alive over lives: those lost, and the one still going.
            let living = if p.alive {
                (now - p.spawned_at).max(0.0)
            } else {
                0.0
            };
            let lives = p.deaths + p.alive as u32;
            let medals = Medal::ALL
                .iter()
                .zip(s.medals)
                .filter(|m| m.1 > 0)
                .map(|(&m, n)| (text.medal_name(m), n))
                .collect();
            ScoreLine {
                name: crate::local::player_name(game, usize::MAX, i),
                score: p.score,
                timed: game.rules.game_type.timed(),
                kills: p.kills,
                deaths: p.deaths,
                assists: s.assists,
                suicides: s.suicides,
                best_spree: s.best_spree,
                avg_life: (s.lived as f64 + living) as f32 / lives.max(1) as f32,
                medals,
                color: player_colors(game, i)[0],
                emblem: Some(p.look.emblem),
                level: self
                    .match_level(&p.name)
                    .or_else(|| crate::rank::test_level(i)),
                local: self.locals.iter().any(|l| l.player == i),
                header: false,
            }
        };
        let by_score =
            |a: &ScoreLine, b: &ScoreLine| b.score.cmp(&a.score).then(a.deaths.cmp(&b.deaths));
        let mut players: Vec<ScoreLine> = (0..game.players.len()).map(line).collect();
        players.sort_by(by_score);
        if !game.rules.game_type.teams() {
            return players;
        }
        let mut teams: Vec<u8> = (0..TEAMS)
            .filter(|&t| game.players.iter().any(|p| p.team == t))
            .collect();
        teams.sort_by_key(|&t| std::cmp::Reverse(game.team_score(t)));
        let mut lines = Vec::new();
        for t in teams {
            let members = || game.players.iter().filter(|p| p.team == t);
            lines.push(ScoreLine {
                name: text.team(t),
                score: game.team_score(t),
                timed: game.rules.game_type.timed(),
                kills: members().map(|p| p.kills).sum(),
                deaths: members().map(|p| p.deaths).sum(),
                assists: members().map(|p| p.stats.assists).sum(),
                suicides: members().map(|p| p.stats.suicides).sum(),
                color: TEAM_COLORS[t as usize],
                header: true,
                ..ScoreLine::default()
            });
            let mut mine: Vec<ScoreLine> = (0..game.players.len())
                .filter(|&i| game.players[i].team == t)
                .map(line)
                .collect();
            mine.sort_by(by_score);
            lines.extend(mine);
        }
        lines
    }

    /// How the game ended, once it has: who won, or a draw.
    fn outcome(&self) -> Option<String> {
        let game = &self.game;
        if !game.over() {
            return None;
        }
        Some(match (game.winning_team, game.winner) {
            (Some(t), _) => format!("{} TEAM WINS", TEAM_NAMES[t.min(1) as usize]),
            (None, Some(w)) => format!("{} WINS", crate::local::player_name(game, usize::MAX, w)),
            (None, None) => "DRAW".into(),
        })
    }

    /// Bring up a menu over the game: local player `owner`'s pause menu,
    /// or (None) one for everyone.
    pub(crate) fn open_menu(&mut self, screen: Screen, owner: Option<usize>) {
        self.menu.show(screen);
        self.menu_open = true;
        self.menu_owner = owner;
        if self.keyboard_in_menu() {
            self.fire_held = false;
            self.zoom_held = false;
            self.set_capture(false);
        }
    }

    /// Local player `k` pauses (Start, or Esc at the keyboard): the pause
    /// menu comes up in their view, unless another menu is up.
    pub(crate) fn pause(&mut self, k: usize) {
        if !self.menu_open {
            self.open_menu(Screen::Pause, Some(k));
        }
    }

    /// Start the lobby's game, loading its map first if it isn't loaded.
    fn start_selected(&mut self) {
        let Some(map) = self.maps.get(self.menu.settings.map).cloned() else {
            return;
        };
        if map.path == self.map_path {
            self.start_game();
        } else {
            self.begin_load(map.path, Then::Play);
        }
    }

    /// Play a campaign mission, loading it first if it isn't loaded.
    fn start_mission(&mut self, i: usize) {
        let Some(mission) = self.missions.get(i).cloned() else {
            return;
        };
        self.campaign = true;
        if mission.path == self.map_path {
            self.start_campaign();
        } else {
            self.begin_load(mission.path, Then::Campaign);
        }
    }

    /// Play the loaded mission: the people here, alone together against
    /// the Covenant (not open to the network).
    pub(crate) fn start_campaign(&mut self) {
        self.net = Net::Offline;
        // The Master Chief in his own olive armour, whatever the profile,
        // or the Arbiter, on the Covenant's side, in his missions.
        let arbiter = campaign::arbiter(&self.scene);
        for seat in &mut self.seats {
            seat.team = campaign::players_team(&self.scene);
        }
        self.seat_players(&[], &GameOptions::default());
        for l in &self.locals {
            let mut look = self.game.players[l.player].look;
            look.elite = arbiter;
            look.colors = [CHIEF_OLIVE; 2];
            self.game.set_look(l.player, look);
        }
        let difficulty = self.menu.difficulty as u8;
        self.mission = Some(campaign::Mission::new(
            &self.scene,
            &mut self.game,
            &mut self.bots,
            difficulty,
        ));
        // H2_SQUADS=<names>: place those squads at the start too.
        if let Ok(names) = std::env::var("H2_SQUADS") {
            let squads = campaign::squads_named(&self.scene, &names);
            self.place_squads(&squads);
        }
    }

    /// Bring a mission's squads into play.
    pub(crate) fn place_squads(&mut self, squads: &[usize]) {
        let difficulty = self.menu.difficulty as u8;
        let team = self
            .locals
            .first()
            .map_or(0, |l| self.game.players[l.player].team);
        for &s in squads {
            let placed =
                campaign::place_squad(&mut self.game, &self.scene, s, difficulty, team, None, None);
            self.bots
                .retain(|(i, _)| !placed.iter().any(|(j, _)| j == i));
            self.bots
                .extend(placed.into_iter().filter_map(|(i, b)| Some((i, b?))));
        }
    }

    fn join_game(&mut self, game: &LanGame) {
        // A lobby takes PCs on any map (they need its map only once a game
        // starts), so one whose map isn't here is joined on the map that is.
        let path = self.map_named(&game.map);
        match path.filter(|_| !game.map.eq_ignore_ascii_case(&self.map_name)) {
            Some(path) => self.begin_load(path, Then::Join(game.clone())),
            None => self.begin_join(game),
        }
    }

    /// The file of the map LAN games call `name`, if this PC has it.
    pub(crate) fn map_named(&self, name: &str) -> Option<PathBuf> {
        let listed = self.maps.iter().find(|m| m.name.eq_ignore_ascii_case(name));
        let path = listed.map_or_else(
            || self.map_path.with_file_name(format!("{name}.map")),
            |m| m.path.clone(),
        );
        path.exists().then_some(path)
    }

    pub(crate) fn begin_load(&mut self, path: PathBuf, then: Then) {
        println!("loading {}", path.display());
        let title = MapChoice::new(&path).title;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(load_level(&path));
        });
        self.loading = Some(Loading {
            title,
            level: rx,
            then,
        });
    }

    /// Take a map that finished loading, then go on to what it was for.
    pub(crate) fn poll_loading(&mut self) {
        let Some(loading) = &self.loading else {
            return;
        };
        let result = match loading.level.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the loader stopped".into()),
        };
        let Some(loading) = self.loading.take() else {
            return;
        };
        match result {
            Ok(level) => {
                self.install(level);
                match loading.then {
                    Then::Play => self.start_game(),
                    Then::Campaign => self.start_campaign(),
                    Then::Join(game) => self.begin_join(&game),
                    Then::Rejoin => self.rejoin_host(),
                    // The match goes on when its game starts.
                    Then::Match => {}
                }
            }
            Err(e) => {
                println!("couldn't load the map: {e}");
                self.campaign = false;
                let why = format!("COULDN'T LOAD {}", loading.title);
                match loading.then {
                    Then::Rejoin => self.drop_out(why),
                    Then::Match => {
                        self.quit_match(false);
                        self.menu.notice = Some(why);
                    }
                    _ => self.menu.notice = Some(why),
                }
            }
        }
    }

    /// Make a loaded level the one being played.
    pub(crate) fn install(&mut self, level: Level) {
        if let Some(g) = &mut self.gpu {
            g.load_scene(&level.scene);
        }
        self.scene = level.scene;
        self.world = level.world;
        self.nav = level.nav;
        self.map_path = level.path;
        self.level_changed();
        crate::memory::release();
        if let Some((now, most)) = crate::memory::in_use() {
            println!("memory: {now} MB, at most {most} MB");
        }
    }

    /// Settle everything that follows from the level in play.
    pub(crate) fn level_changed(&mut self) {
        self.focus = level_focus(&self.scene.collision);
        let here = MapChoice::new(&self.map_path);
        self.map_name = here.name.clone();
        // A mission isn't one of the lobby's maps.
        if self.scene.campaign.is_none() {
            self.menu.settings.map = match self.maps.iter().position(|m| m.path == here.path) {
                Some(i) => i,
                None => {
                    self.maps.push(here);
                    self.maps.len() - 1
                }
            };
        }
        self.reset_match();
        self.game = self.fresh_game(&self.menu.settings.options);
    }

    /// A game with no one in it yet, under the lobby's settings and
    /// `options`.
    fn fresh_game(&self, options: &GameOptions) -> Game {
        if self.campaign && self.scene.campaign.is_some() {
            return crate::campaign_game(&self.scene);
        }
        let settings = &self.menu.settings;
        let (game_type, score) = self
            .match_rules()
            .unwrap_or((settings.game_type(), settings.score_to_win()));
        new_game(&self.scene, game_type, score, options)
    }

    /// Clear away the last game: its players, effects and sounds.
    fn reset_match(&mut self) {
        self.bots.clear();
        self.mission = None;
        self.locals.clear();
        self.effects = crate::Effects::new();
        self.bodies.clear();
        self.body_actions.clear();
        self.body_poses.clear();
        self.frame_events.clear();
        self.welcome = None;
        self.pending = 0.0;
        self.game_over = None;
        self.ended_at = None;
        self.sound.reset();
        self.music_voice = None;
        // A new game is new ground for the autopilot (testing).
        if let Some(bot) = &mut self.autopilot {
            *bot = Bot::new(4099);
        }
    }

    /// A fresh game for the people here; computer players join them, each
    /// `bots` entry one: its team (`ANY_TEAM`: the one with the fewest
    /// players) and the number of its name and look (see `bot_name`).
    pub(crate) fn seat_players(&mut self, bots: &[(u8, usize)], options: &GameOptions) {
        self.reset_match();
        self.game = self.fresh_game(options);
        let seats = self.seats.clone();
        let teams = self.game.rules.game_type.teams();
        for (k, seat) in seats.into_iter().enumerate() {
            if self.game.players.len() >= scene::MAX_BODIES {
                break;
            }
            let i = if teams {
                self.game.add_player_on(seat.team)
            } else {
                self.game.add_player()
            };
            let profile = &self.menu.profile;
            self.game.set_name(i, &guest_name(&profile.name, k));
            self.game.set_look(i, profile.look.guest(k));
            let mut l = LocalPlayer::new(i, &self.game);
            l.keyboard = k == 0;
            l.pad = seat.pad;
            self.locals.push(l);
        }
        if let Some(s) = self.start_pos {
            let p = &mut self.game.players[0];
            p.body.position = glam::Vec3::from(s.position) + glam::Vec3::Z * 0.05;
            p.yaw = s.facing;
            self.locals[0] = LocalPlayer {
                keyboard: true,
                pad: self.locals[0].pad,
                ..LocalPlayer::new(0, &self.game)
            };
            // Splitscreen player two stands in front, facing them (testing).
            if self.locals.len() > 1 {
                let me = &self.game.players[0];
                let (at, yaw) = (
                    me.body.position + me.aim() * 2.0,
                    me.yaw + std::f32::consts::PI,
                );
                let them = self.locals[1].player;
                let p = &mut self.game.players[them];
                p.body.position = at;
                p.yaw = yaw;
                self.locals[1] = LocalPlayer {
                    pad: self.locals[1].pad,
                    ..LocalPlayer::new(them, &self.game)
                };
            }
        } else if self.scene.spawns.is_empty() {
            // No spawn points: look over the level from above.
            self.locals[0].flying = true;
            self.locals[0].camera = overview(self.focus, -std::f32::consts::FRAC_PI_4);
        }
        for &(team, k) in bots {
            if self.game.players.len() >= scene::MAX_BODIES {
                break;
            }
            let i = match team {
                h2net::ANY_TEAM => self.game.add_player(),
                t => self.game.add_player_on(t),
            };
            self.game.set_name(i, bot_name(k));
            self.game.set_look(i, bot_look(k));
            self.bots.push(bot_for(i));
        }
        self.game.events.clear();
        if let Some(w) = self.start_weapon {
            self.give_weapon(0, w);
            // H2_DUAL=1: a second one in the left hand (for testing).
            if std::env::var_os("H2_DUAL").is_some() {
                self.give_weapon(0, w);
            }
        }
        // H2_POWERUP=camo or overshield: everyone starts with it (testing).
        let powerup = match std::env::var("H2_POWERUP").as_deref() {
            Ok("camo") => Some(h2sim::game::Powerup::Camouflage),
            Ok("overshield") => Some(h2sim::game::Powerup::Overshield),
            _ => None,
        };
        if let Some(kind) = powerup {
            for i in 0..self.game.players.len() {
                self.game.give_powerup(i, kind);
            }
        }
        // H2_VEHICLE_HEALTH=<fraction>: start the vehicles damaged (testing).
        if let Some(f) = std::env::var("H2_VEHICLE_HEALTH")
            .ok()
            .and_then(|f| f.parse::<f32>().ok())
        {
            for v in &mut self.game.vehicles {
                v.health *= f;
            }
        }
        self.mode = Mode::Playing;
        self.menu_open = false;
        self.set_capture(true);
    }

    /// Play the lobby's game here, open to the network; PCs in the lobby
    /// load the map and join it.
    pub(crate) fn start_game(&mut self) {
        let options = self.menu.settings.options.clone();
        // Bots make way for people.
        let people = self.seats.len() + self.lan_players();
        let bots = self
            .menu
            .settings
            .bots
            .min(scene::MAX_BODIES.saturating_sub(people));
        // Each by its seat's name; in team games, filling the smaller team
        // once the people on PCs in the lobby (who come into the game
        // after it starts) are on theirs.
        let mut people: Vec<u8> = self.seats.iter().map(|s| s.team).collect();
        if let Net::Hosting(host) = &self.net {
            people.extend(host.members().into_iter().flat_map(|m| m.2));
        }
        let teams = match self.menu.settings.game_type().teams() {
            true => bot_teams(&people, bots),
            false => vec![h2net::ANY_TEAM; bots],
        };
        let first = self.seats.len();
        let bots: Vec<_> = teams.into_iter().zip(first..).collect();
        self.seat_players(&bots, &options);
        match &mut self.net {
            Net::Hosting(host) => {
                host.start(&self.map_name);
                self.lan_wait = 0.0;
            }
            _ => self.start_hosting(),
        }
    }

    /// Join a LAN game on the map that's loaded.
    pub(crate) fn begin_join(&mut self, game: &LanGame) {
        self.connect(game);
    }

    /// End the game for the lobby, everyone here (and on PCs that joined)
    /// staying together; online, leave the match for the party.
    pub(crate) fn end_game(&mut self) {
        if self.online.in_match() {
            self.quit_match(false);
            return;
        }
        self.back_to_lobby();
    }

    /// Back to the lobby, keeping who plays here and their teams.
    pub(crate) fn back_to_lobby(&mut self) {
        if self.mode == Mode::Playing {
            let seat = |l: &LocalPlayer| Seat {
                pad: l.pad,
                team: self.game.players.get(l.player).map_or(0, |p| p.team),
                picked: false,
            };
            let mut seats: Vec<Seat> = Vec::new();
            for l in self.locals.iter().filter(|l| l.keyboard) {
                seats.push(seat(l));
            }
            // A guest whose controller went doesn't come back with it.
            for l in self
                .locals
                .iter()
                .filter(|l| !l.keyboard && l.lost_pad.is_none())
            {
                if seats.is_empty() {
                    seats.push(Seat::default());
                }
                seats.push(seat(l));
            }
            if !seats.is_empty() {
                for (new, old) in seats.iter_mut().zip(&self.seats) {
                    new.picked = old.picked;
                }
                self.seats = seats;
            }
        }
        let mission = std::mem::take(&mut self.campaign);
        self.reset_match();
        self.game = self.fresh_game(&self.menu.settings.options);
        self.mode = Mode::Menu;
        self.menu_open = false;
        self.menu.show(if mission {
            Screen::Campaign
        } else {
            Screen::Lobby
        });
        self.set_capture(false);
    }

    /// Back to the menus because a LAN game (or an online match's, or a
    /// custom game's) went wrong: to the games on the network, or the party
    /// when online.
    pub(crate) fn drop_out(&mut self, why: String) {
        if self.online.in_match() {
            self.quit_match(true);
        } else if self.online.in_custom() {
            self.custom_over();
        } else {
            self.net = Net::Offline;
            self.back_to_lobby();
            let online = self.online.signed_in();
            self.menu.show(if online {
                Screen::Live
            } else {
                Screen::SystemLink
            });
        }
        self.menu.notice = Some(why);
    }

    /// The camera circling the level behind the menus.
    pub(crate) fn menu_camera(&self) -> crate::camera::FlyCamera {
        overview(self.focus, self.menu_time * 0.05)
    }

    /// Menu music while the menus are up, faded out for a game.
    pub(crate) fn update_music(&mut self) {
        if let Some(rx) = &self.music_loading {
            match rx.try_recv() {
                Ok(m) => {
                    if m.is_none() {
                        println!("warning: couldn't read the menu music");
                    }
                    self.music = m;
                    self.music_loading = None;
                }
                Err(TryRecvError::Disconnected) => self.music_loading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let wanted = self.mode == Mode::Menu
            && !matches!(&self.loading, Some(l) if matches!(l.then, Then::Play | Then::Join(_) | Then::Rejoin));
        match (wanted, self.music_voice, &self.music) {
            (true, None, Some(m)) => {
                let voice = self
                    .sound
                    .audio
                    .play_music(&m.intro, &m.loops, MUSIC_VOLUME);
                self.music_voice = Some(voice);
            }
            (false, Some(voice), _) => {
                self.sound.audio.fade_out(voice, 1.5);
                self.music_voice = None;
            }
            _ => {}
        }
    }

    /// Hold up the carnage report a little after the game ends.
    pub(crate) fn check_game_over(&mut self, dt: f32) {
        // A mission won: on to the next one, as in the story; after the
        // last one here, say so and go back to the missions.
        if self.mission.as_ref().is_some_and(|m| m.won()) {
            let next = self
                .missions
                .iter()
                .position(|m| m.path == self.map_path)
                .map(|i| i + 1)
                .filter(|&i| i < self.missions.len());
            if self.game_over.is_none() && next.is_none() {
                self.announce("MISSION COMPLETE");
            }
            let t = self.game_over.get_or_insert(0.0);
            *t += dt;
            if *t >= crate::GAME_OVER_DELAY && self.loading.is_none() {
                match next {
                    Some(i) => self.start_mission(i),
                    None => self.back_to_lobby(),
                }
            }
            return;
        }
        if !self.game.over() {
            return;
        }
        if self.game_over.is_none() {
            self.ended_at = Some(self.game.time);
            // Who won, in the kill feed.
            let line = self.scene.text.winner(&self.game);
            self.announce(&line);
        }
        let t = self.game_over.get_or_insert(0.0);
        *t += dt;
        // The carnage report is for everyone, over anyone's pause menu.
        let report = self.menu_open && self.menu_owner.is_none();
        if *t >= crate::GAME_OVER_DELAY && !report {
            self.open_menu(Screen::PostGame, None);
        }
    }

    /// The loading screen, or the menu over the game.
    pub(crate) fn overlay(&mut self, w: f32, h: f32) -> Vec<crate::HudBatch> {
        let (font, white) = (self.scene.hud_font, self.scene.hud_white);
        let mut hb = crate::HudBuilder::new(w, h);
        if let Some(l) = self.loading.as_ref().filter(|l| l.shown()) {
            let s = h / 480.0;
            let black = [0.0, 0.0, 0.0, 1.0];
            hb.quad(
                white,
                [0.0, 0.0, w, h],
                [0.0; 4],
                black,
                crate::hud_mode::PLAIN,
                0.0,
            );
            let dots = ".".repeat((self.menu_time * 2.0) as usize % 4);
            let text = format!("LOADING {}{dots:<3}", l.title);
            let color = [0.72, 0.84, 1.0, 1.0];
            hb.text(font, [w * 0.5, h * 0.5 - 8.0 * s], 16.0 * s, &text, color);
            return hb.finish();
        }
        if self.in_menu() {
            self.with_menu(|m, ctx| m.draw(&mut hb, font, white, w, h, ctx));
        }
        hb.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bots_fill_the_smaller_team_counting_everyone() {
        // Two people on red (one joining from another PC): both bots blue.
        assert_eq!(bot_teams(&[0, 0], 2), [1, 1]);
        // One who'll be put on the smaller team counts there.
        assert_eq!(bot_teams(&[0, h2net::ANY_TEAM], 2), [0, 1]);
        assert_eq!(bot_teams(&[1], 3), [0, 0, 1]);
        assert_eq!(bot_teams(&[], 2), [0, 1]);
    }
}
