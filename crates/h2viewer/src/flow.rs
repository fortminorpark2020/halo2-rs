//! The game's flow: menus, loading maps in the background, starting and
//! ending games, and the menu music.

use crate::input::PadPress;
use crate::lan::Net;
use crate::local::{armor_colors, LocalPlayer};
use crate::menu::{self, Action, Input, MapChoice, Menu, ScoreLine, Screen};
use crate::{
    level_focus, load_level, new_game, scene, App, Level, Loading, Mode, Then, MUSIC_VOLUME,
};
use gilrs::GamepadId;
use h2net::LanGame;
use h2sim::Bot;
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

/// A controller press, as the menus see it.
enum Press {
    Menu(Input),
    Join,
    Leave,
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
        _ => Press::None,
    }
}

pub fn bot_for(player: usize) -> (usize, Bot) {
    (player, Bot::new(player as u32 * 7919 + 13))
}

impl App {
    /// The menus are showing (over the map, or over a game).
    pub(crate) fn in_menu(&self) -> bool {
        self.mode == Mode::Menu || self.menu_open
    }

    /// Run `f` with the menu and what it shows.
    pub(crate) fn with_menu<R>(&mut self, f: impl FnOnce(&mut Menu, &menu::Context) -> R) -> R {
        let scores = self.score_lines();
        let seats = self.seat_labels();
        let joined = self.joined();
        let ctx = menu::Context {
            maps: &self.maps,
            lan: &self.lan_games,
            seats: &seats,
            scores: &scores,
            joined,
        };
        f(&mut self.menu, &ctx)
    }

    pub(crate) fn menu_input(&mut self, input: Input) {
        let action = self.with_menu(|m, ctx| m.input(input, ctx));
        self.after_menu(action);
    }

    pub(crate) fn menu_click(&mut self) {
        let (w, h) = self.window_size();
        let pos = self.mouse;
        let action = self.with_menu(|m, ctx| m.click(pos, w, h, ctx));
        self.after_menu(action);
    }

    pub(crate) fn menu_hover(&mut self) {
        let (w, h) = self.window_size();
        let pos = self.mouse;
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

    fn after_menu(&mut self, action: Action) {
        if let Some(s) = self.menu.sound.take() {
            self.sound.play_ui(&self.scene, s);
        }
        match action {
            Action::None => {}
            Action::Start => self.start_selected(),
            Action::Join(game) => self.join_game(&game),
            Action::Resume => {
                self.menu_open = false;
                self.set_capture(true);
            }
            Action::EndGame => self.end_game(),
            Action::Quit => self.quit = true,
        }
    }

    /// A controller press while the menus are up.
    pub(crate) fn menu_pad(&mut self, id: GamepadId, pad_press: PadPress) {
        let seated = self.seats.iter().position(|s| *s == Some(id));
        let lobby = self.mode == Mode::Menu && self.menu.screen == Screen::Lobby;
        match press(pad_press) {
            Press::Menu(input) => {
                if input == Input::Select && seated.is_none() && self.seat_free() {
                    self.seats[0] = Some(id);
                }
                self.menu_input(input);
            }
            Press::Join if lobby && seated.is_none() => {
                if self.seat_free() {
                    self.seats[0] = Some(id);
                } else if self.seats.len() < crate::MAX_LOCAL {
                    self.seats.push(Some(id));
                }
                self.sound.play_ui(&self.scene, menu::Sound::Forward);
            }
            Press::Join if self.menu.screen == Screen::Pause => self.menu_input(Input::Back),
            Press::Join => self.menu_input(Input::Select),
            Press::Leave if lobby => match seated {
                Some(0) => self.seats[0] = None,
                Some(k) => {
                    self.seats.remove(k);
                    self.sound.play_ui(&self.scene, menu::Sound::Back);
                }
                None => {}
            },
            Press::Leave => self.menu_input(Input::Back),
            Press::None => {}
        }
    }

    /// Player one is free for a controller: no one has used the keyboard or
    /// mouse, and no controller has it yet.
    fn seat_free(&self) -> bool {
        !self.keyboard_used && self.seats[0].is_none()
    }

    /// How each person at this PC plays, for the lobby.
    fn seat_labels(&self) -> Vec<&'static str> {
        self.seats
            .iter()
            .map(|s| {
                if s.is_some() {
                    "CONTROLLER"
                } else {
                    "KEYBOARD"
                }
            })
            .collect()
    }

    /// Everyone's score, best first.
    pub(crate) fn score_lines(&self) -> Vec<ScoreLine> {
        let mut lines: Vec<ScoreLine> = self
            .game
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| ScoreLine {
                name: format!("PLAYER {}", i + 1),
                kills: p.kills,
                deaths: p.deaths,
                color: armor_colors(i)[0],
                local: self.locals.iter().any(|l| l.player == i),
            })
            .collect();
        lines.sort_by(|a, b| b.kills.cmp(&a.kills).then(a.deaths.cmp(&b.deaths)));
        lines
    }

    pub(crate) fn open_menu(&mut self, screen: Screen) {
        self.menu.show(screen);
        self.menu_open = true;
        self.fire_held = false;
        self.zoom_held = false;
        self.set_capture(false);
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

    fn join_game(&mut self, game: &LanGame) {
        if game.map.eq_ignore_ascii_case(&self.map_name) {
            self.begin_join(game);
            return;
        }
        let path = self.map_path.with_file_name(format!("{}.map", game.map));
        if !path.exists() {
            self.menu.notice = Some(format!("YOU DON'T HAVE {}", menu::map_title(&game.map)));
            return;
        }
        self.begin_load(path, Then::Join(game.clone()));
    }

    fn begin_load(&mut self, path: PathBuf, then: Then) {
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
                    Then::Join(game) => self.begin_join(&game),
                }
            }
            Err(e) => {
                println!("couldn't load the map: {e}");
                self.menu.notice = Some(format!("COULDN'T LOAD {}", loading.title));
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
    }

    /// Settle everything that follows from the level in play.
    pub(crate) fn level_changed(&mut self) {
        self.focus = level_focus(&self.scene.collision);
        let here = MapChoice::new(&self.map_path);
        self.map_name = here.name.clone();
        self.menu.settings.map = match self.maps.iter().position(|m| m.path == here.path) {
            Some(i) => i,
            None => {
                self.maps.push(here);
                self.maps.len() - 1
            }
        };
        self.reset_match();
        self.game = new_game(&self.scene, self.score_to_win());
    }

    fn score_to_win(&self) -> u32 {
        menu::SCORES[self.menu.settings.score]
    }

    /// Clear away the last game: its players, effects and sounds.
    fn reset_match(&mut self) {
        self.bots.clear();
        self.locals.clear();
        self.effects = crate::Effects::new();
        self.bodies.clear();
        self.body_actions.clear();
        self.body_poses.clear();
        self.frame_events.clear();
        self.welcome = None;
        self.pending = 0.0;
        self.game_over = None;
        self.sound.reset();
        self.music_voice = None;
    }

    /// A fresh game for the people here; `bots` computer players join them.
    fn seat_players(&mut self, bots: usize) {
        self.reset_match();
        self.game = new_game(&self.scene, self.score_to_win());
        let seats = self.seats.clone();
        for (k, pad) in seats.into_iter().enumerate() {
            if self.game.players.len() >= scene::MAX_BODIES {
                break;
            }
            let i = self.game.add_player();
            let mut l = LocalPlayer::new(i, &self.game);
            l.keyboard = k == 0;
            l.pad = pad;
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
        } else if self.scene.spawns.is_empty() {
            // No spawn points: look over the level from above.
            self.locals[0].flying = true;
            self.locals[0].camera = overview(self.focus, -std::f32::consts::FRAC_PI_4);
        }
        for _ in 0..bots {
            if self.game.players.len() >= scene::MAX_BODIES {
                break;
            }
            let i = self.game.add_player();
            self.bots.push(bot_for(i));
        }
        self.game.events.clear();
        if let Some(w) = self.start_weapon {
            self.give_weapon(0, w);
        }
        self.mode = Mode::Playing;
        self.menu_open = false;
        self.set_capture(true);
    }

    /// Play the lobby's game here, open to the network.
    pub(crate) fn start_game(&mut self) {
        self.seat_players(self.menu.settings.bots);
        self.start_hosting();
    }

    /// Join a LAN game on the map that's loaded.
    pub(crate) fn begin_join(&mut self, game: &LanGame) {
        self.seat_players(0);
        self.connect(game);
    }

    /// Leave the game for the lobby, everyone here staying together.
    pub(crate) fn end_game(&mut self) {
        self.net = Net::Offline;
        if self.mode == Mode::Playing {
            let mut seats: Vec<Option<GamepadId>> = Vec::new();
            for l in self.locals.iter().filter(|l| l.keyboard) {
                seats.push(l.pad);
            }
            for l in self.locals.iter().filter(|l| !l.keyboard) {
                if seats.is_empty() {
                    seats.push(None);
                }
                seats.push(l.pad);
            }
            if !seats.is_empty() {
                self.seats = seats;
            }
        }
        self.reset_match();
        self.game = new_game(&self.scene, self.score_to_win());
        self.mode = Mode::Menu;
        self.menu_open = false;
        self.menu.show(Screen::Lobby);
        self.set_capture(false);
    }

    /// Back to the menus because a LAN game went wrong.
    pub(crate) fn drop_out(&mut self, why: String) {
        self.end_game();
        self.menu.show(Screen::SystemLink);
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
            && !matches!(&self.loading, Some(l) if matches!(l.then, Then::Play | Then::Join(_)));
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

    /// Hold up the carnage report a little after someone wins.
    pub(crate) fn check_game_over(&mut self, dt: f32) {
        if self.game.winner.is_none() {
            return;
        }
        let t = self.game_over.get_or_insert(0.0);
        *t += dt;
        if *t >= crate::GAME_OVER_DELAY && !self.menu_open {
            self.open_menu(Screen::PostGame);
        }
    }

    /// The loading screen, or the menu over the game.
    pub(crate) fn overlay(&mut self, w: f32, h: f32) -> Vec<crate::HudBatch> {
        let (font, white) = (self.scene.hud_font, self.scene.hud_white);
        let mut hb = crate::HudBuilder::new(w, h);
        if let Some(l) = &self.loading {
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
