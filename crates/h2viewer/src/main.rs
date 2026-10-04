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
//! pick up (hold to swap weapons), Q / mouse wheel switch weapon, V (left
//! bumper) the Arbiter's active camouflage, hold Tab for the scoreboard, B
//! add a bot, 1-9 take any weapon (testing), ` toggles
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
mod campaign;
mod effects;
mod emblem;
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
mod options;
mod probe;
mod profile;
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
use gpu::{hud_mode, DrawCall, Frame, Fx, HudBatch};
use h2sim::bot::{bot_look, bot_name};
use h2sim::game::{guest_name, Event, GrenadeKind, HeldWeapon, Powerup, TICK};
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
use scene::{BodyKind, EffectLook, Scene};
use std::collections::{HashMap, HashSet};
use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// Most people sharing one screen.
pub(crate) const MAX_LOCAL: usize = 4;
/// Seconds between someone winning and the carnage report.
const GAME_OVER_DELAY: f32 = 4.0;
/// Frames this far apart mean the window isn't being drawn (it's
/// minimized, say).
const NOT_DRAWING: Duration = Duration::from_millis(250);
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
    // H2_START_WEAPONS=needler,smg: what everyone spawns with (testing).
    let start = std::env::var("H2_START_WEAPONS").unwrap_or_default();
    let start: Vec<&str> = start.split(',').filter(|w| !w.is_empty()).collect();
    Rules {
        starting_weapons: if start.is_empty() {
            index(&["battle_rifle", "smg"])
        } else {
            index(&start)
        },
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
        overshield_time: scene.overshield_time.unwrap_or(defaults.overshield_time),
        camo_time: scene.camo_time.unwrap_or(defaults.camo_time),
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

/// A game on the scene's level under `options`, with no one in it yet.
fn new_game(
    scene: &Scene,
    game_type: GameType,
    score_to_win: u32,
    options: &options::GameOptions,
) -> Game {
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
        ..options.rules(scene, rules(scene))
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
    game.teleporters = objective::teleporters(scene);
    game.set_vehicles(scene.vehicles.defs.clone(), scene.vehicles.spawns.clone());
    game.apply_options(options.shared(scene));
    game
}

/// A campaign mission on the scene's level, with no one in it yet: the
/// players start with what the mission gives them.
fn campaign_game(scene: &Scene) -> Game {
    let mut game = new_game(
        scene,
        GameType::Campaign,
        0,
        &options::GameOptions::default(),
    );
    let start = scene.campaign.clone().unwrap_or_default();
    game.rules.starting_weapons = start.weapons;
    game.rules.starting_frags = start.frags;
    game.rules.starting_plasmas = start.plasmas;
    game.rules.friendly_fire = true;
    game.characters = scene.ai.characters.clone();
    // Squads' vehicles wait until their squads are placed.
    for &v in scene.vehicles.squad_vehicles.values() {
        game.remove_vehicle(v);
    }
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
    if !scene.ai.squads.is_empty() {
        println!(
            "{} squads, {} characters, {} actor bodies, {} scripts, {} doors, {} lifts, {} switches",
            scene.ai.squads.len(),
            scene.ai.characters.len(),
            scene.ai.bodies.len(),
            scene.ai.scripts.scripts.len(),
            scene.doors.len(),
            scene.lifts.len(),
            scene.switches.len()
        );
        let cinema = &scene.ai.cinema;
        println!(
            "cutscenes: {} animation graphs, {} cast ({} other looks), {} subtitles, {} effects",
            cinema.graphs.len(),
            cinema.bodies.len(),
            cinema.bodies.values().map(|b| b.looks.len()).sum::<usize>(),
            cinema.subtitles.len(),
            scene.ai.effects.len()
        );
    }
    // H2_LIST_WEAPONS=1: each weapon's crosshair range and HUD pieces.
    if std::env::var_os("H2_LIST_WEAPONS").is_some() {
        for w in &scene.weapons {
            let hud: Vec<&str> = w.hud.iter().map(|h| h.name.as_str()).collect();
            println!(
                "weapon {} autoaim {:.1} range {:.1} damage {:.0} over {:?} vs shield/body {}/{} blast {:?} flight sound {:?} hud {hud:?}",
                w.def.name,
                w.autoaim_range,
                w.def.range,
                w.def.damage,
                w.def.damage_range,
                w.def.armor.shield,
                w.def.armor.body,
                w.def.flight.and_then(|f| f.blast).map(|b| (b.damage, b.armor.shield)),
                w.round.flight
            );
        }
    }
    // H2_LIST_VEHICLES=1: where the map's vehicles are.
    if std::env::var_os("H2_LIST_VEHICLES").is_some() {
        for s in &scene.vehicles.spawns {
            let def = &scene.vehicles.defs[s.def];
            let kind = &scene.vehicles.kinds[s.def];
            println!(
                "vehicle {} at {:.2} facing {:.2} ({:?}, {} seats, radius {:.2}, engine {:?}, boost {:?}, horn {:?}, enter {:?}, exit {:?}, board {:?})",
                def.name,
                s.position,
                s.yaw,
                def.drive,
                def.seats.len(),
                def.radius,
                kind.engine,
                kind.boost,
                kind.horn,
                kind.enter_sounds,
                kind.exit_sounds,
                kind.board_sounds,
            );
        }
    }
    let mut world = World::new_grouped(
        &scene.collision.positions,
        &scene.collision.indices,
        &scene.collision_bsp,
    );
    let mut spots: Vec<Vec3> = level_spawns(&scene).iter().map(|s| s.0).collect();
    spots.extend(scene.items.iter().map(|i| i.position));
    spots.extend(objective::objective_points(&scene));
    let started = Instant::now();
    // Campaign levels come with the AI's own map of where it can walk.
    let nav = if scene.nav_mesh.edges.is_empty() {
        NavGraph::for_level(
            &world,
            &spots,
            &kill_zones(&scene),
            &objective::teleporters(&scene),
        )
    } else {
        NavGraph::from_sectors(&scene.nav_mesh.sectors, &scene.nav_mesh.edges)
    };
    // Doors block the way while shut (bots' routes go through them: they
    // open as bots come).
    for d in &scene.doors {
        let k = world.add_door(&d.triangles);
        world.set_door(k, !d.open);
    }
    // Lifts carry whoever stands on them (the mission moves them).
    for part in scene.lifts.iter().flat_map(|l| &l.parts) {
        if let Some(m) = part.mover {
            let k = world.add_mover(&part.triangles);
            debug_assert_eq!(k, m);
        }
    }
    println!(
        "bot routes: {} points, {} links, {} through teleporters ({:.1?})",
        nav.points.len(),
        nav.links.iter().map(Vec::len).sum::<usize>(),
        nav.hops.len(),
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
    let mut game = new_game(&level.scene, GameType::Slayer, 0, &Default::default());
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
                "seat {k} {:?} at {:.2} entry {:.2} (world {:.2}) r {:.2} eye {:.2} pivot {:.2?} pitch {:.2?} turret {:.2?} guns {:?}",
                s.role,
                s.position,
                s.entry,
                veh.to_world(def, s.entry),
                s.entry_radius,
                s.eye,
                s.pivot,
                s.pitch_range,
                s.turret,
                [s.weapon, s.alt_weapon].map(|w| w.and_then(|w| game.weapons.get(w)).map(|d| &d.name)),
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
    // H2_DRIVE_FIRE=1 (2 for the second trigger): pull the trigger now and
    // then, and say where the shots land.
    let firing = std::env::var("H2_DRIVE_FIRE").unwrap_or_default();
    for tick in 0..(seconds / TICK) as usize {
        cmd.movement = glam::vec2(right, forward);
        cmd.yaw = look;
        let pull = tick % 60 < 2;
        cmd.fire = firing == "1" && pull;
        cmd.melee = firing == "2" && pull;
        cmd.throw_grenade = firing == "2" && pull;
        game.events.clear();
        game.step(&level.world, &[cmd]);
        for e in &game.events {
            if matches!(e, Event::Shot { .. } | Event::Impact { .. }) {
                println!("{:5.2}s {e:?}", tick as f32 * TICK);
            }
        }
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

/// A PC joined to a simulated game in memory, as online: the game goes to
/// it as a host sends it, and must arrive as if sent whole (H2_SIM_NET).
struct SimNet {
    host: h2net::Host,
    client: h2net::Client,
    joined: Game,
    /// Another copy that takes the game whole each time. Vehicles turn a
    /// hair on the way (rotations arrive normalized), so the joined game
    /// is held to this rather than to the host's.
    reference: Game,
    /// Ticks between snapshots, and what happened since the last.
    every: usize,
    events: Vec<Event>,
    /// Snapshots sent, the bytes they'd be whole, and how many arrived
    /// wrong.
    snapshots: usize,
    whole: u64,
    wrong: usize,
}

/// The game as a snapshot carries it.
fn game_state(game: &Game) -> Vec<u8> {
    let mut w = h2sim::game::Writer::default();
    game.write_state(&mut w);
    w.0
}

impl SimNet {
    /// Join `game` (on copies of it as it started, `joined` and
    /// `reference`) with a player a bot here plays, sending the game `hz`
    /// times a second.
    fn join(
        game: &mut Game,
        [mut joined, reference]: [Game; 2],
        hz: usize,
        bots: &mut Vec<(usize, Bot)>,
    ) -> SimNet {
        // Paced in game time here, not real time.
        let mut host = h2net::Host::online("sim");
        host.set_rate(0);
        let (a, b) = h2net::Connection::pair();
        let who = h2net::Verified {
            account: 1,
            gamertag: "JOINED".into(),
            level: 1,
            team: h2net::ANY_TEAM,
        };
        host.add_connection(a, who);
        let me = ("JOINED", bot_look(game.players.len()));
        let mut client = h2net::Client::over(b, &joined, "sim", &[h2net::ANY_TEAM], me);
        while client.players.is_empty() || joined.players.len() < game.players.len() {
            host.poll(game, scene::MAX_BODIES);
            host.send(game, &[], false);
            let (status, _) = client.poll(&mut joined);
            if let Some(s) = status.first() {
                println!("net: {s:?}");
            }
        }
        let p = client.players[0];
        bots.push((p, Bot::new(p as u32 * 7919 + 13)));
        SimNet {
            host,
            client,
            joined,
            reference,
            every: (60 / hz.clamp(1, 60)).max(1),
            events: Vec::new(),
            snapshots: 0,
            whole: 0,
            wrong: 0,
        }
    }

    /// After a tick: send the game if it's time, and check what arrives.
    fn tick(&mut self, game: &mut Game, tick: usize) {
        self.events.extend_from_slice(&game.events);
        if !tick.is_multiple_of(self.every) {
            return;
        }
        let events = std::mem::take(&mut self.events);
        // Hearing how many have arrived, as a host does every frame.
        for e in self.host.poll(game, scene::MAX_BODIES) {
            println!("net: {e:?}");
        }
        self.host.send(game, &events, true);
        let (status, got) = self.client.poll(&mut self.joined);
        let mut w = h2sim::game::Writer::default();
        for e in &events {
            e.write(&mut w);
        }
        let state = game_state(game);
        self.snapshots += 1;
        self.whole += (4 + state.len() + 2 + w.0.len()) as u64;
        let whole = self
            .reference
            .read_state(&mut h2sim::game::Reader::new(&state));
        if !status.is_empty()
            || got != events
            || whole.is_err()
            || game_state(&self.joined) != game_state(&self.reference)
        {
            self.wrong += 1;
        }
    }

    fn report(&self) {
        let n = self.snapshots.max(1) as u64;
        let (whole, sent) = (self.whole / n, self.host.sent() / n);
        println!(
            "net: {} snapshots: {whole} bytes whole, {sent} sent ({}%), {} arrived wrong",
            self.snapshots,
            sent * 100 / whole.max(1),
            self.wrong
        );
    }
}

/// Bots only, no window: play `seconds` of a game and print the kills, flag
/// moves and score.
fn simulate(level: &Level, settings: &Settings, seconds: f32) {
    let campaign = level.scene.campaign.is_some();
    let mut game = if campaign {
        campaign_game(&level.scene)
    } else {
        new_game(
            &level.scene,
            settings.game_type(),
            settings.score_to_win(),
            &settings.options,
        )
    };
    // H2_SEED=<n> plays a different game.
    let seed: u32 = std::env::var("H2_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    game.reseed(seed);
    // A mission has one player (a bot here) against its squads
    // (H2_SQUADS=<names>, "all" for every one).
    let players = if campaign { 1 } else { settings.bots.max(2) };
    let mut bots: Vec<(usize, Bot)> = (0..players)
        .map(|_| {
            let i = game.add_player();
            game.set_name(i, bot_name(i));
            let mut look = bot_look(i);
            if campaign {
                look.elite = campaign::arbiter(&level.scene);
                game.players[i].team = campaign::players_team(&level.scene);
            }
            game.set_look(i, look);
            (i, Bot::new(i as u32 * 7919 + 13 + seed * 104_729))
        })
        .collect();
    // H2_SIM_NET=<snapshots a second>: and a PC joins in memory, its
    // player a bot here too, and gets the game as a host sends it.
    let mut net = std::env::var("H2_SIM_NET")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|_| !campaign)
        .map(|hz| {
            let copy = || {
                new_game(
                    &level.scene,
                    settings.game_type(),
                    settings.score_to_win(),
                    &settings.options,
                )
            };
            SimNet::join(&mut game, [copy(), copy()], hz, &mut bots)
        });
    // The mission's scripts place its squads as the player gets to them
    // (H2_SQUADS=<names> places those at the start too, "all" for every
    // one).
    let mut mission =
        campaign.then(|| campaign::Mission::new(&level.scene, &mut game, &mut bots, 1));
    if let (true, Ok(names)) = (campaign, std::env::var("H2_SQUADS")) {
        for s in campaign::squads_named(&level.scene, &names) {
            let team = campaign::players_team(&level.scene);
            let placed = campaign::place_squad(&mut game, &level.scene, s, 1, team, None, None);
            let name = &level.scene.ai.squads[s].name;
            println!("placed {name}: {} actors", placed.len());
            bots.extend(placed.into_iter().filter_map(|(i, b)| Some((i, b?))));
        }
    }
    // H2_SIM_POS="x y z": the player starts there and stands still (for
    // testing lifts and the like).
    let still = std::env::var("H2_SIM_POS").ok().and_then(|v| {
        let n: Vec<f32> = v
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        (n.len() == 3).then(|| Vec3::new(n[0], n[1], n[2]))
    });
    if let Some(at) = still {
        game.players[0].body.position = at;
    }
    // H2_SIM_PATH="seconds=x y z;...": and moves on to each place then
    // ("seconds=use" presses the action button there instead).
    let mut path: Vec<(f32, Option<Vec3>)> = std::env::var("H2_SIM_PATH")
        .unwrap_or_default()
        .split(';')
        .filter_map(|step| {
            let (t, at) = step.split_once('=')?;
            let t = t.trim().parse().ok()?;
            if at.trim() == "use" {
                return Some((t, None));
            }
            let n: Vec<f32> = at
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            Some((t, Some(Vec3::new(*n.first()?, *n.get(1)?, *n.get(2)?))))
        })
        .collect();
    path.reverse();
    let still = still.or(path.last().map(|_| Vec3::ZERO));
    // H2_SIM_FREE=<seconds>: the player's bot plays on its own from then.
    let free_at: Option<f32> = std::env::var("H2_SIM_FREE")
        .ok()
        .and_then(|v| v.parse().ok());
    // H2_SIM_GOD=1: the player can't be hurt (to test a whole level).
    let god = std::env::var_os("H2_SIM_GOD").is_some();
    // H2_SIM_SKIP=<seconds>: skip cutscenes from then on.
    let skip_at: Option<f32> = std::env::var("H2_SIM_SKIP")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut won = false;
    let mut kills_with = std::collections::HashMap::<String, u32>::new();
    for tick in 0..(seconds / TICK) as usize {
        let mut commands = vec![Command::default(); game.players.len()];
        let free = free_at.is_some_and(|at| tick as f32 * TICK >= at);
        for (i, bot) in &mut bots {
            if *i == 0 && still.is_some() && !free {
                continue;
            }
            commands[*i] = bot.think(&game, &level.world, &level.nav, *i);
        }
        let t = tick as f32 * TICK;
        let step = path.last().filter(|(when, _)| t >= *when).map(|s| s.1);
        if step == Some(None) {
            commands[0].action = true;
        }
        if god {
            let p = &mut game.players[0];
            (p.health, p.shield) = (p.full.health, p.full.shield);
        }
        game.step(&level.world, &commands);
        h2sim::bot::alert_actors(&mut bots, &game, &game.events);
        if let Some(net) = &mut net {
            net.tick(&mut game, tick);
        }
        match step {
            Some(Some(at)) => {
                if game.players[0].seat.is_some() {
                    game.exit(&level.world, 0);
                }
                game.players[0].body.position = at;
                game.players[0].body.velocity = Vec3::ZERO;
                println!("{t:6.1} player moves to {at}");
            }
            Some(None) => println!("{t:6.1} player presses action"),
            None => {}
        }
        if step.is_some() {
            path.pop();
        }
        if let Some(m) = &mut mission {
            if skip_at.is_some_and(|at| t >= at) && m.skip_cutscene() {
                println!("{t:6.1} cutscene skipped");
            }
            let (bsp, placed) = (m.bsp(), game.players.len());
            m.step(&level.scene, &level.world, &mut game, &mut bots);
            if m.bsp() != bsp {
                println!("{t:6.1} into bsp {}", m.bsp());
            }
            if game.players.len() != placed {
                println!("{t:6.1} {} actors in the level", game.players.len() - 1);
            }
            if m.won() && !won {
                won = true;
                println!("{t:6.1} mission complete");
            }
            for s in m.take_sounds() {
                if std::env::var_os("H2_SCRIPT_LOG").is_some() {
                    println!("{t:6.1} {s:?}");
                }
            }
        }
        for e in std::mem::take(&mut game.events) {
            match e {
                Event::Killed { killer, victim, .. } => {
                    let at = game.players[victim].body.position;
                    // What the killer had in hand (and in the left).
                    let gun = |h: Option<&HeldWeapon>| {
                        h.and_then(|h| game.weapons.get(h.weapon))
                            .map(|d| d.name.clone())
                    };
                    let with = killer.filter(|&k| k != victim).and_then(|k| {
                        let p = &game.players[k];
                        let right = gun(p.held())?;
                        Some(match gun(p.left.as_ref()) {
                            Some(left) => format!("{right}+{left}"),
                            None => right,
                        })
                    });
                    if let Some(w) = &with {
                        *kills_with.entry(w.clone()).or_insert(0) += 1;
                    }
                    let killer = killer.map_or("THE LEVEL", |k| game.name(k));
                    let victim = game.name(victim);
                    let with = with.map_or(String::new(), |w| format!(" with {w}"));
                    println!("{t:6.1} {killer} killed {victim}{with} at {at:.1}");
                }
                Event::Flag { team, player, what } => {
                    println!("{t:6.1} flag {team} {what:?} by {player:?}");
                }
                Event::Hill { .. } | Event::Territory { .. } | Event::Juggernaut { .. } => {
                    println!("{t:6.1} {e:?}");
                }
                Event::Entered { .. }
                | Event::Exited { .. }
                | Event::Hijacked { .. }
                | Event::Splattered { .. }
                | Event::VehicleDestroyed { .. }
                | Event::VehicleSpawned { .. } => println!("{t:6.1} {e:?}"),
                Event::PickedUp {
                    kind: ItemKind::Powerup(_) | ItemKind::Ammo { .. },
                    ..
                }
                | Event::Teleported { .. } => println!("{t:6.1} {e:?}"),
                _ => {}
            }
        }
        if game.over() {
            println!("{t:6.1} game over");
            break;
        }
        // H2_SIM_WHERE=<seconds>: where everyone is that often (1: every
        // 20 seconds).
        let every = std::env::var("H2_SIM_WHERE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .map(|s| if s == 1.0 { 20.0 } else { s });
        if every.is_some_and(|s| tick % ((s / TICK) as usize).max(1) == 0) {
            let at: Vec<String> = game
                .players
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let guns: Vec<&str> = p
                        .weapons
                        .iter()
                        .chain(&p.left)
                        .filter_map(|h| game.weapons.get(h.weapon))
                        .map(|d| d.name.as_str())
                        .collect();
                    let dead = if p.alive { "" } else { "x" };
                    let name = game.name(i);
                    format!(
                        "\n  {i} {name} {:.0?}{dead} {}",
                        p.body.position,
                        guns.join("/")
                    )
                })
                .collect();
            println!("{t:6.1} at{}", at.join(""));
            for (i, bot) in &bots {
                println!("   {i}: {}", bot.describe());
            }
            // H2_SIM_SIGHT=1: and what blocks each one's view of each enemy.
            if std::env::var_os("H2_SIM_SIGHT").is_some() {
                for (i, a) in game.players.iter().enumerate().filter(|(_, p)| p.alive) {
                    for (j, b) in game.players.iter().enumerate().filter(|(_, p)| p.alive) {
                        if !game.is_enemy(i, j) {
                            continue;
                        }
                        let (from, to) = (a.eye(), b.eye() - Vec3::Z * 0.1);
                        let d = to - from;
                        let hit = level
                            .world
                            .raycast_what(from, d.normalize_or_zero(), d.length());
                        println!("   sight {i}->{j} {:.1} m hit {hit}", d.length());
                    }
                }
            }
        }
    }
    let scores: Vec<i32> = game.players.iter().map(|p| p.score).collect();
    println!("scores {scores:?}");
    if let Some(net) = &net {
        net.report();
    }
    let mut kills_with: Vec<_> = kills_with.into_iter().collect();
    kills_with.sort_by_key(|(w, n)| (std::cmp::Reverse(*n), w.clone()));
    println!("kills by weapon {kills_with:?}");
}

/// How much of the view the letterbox bars cover, top and bottom each.
const LETTERBOX: f32 = 0.12;

/// What a mission's scripts put over a view: the HUD faded as they say,
/// letterbox bars, a chapter title and a fade to or from a colour.
fn mission_screen(
    hud: &mut Vec<HudBatch>,
    view: &campaign::ScreenView,
    font: usize,
    white: usize,
    w: f32,
    h: f32,
) {
    if view.hud < 0.05 {
        hud.clear();
    } else if view.hud < 0.999 {
        for v in hud.iter_mut().flat_map(|b| &mut b.vertices) {
            v.color[3] *= view.hud.max(0.0);
        }
    }
    let mut hb = HudBuilder::new(w, h);
    let s = hb.scale();
    let fill = |hb: &mut HudBuilder, rect: [f32; 4], color: [f32; 4]| {
        hb.quad(white, rect, [0.0; 4], color, hud_mode::PLAIN, 0.0);
    };
    if view.letterbox > 0.0 {
        let bar = h * LETTERBOX * view.letterbox.min(1.0);
        fill(&mut hb, [0.0, 0.0, w, bar], [0.0, 0.0, 0.0, 1.0]);
        fill(&mut hb, [0.0, h - bar, w, h], [0.0, 0.0, 0.0, 1.0]);
    }
    if let Some((text, _, shown)) = &view.title {
        // In the bottom bar.
        let size = 13.0 * s;
        let y = h * (1.0 - LETTERBOX * 0.5) - size * 0.5;
        hb.text(
            font,
            [w * 0.5, y],
            size,
            &text.to_uppercase(),
            [1.0, 1.0, 1.0, *shown],
        );
    }
    if let Some(text) = &view.subtitle {
        // In the bottom bar, a line at a time.
        let size = 9.0 * s;
        let lines = wrap(text, 72);
        let bottom = h * (1.0 - LETTERBOX * 0.5) + size * (lines.len() as f32 * 0.6 - 1.1);
        for (k, line) in lines.iter().rev().enumerate() {
            let y = bottom - k as f32 * size * 1.2;
            hb.text(font, [w * 0.5, y], size, line, [1.0, 1.0, 1.0, 1.0]);
        }
    }
    if view.fade[3] > 0.0 {
        fill(&mut hb, [0.0, 0.0, w, h], view.fade);
    }
    hud.extend(hb.finish());
}

/// Text broken into lines of at most `width` characters, at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.len() + 1 + word.len() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
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
    /// Play a campaign mission.
    Campaign,
    Join(h2net::LanGame),
    /// Join the game the host we're with started.
    Rejoin,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// The menus, over the map seen from a circling camera.
    Menu,
    Playing,
}

/// Bodies farther than this from the camera aren't posed or drawn.
const BODY_RANGE: f32 = 80.0;

/// A Spartan's body posed for this frame.
struct BodyPose {
    /// Which body, and the copy of its mesh posed for this player.
    mesh: usize,
    kind: BodyKind,
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
    /// Gestures scripts have actors play, by animation name.
    body_gestures: Vec<(usize, String)>,
    body_poses: Vec<Option<BodyPose>>,
    /// The player's own model beside the profile menu.
    preview: BodyAnimator,
    preview_pose: Option<BodyPose>,
    /// LAN play: hosting, or joined to another PC's game.
    net: Net,
    /// Hosting works here (false once it failed).
    can_host: bool,
    /// Seconds a hosted game has waited for PCs from the lobby
    /// (`lan::LAN_WAIT` once it no longer waits).
    lan_wait: f32,
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
    /// The campaign missions in the maps folder.
    missions: Vec<MapChoice>,
    /// Playing (or loading) a campaign mission rather than multiplayer.
    campaign: bool,
    /// The mission in play: its scripts and what they've done.
    mission: Option<campaign::Mission>,
    /// The maps' pictures, for the lobby.
    map_pictures: Vec<blam_cache::bitmap::Image>,
    /// The emblem atlases, uploaded with the window.
    emblem_art: Vec<blam_cache::bitmap::Image>,
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
        self.game.set_name(i, bot_name(i));
        self.game.set_look(i, bot_look(i));
        self.bots.push((i, Bot::new(i as u32 * 7919 + 13)));
        self.announce(&format!("{} JOINED", self.game.name(i)));
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
        let profile = &self.menu.profile;
        let guest = guest_name(&profile.name, self.locals.len());
        self.game.set_name(i, &guest);
        self.game.set_look(i, profile.look.guest(self.locals.len()));
        let mut l = LocalPlayer::new(i, &self.game);
        l.pad = pad;
        self.locals.push(l);
        self.announce(&format!("{guest} JOINED"));
    }

    fn pad_pressed(&mut self, id: GamepadId, press: PadPress) {
        if self.loading.is_some() {
            return;
        }
        if self.in_menu() {
            self.menu_pad(id, press);
            return;
        }
        // A, Start or the trigger skip a cutscene.
        let skip = matches!(press, PadPress::Claim | PadPress::Join | PadPress::Fire);
        if skip && self.mission.as_mut().is_some_and(|m| m.skip_cutscene()) {
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
                    PadPress::Vision => t.vision = true,
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
            if self.loading.is_none() {
                self.update_lobby_net();
            } else {
                // Other PCs keep hearing from us while a map loads; what
                // they say waits until it's loaded (a game the host
                // starts meanwhile may be on another map).
                self.keep_alive();
            }
            if self.menu.screen == Screen::Profile {
                self.animate_preview(dt);
            }
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
        // Badly damaged vehicles smoke, then burn.
        for v in &self.game.vehicles {
            let Some(def) = self.game.vehicle_defs.get(v.def) else {
                continue;
            };
            let left = v.health / def.health.max(1.0);
            if !v.destroyed && left < 0.5 && def.drive != h2sim::vehicle::Drive::Fixed {
                let top = v.center + v.up() * def.radius * 0.35;
                self.effects.smolder(top, left < 0.25, dt);
            }
        }
        for p in &self.game.projectiles {
            // Just out of the barrel it would fill the shooter's view.
            let near = self
                .locals
                .iter()
                .any(|l| l.camera.position.distance(p.position) < 0.5);
            if let Some(r) = self.scene.weapons.get(p.weapon).map(|w| w.round) {
                if !near {
                    self.effects.round(p.position, r.glow, r.size, r.fiery);
                }
            }
        }
        // Needles stuck in people glow until they pop.
        for n in &self.game.stuck {
            let (Some(q), Some(w)) = (
                self.game.players.get(n.victim),
                self.scene.weapons.get(n.weapon),
            ) else {
                continue;
            };
            let at = q.body.position + n.offset;
            // Not in your own view.
            let near = self
                .locals
                .iter()
                .any(|l| l.player == n.victim || l.camera.position.distance(at) < 0.5);
            if !near {
                self.effects
                    .round(at, w.round.glow, w.round.size * 0.7, false);
            }
        }
        let listeners = self.listeners();
        self.sound.update(&self.scene, &self.game, &listeners, dt);
    }

    /// Run the game here: fixed ticks with everyone's controls (people at
    /// this PC, bots, and players on PCs that joined).
    fn step_game(&mut self, dt: f32) {
        self.poll_host();
        if self.waiting_for_lan(dt) {
            return;
        }
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
            let controls = self.mission.as_ref().is_none_or(|m| m.input_enabled());
            for l in &self.locals {
                commands[l.player] = if self.menu_open || !controls {
                    // Standing still while the menu is up (or a cutscene
                    // has the controls).
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
            h2sim::bot::alert_actors(&mut self.bots, &self.game, &self.game.events);
            if let Some(m) = &mut self.mission {
                // H2_SKIP=1 skips every cutscene (for testing).
                if env_set("H2_SKIP") {
                    m.skip_cutscene();
                }
                m.step(&self.scene, &self.world, &mut self.game, &mut self.bots);
                for (player, yaw) in m.take_turns() {
                    for l in self.locals.iter_mut().filter(|l| l.player == player) {
                        l.camera.yaw = yaw;
                        l.camera.pitch = 0.0;
                    }
                }
                self.body_gestures.extend(m.take_gestures());
                for hint in m.take_hints() {
                    for l in &mut self.locals {
                        l.message(hint.clone());
                    }
                }
                let nav_points = m.nav_points(&self.scene, &self.game);
                for l in &mut self.locals {
                    l.nav_points.clone_from(&nav_points);
                }
                let effects = m.take_effects();
                let sounds = m.take_sounds();
                if !sounds.is_empty() {
                    let listeners = self.listeners();
                    for s in sounds {
                        self.sound.mission(&self.scene, s, &listeners);
                    }
                }
                for (look, at) in effects {
                    match look {
                        EffectLook::Explosion { plasma, scale } => {
                            let (core, edge) = effects::fireball(plasma);
                            self.effects.blast(at, core, edge, !plasma, scale);
                        }
                        EffectLook::Smoke => self.effects.smolder(at, false, 1.0),
                        EffectLook::Glow(c) => self.effects.blast(at, c, c, false, 0.4),
                    }
                }
            }
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
                Event::Impact {
                    weapon,
                    position,
                    normal,
                    hit_player,
                    exploded,
                } => {
                    let r = self
                        .scene
                        .weapons
                        .get(weapon)
                        .map(|w| w.round)
                        .unwrap_or_default();
                    let reach = self
                        .game
                        .weapons
                        .get(weapon)
                        .and_then(|w| w.flight?.blast)
                        .map_or(1.5, |b| b.radius.1);
                    if exploded {
                        let core = [1.0, 0.95, 0.8, 1.0];
                        let edge = if r.fiery {
                            [1.0, 0.45, 0.12, 0.9]
                        } else {
                            [r.glow[0], r.glow[1], r.glow[2], 0.9]
                        };
                        self.effects.blast(
                            position,
                            core,
                            edge,
                            r.fiery,
                            (reach / 1.5).clamp(0.4, 2.0),
                        );
                    } else if let Some(j) = hit_player {
                        let shielded = self.game.players[j].shield > 0.0;
                        self.effects.player_hit(position, shielded);
                    } else {
                        self.effects.splash(position, normal, r.glow);
                    }
                }
                Event::Killed { killer, victim, .. } => {
                    let betrayal =
                        killer.is_some_and(|k| k != victim && !self.game.is_enemy(k, victim));
                    println!(
                        "{}",
                        kill_message(&self.game, usize::MAX, killer, victim, betrayal)
                            .to_lowercase()
                    );
                    // The campaign has no kill messages.
                    let campaign = self.game.rules.game_type == GameType::Campaign;
                    for l in &mut self.locals {
                        if !campaign {
                            l.message(kill_message(&self.game, l.player, killer, victim, betrayal));
                        }
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
                Event::Teleported { player, .. } => {
                    // Straight there, without the view gliding across.
                    let eye = local::view_point(&self.game, player);
                    if let Some(l) = self.local_of(player) {
                        l.eyes = (eye, eye);
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
                        ItemKind::Powerup(Powerup::Overshield) => "OVERSHIELD".into(),
                        ItemKind::Powerup(Powerup::Camouflage) => "ACTIVE CAMOUFLAGE".into(),
                        ItemKind::Ammo { weapon, .. } => self
                            .scene
                            .weapons
                            .get(weapon)
                            .map(|a| format!("{} AMMO", display_name(&a.def.name)))
                            .unwrap_or_default(),
                    };
                    if let Some(l) = self.local_of(player) {
                        l.message(format!("PICKED UP {what}"));
                    }
                }
                Event::Entered {
                    player, vehicle, ..
                }
                | Event::Hijacked {
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

    /// Pose every Spartan's body (the closest, while each kind of body
    /// has copies of its mesh to go round). Each view leaves out its own
    /// player's body while it looks through their eyes.
    fn animate_bodies(&mut self, dt: f32) {
        let actions = std::mem::take(&mut self.body_actions);
        let gestures = std::mem::take(&mut self.body_gestures);
        let n = self.game.players.len();
        self.bodies.resize_with(n, BodyAnimator::default);
        self.body_poses.resize_with(n, || None);
        let eye = self
            .locals
            .first()
            .map_or(Vec3::ZERO, |l| l.camera.position);
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            let d = |i: usize| self.game.players[i].body.position.distance_squared(eye);
            d(a).total_cmp(&d(b))
        });
        let mut used: HashMap<BodyKind, usize> = HashMap::new();
        for i in order {
            let p = &self.game.players[i];
            // Out of sight anyway, or taken out of the level.
            if p.body.position.distance_squared(eye) > BODY_RANGE * BODY_RANGE
                || p.actor.is_some_and(|a| a.gone)
            {
                self.body_poses[i] = None;
                continue;
            }
            let kind = self.scene.body_kind(p);
            let Some(body) = self.scene.body_of_kind(kind) else {
                self.body_poses[i] = None;
                continue;
            };
            let copy = used.entry(kind).or_insert(0);
            let Some(&mesh) = body.meshes.get(*copy) else {
                self.body_poses[i] = None;
                continue;
            };
            *copy += 1;
            let rig = &body.rig;
            // Each kind of body has its own animation graph: start over
            // when a player's changes.
            if self.body_poses[i].as_ref().is_some_and(|b| b.kind != kind) {
                self.bodies[i] = BodyAnimator::default();
            }
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
                pitch: p.pitch,
            };
            // (A Sentinel's model is centred on its middle.)
            let origin = p.body.position + Vec3::Z * p.body.origin_height();
            let mut object = Mat4::from_translation(origin) * Mat4::from_rotation_z(p.yaw);
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
                    // Only a passenger with a gun aims their own.
                    pitch: if seat.role == SeatRole::Passenger {
                        p.pitch
                    } else {
                        0.0
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
            for (_, name) in gestures.iter().filter(|g| g.0 == i) {
                self.bodies[i].play(rig, name);
            }
            let pose = self.bodies[i].update(rig, &input, dt);
            let world = rig.world(&pose);
            self.body_poses[i] = Some(BodyPose {
                mesh,
                kind,
                vertices: rig.skin.pose(&rig.skin_matrices(&world)),
                object,
                weapons: [false, true].map(|left| rig.weapon_frame(&world, left)),
            });
        }
    }

    /// Pose the player's model for the profile menu, standing with a
    /// Battle Rifle.
    fn animate_preview(&mut self, dt: f32) {
        let look = self.menu.profile.look;
        let kind = if look.elite && self.scene.elite.is_some() {
            BodyKind::Elite
        } else {
            BodyKind::Spartan
        };
        let Some(body) = self.scene.body_of_kind(kind) else {
            self.preview_pose = None;
            return;
        };
        if self.preview_pose.as_ref().is_some_and(|p| p.kind != kind) {
            self.preview = BodyAnimator::default();
        }
        let rig = &body.rig;
        let input = BodyInput {
            velocity: glam::Vec2::ZERO,
            grounded: true,
            crouching: false,
            alive: true,
            style: ("rifle", "br"),
            seat: None,
            pitch: 0.0,
        };
        let pose = self.preview.update(rig, &input, dt);
        let world = rig.world(&pose);
        self.preview_pose = Some(BodyPose {
            mesh: body.preview,
            kind,
            vertices: rig.skin.pose(&rig.skin_matrices(&world)),
            object: Mat4::IDENTITY,
            weapons: [false, true].map(|left| rig.weapon_frame(&world, left)),
        });
    }

    /// The profile menu's model, standing to the right of the menu and
    /// slowly turning: its draws, and its posed mesh to upload.
    fn preview_draws(
        &self,
        camera: &FlyCamera,
        (w, h): (f32, f32),
    ) -> (Vec<DrawCall>, Option<(usize, Vec<scene::Vertex>)>) {
        let Some(pose) = self
            .preview_pose
            .as_ref()
            .filter(|_| self.mode == Mode::Menu && self.menu.screen == Screen::Profile)
        else {
            return (Vec::new(), None);
        };
        // Where on screen, in the menus' 640x480 layout.
        const SPOT: [f32; 2] = [470.0, 290.0];
        const DISTANCE: f32 = 1.35;
        const MIDDLE: f32 = 0.37;
        let s = (h / 480.0).min(w / 640.0);
        let px = (w - 640.0 * s) * 0.5 + SPOT[0] * s;
        let py = (h - 480.0 * s) * 0.5 + SPOT[1] * s;
        let aspect = w / h.max(1.0);
        let half = camera::half_height(aspect) * DISTANCE;
        let x = (px / (w * 0.5) - 1.0) * half * aspect;
        let y = (1.0 - py / (h * 0.5)) * half;
        let (f, r, u) = camera.basis();
        let feet = camera.position + f * DISTANCE + r * x + u * (y - MIDDLE);
        let turn = 0.6 + self.menu_time * 0.5;
        let toward = -f;
        let facing = toward * turn.cos() + u.cross(toward) * turn.sin();
        let object = Mat4::from_cols(
            facing.extend(0.0),
            u.cross(facing).extend(0.0),
            u.extend(0.0),
            feet.extend(1.0),
        );
        let light = Some([0.9, 0.9, 0.95]);
        let mut draws = vec![DrawCall {
            mesh: pose.mesh,
            model: object,
            light,
            colors: Some(local::armor_colors(self.menu.profile.look)),
            emblem: Some(self.menu.profile.look.emblem),
            fx: Fx::default(),
        }];
        let rifle = self
            .scene
            .weapons
            .iter()
            .find(|w| w.def.name == "battle_rifle")
            .and_then(|w| w.world_mesh);
        if let Some(mesh) = rifle {
            draws.push(DrawCall {
                mesh,
                model: object * pose.weapons[0],
                light,
                colors: None,
                emblem: None,
                fx: Fx::default(),
            });
        }
        (draws, Some((pose.mesh, pose.vertices.clone())))
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
            emblem: None,
            fx: Fx::default(),
        }];
        for (k, o) in scene.objects.iter().enumerate() {
            let shown = match &self.mission {
                Some(m) => m.shows(scene, k),
                None => o.automatic,
            };
            if !shown {
                continue;
            }
            let carried = self
                .mission
                .as_ref()
                .map_or(Mat4::IDENTITY, |m| m.carried(scene, k));
            // Scenery a cutscene flies about (In Amber Clad).
            let animated = self
                .mission
                .as_ref()
                .zip(o.name)
                .and_then(|(m, n)| m.cutscene_object(scene, n));
            world.push(DrawCall {
                mesh: o.mesh,
                model: animated.unwrap_or(carried * o.transform),
                light: o.light,
                colors: None,
                emblem: None,
                fx: Fx::default(),
            });
        }
        for (mesh, model, light) in self.mission.iter().flat_map(|m| m.lift_draws(scene)) {
            world.push(DrawCall {
                mesh,
                model,
                light,
                colors: None,
                emblem: None,
                fx: Fx::default(),
            });
        }
        let spots = self.game.item_spawns.iter().zip(&self.game.item_timers);
        for (item, (spot, timer)) in scene.items.iter().zip(spots) {
            // The game options may put another weapon on the spot.
            let mesh = match spot.kind {
                ItemKind::Weapon(w) if spot.kind != item.kind => {
                    scene.weapons.get(w).and_then(|w| w.world_mesh)
                }
                _ => item.mesh,
            };
            if let (Some(mesh), true) = (mesh, *timer <= 0.0) {
                world.push(DrawCall {
                    mesh,
                    model: item.transform,
                    light: item.light,
                    colors: None,
                    emblem: None,
                    fx: Fx::default(),
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
                    emblem: None,
                    fx: Fx::default(),
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
                    emblem: None,
                    fx: Fx::default(),
                });
            }
        }
        // Rounds with a model of their own (rockets), nose first.
        for p in &self.game.projectiles {
            let Some(mesh) = scene.weapons.get(p.weapon).and_then(|w| w.round.mesh) else {
                continue;
            };
            let turn = glam::Quat::from_rotation_arc(Vec3::X, p.velocity.normalize_or(Vec3::X));
            world.push(DrawCall {
                mesh,
                model: Mat4::from_rotation_translation(turn, p.position),
                light: light_at(p.position),
                colors: None,
                emblem: None,
                fx: Fx::default(),
            });
        }
        world.extend(self.flag_draws());
        world
    }

    /// A player's body and the weapon in their hands.
    fn body_draws(&self, player: usize) -> Vec<DrawCall> {
        let Some(Some(pose)) = self.body_poses.get(player) else {
            return Vec::new();
        };
        if self.mission.as_ref().is_some_and(|m| m.hides(player)) {
            return Vec::new();
        }
        let p = &self.game.players[player];
        let light = self
            .scene
            .level_light
            .at(&self.scene.textures, p.body.position + Vec3::Z * 0.2);
        // Actors wear their rank's colours.
        let colors = p
            .actor
            .and_then(|a| self.scene.ai.colors.get(a.character).copied())
            .unwrap_or_else(|| player_colors(&self.game, player));
        let mut out = vec![DrawCall {
            mesh: pose.mesh,
            model: pose.object,
            light,
            colors: Some(colors),
            emblem: p.actor.is_none().then_some(p.look.emblem),
            fx: local::player_fx(&self.game, player),
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
                    emblem: None,
                    // Camouflage hides the gun too; the glow is the body's.
                    fx: Fx {
                        overshield: 0.0,
                        ..out[0].fx
                    },
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
        for pose in self.body_poses.iter().flatten() {
            body_meshes.push((pose.mesh, pose.vertices.clone()));
        }
        // A cutscene's cast, and its camera while it has the view.
        let mut cutscene_camera = None;
        if let Some(m) = &self.mission {
            let player = self.locals.first().map_or(0, |l| l.player);
            for (mesh, vertices, model, body) in m.cutscene_bodies(&self.scene) {
                body_meshes.push((mesh, vertices));
                let colors = match body.chief {
                    true => Some(player_colors(&self.game, player)),
                    false => body.colors,
                };
                shared.push(DrawCall {
                    mesh,
                    model,
                    light: None,
                    colors,
                    emblem: None,
                    fx: Fx::default(),
                });
            }
            cutscene_camera = m.cutscene_camera(&self.scene);
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
            let mut world = shared.clone();
            let (preview, posed) = self.preview_draws(&camera, (w, h));
            world.extend(preview);
            body_meshes.extend(posed);
            views.push(View {
                viewport,
                aspect: viewport[2] as f32 / viewport[3].max(1) as f32,
                magnification: 1.0,
                camera,
                world,
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
            let own = l
                .first_person(&self.game)
                .then_some(l.player)
                .filter(|_| cutscene_camera.is_none());
            for i in 0..self.body_poses.len() {
                if Some(i) != own {
                    world.extend(self.body_draws(i));
                }
            }
            // A cutscene sees through its own camera, with no HUD or
            // weapon in view.
            let mut camera = match cutscene_camera {
                Some((at, forward, _)) => FlyCamera::looking_at(at, at + forward),
                None => FlyCamera {
                    position: l.camera.position,
                    yaw: l.camera.yaw,
                    pitch: l.camera.pitch,
                },
            };
            if let Some((yaw, pitch)) = self.mission.as_ref().map(|m| m.shake()) {
                camera.yaw += yaw;
                camera.pitch += pitch;
            }
            let (_, r, u) = camera.basis();
            let mut draws = if cutscene_camera.is_some() {
                local::ViewDraws {
                    view_models: Vec::new(),
                    view_sprites: Vec::new(),
                    posed: Vec::new(),
                }
            } else {
                l.view_draws(&self.scene, &self.game)
            };
            if k == 0 {
                draws.posed.splice(0..0, std::mem::take(&mut body_meshes));
            }
            let (vw, vh) = (viewport[2] as f32, viewport[3] as f32);
            // The menu replaces the HUD while it's up.
            let mut hud = if self.menu_open || cutscene_camera.is_some() {
                Vec::new()
            } else {
                l.build_hud(&self.scene, &self.game, vw, vh)
            };
            if let Some(m) = self.mission.as_ref().filter(|_| !self.menu_open) {
                mission_screen(
                    &mut hud,
                    &m.screen(&self.scene),
                    self.scene.hud_font,
                    self.scene.hud_white,
                    vw,
                    vh,
                );
            }
            let prompt = self
                .mission
                .as_ref()
                .and_then(|m| m.switch_prompt(&self.scene, &self.game, l.player, l.keyboard));
            if let Some(text) = prompt.filter(|_| !self.menu_open) {
                let mut hb = HudBuilder::new(vw, vh);
                let s = hb.scale();
                let at = [vw * 0.5, vh * 0.5 + 76.0 * s];
                hb.text(self.scene.hud_font, at, 9.0 * s, &text, hud::BLUE);
                hud.extend(hb.finish());
            }
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
            let magnification = match cutscene_camera {
                Some((_, _, fov)) => {
                    let half_x = (camera::half_height(aspect) * aspect).atan();
                    half_x.tan() / (fov.to_radians() * 0.5).tan().max(1e-3)
                }
                None => l.magnification(&self.scene, &self.game),
            };
            views.push(View {
                viewport,
                aspect,
                magnification,
                camera,
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
                        emblem: None,
                        fx: Fx::default(),
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
                g.set_emblem_textures(&self.emblem_art);
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
                if self.menu.editing && self.in_menu() && event.state == ElementState::Pressed {
                    self.type_key(code, event.text.as_deref());
                    return;
                }
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
        // No frames, no game; but the PCs we play with still hear from us.
        if self.last_frame.elapsed() > NOT_DRAWING {
            self.keep_alive();
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl App {
    /// A key pressed while typing a gamertag.
    fn type_key(&mut self, code: KeyCode, text: Option<&str>) {
        let typed = match code {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape => menu::Typed::Done,
            KeyCode::Backspace => menu::Typed::Erase,
            _ => match text {
                Some(t) => menu::Typed::Text(t.to_string()),
                None => return,
            },
        };
        let action = self.menu.typed(typed);
        self.after_menu(action);
    }

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
        // Space, Enter or E skip a cutscene.
        let skip = matches!(
            code,
            KeyCode::Space | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::KeyE
        );
        if skip && self.mission.as_mut().is_some_and(|m| m.skip_cutscene()) {
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
            KeyCode::KeyV => l.taps.vision = true,
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
            // Unknown: the mission works it out.
            bsp: u16::MAX,
            game_types: [12, 0, 0, 0],
            campaign_player: 0,
        })
    });
    let mut maps = path.parent().map(menu::find_maps).unwrap_or_default();
    let missions = path.parent().map(menu::find_missions).unwrap_or_default();
    let map_pictures = path
        .parent()
        .map(|dir| mapinfo::describe_maps(dir, &mut maps))
        .unwrap_or_default();
    let emblem_art = path.parent().and_then(emblem::load).unwrap_or_default();
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
    // H2_VARIANT=swat, rockets, snipers, swords or shotguns: those game
    // options (for testing).
    let mut options = env("H2_VARIANT")
        .and_then(|v| {
            let all = options::presets();
            all.into_iter().find(|p| p.0.eq_ignore_ascii_case(&v))
        })
        .map(|p| p.1)
        .unwrap_or_default();
    // H2_TIME_LIMIT=<seconds>: games end after that long (for testing).
    if let Some(t) = env("H2_TIME_LIMIT").and_then(|v| v.parse().ok()) {
        options.time_limit = t;
    }
    let settings = Settings {
        game_type,
        map: 0,
        score: menu::scores(menu::GAME_TYPES[game_type].0).1,
        bots,
        options,
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
        game: new_game(&level.scene, GameType::Slayer, 0, &Default::default()),
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
        body_gestures: Vec::new(),
        body_poses: Vec::new(),
        preview: BodyAnimator::default(),
        preview_pose: None,
        net: Net::Offline,
        can_host: true,
        lan_wait: lan::LAN_WAIT,
        browser: h2net::Browser::new(session),
        lan_games: Vec::new(),
        session,
        map_path: level.path,
        map_name: String::new(),
        frame_events: Vec::new(),
        welcome: None,
        sound: soundscape::Soundscape::new(),
        mode: Mode::Menu,
        menu: Menu::new(settings, profile::Profile::load()),
        menu_open: false,
        maps,
        missions,
        campaign: false,
        mission: None,
        map_pictures,
        emblem_art,
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
    } else if env("H2_PLAY").is_some() && menu::is_mission(&app.map_path) {
        app.campaign = true;
        app.start_campaign();
    } else if env("H2_PLAY").is_some() {
        app.start_game();
    }
    // H2_CAM="x y z yaw pitch" (degrees): look from there with a free
    // camera (for testing).
    let cam: Vec<f32> = env("H2_CAM")
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    if let (Some(l), &[x, y, z, yaw, pitch]) = (app.locals.first_mut(), &cam[..]) {
        l.flying = true;
        l.camera.position = Vec3::new(x, y, z);
        l.camera.yaw = yaw.to_radians();
        l.camera.pitch = pitch.to_radians();
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}
