//! Computer-controlled players. A bot roams the map along the walking graph
//! (or, in objective games, heads for the flags) until it sees someone, then
//! strafes, shoots, throws grenades and melees like a (forgiving) player.
//! Bots produce the same `Command`s as people.

use crate::collision::World;
use crate::game::{Command, Emblem, Game, GameType, Look, Spartan, VehicleAction, PROFILE_COLORS};
use crate::nav::NavGraph;
use crate::player::GRAVITY;
use crate::weapon::WeaponDef;
use blam_cache::weapon::TriggerBehavior;
use glam::{Vec2, Vec3};

mod actor;
mod arms;
mod ride;
pub use actor::{alert_actors, ActorMind, Scripted};
use ride::Riding;

/// How far a bot sees.
const SIGHT: f32 = 35.0;
/// Camouflaged enemies are noticed within this distance.
const CAMO_NOTICE: f32 = 3.0;
/// Seconds between seeing someone and shooting at them.
const REACTION: f32 = 0.35;
/// Radians per second a bot can turn.
const TURN_RATE: f32 = 5.0;
/// Within this angle of its target a bot pulls the trigger.
const FIRE_CONE: f32 = 0.06;
/// Guns that reach no farther than this (the shotgun) are used up close.
const SHORT_RANGE: f32 = 10.0;
/// Close enough to a route point to head for the next one.
const ARRIVED: f32 = 0.5;
/// Seconds without getting closer to the next route point before a bot
/// gives up on that stretch and finds another way.
const GIVE_UP: f32 = 3.0;
/// Route points a bot remembers it couldn't get to.
const MAX_BLOCKED: usize = 32;
/// An objective that moved this far gets a new route.
const REPLAN_DISTANCE: f32 = 2.0;
/// Defenders keep within this distance of their flag.
const GUARD_RADIUS: f32 = 6.0;
/// How far ahead a bot checks for a floor, and the deepest drop it will
/// walk off.
const LEDGE_LOOKAHEAD: f32 = 0.7;
/// Ground no steeper than this (cosine of the slope) holds a Spartan.
const STANDABLE: f32 = 0.64;
const SAFE_DROP: f32 = 3.0;
/// The share of lives a bot takes vehicles it comes across.
const RIDES: f32 = 0.6;

pub struct Bot {
    rng: u32,
    route: Vec<usize>,
    target: Option<usize>,
    seen_for: f32,
    strafe: f32,
    strafe_left: f32,
    /// Aim wobble that settles while tracking a target.
    aim_error: Vec2,
    yaw: f32,
    pitch: f32,
    last_position: Vec3,
    stuck_for: f32,
    pulse: bool,
    grenade_wait: f32,
    /// The objective the current route leads to.
    heading_for: Option<Vec3>,
    /// Progress toward the next route point: which, the closest the bot
    /// has been, and for how long it hasn't got closer.
    watching: Option<usize>,
    closest: f32,
    no_progress: f32,
    /// Route points this bot tried and couldn't get to.
    blocked: Vec<usize>,
    /// The walking graph has no way to the objective: roam instead.
    no_way: bool,
    riding: Riding,
    /// Seconds before switching weapons again.
    switch_wait: f32,
    /// The weapon being fetched and for how long; ones given up on.
    fetching: Option<(Vec3, f32)>,
    shunned_weapons: Vec<Vec3>,
    /// A campaign actor's character and what it knows.
    pub actor: Option<ActorMind>,
    /// Stood still on purpose last tick (not stuck).
    idle: bool,
}

/// Computer players' names, picked by player number.
const BOT_NAMES: [&str; 16] = [
    "SARGE", "VIPER", "NOMAD", "RAZOR", "HAVOC", "SPECTER", "BLITZ", "RAVEN", "TITAN", "COBRA",
    "WARDEN", "ROOK", "FURY", "DRIFTER", "ONYX", "BANDIT",
];

/// The name a computer player in seat `player` goes by.
pub fn bot_name(player: usize) -> &'static str {
    BOT_NAMES[player % BOT_NAMES.len()]
}

/// A computer player's look, the same every game for the same name: about
/// a third of them are Elites.
pub fn bot_look(player: usize) -> Look {
    let h = bot_name(player).bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    });
    let n = PROFILE_COLORS as u32;
    let primary = (h >> 4) % n;
    let mut secondary = (h >> 12) % n;
    if secondary == primary {
        secondary = (primary + n / 2) % n;
    }
    Look {
        elite: h % 3 == 0,
        colors: [primary as u8, secondary as u8],
        emblem: Emblem::from_number(h.rotate_left(11)),
    }
}

fn wrap(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

/// A direction on the ground as movement (strafe, forward) for someone
/// facing `yaw`.
fn local(yaw: f32, dir: Vec2) -> Vec2 {
    let (s, c) = yaw.sin_cos();
    Vec2::new(dir.x * s - dir.y * c, dir.x * c + dir.y * s)
}

impl Bot {
    /// Where to point `def` to hit player `q` from `eye`: where they'll
    /// be when its rounds get there, and higher for rounds that fall.
    /// Explosive rounds go at the feet, to catch them in the blast.
    pub(super) fn aim_point(def: Option<&WeaponDef>, eye: Vec3, q: &Spartan) -> Vec3 {
        let chest = q.eye() - Vec3::Z * 0.12;
        let Some(f) = def.and_then(|d| d.flight) else {
            return chest + q.body.velocity * 0.1;
        };
        let at = match f.blast {
            Some(_) if q.body.grounded => q.body.position + Vec3::Z * 0.1,
            _ => chest,
        };
        let mut ahead = at;
        let mut t = 0.0;
        for _ in 0..3 {
            let d = eye.distance(ahead);
            let speed = (f.speed_at(0.0) + f.speed_at(d)) * 0.5;
            t = d / speed.max(0.5);
            ahead = at + q.body.velocity * t;
        }
        ahead + Vec3::Z * 0.5 * GRAVITY * f.gravity * t * t
    }

    pub fn new(seed: u32) -> Bot {
        let mut bot = Bot {
            rng: seed.max(1).wrapping_mul(0x9E37_79B9) | 1,
            route: Vec::new(),
            target: None,
            seen_for: 0.0,
            strafe: 0.0,
            strafe_left: 0.0,
            aim_error: Vec2::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            last_position: Vec3::ZERO,
            stuck_for: 0.0,
            pulse: false,
            grenade_wait: 3.0,
            heading_for: None,
            watching: None,
            closest: 0.0,
            no_progress: 0.0,
            blocked: Vec::new(),
            no_way: false,
            riding: Riding::default(),
            switch_wait: 0.0,
            fetching: None,
            shunned_weapons: Vec::new(),
            actor: None,
            idle: false,
        };
        let rides = bot.random() < RIDES;
        bot.riding.reset(rides);
        bot
    }

    /// What the bot is up to, for testing.
    /// In a fight: has someone to shoot at, or knows of an enemy.
    /// Who it's fighting, if anyone.
    pub fn target(&self) -> Option<usize> {
        self.target
    }

    pub fn fighting(&self) -> bool {
        self.target.is_some() || self.actor.as_ref().is_some_and(|a| a.alert.is_some())
    }

    pub fn describe(&self) -> String {
        format!(
            "route {} heading {:?} target {:?} stuck {:.1} blocked {} {}",
            self.route.len(),
            self.heading_for,
            self.target,
            self.stuck_for,
            self.blocked.len(),
            self.riding.describe()
        )
    }

    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn visible(world: &World, from: Vec3, to: Vec3) -> bool {
        let d = to - from;
        let len = d.length();
        len < SIGHT && world.raycast(from, d / len.max(1e-4), len).is_none()
    }

    /// The nearest enemy in sight. Camouflaged ones are only spotted close
    /// up, or when firing gives them away.
    fn find_target(&self, game: &Game, world: &World, me: usize) -> Option<usize> {
        let eye = game.players[me].eye();
        game.players
            .iter()
            .enumerate()
            .filter(|(j, p)| game.is_enemy(me, *j) && p.alive)
            .map(|(j, p)| {
                let sight = (SIGHT * p.visibility()).max(CAMO_NOTICE);
                (j, p.eye().distance(eye), p.eye() - Vec3::Z * 0.1, sight)
            })
            .filter(|&(_, d, chest, sight)| d < sight && Bot::visible(world, eye, chest))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(j, ..)| j)
    }

    /// Where the game sends this bot while no one is in sight, and whether
    /// it should stay around there (guarding) rather than go right up to it.
    fn objective(game: &Game, me: usize) -> Option<(Vec3, bool)> {
        let feet = game.players[me].body.position;
        let team = game.players[me].team;
        if let Some(hill) = game.current_hill() {
            return Some((hill.centre(), false));
        }
        match game.rules.game_type {
            GameType::Juggernaut => {
                return game
                    .juggernaut
                    .filter(|&j| j != me && game.players[j].alive)
                    .map(|j| (game.players[j].body.position, false));
            }
            GameType::Territories => {
                // Spread out over the territories the team doesn't hold.
                let open: Vec<Vec3> = game
                    .territories
                    .iter()
                    .filter(|t| t.owner != Some(team))
                    .map(|t| t.centre())
                    .collect();
                if open.is_empty() {
                    let held = game.territories.get(me % game.territories.len().max(1));
                    return held.map(|t| (t.centre(), true));
                }
                return Some((open[me % open.len()], false));
            }
            _ => {}
        }
        if !game.has_flags() {
            return None;
        }
        if game.rules.game_type.oddball() {
            let ball = game.flags.first()?;
            return match ball.carrier {
                // Holding it: keep moving, away from trouble.
                Some(c) if c == me => None,
                Some(c) if !game.is_enemy(me, c) => Some((ball.position, true)),
                _ => Some((ball.position, false)),
            };
        }
        let assault = game.rules.game_type == GameType::Assault;
        if game.carried_flag(me).is_some() {
            return if assault {
                game.bomb_target(team, feet).map(|b| (b, false))
            } else {
                game.flag_base(team).map(|b| (b, false))
            };
        }
        let own = game.flags.iter().find(|f| f.team == team)?;
        let enemy = game.flags.iter().find(|f| f.team != team)?;
        if assault {
            // An enemy bomb ticking in the base: defuse it.
            if enemy.armed.is_some() {
                return Some((enemy.position, false));
            }
            if me.is_multiple_of(3) {
                return game.flag_base(team).map(|b| (b, true));
            }
            // Take the team's bomb, or go along with whoever has it.
            return Some((own.position, own.carrier.is_some() || own.armed.is_some()));
        }
        if !own.at_home() {
            return Some((own.position, false));
        }
        // A third of the team guards the flag; the rest go for the enemy's.
        if me.is_multiple_of(3) {
            return Some((own.home, true));
        }
        // A teammate has it: go along with them.
        if enemy.carrier.is_some() {
            return Some((enemy.position, true));
        }
        Some((enemy.position, false))
    }

    /// Head for an objective: along the walking graph, then straight to it.
    fn walk_to_objective(
        &mut self,
        nav: &NavGraph,
        world: &World,
        feet: Vec3,
        (goal, guard): (Vec3, bool),
    ) -> Option<Vec3> {
        let near = (goal - feet).truncate().length();
        if guard && near < GUARD_RADIUS {
            // Walk around near the flag.
            if self.route.is_empty() || self.heading_for.is_some() {
                self.heading_for = None;
                self.new_route_near(nav, world, feet, goal);
            }
            return None;
        }
        let moved = self
            .heading_for
            .is_none_or(|h| h.distance(goal) > REPLAN_DISTANCE);
        if moved {
            self.heading_for = Some(goal);
            self.route.clear();
            self.no_way = false;
            if let (Some(a), Some(b)) = (nav.nearest(world, feet), nav.nearest(world, goal)) {
                // Places given up on may be the only way: forget them.
                let route = nav.path_avoiding(a, b, &self.blocked).or_else(|| {
                    self.blocked.clear();
                    nav.path(a, b)
                });
                self.route = route.map(|r| nav.smooth(world, &r)).unwrap_or_default();
                // Somewhere the graph doesn't reach (up on a ledge, say):
                // walking straight at it would only find walls.
                let close = (goal - feet).length() < 2.0;
                let lands_near = nav.points[b].distance(goal) < 2.0;
                self.no_way = !close && (self.route.is_empty() || !lands_near);
                if self.no_way {
                    // Roam (a new route) until the objective moves.
                    self.route.clear();
                }
            }
        }
        if self.no_way {
            return None;
        }
        if self.route.is_empty() {
            return Some(goal);
        }
        None
    }

    /// A route to somewhere within the guard radius of `centre`.
    fn new_route_near(&mut self, nav: &NavGraph, world: &World, from: Vec3, centre: Vec3) {
        self.route.clear();
        let Some(start) = nav.nearest(world, from) else {
            return;
        };
        let close: Vec<usize> = (0..nav.points.len())
            .filter(|&i| nav.points[i].distance(centre) < GUARD_RADIUS)
            .collect();
        if close.is_empty() {
            return;
        }
        let goal = close[(self.random() * close.len() as f32) as usize % close.len()];
        self.route = nav
            .path_avoiding(start, goal, &self.blocked)
            .map(|r| nav.smooth(world, &r))
            .unwrap_or_default();
    }

    /// Pick somewhere to go and plan the way there (one with no
    /// teleporters in a vehicle).
    fn new_route(&mut self, nav: &NavGraph, world: &World, from: Vec3, on_foot: bool) {
        self.route.clear();
        let Some(start) = nav.nearest(world, from) else {
            return;
        };
        for _ in 0..4 {
            let goal = (self.random() * nav.points.len() as f32) as usize;
            let goal = goal.min(nav.points.len() - 1);
            let route = if on_foot {
                nav.path_avoiding(start, goal, &self.blocked)
            } else {
                nav.drive_path(start, goal)
            };
            if let Some(route) = route {
                if route.len() > 1 {
                    self.route = nav.smooth(world, &route);
                    return;
                }
            }
        }
        self.route = vec![start];
    }

    /// Turn toward a direction at the bot's turn rate.
    fn turn_to(&mut self, yaw: f32, pitch: f32, dt: f32) {
        let step = TURN_RATE * dt;
        self.yaw = wrap(self.yaw + wrap(yaw - self.yaw).clamp(-step, step));
        self.pitch += (pitch - self.pitch).clamp(-step, step);
    }

    /// This tick's controls for player `me`.
    pub fn think(&mut self, game: &Game, world: &World, nav: &NavGraph, me: usize) -> Command {
        let dt = crate::game::TICK;
        let p = &game.players[me];
        if !p.alive {
            self.route.clear();
            self.heading_for = None;
            self.watching = None;
            self.blocked.clear();
            self.target = None;
            self.fetching = None;
            self.shunned_weapons.clear();
            self.yaw = p.yaw;
            self.pitch = 0.0;
            let rides = self.random() < RIDES && self.actor.is_none();
            self.riding.reset(rides);
            return Command {
                yaw: p.yaw,
                ..Command::default()
            };
        }
        if let Some(seat) = game.riding(me) {
            return self.ride(game, world, nav, me, seat);
        }
        self.on_foot();
        let feet = p.body.position;
        let eye = p.eye();
        let mut cmd = Command::default();
        // Stuck against something: hop, then give up on the route.
        let moved = (feet - self.last_position).truncate().length();
        self.last_position = feet;
        if moved < 0.4 * dt && !self.idle {
            self.stuck_for += dt;
        } else {
            self.stuck_for = 0.0;
        }
        if self.stuck_for > 0.6 {
            cmd.jump = true;
        }
        if self.stuck_for > 2.0 {
            self.stuck_for = 0.0;
            self.route.clear();
            self.heading_for = None;
        }

        // A flag carrier runs for home, fighting only those in its way.
        let carrying = p.objective.is_some();
        let found = if self.actor.is_some() {
            self.actor_target(game, world, me)
        } else {
            self.find_target(game, world, me)
        };
        // An actor a script sends somewhere goes without stopping.
        let going = self.scripted_going();
        let target = found.filter(|&t| {
            !going && (!carrying || game.players[t].body.position.distance(feet) < 2.5)
        });
        if target != self.target {
            // An actor that loses sight of someone goes to look for them.
            let lost = self
                .target
                .filter(|&t| target.is_none() && game.players[t].alive);
            if let (Some(a), Some(t)) = (&mut self.actor, lost) {
                a.alert = Some((game.players[t].body.position, 0.0));
            }
            self.target = target;
            self.seen_for = 0.0;
            self.aim_error = Vec2::new(self.random() - 0.5, self.random() - 0.5) * 0.4;
        }
        self.strafe_left -= dt;
        if self.strafe_left <= 0.0 {
            self.strafe_left = 0.4 + self.random() * 1.0;
            self.strafe = [-1.0, 0.0, 1.0][(self.random() * 3.0) as usize % 3];
        }
        self.grenade_wait -= dt;
        self.switch_wait -= dt;
        self.pulse = !self.pulse;

        let mut walk_to: Option<Vec3> = None;
        let mut boarding = false;
        // Route links were checked for drops when the graph was made.
        let mut on_route = false;
        if let Some(t) = target {
            self.seen_for += dt;
            if self.actor.is_some() {
                self.actor_aim(dt);
            } else {
                self.aim_error *= 1.0 - (2.5 * dt).min(1.0);
            }
            let q = &game.players[t];
            let def = p.held().and_then(|h| game.weapons.get(h.weapon));
            let to = Bot::aim_point(def, eye, q) - eye;
            let dist = to.length();
            let yaw = to.y.atan2(to.x) + self.aim_error.x;
            let pitch = (to.z / dist.max(1e-4)).asin() + self.aim_error.y;
            self.turn_to(yaw, pitch, dt);
            let off = wrap(yaw - self.yaw).abs() + (pitch - self.pitch).abs();
            let melee_only = carrying
                || p.held()
                    .is_some_and(|h| game.rules.lunge_weapons.contains(&h.weapon));
            // Not a rocket at your own feet.
            let too_close = def
                .and_then(|d| d.flight?.blast)
                .is_some_and(|b| dist < b.radius.1 + 0.5);
            // Actors hold fire beyond their weapon's range.
            let too_far = self
                .actor
                .as_ref()
                .is_some_and(|a| dist > a.mind.fire_range.max(8.0) * 1.25);
            let (actor_fwd, charging) = self.actor_range(dist, dt);
            let melee_only = melee_only || charging;
            if self.seen_for > REACTION && off < FIRE_CONE && !melee_only && !too_close && !too_far
            {
                // Semi-automatic weapons need the trigger released between shots.
                cmd.fire = match def.map(|d| d.behavior) {
                    Some(TriggerBehavior::Spew) | None => true,
                    _ => self.pulse,
                };
                // Dual wielding: the left trigger fires the left gun, the
                // two semi-automatics in turn.
                let left = p.left.as_ref().and_then(|h| game.weapons.get(h.weapon));
                if let Some(l) = left {
                    cmd.throw_grenade = match l.behavior {
                        TriggerBehavior::Spew => true,
                        _ => !self.pulse,
                    };
                }
            }
            self.choose_weapon(game, me, dist, &mut cmd);
            let reach = if melee_only && !carrying { 2.0 } else { 0.9 };
            if dist < reach && self.seen_for > REACTION {
                cmd.melee = self.pulse;
            }
            let has_grenade = p.frags + p.plasmas > 0 && p.left.is_none();
            if self.actor.is_some() {
                cmd.throw_grenade |= has_grenade && self.actor_grenade(dist, dt);
            } else if has_grenade && self.grenade_wait <= 0.0 && (4.0..12.0).contains(&dist) {
                cmd.throw_grenade = true;
                self.grenade_wait = 4.0 + self.random() * 6.0;
            }
            // Close in from afar (right up close with a short-range gun
            // like the shotgun), back off when too close, strafe always.
            let short = def.is_some_and(|d| d.range < SHORT_RANGE);
            let fwd = if self.actor.is_some() {
                actor_fwd
            } else if melee_only || dist > 8.0 || (short && dist > 2.0) {
                1.0
            } else if dist < 2.5 && !short {
                -1.0
            } else {
                0.0
            };
            cmd.movement = Vec2::new(self.strafe, fwd);
            // An actor with an area fights from its firing positions; one a
            // script holds still stays put.
            let held = self.scripted().is_some_and(|s| !s.moving);
            if held {
                cmd.movement = Vec2::ZERO;
            } else if !charging {
                if let Some(at) = self.actor_position(game, world, me, t, dt) {
                    let to = (at - feet).truncate();
                    cmd.movement = if to.length() < ARRIVED {
                        Vec2::ZERO
                    } else {
                        local(self.yaw, to.normalize())
                    };
                }
            }
            if self.held_empty(game, me) {
                cmd.reload = true;
            }
            // An enemy sitting in a slow vehicle close by: board it.
            if !carrying && self.actor.is_none() {
                walk_to = Bot::board_point(game, me, t);
            }
        } else if let Some((v, _, entry)) = self
            .riding
            .rides
            .then(|| self.seat_nearby(game, world, me))
            .flatten()
            .filter(|_| !carrying && Bot::worth_a_ride(game, me))
        {
            // A vehicle to take: go to it, and get in.
            walk_to = Some(entry);
            boarding = Bot::board_now(game, me, v);
        } else {
            // A better weapon lying close by comes first.
            let actor = self.actor.is_some();
            let weapon = if carrying || actor {
                None
            } else {
                self.weapon_to_fetch(game, world, me, dt)
            };
            let objective = if actor {
                self.actor_goal(game, world, me, dt).map(|at| (at, false))
            } else {
                weapon
                    .map(|at| (at, false))
                    .or_else(|| Bot::objective(game, me))
            };
            if let Some(o) = objective {
                walk_to = self.walk_to_objective(nav, world, feet, o);
                if weapon.is_some() && self.no_way {
                    self.shun_weapon(o.0);
                }
            } else {
                self.heading_for = None;
            }
            if self.route.is_empty() && walk_to.is_none() && !actor {
                self.new_route(nav, world, feet, true);
            }
            while let Some(&next) = self.route.first() {
                let point = nav.points[next];
                // A teleporter pad is passed once out of the far side; till
                // then, walk right onto it.
                let hop = self.route.get(1).filter(|&&after| nav.is_hop(next, after));
                let arrived = match hop {
                    Some(&after) => nav.points[after].distance(feet) < 1.0,
                    None => {
                        (point - feet).truncate().length() < ARRIVED
                            && (point.z - feet.z).abs() < 1.0
                    }
                };
                if arrived {
                    self.route.remove(0);
                } else {
                    walk_to = Some(point);
                    on_route = true;
                    self.watch_progress(next, point.distance(feet), dt);
                    break;
                }
            }
            if walk_to.is_none() {
                self.route.clear();
                if let Some((goal, false)) = objective {
                    walk_to = Some(goal);
                }
            }
            // Reload while nothing is around.
            cmd.reload = self.held_partly_empty(game, me);
            // An actor with nowhere to go stands facing its way, or
            // shoots where a script says.
            if let (None, Some(facing)) = (walk_to, self.actor_facing(feet)) {
                match self.scripted().and_then(|s| s.shoot) {
                    Some(at) => {
                        let to = at - eye;
                        let pitch = (to.z / to.length().max(1e-4)).asin();
                        self.turn_to(facing, pitch, dt);
                        cmd.fire = self.pulse;
                    }
                    None => self.turn_to(facing, 0.0, dt),
                }
            }
        }
        if let Some(goal) = walk_to {
            let to = (goal - feet).truncate();
            self.turn_to(to.y.atan2(to.x), 0.0, dt);
            // Walk toward the goal whichever way the bot faces.
            cmd.movement = local(self.yaw, to.normalize_or_zero());
        }
        // An enemy's vehicle in reach: board it.
        boarding |= matches!(game.vehicle_action(me), Some(VehicleAction::Hijack { .. }));
        // Take a flag (or the ball, or the team's bomb) on reaching it,
        // fighting or not, and defuse enemy bombs.
        let team = p.team;
        cmd.action = boarding
            || (0..game.flags.len()).any(|f| {
                let flag = &game.flags[f];
                let takes = game.can_take(me, f) || (flag.armed.is_some() && flag.team != team);
                takes && flag.position.distance(feet) < 1.5
            }) && p.objective.is_none();
        if target.is_none() && !boarding && self.actor.is_none() {
            self.pick_up(game, me, &mut cmd);
        }
        if !on_route {
            let drift = game.players[me].body.velocity.truncate();
            cmd.movement = self.keep_off_ledges(world, feet, drift, cmd.movement);
        }
        // Standing still on purpose isn't being stuck (actors hold their
        // ground while they fight).
        self.idle = cmd.movement == Vec2::ZERO && (target.is_none() || self.actor.is_some());
        cmd.crouch |= self.scripted().is_some_and(|s| s.crouch);
        cmd.yaw = self.yaw;
        cmd.pitch = self.pitch;
        cmd
    }

    /// Give up on a route point the bot isn't getting any closer to, and
    /// remember not to try it again.
    fn watch_progress(&mut self, next: usize, distance: f32, dt: f32) {
        if self.watching != Some(next) || distance < self.closest - 0.25 {
            self.watching = Some(next);
            self.closest = distance;
            self.no_progress = 0.0;
            return;
        }
        self.no_progress += dt;
        if self.no_progress > GIVE_UP {
            if self.blocked.len() >= MAX_BLOCKED {
                self.blocked.remove(0);
            }
            self.blocked.push(next);
            self.route.clear();
            self.heading_for = None;
            self.watching = None;
        }
    }

    /// Don't walk (or strafe) off a drop that would hurt: where the floor a
    /// step ahead is missing or far below, stop going that way.
    fn keep_off_ledges(&mut self, world: &World, feet: Vec3, drift: Vec2, movement: Vec2) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        let (forward, right) = (Vec2::new(c, s), Vec2::new(s, -c));
        // Height of ground a Spartan can stand on (not a steep slope that
        // slides it off) at a spot.
        let floor_at = |at: Vec2| {
            let top = at.extend(feet.z + 0.5);
            world
                .raycast_hit(top, Vec3::NEG_Z, 0.5 + SAFE_DROP)
                .filter(|(_, n)| n.z >= STANDABLE)
                .map(|(t, _)| top.z - t)
        };
        // Ground just ahead; off a drop, room to land beyond it too.
        let safe = |dir: Vec2| {
            let (from, dir) = (feet.truncate(), dir.normalize_or_zero());
            let mut low = feet.z;
            for k in [0.5, 1.0] {
                match floor_at(from + dir * LEDGE_LOOKAHEAD * k) {
                    Some(z) => low = low.min(z),
                    None => return false,
                }
            }
            feet.z - low < 0.6 || (2..=5).all(|k| floor_at(from + dir * (k as f32 * 0.5)).is_some())
        };
        // Already sliding toward a drop: brake.
        if drift.length() > 0.5 && !safe(drift) {
            let back = -drift.normalize();
            return Vec2::new(back.dot(right), back.dot(forward));
        }
        let mut m = movement;
        if m.y != 0.0 && !safe(forward * m.y) {
            m.y = 0.0;
        }
        if m.x != 0.0 && !safe(right * m.x) {
            m.x = 0.0;
            self.strafe = -self.strafe;
        }
        if m != Vec2::ZERO && !safe(forward * m.y + right * m.x) {
            m = Vec2::ZERO;
        }
        m
    }

    fn held_empty(&self, game: &Game, me: usize) -> bool {
        let p = &game.players[me];
        p.held().is_some_and(|h| {
            game.weapons
                .get(h.weapon)
                .is_some_and(|d| d.uses_ammo() && h.state.loaded == 0 && h.state.reserve > 0)
        })
    }

    fn held_partly_empty(&self, game: &Game, me: usize) -> bool {
        let p = &game.players[me];
        p.held().is_some_and(|h| {
            game.weapons.get(h.weapon).is_some_and(|d| {
                d.uses_ammo() && h.state.loaded < d.magazine_size && h.state.reserve > 0
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bots_turn_toward_and_shoot_players_they_see() {
        let world = crate::game::tests::floor();
        let mut game = crate::game::tests::game();
        let a = game.add_player();
        let b = game.add_player();
        game.players[a].body.position = Vec3::new(0.0, 0.0, 0.0);
        game.players[b].body.position = Vec3::new(5.0, 3.0, 0.0);
        let nav = NavGraph::build(&world, &[Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)]);
        let mut bot = Bot::new(7);
        let mut fired = false;
        let mut hurt = false;
        for _ in 0..120 {
            let cmd = bot.think(&game, &world, &nav, a);
            fired |= cmd.fire;
            let mut cmds = vec![Command::default(); 2];
            cmds[a] = cmd;
            game.step(&world, &cmds);
            // Keep the target standing still.
            game.players[b].body.position = Vec3::new(5.0, 3.0, 0.0);
            hurt |= game.players[b].shield < 70.0 || !game.players[b].alive;
        }
        assert!(fired);
        assert!(hurt);
    }

    #[test]
    fn bots_on_both_teams_fight() {
        let world = crate::game::tests::floor();
        let mut game = crate::game::tests::game();
        game.rules.game_type = crate::GameType::TeamSlayer;
        game.rules.score_to_win = 0;
        let nav = NavGraph::build(&world, &[Vec3::ZERO, Vec3::new(8.0, 0.0, 0.0)]);
        let mut bots: Vec<(usize, Bot)> = (0..4)
            .map(|k| (game.add_player(), Bot::new(k * 31 + 5)))
            .collect();
        for (i, _) in &bots {
            let x = if game.players[*i].team == 0 { 0.0 } else { 8.0 };
            game.players[*i].body.position = Vec3::new(x, *i as f32, 0.0);
        }
        for _ in 0..60 * 60 {
            let mut cmds = vec![Command::default(); game.players.len()];
            for (i, bot) in &mut bots {
                cmds[*i] = bot.think(&game, &world, &nav, *i);
            }
            game.step(&world, &cmds);
        }
        let kills = |t: u8| -> u32 {
            game.players
                .iter()
                .filter(|p| p.team == t)
                .map(|p| p.kills)
                .sum()
        };
        assert!(kills(0) > 0 && kills(1) > 0);
    }

    #[test]
    fn bots_capture_flags() {
        let world = crate::game::tests::floor();
        let mut game = crate::game::tests::game();
        game.rules.game_type = crate::GameType::Ctf;
        game.weapons.push(game.weapons[0].clone());
        game.rules.flag_weapon = Some(game.weapons.len() - 1);
        game.set_flags(
            &[
                (0, Vec3::new(-12.0, 0.0, 0.0)),
                (1, Vec3::new(12.0, 0.0, 0.0)),
            ],
            &[],
        );
        let points: Vec<Vec3> = (-6..=6)
            .flat_map(|x| (-2..=2).map(move |y| Vec3::new(x as f32 * 2.0, y as f32 * 2.0, 0.0)))
            .collect();
        let nav = NavGraph::build(&world, &points);
        // A red attacker (player 1; player 0 would guard) against no one.
        game.add_player_on(0);
        let me = game.add_player_on(0);
        let mut bot = Bot::new(3);
        let mut taken = false;
        for _ in 0..60 * 30 {
            let mut cmds = vec![Command::default(); game.players.len()];
            cmds[me] = bot.think(&game, &world, &nav, me);
            game.step(&world, &cmds);
            taken |= game.flags[1].carrier == Some(me);
            if game.team_score(0) > 0 {
                break;
            }
        }
        assert!(taken, "the bot didn't take the flag");
        assert_eq!(game.team_score(0), 1, "the bot didn't score");
    }

    #[test]
    fn bots_drive_vehicles_they_come_across() {
        let world = crate::game::tests::floor();
        let mut game = crate::game::tests::game();
        game.set_vehicles(
            vec![crate::vehicle::tests::jeep()],
            vec![crate::game::VehicleSpawn {
                def: 0,
                position: Vec3::new(4.0, 0.0, 0.05),
                yaw: 0.0,
                respawn: 30.0,
            }],
        );
        let points: Vec<Vec3> = (-8..=8)
            .flat_map(|x| (-8..=8).map(move |y| Vec3::new(x as f32 * 3.0, y as f32 * 3.0, 0.0)))
            .collect();
        let nav = NavGraph::build(&world, &points);
        let me = game.add_player();
        game.players[me].body.position = Vec3::new(1.0, 2.0, 0.0);
        let mut bot = Bot::new(3);
        bot.riding.reset(true);
        let mut drove = 0.0;
        for _ in 0..60 * 20 {
            let cmd = bot.think(&game, &world, &nav, me);
            game.step(&world, &[cmd]);
            if game.riding(me) == Some((0, 0)) && game.vehicles[0].speed() > 2.0 {
                drove += crate::game::TICK;
            }
        }
        assert!(drove > 3.0, "drove for {drove} s");
        assert!(game.vehicles[0].up().z > 0.8, "kept it on its wheels");
    }

    #[test]
    fn bots_fight_without_falling_off_ledges() {
        // A 6x6 platform over nothing.
        let world = World::new(
            &[[-3., -3., 0.], [3., -3., 0.], [3., 3., 0.], [-3., 3., 0.]],
            &[0, 1, 2, 0, 2, 3],
        );
        let mut game = crate::game::tests::game();
        game.rules.score_to_win = 0;
        game.spawns = vec![
            (Vec3::new(-2.0, 0.0, 0.0), 0.0),
            (Vec3::new(2.0, 0.0, 0.0), 0.0),
        ];
        let nav = NavGraph::for_level(&world, &[Vec3::ZERO], &[], &[]);
        let mut bots: Vec<(usize, Bot)> = (0..2)
            .map(|k| (game.add_player(), Bot::new(k * 17 + 3)))
            .collect();
        game.players[0].body.position = Vec3::new(-2.0, 0.0, 0.0);
        game.players[1].body.position = Vec3::new(2.0, 0.0, 0.0);
        for _ in 0..60 * 30 {
            let mut cmds = vec![Command::default(); 2];
            for (i, bot) in &mut bots {
                cmds[*i] = bot.think(&game, &world, &nav, *i);
            }
            game.step(&world, &cmds);
            for e in &game.events {
                if let crate::game::Event::Killed { killer: None, .. } = e {
                    panic!("a bot fell off");
                }
            }
        }
        assert!(game.players.iter().map(|p| p.kills).sum::<u32>() > 0);
    }

    #[test]
    fn bots_take_teleporters_to_get_somewhere() {
        use crate::game::{ItemKind, ItemSpawn, Powerup, Teleporter};
        let world = crate::nav::tests::islands();
        let pads = [
            Teleporter {
                entry: Vec3::new(2.0, 0.0, 0.35),
                exit: Vec3::new(10.0, 0.0, 0.35),
            },
            Teleporter {
                entry: Vec3::new(14.0, 0.0, 0.35),
                exit: Vec3::new(-2.0, 0.0, 0.35),
            },
        ];
        let overshield = Vec3::new(12.0, 1.0, 0.0);
        let nav = NavGraph::for_level(&world, &[Vec3::new(-1.0, 1.0, 0.0), overshield], &[], &pads);
        let mut game = crate::game::tests::game();
        game.teleporters = pads.to_vec();
        game.item_spawns = vec![ItemSpawn {
            kind: ItemKind::Powerup(Powerup::Overshield),
            position: overshield,
            respawn: 1000.0,
        }];
        game.item_timers = vec![0.0];
        let me = game.add_player();
        game.players[me].body.position = Vec3::new(-1.0, 1.0, 0.0);
        let mut bot = Bot::new(11);
        for _ in 0..60 * 10 {
            let cmd = bot.think(&game, &world, &nav, me);
            game.step(&world, &[cmd]);
            assert!(game.players[me].alive, "fell off");
            if game.item_timers[0] > 0.0 {
                return;
            }
        }
        panic!("never got there: at {}", game.players[me].body.position);
    }
}
