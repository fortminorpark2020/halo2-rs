//! Campaign missions in play: the mission's scripts running, placing the
//! scenario's squads as actors when the player gets to them, opening doors
//! and following the player from one part of the level to the next.

use crate::scene::{EffectLook, Scene};
use blam_cache::ai::{AiTeam, SeatType};
use blam_cache::scenario::ControlKind;
use blam_cache::script::value_type;
use glam::{Mat4, Vec3};
use h2sim::bot::ActorMind;
use h2sim::game::{ActorSpawn, Side};
use h2sim::script::{Host, Obj, Value, Vm};
use h2sim::{Bot, Game, World};
use std::collections::{HashMap, HashSet};

mod cinematic;
mod commands;
mod cutscene;
mod dialogue;
mod orders;
mod vehicles;

/// Seconds a machine takes to open when its tag doesn't say.
const MACHINE_TIME: f32 = 1.5;
/// How far an actor told to see someone knows where they are.
const SEE_RANGE: f32 = 60.0;
/// A door opens for anyone within this distance of its middle.
const DOOR_REACH: f32 = 3.0;
/// The middle of a door, above where it's placed.
const DOOR_MIDDLE: f32 = 1.0;
/// Standing this close above a lift, it carries you.
const CARRY_REACH: f32 = 0.15;
/// A switch is in reach this close to a player's middle.
const SWITCH_REACH: f32 = 1.2;
/// A player's middle, above their feet.
const MIDDLE: f32 = 0.4;
/// How far under the players the floor of a structure BSP they move
/// into may be (they may be in a vehicle), and under actors it keeps.
const HUMAN_FLOOR: f32 = 6.0;
const ACTOR_FLOOR: f32 = 4.0;
/// How far up a flier may be over the floor of its BSP.
const FLIER_FLOOR: f32 = 15.0;
/// Floors of two BSPs this close in height are the same floor.
const SAME_FLOOR: f32 = 0.3;
/// A switch with no device group calls the nearest lift this close.
const CALL_REACH: f32 = 10.0;

/// The team of a squad that says which side it's on ("player" being the
/// players' own).
fn squad_team(team: AiTeam, players_team: u8) -> Option<u8> {
    match team {
        AiTeam::Player => Some(players_team),
        AiTeam::Human => Some(0),
        AiTeam::Covenant | AiTeam::Prophet => Some(1),
        AiTeam::Flood => Some(2),
        AiTeam::Sentinel => Some(3),
        AiTeam::Heretic => Some(4),
        AiTeam::Default | AiTeam::Other(_) => None,
    }
}

/// Place a squad's actors at its starting locations (as many as the
/// difficulty calls for, or just the one at location `only`); returns
/// them and their bots (none for a braindead squad). Allies are those on
/// `players_team`.
pub fn place_squad(
    game: &mut Game,
    scene: &Scene,
    squad: usize,
    difficulty: u8,
    players_team: u8,
    limit: Option<usize>,
    only: Option<usize>,
) -> Vec<(usize, Option<Bot>)> {
    let ai = &scene.ai;
    let Some(s) = ai.squads.get(squad) else {
        return Vec::new();
    };
    let n = s.locations.len();
    let (normal, legendary) = (s.counts.0 as usize, s.counts.1 as usize);
    let count = match difficulty {
        0 | 1 => normal,
        2 => (normal + legendary).div_ceil(2),
        _ => legendary,
    };
    // No count: one at each starting location.
    let count = if count == 0 { n } else { count.min(n) };
    let count = limit.map_or(count, |l| count.min(l));
    let always = s.locations.iter().filter(|l| l.always).count();
    let mut order: Vec<usize> = (0..n).filter(|&k| s.locations[k].always).collect();
    order.extend((0..n).filter(|&k| !s.locations[k].always));
    if let Some(k) = only {
        order = vec![k].into_iter().filter(|&k| k < n).collect();
    }
    let weapon = |i: Option<u16>| i.and_then(|i| ai.weapons.get(i as usize).copied().flatten());
    let mut out = Vec::new();
    // The vehicles placed for the squad, for others to join as gunners
    // or passengers.
    let mut vehicles: Vec<usize> = Vec::new();
    let count = if only.is_some() {
        1
    } else if limit.is_some() {
        count
    } else {
        count.max(always)
    };
    for &k in order.iter().take(count) {
        let l = &s.locations[k];
        // Its vehicle: one the squad has placed, with a seat to take; else
        // its own (left empty, with no actor at all, for "no driver").
        let joins = matches!(l.seat, SeatType::Gunner | SeatType::Passenger)
            .then(|| {
                vehicles
                    .iter()
                    .find_map(|&v| Some((v, vehicles::starting_seat(game, v, l.seat)?)))
            })
            .flatten();
        let own = match joins {
            Some(_) => None,
            None => scene
                .vehicles
                .squad_vehicles
                .get(&(squad as u16, k as u16))
                .copied(),
        };
        if let Some(v) = own {
            game.place_vehicle(v, Vec3::from(l.position) + Vec3::Z * 0.05, l.facing);
            vehicles.push(v);
        }
        if l.seat == SeatType::NoDriver {
            continue;
        }
        let seat = joins
            .or_else(|| own.and_then(|v| Some((v, vehicles::starting_seat(game, v, l.seat)?))));
        let Some(character) = l
            .character
            .or(s.character)
            .map(usize::from)
            .filter(|&c| c < ai.characters.len())
        else {
            continue;
        };
        let spawn = ActorSpawn {
            character,
            squad: squad as u16,
            location: k as u16,
            position: l.position.into(),
            yaw: l.facing,
            weapon: weapon(l.weapon.or(s.weapon)),
            secondary: weapon(l.secondary.or(s.secondary)),
            difficulty,
            team: squad_team(s.team, players_team),
        };
        let Some(i) = game.spawn_actor(spawn) else {
            continue;
        };
        if let Some((v, seat)) = seat {
            game.enter_vehicle(i, v, seat);
        }
        if s.braindead {
            out.push((i, None));
            continue;
        }
        let mut mind = ai.characters[character].mind;
        if difficulty >= 3 {
            mind.accuracy = ai.legendary_accuracy[character];
        }
        if s.blind {
            mind.sight = 0.0;
        }
        let ally = game.players[i].team == players_team;
        let post = spawn.position;
        let mind = ActorMind::new(mind, post, l.facing, squad as u16, ally);
        let seed = (i as u32).wrapping_mul(7919) ^ (squad as u32).wrapping_mul(104_729);
        out.push((i, Some(Bot::actor(seed, mind))));
    }
    out
}

/// Whether the mission's players are the Arbiter (on the Covenant's side)
/// rather than the Master Chief.
pub fn arbiter(scene: &Scene) -> bool {
    scene.elite.is_some() && scene.spawns.first().is_some_and(|s| s.campaign_player == 1)
}

/// The team a mission's players are on.
pub fn players_team(scene: &Scene) -> u8 {
    match arbiter(scene) {
        true => Side::Covenant,
        false => Side::Human,
    }
    .team()
}

/// The squads whose names start with any of `names` (comma separated;
/// "all" for every squad).
pub fn squads_named(scene: &Scene, names: &str) -> Vec<usize> {
    let names: Vec<&str> = names.split(',').map(str::trim).collect();
    (0..scene.ai.squads.len())
        .filter(|&k| {
            let n = &scene.ai.squads[k].name;
            names.iter().any(|p| *p == "all" || n.starts_with(p))
        })
        .collect()
}

/// A door or lift the scripts move: where it is (0 closed ... 1 open),
/// where it's headed, and its power.
#[derive(Debug, Clone, Copy)]
struct Device {
    position: f32,
    target: f32,
    power: f32,
    /// Opens by itself for someone near.
    automatic: bool,
    /// Opens by itself only for those behind it.
    one_sided: bool,
    /// Seconds an open door waits before it shuts again.
    wait: f32,
}

impl Default for Device {
    fn default() -> Device {
        Device {
            position: 0.0,
            target: 0.0,
            power: 1.0,
            automatic: false,
            one_sided: false,
            wait: 0.0,
        }
    }
}

/// A sound a script plays or stops (dialogue, music).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MissionSound {
    /// A line (in `Scene::sounds`), from where someone stands or over the
    /// radio.
    Line {
        sound: usize,
        at: Option<Vec3>,
        gain: f32,
    },
    StopLine(usize),
    /// Stop every line (a cutscene skipped).
    Hush,
    /// Music or another looping sound, by tag (`CampaignAi::loops`).
    StartLoop(u32),
    StopLoop(u32),
}

/// A value easing from one level to another over a while.
#[derive(Debug, Clone, Copy, Default)]
struct Ramp {
    from: f32,
    to: f32,
    start: f32,
    length: f32,
}

impl Ramp {
    fn level(level: f32) -> Ramp {
        Ramp {
            from: level,
            to: level,
            ..Ramp::default()
        }
    }

    fn at(&self, now: f32) -> f32 {
        if self.length <= 0.0 {
            return self.to;
        }
        let t = ((now - self.start) / self.length).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * t
    }

    /// Head for `to` over `length` seconds from where it is now.
    fn go(&mut self, now: f32, to: f32, length: f32) {
        *self = Ramp {
            from: self.at(now),
            to,
            start: now,
            length,
        };
    }
}

/// What the scripts put over the view: a fade, letterbox bars, a title,
/// and how much of the HUD shows.
#[derive(Debug, Clone, Copy, Default)]
struct Screen {
    fade: Ramp,
    fade_color: [f32; 3],
    letterbox: Ramp,
    hud: Ramp,
    /// A title (in `CampaignAi::titles`) and when it went up.
    title: Option<(usize, f32)>,
    /// The view shaking: how hard, and the most it turns (yaw, pitch,
    /// roll, degrees) at full strength.
    shake: Ramp,
    shake_rotation: [f32; 3],
}

/// The screen as it looks now: a colour over everything, how far the
/// letterbox bars are in (0-1), how much the HUD shows (0-1), and a title
/// with how much it shows.
#[derive(Debug, Clone, Default)]
pub struct ScreenView {
    pub fade: [f32; 4],
    pub letterbox: f32,
    pub hud: f32,
    pub title: Option<(String, [f32; 4], f32)>,
    /// A cutscene's subtitle.
    pub subtitle: Option<String>,
}

/// Most script ticks a skipped cutscene runs on in one game tick.
const SKIP_TICKS: usize = 30 * 600;

/// Seconds the letterbox bars take to come in or go.
const LETTERBOX_TIME: f32 = 0.5;

/// The level as the mission's scripts have left it.
#[derive(Default)]
struct State {
    /// Seconds since the mission started.
    time: f32,
    /// The players' side: the humans', or the Covenant's in the Arbiter's
    /// missions.
    players_team: u8,
    screen: Screen,
    /// Gravity as a share of normal.
    gravity: f32,
    /// How many of the mission's objectives are shown, and done.
    objectives: (usize, usize),
    /// Sounds to play, for whoever's listening.
    sounds: Vec<MissionSound>,
    /// Effects the scripts set off, and where.
    effects: Vec<(EffectLook, Vec3)>,
    /// Tips for the players ("press V for active camouflage").
    hints: Vec<String>,
    /// Waypoints the scripts show the players: over an object or a
    /// cutscene flag, so high above it.
    nav_points: Vec<(NavPoint, f32)>,
    /// The structure BSP the players are in.
    bsp: u16,
    /// The BSP from before a cutscene, to go back to if the players are
    /// still there rather than in the one it ended in.
    bsp_after_cutscene: Option<u16>,
    devices: HashMap<u16, Device>,
    /// The level's doors (`Scene::doors`), and which name each has.
    doors: Vec<Device>,
    door_named: HashMap<u16, usize>,
    device_groups: HashMap<u16, f32>,
    /// Device groups that change only once and have.
    spent: HashSet<u16>,
    /// The level's lifts (`Scene::lifts`), which name each has, and how
    /// each one's nodes have moved (`Lift::poses`) for which position.
    lifts: Vec<Device>,
    lift_named: HashMap<u16, usize>,
    lift_poses: Vec<(f32, Vec<Mat4>)>,
    /// The lift and node each of the world's movers is.
    mover_lift: HashMap<usize, (usize, usize)>,
    /// The switches (`Scene::switches`) and which name each has.
    switches: Vec<Device>,
    switch_named: HashMap<u16, usize>,
    /// Objects scripts attached to others: the child's name to the
    /// parent's.
    attached: HashMap<u16, u16>,
    /// Whether each player held the action button the tick before.
    held: Vec<bool>,
    /// Named objects scripts created (true) or destroyed (false).
    created: HashMap<u16, bool>,
    /// Actors the scripts placed in each squad.
    placed: HashMap<usize, u32>,
    /// Each squad's orders.
    orders: orders::Orders,
    /// Actors under command scripts.
    commands: commands::Commands,
    /// Actors getting in and out of vehicles.
    rides: vehicles::Rides,
    /// Scenes actors are playing out.
    scenes: dialogue::Scenes,
    /// The scripts have taken the players' controls away.
    input_off: bool,
    /// Players the scripts turned to face a new way (teleporting them),
    /// for their views to follow.
    turns: Vec<(usize, f32)>,
    /// Objects the scripts hid.
    hidden: HashSet<Obj>,
    /// The cutscene playing.
    cutscene: cutscene::Cutscene,
    difficulty: u8,
    won: bool,
    /// Say what the scripts do (H2_SCRIPT_LOG).
    log: bool,
}

/// A campaign mission in play: its scripts and what they've done.
pub struct Mission {
    vm: Vm,
    state: State,
    /// Game ticks so far (scripts run on every other one).
    ticks: u64,
}

impl Mission {
    /// Start a mission: squads the level starts with are placed, and its
    /// scripts are ready to run.
    pub fn new(
        scene: &Scene,
        game: &mut Game,
        bots: &mut Vec<(usize, Bot)>,
        difficulty: u8,
    ) -> Mission {
        let log = std::env::var_os("H2_SCRIPT_LOG").is_some();
        let mut state = State {
            screen: Screen {
                hud: Ramp::level(1.0),
                ..Screen::default()
            },
            gravity: 1.0,
            bsp: start_bsp(scene, game),
            players_team: game
                .players
                .iter()
                .find(|p| p.actor.is_none())
                .map_or(0, |p| p.team),
            difficulty,
            log,
            doors: scene
                .doors
                .iter()
                .map(|d| {
                    let at = if d.open { 1.0 } else { 0.0 };
                    Device {
                        position: at,
                        target: at,
                        power: if d.powered { 1.0 } else { 0.0 },
                        automatic: d.automatic,
                        one_sided: d.one_sided,
                        ..Device::default()
                    }
                })
                .collect(),
            door_named: (0..scene.doors.len())
                .filter_map(|k| Some((scene.doors[k].name?, k)))
                .collect(),
            device_groups: (0..scene.ai.device_groups.len())
                .map(|g| (g as u16, scene.ai.device_groups[g].initial))
                .collect(),
            lifts: scene
                .lifts
                .iter()
                .map(|l| {
                    let at = if l.open { 1.0 } else { 0.0 };
                    Device {
                        position: at,
                        target: at,
                        power: if l.powered { 1.0 } else { 0.0 },
                        ..Device::default()
                    }
                })
                .collect(),
            lift_named: (0..scene.lifts.len())
                .filter_map(|k| Some((scene.lifts[k].name?, k)))
                .collect(),
            lift_poses: vec![(f32::NAN, Vec::new()); scene.lifts.len()],
            mover_lift: scene
                .lifts
                .iter()
                .enumerate()
                .flat_map(|(k, l)| {
                    l.parts
                        .iter()
                        .filter_map(move |p| Some((p.mover?, (k, p.node))))
                })
                .collect(),
            switches: scene
                .switches
                .iter()
                .map(|s| Device {
                    power: if s.powered { 1.0 } else { 0.0 },
                    ..Device::default()
                })
                .collect(),
            switch_named: (0..scene.switches.len())
                .filter_map(|k| Some((scene.switches[k].name?, k)))
                .collect(),
            ..State::default()
        };
        let mut ctx = Ctx {
            st: &mut state,
            scene,
            game,
            bots,
        };
        for s in 0..scene.ai.squads.len() {
            if scene.ai.squads[s].initially_placed {
                ctx.place(s, None, None);
            }
        }
        // H2_LIFTS="k=position,..." sends lifts somewhere (for testing).
        for (k, to) in std::env::var("H2_LIFTS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .filter_map(|(k, v)| {
                Some((
                    k.trim().parse::<usize>().ok()?,
                    v.trim().parse::<f32>().ok()?,
                ))
            })
        {
            if let Some(d) = ctx.st.lifts.get_mut(k) {
                d.target = to;
            }
        }
        // H2_TITLE="k@seconds" shows title k then (for testing).
        if let Some((k, at)) = std::env::var("H2_TITLE").ok().and_then(|v| {
            let (k, at) = v.split_once('@')?;
            Some((k.parse::<usize>().ok()?, at.parse::<f32>().ok()?))
        }) {
            ctx.st.screen.title = Some((k, at));
            ctx.st.screen.letterbox = Ramp {
                from: 0.0,
                to: 1.0,
                start: at - 1.0,
                length: LETTERBOX_TIME,
            };
        }
        if log {
            for t in &scene.ai.titles {
                println!("title {t:?}");
            }
            println!("objectives {:?}", scene.ai.objectives);
            for (k, l) in scene.lifts.iter().enumerate() {
                let name = l.name.and_then(|n| scene.ai.object_names.get(n as usize));
                println!(
                    "lift {k} {} at {:.1}: {} parts, {} triangles, {:.1}s, open {}",
                    name.map_or("", String::as_str),
                    l.transform.w_axis.truncate(),
                    l.parts.len(),
                    l.parts.iter().map(|p| p.triangles.len()).sum::<usize>(),
                    l.time,
                    l.open
                );
            }
        }
        let mut vm = Vm::new(&scene.ai.scripts, &mut ctx);
        vm.log = log;
        // H2_WAKE="<script>,...": wake those scripts at the start (for
        // testing one part of a mission).
        for name in std::env::var("H2_WAKE").unwrap_or_default().split(',') {
            let scripts = &scene.ai.scripts.scripts;
            if let Some(k) = scripts.iter().position(|s| s.name == name.trim()) {
                vm.wake(k);
            }
        }
        Mission {
            vm,
            state,
            ticks: 0,
        }
    }

    /// One game tick: lifts move (carrying whoever's on them), players
    /// use switches and may cross into another part of the level, doors
    /// and machines move, and every other tick the scripts run.
    pub fn step(
        &mut self,
        scene: &Scene,
        world: &World,
        game: &mut Game,
        bots: &mut Vec<(usize, Bot)>,
    ) {
        self.ticks += 1;
        self.state.time = self.ticks as f32 * h2sim::game::TICK;
        for p in &mut game.players {
            p.body.gravity = self.state.gravity;
        }
        let mut ctx = Ctx {
            st: &mut self.state,
            scene,
            game,
            bots,
        };
        ctx.move_lifts(world, h2sim::game::TICK);
        ctx.use_switches();
        ctx.follow_collision(world);
        if self.ticks % 2 == 1 {
            return;
        }
        self.run_scripts(scene, world, game, bots);
        // Skipping a cutscene: its scripts run on to its end at once,
        // without its lines.
        if self.state.cutscene.fast_forward() {
            let heard = self.state.sounds.len();
            let seen = self.state.effects.len();
            for _ in 0..SKIP_TICKS {
                if !self.state.cutscene.fast_forward() {
                    break;
                }
                self.ticks += 2;
                self.state.time = self.ticks as f32 * h2sim::game::TICK;
                self.run_scripts(scene, world, game, bots);
            }
            let mut k = 0;
            self.state.sounds.retain(|s| {
                k += 1;
                k <= heard || !matches!(s, MissionSound::Line { .. })
            });
            self.state.sounds.push(MissionSound::Hush);
            self.state.effects.truncate(seen);
            self.state.cutscene.hush();
        }
    }

    /// One tick of the scripts and what they drive.
    fn run_scripts(
        &mut self,
        scene: &Scene,
        world: &World,
        game: &mut Game,
        bots: &mut Vec<(usize, Bot)>,
    ) {
        let mut ctx = Ctx {
            st: &mut self.state,
            scene,
            game,
            bots,
        };
        let dt = 1.0 / h2sim::script::TICKS_PER_SECOND as f32;
        ctx.follow_bsp();
        ctx.move_devices(dt);
        ctx.move_doors(world, dt);
        self.vm.tick(&scene.ai.scripts, &mut ctx);
        ctx.follow_orders(&mut self.vm, dt);
        ctx.run_commands(&mut self.vm, dt);
        ctx.run_scenes(dt);
        ctx.run_vehicles(world, dt);
    }

    /// Skip the cutscene playing, if the player may; returns whether it
    /// will be.
    pub fn skip_cutscene(&mut self) -> bool {
        self.state.cutscene.skip()
    }

    /// Whether one of the scene's objects is in the level as it is now
    /// (scripts create and destroy some; open doors are out of the way).
    pub fn shows(&self, scene: &Scene, object: usize) -> bool {
        let Some(o) = scene.objects.get(object) else {
            return false;
        };
        let hidden = o
            .name
            .is_some_and(|n| self.state.hidden.contains(&Obj::Name(n)));
        !hidden
            && self.state.exists(o.name, o.automatic)
            && o.door
                .is_none_or(|d| self.state.doors.get(d).is_none_or(|d| d.position < 0.5))
    }

    /// Whether the players have their controls (the scripts take them
    /// away for cutscenes).
    pub fn input_enabled(&self) -> bool {
        !self.state.input_off
    }

    /// Players the scripts turned to face a new way since last asked:
    /// their views turn with them.
    pub fn take_turns(&mut self) -> Vec<(usize, f32)> {
        std::mem::take(&mut self.state.turns)
    }

    /// Whether the scripts hid a player or actor.
    pub fn hides(&self, player: usize) -> bool {
        self.state.hidden.contains(&Obj::Unit(player))
    }

    /// The objectives the mission has given so far, and whether each is
    /// done.
    pub fn objectives(&self, scene: &Scene) -> Vec<(String, bool)> {
        let (shown, done) = self.state.objectives;
        let all = &scene.ai.objectives;
        (0..shown.min(all.len()))
            .map(|k| (all[k].clone(), k < done))
            .collect()
    }

    /// The fade, letterbox bars, title and HUD the scripts want now.
    pub fn screen(&self, scene: &Scene) -> ScreenView {
        let s = &self.state.screen;
        let now = self.state.time;
        let title = s.title.and_then(|(k, start)| {
            let t = scene.ai.titles.get(k)?;
            let age = now - start;
            let showing = if age < t.fade_in {
                age / t.fade_in.max(1e-3)
            } else if age < t.fade_in + t.up {
                1.0
            } else {
                1.0 - (age - t.fade_in - t.up) / t.fade_out.max(1e-3)
            };
            (showing > 0.0 && !t.text.is_empty())
                .then(|| (t.text.clone(), t.bounds, showing.min(1.0)))
        });
        let [r, g, b] = s.fade_color;
        // Once it's won, the HUD says so through the last fade.
        let won = self.state.won;
        let fade = s.fade.at(now);
        ScreenView {
            fade: [r, g, b, if won { fade.min(0.7) } else { fade }],
            letterbox: s.letterbox.at(now),
            hud: if won { 1.0 } else { s.hud.at(now) },
            title,
            subtitle: self.subtitle(scene),
        }
    }

    /// The lifts' parts to draw: mesh, model matrix and light.
    pub fn lift_draws(&self, scene: &Scene) -> Vec<(usize, Mat4, Option<[f32; 3]>)> {
        let mut out = Vec::new();
        for (k, lift) in scene.lifts.iter().enumerate() {
            if !self.state.exists(lift.name, lift.placed) {
                continue;
            }
            let poses = &self.state.lift_poses[k].1;
            for part in &lift.parts {
                let moved = poses.get(part.node).copied().unwrap_or(Mat4::IDENTITY);
                out.push((part.mesh, moved * lift.transform, lift.light));
            }
        }
        out
    }

    /// How far an object has moved with the lift a script attached it to.
    pub fn carried(&self, scene: &Scene, object: usize) -> Mat4 {
        scene
            .objects
            .get(object)
            .and_then(|o| o.name)
            .map_or(Mat4::IDENTITY, |n| self.state.moved(n))
    }

    /// What the action `button` would do here, for a player.
    pub fn switch_prompt(
        &self,
        scene: &Scene,
        game: &Game,
        player: usize,
        button: &str,
    ) -> Option<String> {
        switch_near(&self.state, scene, game, player)?;
        Some(format!("PRESS {button} TO USE THE SWITCH"))
    }

    /// The sounds the scripts played since last asked.
    pub fn take_sounds(&mut self) -> Vec<MissionSound> {
        std::mem::take(&mut self.state.sounds)
    }

    /// How far the view is shaken off now: yaw and pitch, radians.
    pub fn shake(&self) -> (f32, f32) {
        let screen = &self.state.screen;
        let strength = screen.shake.at(self.state.time);
        if strength <= 0.0 {
            return (0.0, 0.0);
        }
        // A jitter of a few uneven frequencies, so it doesn't look regular.
        let t = self.state.time;
        let wobble = |a: f32, b: f32| ((t * a).sin() + (t * b).sin() * 0.6) / 1.6;
        let [yaw, pitch, _] = screen.shake_rotation.map(|d| (d * strength).to_radians());
        (yaw * wobble(23.0, 37.0), pitch * wobble(29.0, 41.0))
    }

    /// The effects the scripts set off since last asked, and where.
    pub fn take_effects(&mut self) -> Vec<(EffectLook, Vec3)> {
        std::mem::take(&mut self.state.effects)
    }

    /// Tips the scripts gave the players since last asked.
    pub fn take_hints(&mut self) -> Vec<String> {
        std::mem::take(&mut self.state.hints)
    }

    /// Where the scripts' waypoints are now.
    pub fn nav_points(&self, scene: &Scene, game: &Game) -> Vec<Vec3> {
        self.state
            .nav_points
            .iter()
            .filter_map(|&(to, up)| {
                let at = match to {
                    NavPoint::Object(o) => object_position(&self.state, scene, game, o)?,
                    NavPoint::Flag(at) => at,
                };
                Some(at + Vec3::Z * up)
            })
            .collect()
    }

    /// The mission's last script said it's won.
    pub fn won(&self) -> bool {
        self.state.won
    }

    /// The structure BSP the players are in.
    pub fn bsp(&self) -> u16 {
        self.state.bsp
    }
}

impl State {
    /// Whether an object is in the level: as placed, unless a script made
    /// or destroyed it.
    fn exists(&self, name: Option<u16>, automatic: bool) -> bool {
        name.and_then(|n| self.created.get(&n).copied())
            .unwrap_or(automatic)
    }

    /// How far a named object has moved: a lift, or what's attached to
    /// one, goes with the lift's base.
    fn moved(&self, name: u16) -> Mat4 {
        let lift = self.attached.get(&name).unwrap_or(&name);
        self.lift_named
            .get(lift)
            .and_then(|&k| self.lift_poses[k].1.first())
            .copied()
            .unwrap_or(Mat4::IDENTITY)
    }
}

/// The switch a player could use: a powered one in reach.
fn switch_near(st: &State, scene: &Scene, game: &Game, player: usize) -> Option<usize> {
    let p = game
        .players
        .get(player)
        .filter(|p| p.alive && p.seat.is_none())?;
    let middle = p.body.position + Vec3::Z * MIDDLE;
    (0..scene.switches.len())
        .filter(|&k| {
            let s = &scene.switches[k];
            st.switches[k].power > 0.0 && st.exists(s.name, s.placed)
        })
        .map(|k| {
            let s = &scene.switches[k];
            let at = s
                .name
                .map_or(s.position, |n| st.moved(n).transform_point3(s.position));
            (k, at.distance(middle))
        })
        .filter(|&(_, d)| d < SWITCH_REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// The BSP the players start in: the one their starting location is in.
fn start_bsp(scene: &Scene, game: &Game) -> u16 {
    let at = game
        .players
        .iter()
        .find(|p| p.actor.is_none())
        .map_or(Vec3::ZERO, |p| p.body.position);
    scene
        .spawns
        .iter()
        .filter(|s| s.bsp != u16::MAX)
        .min_by(|a, b| {
            let d = |s: &blam_cache::PlayerSpawn| Vec3::from(s.position).distance_squared(at);
            d(a).total_cmp(&d(b))
        })
        .map_or(0, |s| s.bsp)
}

/// Whether a squad group is (or is inside) group `g`.
fn in_group(mut at: Option<u16>, g: u16, parents: &[Option<u16>]) -> bool {
    for _ in 0..64 {
        match at {
            Some(x) if x == g => return true,
            Some(x) => at = parents.get(x as usize).copied().flatten(),
            None => return false,
        }
    }
    false
}

/// What the scripts work with while they run.
/// What a waypoint is over.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NavPoint {
    Object(Obj),
    Flag(Vec3),
}

/// Where an object is, if it's in the level.
fn object_position(st: &State, scene: &Scene, game: &Game, o: Obj) -> Option<Vec3> {
    match o {
        Obj::Unit(i) => game
            .players
            .get(i)
            .filter(|p| p.alive)
            .map(|p| p.body.position),
        Obj::Vehicle(v) => game
            .vehicles
            .get(v)
            .filter(|veh| !veh.destroyed)
            .map(|veh| veh.origin(&game.vehicle_defs[veh.def])),
        Obj::Name(n) => {
            if st.created.get(&n) == Some(&false) {
                return None;
            }
            let placed = scene.ai.name_positions.get(n as usize).copied().flatten()?;
            Some(st.moved(n).transform_point3(placed))
        }
    }
}

struct Ctx<'a> {
    st: &'a mut State,
    scene: &'a Scene,
    game: &'a mut Game,
    bots: &'a mut Vec<(usize, Bot)>,
}

impl Ctx<'_> {
    /// Place a squad's actors (at most `limit`), with bots to run them;
    /// returns them.
    fn place(&mut self, squad: usize, limit: Option<usize>, only: Option<usize>) -> Vec<usize> {
        let placed = place_squad(
            self.game,
            self.scene,
            squad,
            self.st.difficulty,
            self.st.players_team,
            limit,
            only,
        );
        if self.st.log {
            let name = &self.scene.ai.squads[squad].name;
            println!("script: placed {name} ({})", placed.len());
        }
        *self.st.placed.entry(squad).or_insert(0) += placed.len() as u32;
        self.bots
            .retain(|(i, _)| !placed.iter().any(|(j, _)| j == i));
        let actors: Vec<usize> = placed.iter().map(|(i, _)| *i).collect();
        self.bots
            .extend(placed.into_iter().filter_map(|(i, b)| Some((i, b?))));
        self.start_orders(squad);
        // Each runs its starting location's command script.
        let s = &self.scene.ai.squads[squad];
        for &i in &actors {
            let script = self.game.players[i]
                .actor
                .and_then(|a| s.locations.get(a.location as usize)?.placement_script)
                .or(s.placement_script);
            if let Some(script) = script {
                self.st.commands.starts.push((i, script as usize, false));
            }
        }
        actors
    }

    /// The squads an `ai` value means: a squad, or every squad in a group.
    fn squads(&self, ai: &Value) -> Vec<usize> {
        let Some(h) = ai.handle() else {
            return Vec::new();
        };
        if let Some(actors) = self.ai_actors(h) {
            let mut squads: Vec<usize> = actors
                .iter()
                .filter_map(|&i| Some(self.game.players[i].actor?.squad as usize))
                .collect();
            let squad = ((h >> 16) & 0x3FFF) as usize;
            if h & (3 << 30) == commands::AI_LOCATION && squad < self.scene.ai.squads.len() {
                squads.push(squad);
            }
            squads.dedup();
            return squads;
        }
        let squads = &self.scene.ai.squads;
        let n = (h & 0xFFFF) as usize;
        match h >> 30 {
            0 if n < squads.len() => vec![n],
            1 => (0..squads.len())
                .filter(|&s| in_group(squads[s].group, n as u16, &self.scene.ai.group_parents))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The living actors of an `ai` value.
    fn actors(&self, ai: &Value) -> Vec<usize> {
        if let Some(actors) = ai.handle().and_then(|h| self.ai_actors(h)) {
            return actors;
        }
        let squads = self.squads(ai);
        (0..self.game.players.len())
            .filter(|&i| {
                let p = &self.game.players[i];
                p.alive
                    && p.actor
                        .is_some_and(|a| squads.contains(&(a.squad as usize)))
            })
            .collect()
    }

    /// The people playing (not actors).
    fn humans(&self) -> Vec<usize> {
        (0..self.game.players.len())
            .filter(|&i| self.game.players[i].actor.is_none())
            .collect()
    }

    /// The player or actor an object is, if it's one.
    fn unit(&self, o: Obj) -> Option<usize> {
        match o {
            Obj::Unit(i) => Some(i).filter(|&i| i < self.game.players.len()),
            Obj::Name(_) | Obj::Vehicle(_) => None,
        }
    }

    /// Where an object is, if it's in the level.
    fn position(&self, o: Obj) -> Option<Vec3> {
        object_position(self.st, self.scene, self.game, o)
    }

    fn in_volume(&self, volume: &Value, o: Obj) -> bool {
        let Some(v) = volume
            .index()
            .and_then(|v| self.scene.ai.volumes.get(v as usize))
        else {
            return false;
        };
        // A unit counts by its middle, not its feet.
        self.position(o)
            .is_some_and(|p| v.contains(p + Vec3::Z * 0.3))
    }

    /// Players walking into a BSP switch volume cross into the BSP it
    /// leads to.
    fn follow_bsp(&mut self) {
        for &(volume, from, to) in &self.scene.ai.bsp_switches {
            if from != self.st.bsp {
                continue;
            }
            let v = Value::Handle(volume as u32);
            if self
                .humans()
                .into_iter()
                .any(|i| self.in_volume(&v, Obj::Unit(i)))
            {
                self.st.bsp = to;
                return;
            }
        }
    }

    /// Only the structure BSP the mission is in blocks, once the players
    /// stand in it; till then (in a cutscene's own BSP, say) all of the
    /// level does. Actors it leaves with nothing to stand on are gone from
    /// the level, as in the original.
    fn follow_collision(&mut self, world: &World) {
        // In a BSP: on a lift, or over its floor, the nearest one there is
        // (where two BSPs overlap, both have it). A flier may be well up.
        let stands = |bsp: u16, p: &h2sim::game::Spartan, reach: f32| {
            let feet = p.body.position + Vec3::Z * 0.5;
            let reach = if p.body.biped.flying {
                FLIER_FLOOR
            } else {
                reach
            };
            if world.mover_under(feet, reach).is_some() {
                return true;
            }
            let Some(own) = world.floor_below(Some(bsp), feet, reach) else {
                return false;
            };
            world
                .floor_below(None, feet, reach)
                .is_none_or(|any| own <= any + SAME_FLOOR)
        };
        let humans: Vec<usize> = self
            .humans()
            .into_iter()
            .filter(|&i| self.game.players[i].alive)
            .collect();
        let all_stand = |bsp: u16| {
            !humans.is_empty()
                && humans
                    .iter()
                    .all(|&i| stands(bsp, &self.game.players[i], HUMAN_FLOOR))
        };
        if let Some(before) = self.st.bsp_after_cutscene.take() {
            if !all_stand(self.st.bsp) && all_stand(before) {
                self.st.bsp = before;
                if self.st.log {
                    println!("back in bsp {before} after the cutscene");
                }
            }
        }
        let bsp = self.st.bsp;
        if !all_stand(bsp) {
            // Until they do (and when they step off it onto another's
            // floor), all of the level is there.
            if world.group().is_some() {
                world.set_group(None);
                if self.st.log {
                    println!("collision of every bsp");
                }
            }
            return;
        }
        if world.group() == Some(bsp) {
            return;
        }
        world.set_group(Some(bsp));
        if self.st.log {
            println!("collision of bsp {bsp}");
        }
        let left: Vec<usize> = (0..self.game.players.len())
            .filter(|&i| {
                let p = &self.game.players[i];
                p.actor.is_some_and(|a| !a.gone)
                    && p.alive
                    && p.seat.is_none()
                    && !stands(bsp, p, ACTOR_FLOOR)
            })
            .collect();
        for i in left {
            if self.st.log {
                let at = self.game.players[i].body.position;
                println!("actor {i} at {at:.1} left behind in another bsp");
            }
            self.erase(i);
        }
    }

    /// Lifts move toward where they're sent (or where their device group
    /// says), carrying whoever stands on them.
    fn move_lifts(&mut self, world: &World, dt: f32) {
        let scene = self.scene;
        for (k, lift) in scene.lifts.iter().enumerate() {
            let group = lift
                .position_group
                .and_then(|g| self.st.device_groups.get(&g));
            if let Some(&v) = group {
                self.st.lifts[k].target = v;
            }
            let d = &mut self.st.lifts[k];
            if d.power > 0.0 {
                let step = dt / lift.time.max(0.05);
                d.position += (d.target - d.position).clamp(-step, step);
            }
            let (at, target) = (d.position, d.target);
            let exists = self.st.exists(lift.name, lift.placed);
            if self.st.lift_poses[k].0 != at {
                let poses = lift.poses(at);
                if exists && !self.st.lift_poses[k].1.is_empty() {
                    self.carry(world, k, &poses);
                }
                let was = self.st.lift_poses[k].0;
                if self.st.log && !was.is_nan() && (at == target) != (was == target) {
                    let what = if at == target { "stops" } else { "moves" };
                    println!(
                        "lift {k} at {:.1} {what} ({at:.2})",
                        lift.transform.w_axis.truncate()
                    );
                }
                self.st.lift_poses[k] = (at, poses);
            }
            let poses = &self.st.lift_poses[k].1;
            for part in &lift.parts {
                if let Some(m) = part.mover {
                    world.move_mover(m, lift.offset(poses, part), exists);
                }
            }
        }
    }

    /// Whoever stands on lift `k` goes where it's going.
    fn carry(&mut self, world: &World, k: usize, poses: &[Mat4]) {
        let was = &self.st.lift_poses[k].1;
        for p in &mut self.game.players {
            if !p.alive || p.seat.is_some() {
                continue;
            }
            let on = world
                .mover_under(p.body.position, CARRY_REACH)
                .and_then(|m| self.st.mover_lift.get(&m));
            let Some(&(_, node)) = on.filter(|on| on.0 == k) else {
                continue;
            };
            let resting = was[node].inverse().transform_point3(p.body.position);
            p.body.position = poses[node].transform_point3(resting);
        }
    }

    /// Players pressing the action button by a switch use it.
    fn use_switches(&mut self) {
        let n = self.game.players.len();
        self.st.held.resize(n, false);
        for i in self.humans() {
            let held = self.game.players[i].holding_action();
            let pressed = held && !self.st.held[i];
            self.st.held[i] = held;
            if !pressed {
                continue;
            }
            if let Some(k) = switch_near(self.st, self.scene, self.game, i) {
                self.flip(k);
            }
        }
    }

    /// Use a switch: it sets its device group, or calls the lift nearest it.
    fn flip(&mut self, k: usize) {
        let s = &self.scene.switches[k];
        if self.st.log {
            println!("switch {k} at {:.1} used", s.position);
        }
        let Some(g) = s.position_group else {
            let at = s.position;
            let nearest = (0..self.scene.lifts.len())
                .filter(|&l| {
                    self.st
                        .exists(self.scene.lifts[l].name, self.scene.lifts[l].placed)
                })
                .map(|l| {
                    (
                        l,
                        self.scene.lifts[l].transform.w_axis.truncate().distance(at),
                    )
                })
                .filter(|&(_, d)| d < CALL_REACH)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((l, _)) = nearest {
                let d = &mut self.st.lifts[l];
                d.target = if d.position > 0.5 { 0.0 } else { 1.0 };
            }
            return;
        };
        if self.st.spent.contains(&g) {
            return;
        }
        let now = self.st.device_groups.get(&g).copied().unwrap_or(0.0);
        let to = match s.kind {
            ControlKind::Toggle if now > 0.5 => 0.0,
            ControlKind::Toggle | ControlKind::On => 1.0,
            ControlKind::Off => 0.0,
            ControlKind::Call => s.call_value,
        };
        self.st.device_groups.insert(g, to);
        self.st.switches[k].position = to;
        if self
            .scene
            .ai
            .device_groups
            .get(g as usize)
            .is_some_and(|d| d.once)
        {
            self.st.spent.insert(g);
        }
    }

    fn move_devices(&mut self, dt: f32) {
        for (&name, d) in &mut self.st.devices {
            let time = self
                .scene
                .ai
                .machines
                .get(name as usize)
                .copied()
                .flatten()
                .map_or(MACHINE_TIME, |m| m.position_time)
                .max(0.05);
            let step = dt / time * d.power.signum().max(0.0);
            d.position += (d.target - d.position).clamp(-step, step);
        }
    }

    /// Doors open for whoever's near, unless locked, and shut again
    /// after; shut ones block the way.
    fn move_doors(&mut self, world: &World, dt: f32) {
        let scene = self.scene;
        for (k, door) in scene.doors.iter().enumerate() {
            let middle = door.position + Vec3::Z * DOOR_MIDDLE;
            let reach = if door.reach > 0.0 {
                door.reach
            } else {
                DOOR_REACH
            };
            let one_sided = self.st.doors[k].one_sided;
            let near = || {
                self.game.players.iter().any(|p| {
                    let at = p.body.position + Vec3::Z * 0.5;
                    let behind = (at - door.position).dot(door.forward) < 0.0;
                    let sided = one_sided || door.one_sided_for_players && p.actor.is_none();
                    p.alive && at.distance(middle) < reach && (behind || !sided)
                })
            };
            let exists = self.st.exists(door.name, door.placed);
            let d = &mut self.st.doors[k];
            if d.automatic && d.power > 0.0 && exists {
                if near() {
                    d.target = 1.0;
                    d.wait = door.stays_open;
                } else if door.closes {
                    d.wait -= dt;
                    if d.wait <= 0.0 {
                        d.target = 0.0;
                    }
                }
            }
            let was_shut = d.position < 0.5;
            if d.power > 0.0 {
                let step = dt / door.time.max(0.05);
                d.position += (d.target - d.position).clamp(-step, step);
            }
            let shut = d.position < 0.5;
            if self.st.log && shut != was_shut {
                let what = if shut { "shuts" } else { "opens" };
                println!("door {k} at {:.1} {what} (reach {reach})", door.position);
            }
            world.set_door(k, exists && shut);
        }
    }

    fn device(&mut self, v: Option<&Value>) -> Option<&mut Device> {
        let name = match v?.objects().first()? {
            Obj::Name(n) => *n,
            Obj::Unit(_) | Obj::Vehicle(_) => return None,
        };
        if let Some(&k) = self.st.door_named.get(&name) {
            return self.st.doors.get_mut(k);
        }
        if let Some(&k) = self.st.lift_named.get(&name) {
            return self.st.lifts.get_mut(k);
        }
        if let Some(&k) = self.st.switch_named.get(&name) {
            return self.st.switches.get_mut(k);
        }
        Some(self.st.devices.entry(name).or_default())
    }

    /// The vitality left of the actors placed in an `ai`'s squads, from 1
    /// (all fresh) to 0 (all dead).
    fn strength(&self, ai: &Value) -> f32 {
        self.squads_strength(&self.squads(ai))
    }

    /// How much of some squads' placed actors' health and shields is
    /// left (0-1).
    fn squads_strength(&self, squads: &[usize]) -> f32 {
        let placed: u32 = squads.iter().filter_map(|s| self.st.placed.get(s)).sum();
        if placed == 0 {
            return 0.0;
        }
        let left: f32 = (0..self.game.players.len())
            .filter(|&i| {
                let p = &self.game.players[i];
                p.alive
                    && p.actor
                        .is_some_and(|a| squads.contains(&(a.squad as usize)))
            })
            .map(|i| {
                let p = &self.game.players[i];
                let full = (p.full.health + p.full.shield).max(1.0);
                ((p.health + p.shield) / full).clamp(0.0, 1.0)
            })
            .sum();
        left / placed as f32
    }

    /// Named objects whose names contain a script's text.
    fn names_containing(&self, text: &Value) -> Vec<u16> {
        let Some(at) = text.handle() else {
            return Vec::new();
        };
        let part = self.scene.ai.scripts.text(at);
        if part.is_empty() {
            return Vec::new();
        }
        let names = &self.scene.ai.object_names;
        (0..names.len())
            .filter(|&n| names[n].contains(part))
            .map(|n| n as u16)
            .collect()
    }

    fn set_created(&mut self, objects: &Value, created: bool) {
        for &o in objects.objects() {
            match o {
                Obj::Name(n) => {
                    self.st.created.insert(n, created);
                }
                Obj::Unit(i) if !created => self.game.erase_actor(i),
                Obj::Vehicle(v) if !created => self.remove_vehicle(v),
                Obj::Unit(_) | Obj::Vehicle(_) => {}
            }
        }
    }
}

impl Host for Ctx<'_> {
    fn set_actor(&mut self, actor: Option<u32>) {
        let runner = actor.map(|a| a as usize);
        self.st.commands.runner = runner;
        self.st.commands.current =
            runner.map(|r| self.st.scenes.switched.get(&r).copied().unwrap_or(r));
    }

    fn engine_global(&mut self, name: &str) -> Option<Value> {
        self.current_ai(name)
    }

    fn waiting(&mut self) -> bool {
        std::mem::take(&mut self.st.commands.waiting)
    }

    fn call(&mut self, function: &str, args: &[Value], _returns: u16) -> Option<Value> {
        if let Some(v) = self.dialogue_call(function, args) {
            return Some(v);
        }
        if let Some(v) = self.cinematic_call(function, args) {
            return Some(v);
        }
        if let Some(v) = self.cutscene_call(function, args) {
            return Some(v);
        }
        if let Some(v) = self.command_call(function, args) {
            return Some(v);
        }
        if let Some(v) = self.vehicle_call(function, args) {
            return Some(v);
        }
        let arg = |k: usize| args.get(k).cloned().unwrap_or_default();
        let num = |k: usize| args.get(k).map_or(0.0, Value::num);
        let objects = |k: usize| args.get(k).map_or(&[][..], Value::objects);
        let units = |list: Vec<usize>| Value::Objects(list.into_iter().map(Obj::Unit).collect());
        Some(match function {
            // The players and where they are.
            "players" => units(self.humans()),
            "player_count" => Value::Real(self.humans().len() as f32),
            "list_count_not_dead" => Value::Real(
                objects(0)
                    .iter()
                    .filter(|&&o| self.position(o).is_some())
                    .count() as f32,
            ),
            "volume_test_objects" | "volume_test_object" => {
                let v = arg(0);
                Value::Bool(objects(1).iter().any(|&o| self.in_volume(&v, o)))
            }
            "volume_test_objects_all" => {
                let v = arg(0);
                let list = objects(1);
                Value::Bool(!list.is_empty() && list.iter().all(|&o| self.in_volume(&v, o)))
            }
            "objects_distance_to_flag" => {
                let to = arg(1)
                    .index()
                    .and_then(|k| self.scene.ai.flags.get(k as usize))
                    .map(|f| f.0);
                let d = to.and_then(|to| {
                    objects(0)
                        .iter()
                        .filter_map(|&o| self.position(o))
                        .map(|p| p.distance(to))
                        .min_by(f32::total_cmp)
                });
                Value::Real(d.unwrap_or(-1.0))
            }
            // Who and what is inside a volume: units (type bit 0) and
            // vehicles (bit 1).
            "volume_return_objects" | "volume_return_objects_by_type" => {
                let mask = if args.len() > 1 {
                    num(1) as u32
                } else {
                    u32::MAX
                };
                let v = arg(0);
                let mut found = Vec::new();
                if mask & 1 != 0 {
                    for i in 0..self.game.players.len() {
                        if self.game.players[i].alive && self.in_volume(&v, Obj::Unit(i)) {
                            found.push(Obj::Unit(i));
                        }
                    }
                }
                if mask & 2 != 0 {
                    for k in 0..self.game.vehicles.len() {
                        if self.in_volume(&v, Obj::Vehicle(k)) {
                            found.push(Obj::Vehicle(k));
                        }
                    }
                }
                Value::Objects(found)
            }
            "kill_volume_enable" | "kill_volume_disable" => {
                let on = function.ends_with("enable");
                let zone = arg(0)
                    .index()
                    .and_then(|k| self.scene.ai.kill_zone_of.get(k as usize).copied())
                    .flatten();
                if let Some(z) = zone.and_then(|z| self.game.kill_zones.get_mut(z)) {
                    z.on = on;
                }
                Value::Void
            }
            "objects_distance_to_object" => {
                let to = objects(1).first().and_then(|&o| self.position(o));
                let d = to.and_then(|to| {
                    objects(0)
                        .iter()
                        .filter_map(|&o| self.position(o))
                        .map(|p| p.distance(to))
                        .min_by(f32::total_cmp)
                });
                Value::Real(d.unwrap_or(-1.0))
            }
            // Looking its way (walls aside).
            "objects_can_see_object" => {
                let cone = num(2).to_radians().cos();
                let target = objects(1).first().and_then(|&o| self.position(o));
                Value::Bool(target.is_some_and(|t| {
                    objects(0).iter().any(|&o| match o {
                        Obj::Unit(i) => self.game.players.get(i).is_some_and(|p| {
                            p.alive && p.aim().dot((t - p.eye()).normalize_or_zero()) >= cone
                        }),
                        Obj::Name(_) | Obj::Vehicle(_) => false,
                    })
                }))
            }
            "unit_get_health" | "object_get_health" | "unit_get_shield" | "object_get_shield" => {
                let shield = function.ends_with("shield");
                let v = match objects(0).first() {
                    Some(&Obj::Unit(i)) => self.game.players.get(i).map_or(-1.0, |p| {
                        if shield {
                            p.shield / p.full.shield.max(1e-3)
                        } else if p.alive {
                            p.health / p.full.health.max(1e-3)
                        } else {
                            0.0
                        }
                    }),
                    Some(&Obj::Vehicle(v)) => self.game.vehicles.get(v).map_or(-1.0, |veh| {
                        let full = self.game.vehicle_defs[veh.def].health.max(1e-3);
                        if shield || veh.destroyed {
                            0.0
                        } else {
                            veh.health / full
                        }
                    }),
                    Some(&Obj::Name(_)) => 1.0,
                    None => -1.0,
                };
                Value::Real(v)
            }
            "unit_in_vehicle" => Value::Bool(matches!(objects(0).first(),
                Some(&Obj::Unit(i)) if self.game.players.get(i).is_some_and(|p| p.seat.is_some()))),
            // The level's parts.
            "structure_bsp_index" => Value::Real(self.st.bsp as f32),
            "switch_bsp" => {
                self.st.bsp = num(0).max(0.0) as u16;
                Value::Void
            }
            "switch_bsp_by_name" => {
                if let Some(b) = arg(0).index() {
                    self.st.bsp = b;
                }
                Value::Void
            }
            // The game.
            "game_difficulty_get" | "game_difficulty_get_real" => {
                Value::Handle(0xFFFF_0000 | self.st.difficulty as u32)
            }
            // Whether the player skipped the cutscene (a real revert to
            // the save before it isn't needed: skipping fast-forwards it).
            "game_reverted" => Value::Bool(self.st.cutscene.skipped()),
            "game_is_cooperative" => Value::Bool(self.humans().len() > 1),
            "game_won" => {
                self.st.won = true;
                Value::Void
            }
            "game_save"
            | "game_save_immediate"
            | "game_save_no_timeout"
            | "game_save_cinematic_skip" => {
                self.game.request_checkpoint();
                Value::Void
            }
            "game_safe_to_save" => Value::Bool(true),
            "game_saving" => Value::Bool(false),
            "game_all_quiet" => Value::Bool(
                !self
                    .bots
                    .iter()
                    .any(|(i, b)| b.fighting() && self.game.players[*i].alive),
            ),
            // Squads.
            "ai_place" => {
                let limit = (args.len() > 1).then(|| num(1).max(0.0) as usize);
                // One starting location, or whole squads.
                match arg(0)
                    .handle()
                    .filter(|h| h & (3 << 30) == commands::AI_LOCATION)
                {
                    Some(h) => {
                        let squad = ((h >> 16) & 0x3FFF) as usize;
                        self.place(squad, None, Some((h & 0xFFFF) as usize));
                    }
                    None => {
                        for s in self.squads(&arg(0)) {
                            self.place(s, limit, None);
                        }
                    }
                }
                Value::Void
            }
            "ai_living_count" | "ai_nonswarm_count" => {
                Value::Real(self.actors(&arg(0)).len() as f32)
            }
            "ai_spawn_count" => {
                let n: u32 = self
                    .squads(&arg(0))
                    .iter()
                    .filter_map(|s| self.st.placed.get(s))
                    .sum();
                Value::Real(n as f32)
            }
            "ai_living_fraction" => {
                let ai = arg(0);
                let placed: u32 = self
                    .squads(&ai)
                    .iter()
                    .filter_map(|s| self.st.placed.get(s))
                    .sum();
                let living = self.actors(&ai).len();
                Value::Real(if placed == 0 {
                    0.0
                } else {
                    living as f32 / placed as f32
                })
            }
            "ai_strength" => Value::Real(self.strength(&arg(0))),
            "ai_fighting_count" => {
                let actors = self.actors(&arg(0));
                let n = self
                    .bots
                    .iter()
                    .filter(|(i, b)| actors.contains(i) && b.fighting())
                    .count();
                Value::Real(n as f32)
            }
            "ai_erase" | "ai_erase_all" => {
                let actors = if function == "ai_erase_all" {
                    (0..self.game.players.len()).collect()
                } else {
                    self.actors(&arg(0))
                };
                for i in actors {
                    self.erase(i);
                }
                Value::Void
            }
            "ai_kill" | "ai_kill_silent" => {
                for i in self.actors(&arg(0)) {
                    self.game.kill_actor(i);
                }
                Value::Void
            }
            "ai_get_object" | "ai_get_unit" => {
                units(self.actors(&arg(0)).into_iter().take(1).collect())
            }
            "ai_actors" => units(self.actors(&arg(0))),
            "ai_combat_status" => {
                let actors = self.actors(&arg(0));
                let status = self
                    .bots
                    .iter()
                    .filter(|(i, _)| actors.contains(i))
                    .map(|(_, b)| commands::combat_status(b))
                    .max()
                    .unwrap_or(0);
                Value::Real(status as f32)
            }
            "object_cannot_die" => {
                let on = arg(1).truthy();
                for &o in objects(0) {
                    if let Some(a) = self
                        .unit(o)
                        .and_then(|i| self.game.players[i].actor.as_mut())
                    {
                        a.immortal = on;
                    }
                }
                Value::Void
            }
            "unit_has_weapon" => {
                let tag = arg(1).handle();
                let has = objects(0)
                    .first()
                    .and_then(|&o| self.unit(o))
                    .is_some_and(|i| {
                        self.game.players[i].weapons.iter().any(|h| {
                            self.scene
                                .weapons
                                .get(h.weapon)
                                .is_some_and(|w| Some(w.tag.0) == tag)
                        })
                    });
                Value::Bool(has)
            }
            // Bookkeeping the game here doesn't need.
            // Two teams side with each other from now on.
            "ai_allegiance" => {
                let team = |v: Value| {
                    let Value::Handle(h) = v else { return None };
                    squad_team(AiTeam::from_number(h as u16), self.st.players_team)
                };
                if let (Some(a), Some(b)) = (team(arg(0)), team(arg(1))) {
                    if a != b && !self.game.allegiances.contains(&(a, b)) {
                        self.game.allegiances.push((a, b));
                    }
                }
                Value::Void
            }
            "ai_renew"
            | "object_set_deleted_when_deactivated"
            | "player_training_activate_flashlight"
            | "ai_disposable"
            | "ai_dialogue_enable"
            | "data_mine_set_mission_segment"
            | "cache_block_for_one_frame"
            | "object_type_predict"
            | "camera_predict_resources_at_point"
            | "game_can_use_flashlights"
            | "weapon_enable_warthog_chaingun_light"
            | "pvs_set_object"
            | "pvs_clear" => Value::Void,
            // Waypoints: (type, team or unit, object or flag, how high
            // above it); taking one away names the object or flag.
            "activate_team_nav_point_object" | "activate_nav_point_object" => {
                let up = num(3);
                for &o in arg(2).objects() {
                    let to = NavPoint::Object(o);
                    if self.st.log {
                        println!("waypoint over {o:?} at {:?}", self.position(o));
                    }
                    self.st.nav_points.retain(|n| n.0 != to);
                    self.st.nav_points.push((to, up));
                }
                Value::Void
            }
            "activate_team_nav_point_flag" | "activate_nav_point_flag" => {
                let flag = arg(2)
                    .index()
                    .and_then(|f| self.scene.ai.flags.get(f as usize));
                if let Some(&(at, _)) = flag {
                    let to = NavPoint::Flag(at);
                    self.st.nav_points.retain(|n| n.0 != to);
                    self.st.nav_points.push((to, num(3)));
                }
                Value::Void
            }
            "deactivate_team_nav_point_object" | "deactivate_nav_point_object" => {
                let gone: Vec<NavPoint> = arg(1)
                    .objects()
                    .iter()
                    .map(|&o| NavPoint::Object(o))
                    .collect();
                self.st.nav_points.retain(|n| !gone.contains(&n.0));
                Value::Void
            }
            "deactivate_team_nav_point_flag" | "deactivate_nav_point_flag" => {
                let flag = arg(1)
                    .index()
                    .and_then(|f| self.scene.ai.flags.get(f as usize));
                if let Some(&(at, _)) = flag {
                    self.st.nav_points.retain(|n| n.0 != NavPoint::Flag(at));
                }
                Value::Void
            }
            // Taught in the Arbiter's first mission.
            "player_training_activate_stealth" => {
                self.st
                    .hints
                    .push("PRESS V (LB) FOR ACTIVE CAMOUFLAGE".into());
                Value::Void
            }
            "cheat_active_camouflage_by_player" => {
                let on = arg(1).truthy();
                let humans = self.humans();
                if let Some(&i) = humans.get(num(0).max(0.0) as usize) {
                    self.game.players[i].camo = if on { 10.0 } else { 0.0 };
                }
                Value::Void
            }
            // Actors scripts protect a while (they can still be hurt).
            "object_cannot_take_damage" | "object_can_take_damage" => {
                let on = function == "object_cannot_take_damage";
                for &o in objects(0) {
                    let Some(i) = self.unit(o) else { continue };
                    if let Some(a) = &mut self.game.players[i].actor {
                        a.immortal = on;
                    }
                }
                Value::Void
            }
            "unit_kill" | "unit_kill_silent" => {
                for &o in objects(0) {
                    if let Some(i) = self
                        .unit(o)
                        .filter(|&i| self.game.players[i].actor.is_some())
                    {
                        self.game.kill_actor(i);
                    }
                }
                Value::Void
            }
            "ai_cannot_die" => {
                let on = arg(1).truthy();
                for i in self.actors(&arg(0)) {
                    if let Some(a) = &mut self.game.players[i].actor {
                        a.immortal = on;
                    }
                }
                Value::Void
            }
            "ai_set_blind" | "ai_suppress_combat" => {
                let (actors, on) = (self.actors(&arg(0)), arg(1).truthy());
                for (i, b) in self.bots.iter_mut() {
                    if let (true, Some(mind)) = (actors.contains(i), &mut b.actor) {
                        if function == "ai_set_blind" {
                            mind.blind = on;
                        } else {
                            mind.peaceful = on;
                        }
                    }
                }
                Value::Void
            }
            "ai_set_orders" => {
                let squads = self.squads(&arg(0));
                self.set_order(&squads, arg(1).index());
                Value::Void
            }
            "ai_migrate" => {
                let actors = self.actors(&arg(0));
                if let Some(&to) = self.squads(&arg(1)).first() {
                    self.migrate(&actors, to);
                }
                Value::Void
            }
            "ai_magically_see_object" | "ai_magically_see" => {
                let at = if function == "ai_magically_see" {
                    self.actors(&arg(1))
                        .first()
                        .map(|&i| self.game.players[i].body.position)
                } else {
                    objects(1).first().and_then(|&o| self.position(o))
                };
                if let Some(at) = at {
                    let actors = self.actors(&arg(0));
                    for (i, b) in self.bots.iter_mut() {
                        let near = self.game.players[*i].body.position.distance(at) < SEE_RANGE;
                        if let (true, true, Some(mind)) = (actors.contains(i), near, &mut b.actor) {
                            mind.alert = Some((at, 0.0));
                        }
                    }
                }
                Value::Void
            }
            // Objects.
            "object_create" | "object_create_anew" | "object_create_clone" => {
                self.set_created(&arg(0), true);
                Value::Void
            }
            "object_destroy" => {
                self.set_created(&arg(0), false);
                Value::Void
            }
            "object_create_containing"
            | "object_create_anew_containing"
            | "object_create_clone_containing"
            | "object_destroy_containing" => {
                let made = !function.starts_with("object_destroy");
                for n in self.names_containing(&arg(0)) {
                    self.st.created.insert(n, made);
                }
                Value::Void
            }
            // Dialogue and music.
            "sound_impulse_start" | "sound_impulse_start_effect" => {
                if let Some(&sound) = arg(0).handle().and_then(|t| self.scene.ai.sounds.get(&t)) {
                    if self.st.log {
                        let tag = arg(0).handle();
                        let scripts = &self.scene.ai.scripts;
                        let name = scripts
                            .expressions
                            .iter()
                            .find(|e| Some(e.value) == tag && e.value_type == value_type::SOUND)
                            .map_or("?", |e| scripts.text(e.text));
                        println!("script: says {name}");
                    }
                    let at = objects(1).first().and_then(|&o| self.position(o));
                    let gain = if args.len() > 2 { num(2) } else { 1.0 };
                    self.st.sounds.push(MissionSound::Line { sound, at, gain });
                }
                Value::Void
            }
            "sound_impulse_stop" => {
                if let Some(&sound) = arg(0).handle().and_then(|t| self.scene.ai.sounds.get(&t)) {
                    self.st.sounds.push(MissionSound::StopLine(sound));
                }
                Value::Void
            }
            // Ticks a line lasts.
            "sound_impulse_language_time" | "sound_impulse_time" => {
                let clip = arg(0)
                    .handle()
                    .and_then(|t| self.scene.ai.sounds.get(&t))
                    .and_then(|&s| self.scene.sounds.get(s))
                    .and_then(|s| s.clips.first());
                let seconds = clip.map_or(0.0, |c| c.duration());
                Value::Real((seconds * h2sim::script::TICKS_PER_SECOND as f32).round())
            }
            "sound_looping_start" => {
                if let Some(tag) = arg(0)
                    .handle()
                    .filter(|t| self.scene.ai.loops.contains_key(t))
                {
                    self.st.sounds.push(MissionSound::StartLoop(tag));
                }
                Value::Void
            }
            "sound_looping_stop" => {
                if let Some(tag) = arg(0).handle() {
                    self.st.sounds.push(MissionSound::StopLoop(tag));
                }
                Value::Void
            }
            // Doors, lifts and the like.
            "device_set_position" | "device_set_position_immediate" | "device_animate_position" => {
                let to = num(1).clamp(0.0, 1.0);
                let now = function.ends_with("immediate");
                if let Some(d) = self.device(args.first()) {
                    d.target = to;
                    if now {
                        d.position = to;
                    }
                }
                Value::Bool(true)
            }
            "device_get_position" => {
                Value::Real(self.device(args.first()).map_or(0.0, |d| d.position))
            }
            "device_set_power" => {
                if let Some(d) = self.device(args.first()) {
                    d.power = num(1);
                }
                Value::Void
            }
            "device_get_power" => Value::Real(self.device(args.first()).map_or(0.0, |d| d.power)),
            "device_operates_automatically_set" => {
                let on = arg(1).truthy();
                if let Some(d) = self.device(args.first()) {
                    d.automatic = on;
                }
                Value::Void
            }
            "device_one_sided_set" => {
                let on = arg(1).truthy();
                if let Some(d) = self.device(args.first()) {
                    d.one_sided = on;
                }
                Value::Void
            }
            // What's over the view.
            "fade_out" | "fade_in" => {
                // (fade_out r g b ticks): to the colour; fade_in: from it.
                let out = function == "fade_out";
                let seconds = num(3) / h2sim::script::TICKS_PER_SECOND as f32;
                let now = self.st.time;
                let screen = &mut self.st.screen;
                screen.fade_color = [num(0), num(1), num(2)];
                let (from, to) = if out {
                    (screen.fade.at(now), 1.0)
                } else {
                    (1.0, 0.0)
                };
                screen.fade = Ramp {
                    from,
                    to,
                    start: now,
                    length: seconds,
                };
                Value::Void
            }
            "cinematic_show_letterbox" | "cinematic_show_letterbox_immediate" => {
                let to = if arg(0).truthy() { 1.0 } else { 0.0 };
                let length = if function.ends_with("immediate") {
                    0.0
                } else {
                    LETTERBOX_TIME
                };
                self.st.screen.letterbox.go(self.st.time, to, length);
                Value::Void
            }
            // The end of a cutscene takes its bars away.
            "cinematic_stop" => {
                self.st.screen.letterbox = Ramp::level(0.0);
                Value::Void
            }
            "hud_cinematic_fade" => {
                self.st.screen.hud.go(self.st.time, num(0), num(1));
                Value::Void
            }
            "cinematic_set_title" => {
                if let Some(k) = arg(0).index() {
                    if self.st.log {
                        let text = self
                            .scene
                            .ai
                            .titles
                            .get(k as usize)
                            .map(|t| t.text.as_str());
                        let at = self.st.time;
                        println!("script: title {:?} at {at:.1}s", text.unwrap_or("?"));
                    }
                    self.st.screen.title = Some((k as usize, self.st.time));
                }
                Value::Void
            }
            // (objectives_show_up_to n): the first n + 1 are given;
            // finishing marks them done.
            "objectives_show_up_to" => {
                let n = num(0).max(0.0) as usize + 1;
                if self.st.log {
                    println!("script: objectives up to {n} at {:.1}s", self.st.time);
                }
                self.st.objectives.0 = self.st.objectives.0.max(n);
                Value::Void
            }
            "objectives_finish_up_to" => {
                let n = num(0).max(0.0) as usize + 1;
                self.st.objectives.1 = self.st.objectives.1.max(n);
                self.st.objectives.0 = self.st.objectives.0.max(n);
                Value::Void
            }
            "objectives_clear" => {
                self.st.objectives = (0, 0);
                Value::Void
            }
            "physics_set_gravity" => {
                self.st.gravity = num(0).max(0.0);
                Value::Void
            }
            // (objects_attach parent marker child child_marker)
            "objects_attach" => {
                if let (Some(&Obj::Name(parent)), Some(&Obj::Name(child))) =
                    (objects(0).first(), objects(2).first())
                {
                    self.st.attached.insert(child, parent);
                }
                Value::Void
            }
            "objects_detach" => {
                if let Some(&Obj::Name(child)) = objects(1).first() {
                    self.st.attached.remove(&child);
                }
                Value::Void
            }
            "device_group_get" => Value::Real(
                arg(0)
                    .index()
                    .and_then(|g| self.st.device_groups.get(&g))
                    .copied()
                    .unwrap_or(0.0),
            ),
            "device_group_set" | "device_group_set_immediate" => {
                // (device_group_set device group value) or (... group value).
                let (g, v) = if function == "device_group_set" {
                    (arg(1), num(2))
                } else {
                    (arg(0), num(1))
                };
                if let Some(g) = g.index() {
                    self.st.device_groups.insert(g, v);
                }
                Value::Bool(true)
            }
            _ => return None,
        })
    }
}
