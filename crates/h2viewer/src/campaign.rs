//! Campaign missions in play: the mission's scripts running, placing the
//! scenario's squads as actors when the player gets to them, opening doors
//! and following the player from one part of the level to the next.

use crate::scene::Scene;
use blam_cache::ai::AiTeam;
use glam::Vec3;
use h2sim::bot::ActorMind;
use h2sim::game::ActorSpawn;
use h2sim::script::{Host, Obj, Value, Vm};
use h2sim::{Bot, Game};
use std::collections::HashMap;

/// Seconds a machine takes to open when its tag doesn't say.
const MACHINE_TIME: f32 = 1.5;
/// How far an actor told to see someone knows where they are.
const SEE_RANGE: f32 = 60.0;

/// The team of a squad that says which side it's on.
fn squad_team(team: AiTeam) -> Option<u8> {
    match team {
        AiTeam::Player | AiTeam::Human => Some(0),
        AiTeam::Covenant | AiTeam::Prophet => Some(1),
        AiTeam::Flood => Some(2),
        AiTeam::Sentinel => Some(3),
        AiTeam::Heretic => Some(4),
        AiTeam::Default | AiTeam::Other(_) => None,
    }
}

/// Place a squad's actors at its starting locations (as many as the
/// difficulty calls for); returns them and their bots (none for a
/// braindead squad). Allies are those on `players_team`.
pub fn place_squad(
    game: &mut Game,
    scene: &Scene,
    squad: usize,
    difficulty: u8,
    players_team: u8,
    limit: Option<usize>,
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
    let weapon = |i: Option<u16>| i.and_then(|i| ai.weapons.get(i as usize).copied().flatten());
    let mut out = Vec::new();
    let count = if limit.is_some() {
        count
    } else {
        count.max(always)
    };
    for &k in order.iter().take(count) {
        let l = &s.locations[k];
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
            position: l.position.into(),
            yaw: l.facing,
            weapon: weapon(l.weapon.or(s.weapon)),
            secondary: weapon(l.secondary.or(s.secondary)),
            difficulty,
            team: squad_team(s.team),
        };
        let Some(i) = game.spawn_actor(spawn) else {
            continue;
        };
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
}

impl Default for Device {
    fn default() -> Device {
        Device {
            position: 0.0,
            target: 0.0,
            power: 1.0,
            automatic: false,
        }
    }
}

/// The level as the mission's scripts have left it.
#[derive(Default)]
struct State {
    /// The structure BSP the players are in.
    bsp: u16,
    devices: HashMap<u16, Device>,
    device_groups: HashMap<u16, f32>,
    /// Named objects scripts created (true) or destroyed (false).
    created: HashMap<u16, bool>,
    /// Actors the scripts placed in each squad.
    placed: HashMap<usize, u32>,
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
            bsp: start_bsp(scene, game),
            difficulty,
            log,
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
                ctx.place(s, None);
            }
        }
        let mut vm = Vm::new(&scene.ai.scripts, &mut ctx);
        vm.log = log;
        Mission {
            vm,
            state,
            ticks: 0,
        }
    }

    /// One game tick: the players may cross into another part of the
    /// level, machines move, and every other tick the scripts run.
    pub fn step(&mut self, scene: &Scene, game: &mut Game, bots: &mut Vec<(usize, Bot)>) {
        self.ticks += 1;
        if self.ticks % 2 == 1 {
            return;
        }
        let dt = 1.0 / h2sim::script::TICKS_PER_SECOND as f32;
        let mut ctx = Ctx {
            st: &mut self.state,
            scene,
            game,
            bots,
        };
        ctx.follow_bsp();
        ctx.move_devices(dt);
        self.vm.tick(&scene.ai.scripts, &mut ctx);
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
struct Ctx<'a> {
    st: &'a mut State,
    scene: &'a Scene,
    game: &'a mut Game,
    bots: &'a mut Vec<(usize, Bot)>,
}

impl Ctx<'_> {
    /// Place a squad's actors (at most `limit`), with bots to run them.
    fn place(&mut self, squad: usize, limit: Option<usize>) {
        let placed = place_squad(self.game, self.scene, squad, self.st.difficulty, 0, limit);
        if self.st.log {
            let name = &self.scene.ai.squads[squad].name;
            println!("script: placed {name} ({})", placed.len());
        }
        *self.st.placed.entry(squad).or_insert(0) += placed.len() as u32;
        self.bots
            .retain(|(i, _)| !placed.iter().any(|(j, _)| j == i));
        self.bots
            .extend(placed.into_iter().filter_map(|(i, b)| Some((i, b?))));
    }

    /// The squads an `ai` value means: a squad, or every squad in a group.
    fn squads(&self, ai: &Value) -> Vec<usize> {
        let Some(h) = ai.handle() else {
            return Vec::new();
        };
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

    /// Where an object is, if it's in the level.
    fn position(&self, o: Obj) -> Option<Vec3> {
        match o {
            Obj::Unit(i) => self
                .game
                .players
                .get(i)
                .filter(|p| p.alive)
                .map(|p| p.body.position),
            Obj::Name(n) => {
                if self.st.created.get(&n) == Some(&false) {
                    return None;
                }
                self.scene
                    .ai
                    .name_positions
                    .get(n as usize)
                    .copied()
                    .flatten()
            }
        }
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

    fn device(&mut self, v: Option<&Value>) -> Option<&mut Device> {
        let name = match v?.objects().first()? {
            Obj::Name(n) => *n,
            Obj::Unit(_) => return None,
        };
        Some(self.st.devices.entry(name).or_default())
    }

    /// The vitality left of the actors placed in an `ai`'s squads, from 1
    /// (all fresh) to 0 (all dead).
    fn strength(&self, ai: &Value) -> f32 {
        let squads = self.squads(ai);
        let placed: u32 = squads.iter().filter_map(|s| self.st.placed.get(s)).sum();
        if placed == 0 {
            return 0.0;
        }
        let left: f32 = self
            .actors(ai)
            .iter()
            .map(|&i| {
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
                Obj::Unit(_) => {}
            }
        }
    }
}

impl Host for Ctx<'_> {
    fn call(&mut self, function: &str, args: &[Value], _returns: u16) -> Option<Value> {
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
                        Obj::Name(_) => false,
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
            // Cutscenes aren't played yet: the scripts skip them as if
            // the player had.
            "game_reverted" => Value::Bool(true),
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
                for s in self.squads(&arg(0)) {
                    self.place(s, limit);
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
                    self.game.erase_actor(i);
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
            "ai_cannot_die" => {
                let on = arg(1).truthy();
                for i in self.actors(&arg(0)) {
                    if let Some(a) = &mut self.game.players[i].actor {
                        a.immortal = on;
                    }
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
            "object_create" | "object_create_anew" => {
                self.set_created(&arg(0), true);
                Value::Void
            }
            "object_destroy" => {
                self.set_created(&arg(0), false);
                Value::Void
            }
            "object_create_containing"
            | "object_create_anew_containing"
            | "object_destroy_containing" => {
                let made = !function.starts_with("object_destroy");
                for n in self.names_containing(&arg(0)) {
                    self.st.created.insert(n, made);
                }
                Value::Void
            }
            // Doors, lifts and the like.
            "device_set_position" | "device_set_position_immediate" => {
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
