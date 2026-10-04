//! Command scripts: scripts that take charge of one actor each
//! (`cs_run_command_script`), sending it somewhere, turning it to face
//! something, crouching it or having it shoot at a point, until the script
//! ends or the fight it was told to stop for comes to it.

use super::Ctx;
use glam::Vec3;
use h2sim::bot::Scripted;
use h2sim::script::{Value, Vm};
use std::collections::HashMap;

/// Seconds an actor has to get where a command sends it, and to get any
/// closer, before the script stops waiting.
const GIVE_UP: f32 = 60.0;
const STALLED: f32 = 4.0;
/// Close enough to a point to be there.
const THERE: f32 = 1.0;
/// `ai` handles for one actor (the low bits its player index), and for
/// the actor placed at one of a squad's starting locations (squad in bits
/// 16-29, location in the low 16).
pub(super) const AI_ACTOR: u32 = 2 << 30;
pub(super) const AI_LOCATION: u32 = 3 << 30;
/// Combat status (as `cs_abort_on_combat_status` counts it) of an actor
/// fighting, of one searching, and of one at ease.
pub(super) const ENGAGED: i16 = 8;
pub(super) const SEARCHING: i16 = 4;
pub(super) const IDLE: i16 = 1;
/// The levels' names (`ai_combat_status_alert`...), from 0.
const STATUS_NAMES: [&str; 10] = [
    "asleep",
    "idle",
    "alert",
    "active",
    "uninspected",
    "definite",
    "certain",
    "visible",
    "clear_los",
    "dangerous",
];

#[derive(Debug, Default)]
pub(super) struct Commands {
    /// The actor the script running now commands.
    pub current: Option<usize>,
    /// Command scripts to start (actor, script, queued) and actors whose
    /// scripts to stop, once this tick's scripts have run.
    starts: Vec<(usize, usize, bool)>,
    stops: Vec<usize>,
    /// What each commanded actor is doing.
    actors: HashMap<usize, Command>,
    /// The call just made is still being carried out.
    pub waiting: bool,
}

#[derive(Debug, Clone, Default)]
struct Command {
    scripted: Scripted,
    /// Seconds since it was sent where it's going, the closest it's got,
    /// and seconds since it last got closer.
    going: f32,
    closest: f32,
    stalled: f32,
    /// Its script stops once its combat status gets this high, or once
    /// it's hurt (below this health and shield).
    abort_status: Option<i16>,
    abort_hurt: Option<f32>,
}

/// How roused an actor is.
pub(super) fn combat_status(b: &h2sim::Bot) -> i16 {
    if b.target().is_some() {
        ENGAGED
    } else if b.fighting() {
        SEARCHING
    } else {
        IDLE
    }
}

impl Ctx<'_> {
    /// A point scripts name (point set in the high 16 bits, point in the
    /// low).
    fn point(&self, v: &Value) -> Option<Vec3> {
        let h = v.handle()?;
        let set = self.scene.ai.point_sets.get((h >> 16) as usize)?;
        let p = set.points.get((h & 0xFFFF) as usize)?;
        Some(Vec3::from(p.position))
    }

    /// The actor the running command script commands, if it's alive.
    fn commanded(&self) -> Option<usize> {
        self.st
            .commands
            .current
            .filter(|&i| self.game.players.get(i).is_some_and(|p| p.alive))
    }

    fn command_mut(&mut self, actor: usize) -> &mut Command {
        self.st.commands.actors.entry(actor).or_default()
    }

    /// The nearest player to an actor.
    fn nearest_human(&self, actor: usize) -> Option<Vec3> {
        let at = self.game.players[actor].body.position;
        self.humans()
            .into_iter()
            .map(|h| self.game.players[h].body.position)
            .min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)))
    }

    /// Send the commanded actor somewhere and wait till it's there
    /// (within `near`), or till it gives up.
    fn go(&mut self, actor: usize, to: Vec3, near: f32) -> Value {
        let feet = self.game.players[actor].body.position;
        let c = self.command_mut(actor);
        if c.scripted.go_to.is_none_or(|g| g.distance(to) > 0.5) {
            c.scripted.go_to = Some(to);
            c.going = 0.0;
            c.closest = f32::INFINITY;
            c.stalled = 0.0;
        }
        let there =
            (to - feet).truncate().length() < near.max(THERE) && (to.z - feet.z).abs() < 2.0;
        if there || c.going > GIVE_UP || c.stalled > STALLED {
            c.scripted.go_to = None;
            if self.st.log {
                let how = if there { "got to" } else { "gave up on" };
                println!("command: {actor} {how} {to:.1}");
            }
            return Value::Void;
        }
        self.st.commands.waiting = true;
        Value::Void
    }

    /// The `cs_` functions, and starting command scripts; `None` for any
    /// other function.
    pub(super) fn command_call(&mut self, function: &str, args: &[Value]) -> Option<Value> {
        let arg = |k: usize| args.get(k).cloned().unwrap_or_default();
        let on = |k: usize| args.get(k).is_none_or(Value::truthy);
        match function {
            "cs_run_command_script" | "cs_queue_command_script" | "cs_stack_command_script" => {
                let queue = function == "cs_queue_command_script";
                if let Some(script) = arg(1).index() {
                    for i in self.actors(&arg(0)) {
                        self.st.commands.starts.push((i, script as usize, queue));
                    }
                }
                return Some(Value::Void);
            }
            "cs_command_script_running" | "cs_command_script_queued" => {
                return Some(Value::Bool(false));
            }
            "cs_number_queued" => return Some(Value::Real(0.0)),
            _ if !function.starts_with("cs_") => return None,
            _ => {}
        }
        let Some(actor) = self.commanded() else {
            return Some(Value::Void);
        };
        let here = self.game.players[actor].body.position;
        Some(match function {
            "cs_go_to" | "cs_go_by" | "cs_go_to_and_face" | "cs_go_to_and_posture" => {
                let near = if function == "cs_go_to" && args.len() > 1 {
                    arg(1).num()
                } else {
                    THERE
                };
                if function == "cs_go_to_and_face" {
                    self.command_mut(actor).scripted.face = self.point(&arg(1));
                }
                match self.point(&arg(0)) {
                    Some(to) => self.go(actor, to, near),
                    None => Value::Void,
                }
            }
            "cs_go_to_nearest" => {
                let set = arg(0).handle().map(|h| (h >> 16) as usize);
                let nearest = set
                    .and_then(|s| self.scene.ai.point_sets.get(s))
                    .and_then(|s| {
                        s.points
                            .iter()
                            .map(|p| Vec3::from(p.position))
                            .min_by(|a, b| a.distance(here).total_cmp(&b.distance(here)))
                    });
                match nearest {
                    Some(to) => self.go(actor, to, THERE),
                    None => Value::Void,
                }
            }
            "cs_approach_player" | "cs_approach" => {
                let to = if function == "cs_approach" {
                    arg(0).objects().first().and_then(|&o| self.position(o))
                } else {
                    self.nearest_human(actor)
                };
                let near = if function == "cs_approach" {
                    arg(1).num()
                } else {
                    arg(0).num()
                };
                match to {
                    Some(to) => self.go(actor, to, near),
                    None => Value::Void,
                }
            }
            "cs_start_to" => {
                let to = self.point(&arg(0));
                self.command_mut(actor).scripted.go_to = to;
                Value::Void
            }
            "cs_moving" => Value::Bool(
                self.st
                    .commands
                    .actors
                    .get(&actor)
                    .is_some_and(|c| c.scripted.go_to.is_some()),
            ),
            "cs_face" | "cs_look" | "cs_aim" => {
                let at = on(0).then(|| self.point(&arg(1))).flatten();
                self.command_mut(actor).scripted.face = at;
                Value::Void
            }
            "cs_face_player" | "cs_look_player" | "cs_aim_player" => {
                let at = on(0).then(|| self.nearest_human(actor)).flatten();
                self.command_mut(actor).scripted.face = at;
                Value::Void
            }
            "cs_face_object" | "cs_look_object" | "cs_aim_object" => {
                let at = on(0)
                    .then(|| arg(1).objects().first().and_then(|&o| self.position(o)))
                    .flatten();
                self.command_mut(actor).scripted.face = at;
                Value::Void
            }
            "cs_shoot_point" => {
                let at = on(0).then(|| self.point(&arg(1))).flatten();
                self.command_mut(actor).scripted.shoot = at;
                Value::Void
            }
            "cs_shoot" => {
                let at = on(0)
                    .then(|| {
                        arg(1)
                            .objects()
                            .first()
                            .and_then(|&o| self.position(o))
                            .map(|p| p + Vec3::Z * 0.5)
                    })
                    .flatten();
                self.command_mut(actor).scripted.shoot = at;
                Value::Void
            }
            "cs_crouch" => {
                self.command_mut(actor).scripted.crouch = on(0);
                Value::Void
            }
            "cs_enable_moving" => {
                self.command_mut(actor).scripted.moving = on(0);
                Value::Void
            }
            "cs_abort_on_combat_status" => {
                self.command_mut(actor).abort_status = Some(arg(0).num() as i16);
                Value::Void
            }
            "cs_abort_on_alert" => {
                if on(0) {
                    self.command_mut(actor).abort_status = Some(SEARCHING);
                }
                Value::Void
            }
            "cs_abort_on_damage" => {
                let p = &self.game.players[actor];
                let left = p.health + p.shield;
                self.command_mut(actor).abort_hurt = on(0).then_some(left);
                Value::Void
            }
            "cs_teleport" => {
                if let Some(to) = self.point(&arg(0)) {
                    let p = &mut self.game.players[actor];
                    p.body.position = to;
                    p.body.velocity = Vec3::ZERO;
                }
                Value::Void
            }
            // Vehicles, flying, dialogue and the rest: not yet.
            _ => Value::Void,
        })
    }

    /// `ai_current_actor` and `ai_current_squad`, and the combat status
    /// levels.
    pub(super) fn current_ai(&self, name: &str) -> Option<Value> {
        let none = Value::Handle(u32::MAX);
        if let Some(level) = name.strip_prefix("ai_combat_status_") {
            let k = STATUS_NAMES.iter().position(|&n| n == level)?;
            return Some(Value::Real(k as f32));
        }
        match name {
            "ai_current_actor" => Some(
                self.st
                    .commands
                    .current
                    .map_or(none, |i| Value::Handle(AI_ACTOR | i as u32)),
            ),
            "ai_current_squad" => Some(
                self.st
                    .commands
                    .current
                    .and_then(|i| self.game.players.get(i)?.actor)
                    .map_or(none, |a| Value::Handle(u32::from(a.squad))),
            ),
            _ => None,
        }
    }

    /// After the scripts have run: start and stop command scripts, end
    /// those whose actors died or were told to stop for a fight, and give
    /// the actors what their scripts have them do.
    pub(super) fn run_commands(&mut self, vm: &mut Vm, dt: f32) {
        let scripts = &self.scene.ai.scripts;
        for i in std::mem::take(&mut self.st.commands.stops) {
            vm.stop_command(i as u32);
        }
        for (i, script, queue) in std::mem::take(&mut self.st.commands.starts) {
            if self.game.players.get(i).is_some_and(|p| p.alive) {
                if self.st.log {
                    let name = scripts.scripts.get(script).map_or("?", |s| s.name.as_str());
                    println!("command: {i} runs {name}");
                }
                vm.command(scripts, script, i as u32, queue);
                if !queue {
                    self.st.commands.actors.insert(i, Command::default());
                }
            }
        }
        let status: HashMap<usize, i16> = self
            .bots
            .iter()
            .map(|(i, b)| (*i, combat_status(b)))
            .collect();
        let game = &*self.game;
        let log = self.st.log;
        self.st.commands.actors.retain(|&i, c| {
            let p = &game.players[i];
            let hurt = c.abort_hurt.is_some_and(|h| p.health + p.shield < h - 0.01);
            let fighting = c
                .abort_status
                .is_some_and(|s| status.get(&i).is_some_and(|&now| now >= s));
            let keep = p.alive && vm.commanding(i as u32) && !hurt && !fighting;
            if !keep {
                if log && (hurt || fighting) {
                    println!("command: {i} stops to fight");
                }
                vm.stop_command(i as u32);
            }
            c.going += dt;
            if let Some(to) = c.scripted.go_to {
                let d = to.distance(p.body.position);
                if d < c.closest - 0.1 {
                    c.closest = d;
                    c.stalled = 0.0;
                } else {
                    c.stalled += dt;
                }
            }
            keep
        });
        for (i, b) in self.bots.iter_mut() {
            if let Some(mind) = &mut b.actor {
                mind.scripted = self.st.commands.actors.get(i).map(|c| c.scripted);
            }
        }
    }

    /// The actor an `ai` handle names directly (one actor, or the one at
    /// a starting location), if it does.
    pub(super) fn ai_actors(&self, h: u32) -> Option<Vec<usize>> {
        let alive = |i: usize| self.game.players.get(i).is_some_and(|p| p.alive);
        match h & (3 << 30) {
            AI_ACTOR => Some(
                Some((h & 0xFFFF) as usize)
                    .filter(|&i| alive(i) && self.game.players[i].actor.is_some())
                    .into_iter()
                    .collect(),
            ),
            AI_LOCATION => {
                let squad = ((h >> 16) & 0x3FFF) as u16;
                let location = (h & 0xFFFF) as u16;
                Some(
                    (0..self.game.players.len())
                        .filter(|&i| {
                            alive(i)
                                && self.game.players[i]
                                    .actor
                                    .is_some_and(|a| a.squad == squad && a.location == location)
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }
}
