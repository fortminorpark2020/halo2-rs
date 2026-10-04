//! h2viewer: Halo 2 multiplayer on Halo 2's own maps: menus, a lobby,
//! bots, splitscreen and LAN games.
//!
//! Usage: h2viewer [path\to\level.map]
//! With no argument it looks for the maps in the usual install folders.
//!
//! Menus: arrows / WASD or the d-pad move, Enter or A chooses, Esc or B goes
//! back; the mouse works too.
//!
//! In a game, keyboard and mouse: click to capture the mouse, WASD move,
//! Space jump, Ctrl/C crouch, left mouse fire, right mouse / Z zoom, R
//! reload, F melee, G / middle mouse throw a grenade, X switch grenades, E
//! pick up (hold to swap weapons), Q / mouse wheel switch weapon, hold Tab
//! for the scoreboard, B add a bot, 1-9 take any weapon (testing), ` toggles
//! walking / flying (fly: Space/C up/down, Shift fast), Esc the pause menu.
//!
//! Vehicles: hold E (X on a controller) by one to drive, gun or ride, and
//! again to get out; the view follows the vehicle and it steers toward
//! where you look. G (left trigger) boosts a Ghost or Banshee.
//!
//! Controllers (Halo 2's layout, see `input`): A takes over player one,
//! Start joins as another splitscreen player (in a game: the pause menu),
//! hold Back for the scoreboard.
//!
//! LAN: every game is open to other PCs on the network; System Link lists
//! the games other PCs host (see `lan`).

mod audio;
mod body;
mod camera;
mod effects;
mod flow;
mod font;
mod gpu;
mod hud;
mod input;
mod lan;
mod local;
mod mapinfo;
mod menu;
mod objective;
mod probe;
mod rig;
mod scene;
mod soundscape;
mod vehicles;

use blam_cache::geometry::Mesh;
use blam_cache::PlayerSpawn;
use body::{BodyAnimator, BodyInput};
use camera::FlyCamera;
use effects::Effects;
use gilrs::GamepadId;
use glam::{Mat4, Vec3};
use gpu::{hud_mode, DrawCall, Frame, HudBatch};
use h2sim::game::{Event, GrenadeKind, HeldWeapon, TICK};
use h2sim::vehicle::SeatRole;
use h2sim::{
    Bot, Command, Game, GameType, ItemKind, ItemSpawn, KillZone, NavGraph, Rules, WeaponState,
    World,
};
use hud::HudBuilder;
use input::{PadPress, Pads};
use lan::Net;
use local::{display_name, kill_message, player_colors, Keyboard, LocalPlayer, Taps};
use menu::{MapChoice, Menu, Screen, Settings};
use scene::Scene;
use std::collections::HashSet;
use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// Most people sharing one screen.
pub(crate) const MAX_LOCAL: usize = 4;
/// Seconds between someone winning and the carnage report.
const GAME_OVER_DELAY: f32 = 4.0;
const MUSIC_VOLUME: f32 = 0.5;

const DEFAULT_MAP_DIRS: &[&str] = &[
    r"C:\Games\Halo 2 Project Cartographer\maps",
    r"C:\Program Files (x86)\Microsoft Games\Halo 2\maps",
    r"C:\Program Files\Microsoft Games\Halo 2\maps",
    "maps",
];

fn find_map() -> Result<PathBuf, String> {
    if let Some(arg) = std::env::args_os().nth(1) {
        return Ok(PathBuf::from(arg));
    }
    DEFAULT_MAP_DIRS
        .iter()
        .map(|d| PathBuf::from(d).join("lockout.map"))
        .find(|p| p.exists())
        .ok_or_else(|| {
            "Couldn't find lockout.map. Drag a .map file onto h2viewer.exe, or run: h2viewer <path to .map>".into()
        })
}

/// Center of the densest part of the level, used as the spawn point.
fn level_focus(mesh: &Mesh) -> (Vec3, f32) {
    let mut lo = [0f32; 3];
    let mut hi = [0f32; 3];
    for k in 0..3 {
        let mut v: Vec<f32> = mesh.positions.iter().map(|p| p[k]).collect();
        v.sort_by(f32::total_cmp);
        lo[k] = v[v.len() / 10];
        hi[k] = v[v.len() * 9 / 10];
    }
    let lo = Vec3::from(lo);
    let hi = Vec3::from(hi);
    ((lo + hi) * 0.5, (hi - lo).length() * 0.5)
}

/// Slayer on the map's weapons: spawn with a Battle Rifle and an SMG.
fn rules(scene: &Scene) -> Rules {
    let index = |names: &[&str]| -> Vec<usize> {
        names
            .iter()
            .filter_map(|n| scene.weapons.iter().position(|w| w.def.name == *n))
            .collect()
    };
    let defaults = Rules::default();
    let mut frag = defaults.frag;
    let mut plasma = defaults.plasma;
    if let Some(v) = scene.grenades[0].speed {
        frag.speed = v;
    }
    if let Some(v) = scene.grenades[1].speed {
        plasma.speed = v;
    }
    Rules {
        starting_weapons: index(&["battle_rifle", "smg"]),
        headshot_weapons: index(&[
            "battle_rifle",
            "magnum",
            "covenant_carbine",
            "sniper_rifle",
            "beam_rifle",
        ]),
        lunge_weapons: index(&["energy_blade"]),
        frag,
        plasma,
        falling: scene.falling.unwrap_or(defaults.falling),
        ..defaults
    }
}

/// A weapon's game values, timed to its first person animations.
fn weapon_def(w: &scene::WeaponAssets) -> h2sim::WeaponDef {
    let mut def = w.def.clone();
    let ready = w.rig.as_ref().and_then(|rig| {
        let i = rig.find("first_person:ready", 0)?;
        Some(rig.graph.animations.get(i)?.duration())
    });
    if let Some(t) = ready.filter(|t| *t > 0.0) {
        def.ready_time = t;
    }
    def
}

/// Where players spawn: the map's spawn points, or the middle of the level.
fn level_spawns(scene: &Scene) -> Vec<(Vec3, f32)> {
    let mut spawns: Vec<(Vec3, f32)> = scene
        .spawns
        .iter()
        .map(|s| (Vec3::from(s.position), s.facing))
        .collect();
    if spawns.is_empty() {
        spawns.push((level_focus(&scene.collision).0, 0.0));
    }
    spawns
}

/// A game on the scene's level, with no one in it yet.
fn new_game(scene: &Scene, game_type: GameType, score_to_win: u32) -> Game {
    let items = scene
        .items
        .iter()
        .map(|i| ItemSpawn {
            kind: i.kind,
            position: i.position,
            respawn: i.respawn_seconds,
        })
        .collect();
    let rules = Rules {
        game_type,
        score_to_win,
        flag_weapon: scene.flag.as_ref().map(|f| f.weapon),
        ball_weapon: scene.ball,
        bomb_weapon: scene.bomb,
        ..rules(scene)
    };
    let mut game = Game::new(
        rules,
        scene.weapons.iter().map(weapon_def).collect(),
        level_spawns(scene),
        items,
        scene.movement,
        scene.biped,
    );
    // Flags, balls and bombs for this game type (a joined game takes the
    // host's); hills and territories always, so a joined game has them.
    let (homes, bases) = objective::carried_spots(scene, game_type);
    game.set_flags(&homes, &bases);
    game.hills = objective::hills(scene);
    game.territories = objective::territories(scene);
    game.kill_zones = kill_zones(scene);
    game.set_vehicles(scene.vehicles.defs.clone(), scene.vehicles.spawns.clone());
    game
}

/// The places on the level that kill.
fn kill_zones(scene: &Scene) -> Vec<KillZone> {
    scene
        .kill_volumes
        .iter()
        .map(|k| {
            KillZone::new(
                k.position.into(),
                k.forward.into(),
                k.up.into(),
                k.extents.into(),
            )
        })
        .collect()
}

/// A map's level, ready to play.
struct Level {
    scene: Scene,
    world: World,
    nav: NavGraph,
    path: PathBuf,
}

/// Read a map and work out what bots need to find their way around it
/// (slow enough to run on another thread).
fn load_level(path: &Path) -> Result<Level, String> {
    let scene = Scene::load(path).map_err(|e| e.to_string())?;
    println!(
        "{} triangles, {} textures, {} weapons, {} items, {} vehicles, {} game type points, {} kill zones",
        scene.triangle_count(),
        scene.textures.len() - 1,
        scene.weapons.len(),
        scene.items.len(),
        scene.vehicles.spawns.len(),
        scene.netgame_flags.len(),
        scene.kill_volumes.len(),
    );
    // H2_LIST_VEHICLES=1: where the map's vehicles are.
    if std::env::var_os("H2_LIST_VEHICLES").is_some() {
        for s in &scene.vehicles.spawns {
            let def = &scene.vehicles.defs[s.def];
            let kind = &scene.vehicles.kinds[s.def];
            println!(
                "vehicle {} at {:.2} facing {:.2} ({:?}, {} seats, radius {:.2}, engine {:?}, boost {:?}, enter {:?})",
                def.name,
                s.position,
                s.yaw,
                def.drive,
                def.seats.len(),
                def.radius,
                kind.engine,
                kind.boost,
                kind.enter_sounds
            );
        }
    }
    let world = World::new(&scene.collision.positions, &scene.collision.indices);
    let mut spots: Vec<Vec3> = level_spawns(&scene).iter().map(|s| s.0).collect();
    spots.extend(scene.items.iter().map(|i| i.position));
    spots.extend(objective::objective_points(&scene));
    let started = Instant::now();
    let nav = NavGraph::for_level(&world, &spots, &kill_zones(&scene));
    println!(
        "bot routes: {} points, {} links ({:.1?})",
        nav.points.len(),
        nav.links.iter().map(Vec::len).sum::<usize>(),
        started.elapsed()
    );
    Ok(Level {
        scene,
        world,
        nav,
        path: path.to_path_buf(),
    })
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

/// No window: player one gets into a vehicle's driver seat and drives,
/// printing where the vehicle goes. `spec` is "vehicle forward right yaw
/// seconds" (yaw in degrees: where the driver looks, relative to the
/// vehicle).
fn drive_test(level: &Level, spec: &str) {
    let n: Vec<f32> = spec
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    let [v, forward, right, yaw, seconds] = n[..] else {
        println!("H2_DRIVE wants \"vehicle forward right yaw seconds\"");
        return;
    };
    let mut game = new_game(&level.scene, GameType::Slayer, 0);
    let me = game.add_player();
    let v = v as usize;
    let mut cmd = Command::default();
    for _ in 0..60 {
        game.step(&level.world, &[cmd]);
    }
    let veh = &game.vehicles[v];
    let def = &game.vehicle_defs[veh.def];
    let seat = def.driver_seat().unwrap_or(0);
    let entry = veh.to_world(def, def.seats[seat].entry);
    game.players[me].body.position = entry - Vec3::Z * 0.3;
    println!("{} at {:.2}, entry {entry:.2}", def.name, veh.center);
    if env_set("H2_DRIVE_DEF") {
        for b in &def.hull {
            println!("box at {:.2} half {:.2}", b.center, b.half_extents);
        }
        for s in &def.spheres {
            println!("sphere {s:.2?}");
        }
        for w in &def.wheels {
            println!("wheel {:.2} r {}", w.position, w.radius);
        }
        for (k, s) in def.seats.iter().enumerate() {
            println!(
                "seat {k} {:?} at {:.2} entry {:.2} (world {:.2}) r {:.2} eye {:.2} pivot {:.2?} pitch {:.2?}",
                s.role,
                s.position,
                s.entry,
                veh.to_world(def, s.entry),
                s.entry_radius,
                s.eye,
                s.pivot,
                s.pitch_range
            );
        }
        println!("center {:.2} radius {:.2}", def.center, def.radius);
    }
    cmd.action = true;
    for _ in 0..90 {
        game.step(&level.world, &[cmd]);
    }
    println!("riding {:?}", game.riding(me));
    cmd.action = false;
    let look = game.vehicles[v].yaw() + yaw.to_radians();
    for tick in 0..(seconds / TICK) as usize {
        cmd.movement = glam::vec2(right, forward);
        cmd.yaw = look;
        game.step(&level.world, &[cmd]);
        if tick % 15 == 0 {
            let veh = &game.vehicles[v];
            println!(
                "{:5.2}s at {:.2} speed {:.2} yaw {:.0} up {:.2} steer {:.2} squash {:.2?} asleep {} {:?}",
                tick as f32 * TICK,
                veh.center,
                veh.speed(),
                veh.yaw().to_degrees(),
                veh.up().z,
                veh.steer,
                veh.compression,
                veh.asleep,
                veh.impact
            );
        }
    }
}

/// Bots only, no window: play `seconds` of a game and print the kills, flag
/// moves and score.
fn simulate(level: &Level, settings: &Settings, seconds: f32) {
    let mut game = new_game(&level.scene, settings.game_type(), settings.score_to_win());
    // H2_SEED=<n> plays a different game.
    let seed: u32 = std::env::var("H2_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    game.reseed(seed);
    let mut bots: Vec<(usize, Bot)> = (0..settings.bots.max(2))
        .map(|_| {
            let i = game.add_player();
            (i, Bot::new(i as u32 * 7919 + 13 + seed * 104_729))
        })
        .collect();
    for tick in 0..(seconds / TICK) as usize {
        let commands: Vec<Command> = bots
            .iter_mut()
            .map(|(i, bot)| bot.think(&game, &level.world, &level.nav, *i))
            .collect();
        game.step(&level.world, &commands);
        let t = tick as f32 * TICK;
        for e in std::mem::take(&mut game.events) {
            match e {
                Event::Killed { killer, victim, .. } => {
                    let at = game.players[victim].body.position;
                    println!("{t:6.1} {killer:?} killed {victim} at {at:.1}");
                }
                Event::Flag { team, player, what } => {
                    println!("{t:6.1} flag {team} {what:?} by {player:?}");
                }
                Event::Hill { .. } | Event::Territory { .. } | Event::Juggernaut { .. } => {
                    println!("{t:6.1} {e:?}");
                }
                _ => {}
            }
        }
        if game.winner.is_some() {
            println!("{t:6.1} game over");
            break;
        }
        // H2_SIM_WHERE=1: where everyone is every 20 seconds.
        if std::env::var_os("H2_SIM_WHERE").is_some() && tick % (20 * 60) == 0 {
            let at: Vec<String> = game
                .players
                .iter()
                .map(|p| format!("{:.0?}{}", p.body.position, if p.alive { "" } else { "x" }))
                .collect();
            println!("{t:6.1} at {}", at.join(" "));
            for (i, bot) in &bots {
                println!("   {i}: {}", bot.describe());
            }
        }
    }
    let scores: Vec<i32> = game.players.iter().map(|p| p.score).collect();
    println!("scores {scores:?}");
}

/// A map loading in the background, behind the loading screen.
struct Loading {
    title: String,
    level: Receiver<Result<Level, String>>,
    then: Then,
}

/// What to do once a map has loaded.
enum Then {
    Play,
    Join(h2net::LanGame),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// The menus, over the map seen from a circling camera.
    Menu,
    Playing,
}

/// A Spartan's body posed for this frame.
struct BodyPose {
    vertices: Vec<scene::Vertex>,
    object: Mat4,
    /// Where the weapons go in the right and left hands.
    weapons: [Mat4; 2],
}

struct App {
    scene: Scene,
    world: World,
    game: Game,
    /// Computer players and the player each one drives.
    bots: Vec<(usize, Bot)>,
    nav: NavGraph,
    /// The middle of the level and its size, for the menu camera.
    focus: (Vec3, f32),
    /// The people playing at this computer, one view each.
    locals: Vec<LocalPlayer>,
    /// The people at this computer between games: player one (keyboard and
    /// maybe a controller) and the others with controllers.
    seats: Vec<flow::Seat>,
    pads: Pads,
    window: Option<Arc<Window>>,
    gpu: Option<gpu::Gpu>,
    keys: HashSet<KeyCode>,
    captured: bool,
    fire_held: bool,
    zoom_held: bool,
    /// The mouse pointer, in window pixels.
    mouse: [f32; 2],
    last_frame: Instant,
    /// Time not yet simulated, less than a tick.
    pending: f32,
    effects: Effects,
    /// Third person animation of each player.
    bodies: Vec<BodyAnimator>,
    /// Actions players started this frame (reload, melee...), for their bodies.
    body_actions: Vec<(usize, &'static str)>,
    body_poses: Vec<Option<BodyPose>>,
    /// LAN play: hosting, or joined to another PC's game.
    net: Net,
    browser: h2net::Browser,
    lan_games: Vec<h2net::LanGame>,
    /// Tells this running game apart from others on the network.
    session: u64,
    map_path: PathBuf,
    map_name: String,
    /// Events of this frame's ticks, for joined PCs.
    frame_events: Vec<Event>,
    /// Players a host gave us, until its game includes them.
    welcome: Option<Vec<usize>>,
    sound: soundscape::Soundscape,
    mode: Mode,
    menu: Menu,
    /// The menu is up over a game (paused, or the carnage report).
    menu_open: bool,
    maps: Vec<MapChoice>,
    /// The maps' pictures, for the lobby.
    map_pictures: Vec<blam_cache::bitmap::Image>,
    loading: Option<Loading>,
    /// Seconds the menus have been up, for the camera circling the map.
    menu_time: f32,
    /// Seconds since someone won.
    game_over: Option<f32>,
    music: Option<scene::Music>,
    music_loading: Option<Receiver<Option<scene::Music>>>,
    music_voice: Option<u64>,
    quit: bool,
    /// Someone has used the keyboard or mouse (so player one plays with them).
    keyboard_used: bool,
    /// For testing: where player one starts (H2_POS) and a weapon to hold
    /// (H2_WEAPON).
    start_pos: Option<PlayerSpawn>,
    start_weapon: Option<usize>,
}

impl App {
    fn set_capture(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        if on {
            // Some systems refuse to grab; raw mouse motion still works then.
            let _ = w
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined));
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
        }
        w.set_cursor_visible(!on);
        self.captured = on;
    }

    /// The player at the keyboard and mouse.
    fn keyboard_local(&mut self) -> Option<&mut LocalPlayer> {
        self.locals.iter_mut().find(|l| l.keyboard)
    }

    fn local_of(&mut self, player: usize) -> Option<&mut LocalPlayer> {
        self.locals.iter_mut().find(|l| l.player == player)
    }

    /// Tell everyone at this computer.
    fn announce(&mut self, text: &str) {
        println!("{}", text.to_lowercase());
        for l in &mut self.locals {
            l.message(text.to_string());
        }
    }

    /// For testing: swap the weapon in hand for any weapon. Asking for the
    /// one-handed gun in hand again puts a second one in the left hand.
    fn give_weapon(&mut self, player: usize, w: usize) {
        let Some(def) = self.scene.weapons.get(w).map(|a| &a.def) else {
            return;
        };
        let p = &mut self.game.players[player];
        let held = HeldWeapon {
            weapon: w,
            state: WeaponState::new(def),
        };
        let in_hand = p.weapons.get(p.current).is_some_and(|h| h.weapon == w);
        if p.alive && in_hand && def.dual.is_some() && p.left.is_none() && p.objective.is_none() {
            p.left = Some(held);
            self.game.events.push(Event::Switched { player });
            return;
        }
        if !p.alive {
            return;
        }
        // On the back: bring it up.
        if let Some(k) = p.weapons.iter().position(|h| h.weapon == w) {
            if k != p.current {
                p.current = k;
                self.game.events.push(Event::Switched { player });
            }
            return;
        }
        match p.weapons.get_mut(p.current) {
            Some(h) => *h = held,
            None => p.weapons.push(held),
        }
        println!("weapon: {}", def.name);
    }

    fn add_bot(&mut self) {
        if self.joined() {
            self.announce("ONLY THE HOST CAN ADD BOTS");
            return;
        }
        if self.game.players.len() >= scene::MAX_BODIES {
            return;
        }
        let i = self.game.add_player();
        self.bots.push((i, Bot::new(i as u32 * 7919 + 13)));
        self.announce(&format!("PLAYER {} JOINED", i + 1));
    }

    /// Another person joins in splitscreen.
    fn add_local(&mut self, pad: Option<GamepadId>) {
        if self.joined() {
            self.request_local(pad);
            return;
        }
        if self.locals.len() >= MAX_LOCAL || self.game.players.len() >= scene::MAX_BODIES {
            return;
        }
        let i = self.game.add_player();
        let mut l = LocalPlayer::new(i, &self.game);
        l.pad = pad;
        self.locals.push(l);
        self.announce(&format!("PLAYER {} JOINED", i + 1));
    }

    fn pad_pressed(&mut self, id: GamepadId, press: PadPress) {
        if self.loading.is_some() {
            return;
        }
        if self.in_menu() {
            self.menu_pad(id, press);
            return;
        }
        let owner = self.locals.iter().position(|l| l.pad == Some(id));
        match (owner, press) {
            (None, PadPress::Claim) => {
                if let Some(l) = self
                    .locals
                    .iter_mut()
                    .find(|l| l.keyboard && l.pad.is_none())
                {
                    l.pad = Some(id);
                    l.message("CONTROLLER CONNECTED".into());
                }
            }
            (None, PadPress::Join) => self.add_local(Some(id)),
            (Some(_), PadPress::Join) => self.open_menu(Screen::Pause),
            (Some(k), p) => {
                let t = &mut self.locals[k].taps;
                match p {
                    PadPress::Fire => t.fire = true,
                    PadPress::Melee => t.melee = true,
                    PadPress::Reload => t.reload = true,
                    PadPress::SwitchWeapon => t.switch_weapon = true,
                    PadPress::Grenade => t.throw_grenade = true,
                    PadPress::SwitchGrenade => t.switch_grenade = true,
                    PadPress::Zoom => t.zoom = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn update(&mut self, dt: f32) {
        self.poll_loading();
        self.update_music();
        for (id, press) in self.pads.presses() {
            self.pad_pressed(id, press);
        }
        self.lan_games = self.browser.poll().to_vec();
        self.menu_time += dt;
        if self.mode == Mode::Menu || self.loading.is_some() {
            self.effects.update(dt);
            return;
        }
        for l in &mut self.locals {
            if self.menu_open {
                break;
            }
            if let Some(state) = l.pad.and_then(|id| self.pads.state(id)) {
                let scale = 1.0 / l.magnification(&self.scene, &self.game);
                l.camera.look_stick(state.right, dt, scale);
            }
            if l.flying {
                l.camera.update(&self.keys, dt);
            }
        }
        self.check_game_over(dt);
        if self.joined() {
            self.update_joined(dt);
        } else {
            self.step_game(dt);
        }
        let notice = self.lan_notice();
        for l in &mut self.locals {
            l.notice = notice.clone().filter(|_| l.keyboard);
            l.update_camera(&self.game, &self.world, self.pending, dt);
        }
        self.animate_bodies(dt);
        for l in &mut self.locals {
            l.animate_view_model(&self.scene, &self.game, dt);
        }
        self.effects.update(dt);
        let listeners = self.listeners();
        self.sound.update(&self.scene, &self.game, &listeners, dt);
    }

    /// Run the game here: fixed ticks with everyone's controls (people at
    /// this PC, bots, and players on PCs that joined).
    fn step_game(&mut self, dt: f32) {
        self.poll_host();
        self.pending += dt;
        let mut ticked = false;
        while self.pending >= TICK {
            self.pending -= TICK;
            let mut commands = vec![Command::default(); self.game.players.len()];
            let keyboard = Keyboard {
                keys: &self.keys,
                captured: self.captured,
                fire_held: self.fire_held,
                zoom_held: self.zoom_held,
            };
            for l in &self.locals {
                commands[l.player] = if self.menu_open {
                    // Standing still while the menu is up.
                    l.command(None, None)
                } else {
                    let pad = l.pad.and_then(|id| self.pads.state(id));
                    l.command(l.keyboard.then_some(&keyboard), pad)
                };
            }
            for (i, bot) in &mut self.bots {
                commands[*i] = bot.think(&self.game, &self.world, &self.nav, *i);
            }
            self.remote_commands(&mut commands);
            self.game.step(&self.world, &commands);
            if !ticked {
                for l in &mut self.locals {
                    l.taps = Taps::default();
                }
                ticked = true;
            }
            for l in &mut self.locals {
                let eye = local::view_point(&self.game, l.player);
                l.eyes = (l.eyes.1, eye);
            }
            self.frame_events.extend_from_slice(&self.game.events);
            self.handle_events();
        }
        self.send_to_joined(ticked);
    }

    /// Where each view hears from.
    fn listeners(&self) -> Vec<soundscape::Listener> {
        self.locals
            .iter()
            .map(|l| soundscape::Listener {
                position: l.camera.position,
                right: l.camera.basis().1,
                player: l.player,
                first_person: l.first_person(&self.game),
            })
            .collect()
    }

    fn handle_events(&mut self) {
        let listeners = self.listeners();
        for e in std::mem::take(&mut self.game.events) {
            self.sound.event(&self.scene, &self.game, &listeners, &e);
            match e {
                Event::Shot {
                    player,
                    left,
                    hit,
                    hit_player,
                    ..
                } => {
                    if let Some(l) = self.local_of(player) {
                        if left {
                            l.view.fired_left = true;
                        } else {
                            l.view.fired = true;
                        }
                    }
                    match (hit, hit_player) {
                        (Some((p, _)), Some(j)) => {
                            let shielded = self.game.players[j].shield > 0.0;
                            self.effects.player_hit(p, shielded);
                        }
                        (Some((p, n)), None) => self.effects.impact(p, n),
                        _ => {}
                    }
                }
                Event::Reloaded {
                    player,
                    left,
                    empty,
                } => {
                    let what = if left { "reload_1_left" } else { "reload_1" };
                    self.body_actions.push((player, what));
                    if let Some(l) = self.local_of(player) {
                        if left {
                            l.view.reload_left = Some(empty);
                        } else {
                            l.view.reload = Some(empty);
                        }
                    }
                }
                Event::Switched { player } => {
                    self.body_actions.push((player, "ready"));
                    if let Some(l) = self.local_of(player) {
                        l.view.switched = true;
                    }
                }
                Event::Melee { player, .. } => {
                    self.body_actions.push((player, "melee_strike_1"));
                    if let Some(l) = self.local_of(player) {
                        l.view.melee = true;
                    }
                }
                Event::Thrown { player } => {
                    self.body_actions.push((player, "throw_grenade"));
                    if let Some(l) = self.local_of(player) {
                        l.view.thrown = true;
                    }
                }
                Event::Exploded { kind, position } => {
                    self.effects
                        .explosion(position, kind == GrenadeKind::Plasma);
                }
                Event::Killed { killer, victim, .. } => {
                    let betrayal =
                        killer.is_some_and(|k| k != victim && !self.game.is_enemy(k, victim));
                    println!(
                        "{}",
                        kill_message(usize::MAX, killer, victim, betrayal).to_lowercase()
                    );
                    for l in &mut self.locals {
                        l.message(kill_message(l.player, killer, victim, betrayal));
                        if victim == l.player {
                            // The death camera starts behind and above the body.
                            l.camera.pitch = -0.6;
                        }
                    }
                }
                Event::Spawned { player, yaw } => {
                    let eye = local::view_point(&self.game, player);
                    if let Some(l) = self.local_of(player) {
                        l.eyes = (eye, eye);
                        l.camera.yaw = yaw;
                        l.camera.pitch = 0.0;
                        l.view.switched = true;
                    }
                }
                Event::PickedUp { player, kind } => {
                    let what = match kind {
                        ItemKind::Weapon(w) => self
                            .scene
                            .weapons
                            .get(w)
                            .map(|a| display_name(&a.def.name))
                            .unwrap_or_default(),
                        ItemKind::FragGrenades => "FRAG GRENADE".into(),
                        ItemKind::PlasmaGrenades => "PLASMA GRENADE".into(),
                    };
                    if let Some(l) = self.local_of(player) {
                        l.message(format!("PICKED UP {what}"));
                    }
                }
                Event::Entered {
                    player, vehicle, ..
                } => {
                    // The view swings round behind the vehicle (or the
                    // rider's own eyes), facing the way it does.
                    let eye = local::view_point(&self.game, player);
                    let yaw = self.game.vehicles[vehicle].yaw();
                    if let Some(l) = self.local_of(player) {
                        l.eyes = (eye, eye);
                        l.camera.yaw = yaw;
                        l.camera.pitch = -0.15;
                    }
                }
                Event::Exited { player, .. } => {
                    let eye = local::view_point(&self.game, player);
                    if let Some(l) = self.local_of(player) {
                        l.eyes = (eye, eye);
                        l.camera.pitch = 0.0;
                        l.view.switched = true;
                    }
                }
                Event::VehicleDestroyed { position, .. } => {
                    self.effects.explosion(position, false);
                    self.effects.explosion(position + Vec3::Z * 0.4, false);
                }
                Event::Flag { .. }
                | Event::Hill { .. }
                | Event::Territory { .. }
                | Event::Juggernaut { .. } => {
                    println!("{e:?}");
                    for l in &mut self.locals {
                        if let Some(m) = objective::event_message(l.player, &self.game, &e) {
                            l.message(m);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Pose every Spartan's body. Each view leaves out its own player's
    /// body while it looks through their eyes.
    fn animate_bodies(&mut self, dt: f32) {
        let actions = std::mem::take(&mut self.body_actions);
        let Some(body) = &self.scene.body else {
            return;
        };
        let rig = &body.rig;
        let n = self.game.players.len().min(scene::MAX_BODIES);
        self.bodies.resize_with(n, BodyAnimator::default);
        self.body_poses.resize_with(n, || None);
        for (i, p) in self.game.players.iter().enumerate().take(n) {
            let mut style = p
                .held()
                .and_then(|h| self.scene.weapons.get(h.weapon))
                .map_or(("rifle", "any"), |w| body::weapon_style(&w.def.name));
            if p.left.is_some() {
                style.0 = "dual";
            }
            let (s, c) = p.yaw.sin_cos();
            let v = p.body.velocity.truncate();
            let mut input = BodyInput {
                velocity: glam::vec2(v.x * c + v.y * s, v.y * c - v.x * s),
                grounded: p.body.grounded,
                crouching: p.body.crouch > 0.5,
                alive: p.alive,
                style,
                seat: None,
            };
            let mut object = Mat4::from_translation(p.body.position) * Mat4::from_rotation_z(p.yaw);
            // Riding: sitting in the seat, turning with the vehicle (and
            // its turret).
            if let Some((veh, s, seat)) = local::seat_of(&self.game, i) {
                let vehicle = &self.game.vehicles[veh];
                let stance = self
                    .scene
                    .vehicles
                    .kinds
                    .get(vehicle.def)
                    .and_then(|k| k.seat_stances.get(s).copied());
                input = BodyInput {
                    velocity: glam::Vec2::ZERO,
                    grounded: true,
                    crouching: false,
                    seat: stance,
                    style: match seat.role {
                        SeatRole::Driver => ("unarmed", ""),
                        SeatRole::Gunner => ("fixed", ""),
                        SeatRole::Passenger => style,
                    },
                    ..input
                };
                let mut turn = vehicle.rotation;
                if seat.pivot.is_some() {
                    turn *= vehicle.turret_turn();
                }
                object = Mat4::from_rotation_translation(turn, p.body.position);
            }
            for &(_, what) in actions.iter().filter(|a| a.0 == i) {
                self.bodies[i].act(rig, &input, what);
            }
            let pose = self.bodies[i].update(rig, &input, dt);
            let world = rig.world(&pose);
            self.body_poses[i] = Some(BodyPose {
                vertices: rig.skin.pose(&rig.skin_matrices(&world)),
                object,
                weapons: [false, true].map(|left| rig.weapon_frame(&world, left)),
            });
        }
    }

    /// Everything in the world besides the level: scenery, items lying on
    /// the map, dropped weapons and grenades in flight.
    fn world_draws(&self) -> Vec<DrawCall> {
        let scene = &self.scene;
        let light_at = |p: Vec3| scene.level_light.at(&scene.textures, p + Vec3::Z * 0.2);
        let mut world = vec![DrawCall {
            mesh: 0,
            model: Mat4::IDENTITY,
            light: None,
            colors: None,
        }];
        for o in &scene.objects {
            world.push(DrawCall {
                mesh: o.mesh,
                model: o.transform,
                light: o.light,
                colors: None,
            });
        }
        for (item, timer) in scene.items.iter().zip(&self.game.item_timers) {
            if let (Some(mesh), true) = (item.mesh, *timer <= 0.0) {
                world.push(DrawCall {
                    mesh,
                    model: item.transform,
                    light: item.light,
                    colors: None,
                });
            }
        }
        for d in &self.game.dropped {
            if let Some(mesh) = scene.weapons.get(d.weapon).and_then(|w| w.world_mesh) {
                world.push(DrawCall {
                    mesh,
                    model: scene::placement_matrix(d.position.into(), [d.yaw, 0.0, FRAC_PI_2], 1.0),
                    light: light_at(d.position),
                    colors: None,
                });
            }
        }
        for g in &self.game.grenades {
            let assets = scene.grenades[(g.kind == GrenadeKind::Plasma) as usize];
            if let Some(mesh) = assets.mesh {
                // Tumbling through the air.
                let spin = if g.velocity.length_squared() > 0.01 {
                    self.game.time as f32 * 12.0
                } else {
                    0.0
                };
                world.push(DrawCall {
                    mesh,
                    model: Mat4::from_translation(g.position) * Mat4::from_rotation_y(spin),
                    light: light_at(g.position),
                    colors: None,
                });
            }
        }
        world.extend(self.flag_draws());
        world
    }

    /// A player's body and the weapon in their hands.
    fn body_draws(&self, player: usize) -> Vec<DrawCall> {
        let (Some(body), Some(Some(pose))) = (&self.scene.body, self.body_poses.get(player)) else {
            return Vec::new();
        };
        let p = &self.game.players[player];
        let light = self
            .scene
            .level_light
            .at(&self.scene.textures, p.body.position + Vec3::Z * 0.2);
        let mut out = vec![DrawCall {
            mesh: body.meshes[player],
            model: pose.object,
            light,
            colors: Some(player_colors(&self.game, player)),
        }];
        // Drivers and gunners hold the controls, not their guns.
        let hands_free = local::seat_of(&self.game, player)
            .is_none_or(|(_, _, seat)| seat.role == SeatRole::Passenger);
        for (held, hand) in [
            (p.held(), pose.weapons[0]),
            (p.left.as_ref(), pose.weapons[1]),
        ] {
            if let Some(mesh) = held
                .filter(|_| p.alive && hands_free)
                .and_then(|h| self.scene.weapons.get(h.weapon))
                .and_then(|w| w.world_mesh)
            {
                out.push(DrawCall {
                    mesh,
                    model: pose.object * hand,
                    light,
                    colors: None,
                });
            }
        }
        if p.alive {
            let pole = pose.object * pose.weapons[0];
            if let Some(cloth) = objective::carried_cloth(&self.scene, &self.game, player, pole) {
                out.push(DrawCall { light, ..cloth });
            }
        }
        out
    }

    fn render(&mut self) {
        let Some(g) = &self.gpu else { return };
        let (w, h) = g.size();
        let overlay = self.overlay(w, h);
        if self.loading.is_some() {
            let frame = Frame::overlay([0, 0, w as u32, h as u32], &overlay);
            if let Some(g) = &mut self.gpu {
                g.render(&[frame]);
            }
            return;
        }
        let in_game = self.mode == Mode::Playing;
        let ports = if in_game {
            local::viewports(self.locals.len(), w as u32, h as u32)
        } else {
            vec![[0, 0, w as u32, h as u32]]
        };
        let mut shared = self.world_draws();
        // Posed bodies and vehicles go up once, with the first view.
        let (vehicle_draws, mut body_meshes) = vehicles::draws(&self.scene, &self.game);
        shared.extend(vehicle_draws);
        if let Some(body) = &self.scene.body {
            for (i, pose) in self.body_poses.iter().enumerate() {
                if let Some(pose) = pose {
                    body_meshes.push((body.meshes[i], pose.vertices.clone()));
                }
            }
        }
        let ctf = self.game.rules.game_type == GameType::Ctf;
        if let (Some(flag), true) = (&self.scene.flag, ctf && self.game.has_flags()) {
            let cloth = objective::cloth_vertices(flag, self.game.time as f32);
            body_meshes.push((flag.cloth, cloth));
        }
        struct View {
            viewport: [u32; 4],
            aspect: f32,
            magnification: f32,
            camera: FlyCamera,
            world: Vec<DrawCall>,
            sprites: Vec<gpu::SpriteVertex>,
            draws: local::ViewDraws,
            hud: Vec<gpu::HudBatch>,
            view_model_proj: Mat4,
        }
        let mut views = Vec::new();
        if !in_game {
            // The level behind the menus.
            let camera = self.menu_camera();
            let (_, r, u) = camera.basis();
            let viewport = ports[0];
            views.push(View {
                viewport,
                aspect: viewport[2] as f32 / viewport[3].max(1) as f32,
                magnification: 1.0,
                camera,
                world: shared.clone(),
                sprites: self.effects.sprites(r, u),
                draws: local::ViewDraws {
                    view_models: Vec::new(),
                    view_sprites: Vec::new(),
                    posed: std::mem::take(&mut body_meshes),
                },
                hud: Vec::new(),
                view_model_proj: Mat4::IDENTITY,
            });
        }
        let scores = self.score_lines();
        for (k, l) in self.locals.iter().enumerate().filter(|_| in_game) {
            let Some(&viewport) = ports.get(k) else {
                break;
            };
            let aspect = viewport[2] as f32 / viewport[3].max(1) as f32;
            let mut world = shared.clone();
            let own = l.first_person(&self.game).then_some(l.player);
            for i in 0..self.body_poses.len() {
                if Some(i) != own {
                    world.extend(self.body_draws(i));
                }
            }
            let (_, r, u) = l.camera.basis();
            let mut draws = l.view_draws(&self.scene, &self.game);
            if k == 0 {
                draws.posed.splice(0..0, std::mem::take(&mut body_meshes));
            }
            let (vw, vh) = (viewport[2] as f32, viewport[3] as f32);
            // The menu replaces the HUD while it's up.
            let mut hud = if self.menu_open {
                Vec::new()
            } else {
                l.build_hud(&self.scene, &self.game, vw, vh)
            };
            // The scoreboard while Tab or Back is held.
            let held = (l.keyboard && self.keys.contains(&KeyCode::Tab))
                || l.pad
                    .and_then(|id| self.pads.state(id))
                    .is_some_and(|p| p.scores);
            if held && !self.menu_open {
                let mut hb = HudBuilder::new(vw, vh);
                let (font, white) = (self.scene.hud_font, self.scene.hud_white);
                menu::draw_scoreboard(&mut hb, font, white, vw, vh, &scores);
                hud.extend(hb.finish());
            }
            views.push(View {
                viewport,
                aspect,
                magnification: l.magnification(&self.scene, &self.game),
                camera: FlyCamera {
                    position: l.camera.position,
                    yaw: l.camera.yaw,
                    pitch: l.camera.pitch,
                },
                world,
                sprites: {
                    let mut sprites = self.effects.sprites(r, u);
                    sprites.extend(self.objective_sprites(r, u));
                    sprites
                },
                draws,
                hud,
                view_model_proj: l.view_model_proj(aspect),
            });
        }
        let mut frames: Vec<Frame> = views
            .iter()
            .map(|v| {
                let sky_view =
                    glam::camera::rh::view::look_to_mat4(Vec3::ZERO, v.camera.forward(), Vec3::Z);
                Frame {
                    viewport: v.viewport,
                    posed: &v.draws.posed,
                    sky: self.scene.sky.map(|mesh| DrawCall {
                        mesh,
                        model: Mat4::IDENTITY,
                        light: None,
                        colors: None,
                    }),
                    sky_proj: camera::projection(v.aspect, v.magnification, 1.0, 10000.0)
                        * sky_view,
                    view_proj: v.camera.view_proj(v.aspect, v.magnification),
                    camera: v.camera.position,
                    world: &v.world,
                    sprites: &v.sprites,
                    view_model_proj: v.view_model_proj,
                    view_models: &v.draws.view_models,
                    view_sprites: &v.draws.view_sprites,
                    hud: &v.hud,
                }
            })
            .collect();
        if !overlay.is_empty() {
            frames.push(Frame::overlay([0, 0, w as u32, h as u32], &overlay));
        }
        if let Some(g) = &mut self.gpu {
            g.render(&frames);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Halo 2 Rust")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        match pollster::block_on(gpu::Gpu::new(window.clone(), &self.scene)) {
            Ok(mut g) => {
                g.set_menu_textures(&self.map_pictures);
                self.gpu = Some(g);
            }
            Err(e) => {
                eprintln!("graphics init failed: {e}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.last_frame = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gpu {
                    g.resize(size.width, size.height);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = [position.x as f32, position.y as f32];
                if self.in_menu() && self.loading.is_none() {
                    self.menu_hover();
                }
            }
            WindowEvent::MouseInput { state, button, .. }
                if self.in_menu() || self.loading.is_some() =>
            {
                if state == ElementState::Pressed
                    && button == MouseButton::Left
                    && self.loading.is_none()
                {
                    self.keyboard_used = true;
                    self.menu_click();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.in_menu() => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                    MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
                };
                if self.loading.is_none() {
                    self.menu_wheel(up);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                let captured = self.captured;
                match button {
                    MouseButton::Left if down && !captured => self.set_capture(true),
                    MouseButton::Left => {
                        self.fire_held = down;
                        if let Some(l) = self.keyboard_local() {
                            l.taps.fire |= down;
                        }
                    }
                    MouseButton::Right => {
                        self.zoom_held = down && captured;
                        if let Some(l) = self.keyboard_local() {
                            l.taps.zoom |= down && captured;
                        }
                    }
                    MouseButton::Middle => {
                        if let Some(l) = self.keyboard_local() {
                            l.taps.throw_grenade |= down && captured;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { .. } if self.captured => {
                if let Some(l) = self.keyboard_local() {
                    l.taps.switch_weapon = true;
                }
            }
            WindowEvent::Focused(false) => {
                self.set_capture(false);
                self.keys.clear();
                self.fire_held = false;
                self.zoom_held = false;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                match event.state {
                    ElementState::Pressed => {
                        if !event.repeat {
                            self.key_pressed(code);
                        }
                        self.keys.insert(code);
                    }
                    ElementState::Released => {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;
                self.update(dt);
                self.render();
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if !self.captured {
                return;
            }
            let Some(k) = self.locals.iter().position(|l| l.keyboard) else {
                return;
            };
            let scale = 1.0 / self.locals[k].magnification(&self.scene, &self.game);
            self.locals[k]
                .camera
                .look(delta.0 as f32, delta.1 as f32, scale);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl App {
    fn key_pressed(&mut self, code: KeyCode) {
        self.keyboard_used = true;
        if self.loading.is_some() {
            return;
        }
        if self.mode == Mode::Menu && self.menu.screen == Screen::Lobby && code == KeyCode::KeyT {
            self.change_team(0);
            return;
        }
        if self.in_menu() {
            let input = match code {
                KeyCode::ArrowUp | KeyCode::KeyW => menu::Input::Up,
                KeyCode::ArrowDown | KeyCode::KeyS => menu::Input::Down,
                KeyCode::ArrowLeft | KeyCode::KeyA => menu::Input::Left,
                KeyCode::ArrowRight | KeyCode::KeyD => menu::Input::Right,
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space | KeyCode::KeyE => {
                    menu::Input::Select
                }
                KeyCode::Escape | KeyCode::Backspace => menu::Input::Back,
                _ => return,
            };
            self.menu_input(input);
            return;
        }
        if code == KeyCode::Escape {
            self.open_menu(Screen::Pause);
            return;
        }
        if code == KeyCode::KeyB {
            self.add_bot();
            return;
        }
        let joined = self.joined();
        let Some(k) = self.locals.iter().position(|l| l.keyboard) else {
            return;
        };
        let player = self.locals[k].player;
        let l = &mut self.locals[k];
        match code {
            KeyCode::Backquote => {
                l.flying = !l.flying;
                if !l.flying && !joined {
                    // Drop in where the fly camera is.
                    let p = &mut self.game.players[player];
                    p.body.position =
                        l.camera.position - Vec3::Z * p.body.biped.standing_camera_height;
                    p.body.velocity = Vec3::ZERO;
                    l.eyes = (p.eye(), p.eye());
                }
            }
            KeyCode::KeyQ => l.taps.switch_weapon = true,
            KeyCode::KeyF => l.taps.melee = true,
            KeyCode::KeyR => l.taps.reload = true,
            KeyCode::KeyG => l.taps.throw_grenade = true,
            KeyCode::KeyX => l.taps.switch_grenade = true,
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9
                if !joined =>
            {
                self.give_weapon(player, code as usize - KeyCode::Digit1 as usize);
            }
            _ => {}
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        // Keep the console open when launched by double-click.
        eprintln!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let path = find_map()?;
    println!("loading {}", path.display());
    let level = load_level(&path)?;
    let env = |name: &str| std::env::var(name).ok();
    // H2_POS="x y z yaw_degrees" starts somewhere else (for testing).
    let start_pos = env("H2_POS").and_then(|v| {
        let n: Vec<f32> = v
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        (n.len() == 4).then(|| PlayerSpawn {
            position: [n[0], n[1], n[2]],
            facing: n[3].to_radians(),
            team: 8,
            game_types: [12, 0, 0, 0],
        })
    });
    let mut maps = path.parent().map(menu::find_maps).unwrap_or_default();
    let map_pictures = path
        .parent()
        .map(|dir| mapinfo::describe_maps(dir, &mut maps))
        .unwrap_or_default();
    // Three computer opponents (H2_BOTS=<n> for another number).
    let bots = env("H2_BOTS")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
        .min(menu::MAX_BOTS);
    // H2_GAME=team, ctf, king, teamking, oddball, teamoddball, juggernaut,
    // territories or assault starts that game type (for testing).
    let game_type = env("H2_GAME")
        .and_then(|g| menu::GAME_TYPES.iter().position(|t| t.2 == g))
        .unwrap_or(0);
    let settings = Settings {
        game_type,
        map: 0,
        score: menu::scores(menu::GAME_TYPES[game_type].0).1,
        bots,
    };
    // H2_SIM=<seconds> plays bots against each other without a window and
    // prints what happens (for testing).
    if let Some(spec) = env("H2_DRIVE") {
        drive_test(&level, &spec);
        return Ok(());
    }
    if let Some(seconds) = env("H2_SIM").and_then(|v| v.parse().ok()) {
        simulate(&level, &settings, seconds);
        return Ok(());
    }
    // The menu music is read in the background.
    let (tx, music) = mpsc::channel();
    let music_map = path;
    std::thread::spawn(move || {
        let _ = tx.send(scene::load_music(&music_map));
    });

    let session = h2net::session_id();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        game: new_game(&level.scene, GameType::Slayer, 0),
        scene: level.scene,
        world: level.world,
        bots: Vec::new(),
        nav: level.nav,
        focus: (Vec3::ZERO, 1.0),
        locals: Vec::new(),
        seats: vec![flow::Seat::default()],
        pads: Pads::new(),
        window: None,
        gpu: None,
        keys: HashSet::new(),
        captured: false,
        fire_held: false,
        zoom_held: false,
        mouse: [0.0; 2],
        last_frame: Instant::now(),
        pending: 0.0,
        effects: Effects::new(),
        bodies: Vec::new(),
        body_actions: Vec::new(),
        body_poses: Vec::new(),
        net: Net::Offline,
        browser: h2net::Browser::new(session),
        lan_games: Vec::new(),
        session,
        map_path: level.path,
        map_name: String::new(),
        frame_events: Vec::new(),
        welcome: None,
        sound: soundscape::Soundscape::new(),
        mode: Mode::Menu,
        menu: Menu::new(settings),
        menu_open: false,
        maps,
        map_pictures,
        loading: None,
        menu_time: 0.0,
        game_over: None,
        music: None,
        music_loading: Some(music),
        music_voice: None,
        quit: false,
        keyboard_used: false,
        start_pos,
        start_weapon: env("H2_WEAPON").and_then(|v| v.parse().ok()),
    };
    app.level_changed();
    // H2_SPLIT=<n> starts with n people in splitscreen (for testing).
    let split = env("H2_SPLIT")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1);
    for _ in 1..split.min(MAX_LOCAL) {
        app.seats.push(flow::Seat::default());
    }
    // H2_JOIN=<address:port> joins a LAN game at once, and H2_PLAY=1 starts
    // a game at once (for testing).
    if let Some(address) = env("H2_JOIN") {
        app.join_address(&address);
    } else if env("H2_PLAY").is_some() {
        app.start_game();
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}
