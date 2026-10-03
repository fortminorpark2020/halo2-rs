//! A multiplayer game: Spartans with shields and health, two-weapon
//! inventories, grenades, items lying on the map, damage, deaths, respawns
//! and the score. Rendering and input live elsewhere; each tick takes one
//! command per player, so local splitscreen and networked players drive the
//! game the same way.

use crate::collision::World;
use crate::player::{Input, Player};
use crate::weapon::{WeaponDef, WeaponInput, WeaponState};
use blam_cache::physics::{BipedPhysics, PlayerMovement};
use glam::{Vec2, Vec3};

mod sync;
pub use sync::{Malformed, Reader, Writer};

/// Simulation step: the game advances in fixed ticks so every machine in a
/// networked game computes the same thing.
pub const TICK: f32 = 1.0 / 60.0;

/// Gravity for thrown grenades, world units per second squared.
const GRENADE_GRAVITY: f32 = crate::player::GRAVITY;
/// How far in front of the eye a melee reaches.
const MELEE_RANGE: f32 = 0.9;
/// The energy sword's lunge reaches further.
const LUNGE_RANGE: f32 = 2.2;
/// Melee needs the target within this angle of where the player looks.
const MELEE_CONE: f32 = 0.6;
const MELEE_COOLDOWN: f32 = 0.8;
const PICKUP_RADIUS: f32 = 0.45;
/// Hold the action key this long to swap weapons.
const SWAP_HOLD: f32 = 0.25;
const DROPPED_WEAPON_LIFETIME: f32 = 60.0;
const GRENADE_THROW_COOLDOWN: f32 = 0.6;
/// Below this distance from the top of the body, a hit is a headshot.
const HEAD_HEIGHT: f32 = 0.16;
/// Seconds between kills that still chain into a multi-kill.
const MULTI_KILL_WINDOW: f64 = 4.0;
/// A killing spree medal every this many kills without dying.
const SPREE_STEP: u32 = 5;
/// The last spree medal.
const SPREE_MAX: u32 = 25;

/// Damage values and timings, from the game's tags where the map has them.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub shield: f32,
    pub health: f32,
    /// Seconds without damage before shields start to recharge.
    pub shield_delay: f32,
    /// Seconds for empty shields to fill.
    pub shield_recharge: f32,
    pub respawn_time: f32,
    /// Weapons (indexes into the weapon list) every player spawns with.
    pub starting_weapons: Vec<usize>,
    pub starting_frags: u8,
    pub starting_plasmas: u8,
    pub max_grenades: u8,
    pub melee_damage: f32,
    pub lunge_damage: f32,
    pub frag: GrenadeDef,
    pub plasma: GrenadeDef,
    /// Weapons whose headshots kill once shields are down.
    pub headshot_weapons: Vec<usize>,
    /// Weapons that lunge and kill with melee (the energy sword).
    pub lunge_weapons: Vec<usize>,
    /// Kills to win; 0 plays forever.
    pub score_to_win: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrenadeDef {
    /// Throw speed, world units per second.
    pub speed: f32,
    /// Seconds from the first bounce (or sticking) to the explosion.
    pub fuse: f32,
    pub sticks: bool,
    pub damage: f32,
    /// Full damage inside the first radius, none beyond the second.
    pub radius: (f32, f32),
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            shield: 70.0,
            health: 45.0,
            shield_delay: 5.0,
            shield_recharge: 2.0,
            respawn_time: 5.0,
            starting_weapons: Vec::new(),
            starting_frags: 2,
            starting_plasmas: 0,
            max_grenades: 4,
            melee_damage: 60.0,
            lunge_damage: 150.0,
            frag: GrenadeDef {
                speed: 7.0,
                fuse: 1.0,
                sticks: false,
                damage: 150.0,
                radius: (0.75, 1.75),
            },
            plasma: GrenadeDef {
                speed: 7.0,
                fuse: 1.5,
                sticks: true,
                damage: 120.0,
                radius: (0.75, 1.5),
            },
            headshot_weapons: Vec::new(),
            lunge_weapons: Vec::new(),
            score_to_win: 25,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// Index into the game's weapon list.
    Weapon(usize),
    FragGrenades,
    PlasmaGrenades,
}

/// A place items appear on the map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemSpawn {
    pub kind: ItemKind,
    pub position: Vec3,
    pub respawn: f32,
}

/// One player's controls for a tick.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Command {
    /// x: right, y: forward, each in -1..=1.
    pub movement: Vec2,
    /// Radians: yaw around +z (0 = +x), pitch up from level.
    pub yaw: f32,
    pub pitch: f32,
    pub jump: bool,
    pub crouch: bool,
    pub fire: bool,
    pub zoom: bool,
    pub reload: bool,
    pub melee: bool,
    /// Pick up / swap weapons (held).
    pub action: bool,
    pub switch_weapon: bool,
    pub throw_grenade: bool,
    pub switch_grenade: bool,
}

#[derive(Debug, Clone)]
pub struct HeldWeapon {
    pub weapon: usize,
    pub state: WeaponState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrenadeKind {
    Frag,
    Plasma,
}

#[derive(Debug, Clone)]
pub struct Spartan {
    pub body: Player,
    pub yaw: f32,
    pub pitch: f32,
    pub shield: f32,
    pub health: f32,
    since_damage: f32,
    pub alive: bool,
    /// Seconds until respawning, while dead.
    pub respawn_in: f32,
    /// At most two; `current` is in hand.
    pub weapons: Vec<HeldWeapon>,
    pub current: usize,
    pub frags: u8,
    pub plasmas: u8,
    pub grenade: GrenadeKind,
    pub kills: u32,
    pub deaths: u32,
    /// Kills since last spawning.
    pub spree: u32,
    /// Kills in the current multi-kill chain, and when the last one was.
    pub multi_kill: u32,
    pub last_kill: f64,
    /// Seconds left bringing the weapon in hand up.
    pub readying: f32,
    melee_cooldown: f32,
    grenade_cooldown: f32,
    action_held: f32,
    last: Command,
}

impl Spartan {
    pub fn held(&self) -> Option<&HeldWeapon> {
        self.weapons.get(self.current)
    }

    pub fn eye(&self) -> Vec3 {
        self.body.eye()
    }

    /// Unit view direction.
    pub fn aim(&self) -> Vec3 {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        Vec3::new(self.yaw.cos() * cp, self.yaw.sin() * cp, sp)
    }

    /// Aim basis: forward, right, up.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.aim();
        let r = f.cross(Vec3::Z).normalize_or(Vec3::X);
        (f, r, r.cross(f))
    }
}

/// A weapon on the ground: a map item, or one a player dropped.
#[derive(Debug, Clone)]
pub struct DroppedWeapon {
    pub weapon: usize,
    pub state: WeaponState,
    pub position: Vec3,
    pub yaw: f32,
    ttl: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grenade {
    pub kind: GrenadeKind,
    pub owner: usize,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Seconds to detonation once armed by a bounce.
    pub fuse: Option<f32>,
    /// Stuck to a player.
    pub stuck: Option<usize>,
    stuck_offset: Vec3,
    age: f32,
}

/// Things that happened during a tick, for sounds, effects and animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Shot {
        player: usize,
        origin: Vec3,
        direction: Vec3,
        /// Where it hit the level or a player, and the surface normal.
        hit: Option<(Vec3, Vec3)>,
        hit_player: Option<usize>,
    },
    Reloaded {
        player: usize,
        empty: bool,
    },
    Switched {
        player: usize,
    },
    Melee {
        player: usize,
        hit: Option<usize>,
    },
    Thrown {
        player: usize,
    },
    Exploded {
        kind: GrenadeKind,
        position: Vec3,
    },
    Damaged {
        player: usize,
        amount: f32,
    },
    Killed {
        killer: Option<usize>,
        victim: usize,
        headshot: bool,
    },
    Spawned {
        player: usize,
        /// The way they face on appearing.
        yaw: f32,
    },
    PickedUp {
        player: usize,
        kind: ItemKind,
    },
    /// Pulled the trigger with no ammo left at all.
    DryFire {
        player: usize,
    },
    /// Earned a medal the announcer calls out.
    Medal {
        player: usize,
        medal: Medal,
    },
    /// Took, lost or tied the lead in kills.
    Lead {
        player: usize,
        change: LeadChange,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medal {
    /// Kills chained within a few seconds of each other: 2 (double kill)
    /// up to 7 (Killimanjaro) and beyond.
    MultiKill(u8),
    /// Kills without dying: 5, 10, 15, 20 and 25.
    Spree(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadChange {
    Gained,
    Lost,
    Tied,
}

pub struct Game {
    pub rules: Rules,
    pub weapons: Vec<WeaponDef>,
    pub spawns: Vec<(Vec3, f32)>,
    pub item_spawns: Vec<ItemSpawn>,
    /// Per item spawn: seconds until it is back (0 = lying there).
    pub item_timers: Vec<f32>,
    pub dropped: Vec<DroppedWeapon>,
    pub grenades: Vec<Grenade>,
    pub players: Vec<Spartan>,
    pub movement: PlayerMovement,
    pub biped: BipedPhysics,
    pub time: f64,
    pub events: Vec<Event>,
    pub winner: Option<usize>,
    rng: u32,
}

/// How long the weapon now in a player's hands takes to come up.
fn ready_time(weapons: &[WeaponDef], p: &Spartan) -> f32 {
    p.held()
        .and_then(|h| weapons.get(h.weapon))
        .map_or(crate::weapon::DEFAULT_READY_TIME, |d| d.ready_time)
}

/// Distance along a ray to a vertical capsule (feet at `base`), if it hits.
fn ray_capsule(origin: Vec3, dir: Vec3, base: Vec3, height: f32, radius: f32) -> Option<f32> {
    let a = base + Vec3::Z * radius;
    let b = base + Vec3::Z * (height - radius).max(radius);
    // Closest approach between the ray and the capsule's segment, sampled
    // finely enough for player-sized capsules.
    let mut best: Option<f32> = None;
    let ab = b - a;
    let steps = 8;
    for i in 0..=steps {
        let c = a + ab * (i as f32 / steps as f32);
        // Ray / sphere at c.
        let oc = origin - c;
        let bq = oc.dot(dir);
        let cq = oc.length_squared() - radius * radius;
        let disc = bq * bq - cq;
        if disc < 0.0 {
            continue;
        }
        let t = -bq - disc.sqrt();
        if t >= 0.0 && best.is_none_or(|bt| t < bt) {
            best = Some(t);
        }
    }
    best
}

impl Game {
    pub fn new(
        rules: Rules,
        weapons: Vec<WeaponDef>,
        spawns: Vec<(Vec3, f32)>,
        item_spawns: Vec<ItemSpawn>,
        movement: PlayerMovement,
        biped: BipedPhysics,
    ) -> Game {
        Game {
            item_timers: vec![0.0; item_spawns.len()],
            rules,
            weapons,
            spawns,
            item_spawns,
            dropped: Vec::new(),
            grenades: Vec::new(),
            players: Vec::new(),
            movement,
            biped,
            time: 0.0,
            events: Vec::new(),
            winner: None,
            rng: 0x2545_F491,
        }
    }

    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Add a player, spawned at once; returns its index.
    pub fn add_player(&mut self) -> usize {
        let i = self.players.len();
        let spartan = self.fresh_spartan(Vec3::ZERO, 0.0);
        self.players.push(spartan);
        self.respawn(i);
        i
    }

    fn fresh_spartan(&self, position: Vec3, yaw: f32) -> Spartan {
        let weapons = self
            .rules
            .starting_weapons
            .iter()
            .filter_map(|&w| {
                Some(HeldWeapon {
                    weapon: w,
                    state: WeaponState::new(self.weapons.get(w)?),
                })
            })
            .take(2)
            .collect();
        Spartan {
            body: Player::new(position, self.movement, self.biped),
            yaw,
            pitch: 0.0,
            shield: self.rules.shield,
            health: self.rules.health,
            since_damage: f32::INFINITY,
            alive: true,
            respawn_in: 0.0,
            weapons,
            current: 0,
            frags: self.rules.starting_frags,
            plasmas: self.rules.starting_plasmas,
            grenade: GrenadeKind::Frag,
            kills: 0,
            deaths: 0,
            spree: 0,
            multi_kill: 0,
            last_kill: f64::NEG_INFINITY,
            readying: 0.0,
            melee_cooldown: 0.0,
            grenade_cooldown: 0.0,
            action_held: 0.0,
            last: Command::default(),
        }
    }

    /// Bring a player back at the spawn point farthest from everyone alive.
    pub fn respawn(&mut self, player: usize) {
        let others: Vec<Vec3> = self
            .players
            .iter()
            .enumerate()
            .filter(|(i, p)| *i != player && p.alive)
            .map(|(_, p)| p.body.position)
            .collect();
        let mut best = (f32::MIN, (Vec3::ZERO, 0.0));
        for k in 0..self.spawns.len() {
            let (pos, yaw) = self.spawns[k];
            let nearest = others
                .iter()
                .map(|o| o.distance(pos))
                .fold(f32::MAX, f32::min);
            // A little randomness so spawns vary when no one is around.
            let score = nearest.min(50.0) + self.random() * 4.0;
            if score > best.0 {
                best = (score, (pos, yaw));
            }
        }
        let (pos, yaw) = best.1;
        let (kills, deaths) = (self.players[player].kills, self.players[player].deaths);
        let mut s = self.fresh_spartan(pos + Vec3::Z * 0.05, yaw);
        s.kills = kills;
        s.deaths = deaths;
        self.players[player] = s;
        self.events.push(Event::Spawned { player, yaw });
    }

    /// Advance one tick; `commands[i]` drives player i.
    pub fn step(&mut self, world: &World, commands: &[Command]) {
        let dt = TICK;
        self.time += dt as f64;
        for i in 0..self.players.len() {
            let cmd = commands.get(i).copied().unwrap_or_default();
            self.step_player(world, i, cmd, dt);
        }
        self.step_grenades(world, dt);
        self.step_items(dt);
    }

    fn step_player(&mut self, world: &World, i: usize, cmd: Command, dt: f32) {
        if !self.players[i].alive {
            let p = &mut self.players[i];
            p.respawn_in -= dt;
            if p.respawn_in <= 0.0 {
                self.respawn(i);
            }
            return;
        }
        let last = self.players[i].last;
        let pressed = |now: bool, before: bool| now && !before;
        {
            let p = &mut self.players[i];
            p.yaw = cmd.yaw;
            p.pitch = cmd.pitch.clamp(-1.5, 1.5);
            p.body.update(
                world,
                Input {
                    movement: cmd.movement,
                    yaw: cmd.yaw,
                    jump: cmd.jump,
                    crouch: cmd.crouch,
                },
                dt,
            );
            // Shields recharge after a while without damage.
            p.since_damage += dt;
            if p.since_damage > self.rules.shield_delay && p.shield < self.rules.shield {
                p.shield = (p.shield + self.rules.shield / self.rules.shield_recharge * dt)
                    .min(self.rules.shield);
            }
            p.readying = (p.readying - dt).max(0.0);
            p.melee_cooldown = (p.melee_cooldown - dt).max(0.0);
            p.grenade_cooldown = (p.grenade_cooldown - dt).max(0.0);
        }
        // Fell out of the level.
        if self.players[i].body.position.z < world.min.z - 1.0 {
            self.kill(i, None, false);
            return;
        }

        if pressed(cmd.switch_weapon, last.switch_weapon) {
            self.switch_weapon(i);
        }
        if pressed(cmd.switch_grenade, last.switch_grenade) {
            let p = &mut self.players[i];
            p.grenade = match p.grenade {
                GrenadeKind::Frag if p.plasmas > 0 => GrenadeKind::Plasma,
                GrenadeKind::Plasma if p.frags > 0 => GrenadeKind::Frag,
                g => g,
            };
        }
        self.pick_up(i, cmd.action, dt);

        let lunges = self.players[i]
            .held()
            .is_some_and(|h| self.rules.lunge_weapons.contains(&h.weapon));
        if pressed(cmd.melee, last.melee) && self.players[i].melee_cooldown <= 0.0 {
            self.melee(i, lunges);
        }
        if pressed(cmd.throw_grenade, last.throw_grenade) {
            self.throw_grenade(i);
        }

        // The weapon in hand. The sword's trigger swings it.
        let (eye, (f, r, u)) = (self.players[i].eye(), self.players[i].basis());
        let ready = self.players[i].readying <= 0.0;
        let mut shots = Vec::new();
        let mut reloaded = None;
        if let Some(def) = self.players[i]
            .held()
            .and_then(|h| self.weapons.get(h.weapon))
            .cloned()
        {
            let p = &mut self.players[i];
            let held = &mut p.weapons[p.current];
            let was = (held.state.reloading.is_some(), held.state.loaded == 0);
            let dry = def.uses_ammo()
                && held.state.loaded < def.rounds_per_shot
                && held.state.reserve == 0
                && held.state.reloading.is_none();
            if dry && ready && !lunges && pressed(cmd.fire, last.fire) {
                self.events.push(Event::DryFire { player: i });
            }
            let input = WeaponInput {
                fire: cmd.fire && ready && !lunges,
                reload: cmd.reload,
                zoom: cmd.zoom,
            };
            for shot in held.state.update(&def, input, dt) {
                shots.push((shot.direction(f, r, u), def.clone()));
            }
            if held.state.reloading.is_some() && !was.0 {
                reloaded = Some(was.1);
            }
        }
        if lunges && pressed(cmd.fire, last.fire) && ready && self.players[i].melee_cooldown <= 0.0
        {
            self.melee(i, true);
        }
        if let Some(empty) = reloaded {
            self.events.push(Event::Reloaded { player: i, empty });
        }
        for (dir, def) in shots {
            self.fire(world, i, eye, dir, &def);
        }
        self.players[i].last = cmd;
    }

    fn switch_weapon(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.weapons.len() < 2 {
            return;
        }
        if let Some(h) = p.weapons.get_mut(p.current) {
            h.state.zoom = 0;
            h.state.reloading = None;
        }
        p.current = (p.current + 1) % p.weapons.len();
        p.readying = ready_time(&self.weapons, p);
        self.events.push(Event::Switched { player: i });
    }

    /// Who a ray from `origin` hits first among living players other than
    /// `shooter`: (player, distance, headshot).
    fn trace_players(&self, shooter: usize, origin: Vec3, dir: Vec3) -> Option<(usize, f32, bool)> {
        let mut best: Option<(usize, f32, bool)> = None;
        for (j, q) in self.players.iter().enumerate() {
            if j == shooter || !q.alive {
                continue;
            }
            let h = q.body.height();
            if let Some(t) = ray_capsule(origin, dir, q.body.position, h, q.body.biped.radius) {
                if best.is_none_or(|b| t < b.1) {
                    let z = origin.z + dir.z * t;
                    let head = z > q.body.position.z + h - HEAD_HEIGHT;
                    best = Some((j, t, head));
                }
            }
        }
        best
    }

    fn fire(&mut self, world: &World, i: usize, eye: Vec3, dir: Vec3, def: &WeaponDef) {
        let range = def.range;
        let wall = world.raycast_hit(eye, dir, range);
        let target = self
            .trace_players(i, eye, dir)
            .filter(|&(_, t, _)| t <= range && wall.is_none_or(|(wt, _)| t < wt));
        let weapon = self.players[i].held().map(|h| h.weapon);
        let (hit, hit_player) = match target {
            Some((j, t, head)) => {
                let damage = WeaponState::damage_at(def, t);
                let headshot =
                    head && weapon.is_some_and(|w| self.rules.headshot_weapons.contains(&w));
                self.damage(j, Some(i), damage, headshot);
                (Some((eye + dir * t, -dir)), Some(j))
            }
            None => (wall.map(|(t, n)| (eye + dir * t, n)), None),
        };
        self.events.push(Event::Shot {
            player: i,
            origin: eye,
            direction: dir,
            hit,
            hit_player,
        });
    }

    fn melee(&mut self, i: usize, lunge: bool) {
        let (eye, aim) = (self.players[i].eye(), self.players[i].aim());
        let reach = if lunge { LUNGE_RANGE } else { MELEE_RANGE };
        let mut target: Option<(usize, f32)> = None;
        for (j, q) in self.players.iter().enumerate() {
            if j == i || !q.alive {
                continue;
            }
            let centre = q.body.position + Vec3::Z * q.body.height() * 0.6;
            let to = centre - eye;
            let d = to.length();
            if d <= reach
                && aim.dot(to / d.max(1e-4)) > MELEE_CONE.cos()
                && target.is_none_or(|t| d < t.1)
            {
                target = Some((j, d));
            }
        }
        self.players[i].melee_cooldown = MELEE_COOLDOWN;
        if let Some((j, _)) = target {
            if lunge {
                // The lunge carries the attacker to the target.
                let to = self.players[j].body.position - self.players[i].body.position;
                self.players[i].body.position += to * 0.6;
            }
            let damage = if lunge {
                self.rules.lunge_damage
            } else {
                self.rules.melee_damage
            };
            self.damage(j, Some(i), damage, false);
        }
        self.events.push(Event::Melee {
            player: i,
            hit: target.map(|t| t.0),
        });
    }

    fn throw_grenade(&mut self, i: usize) {
        let def = self.grenade_def(self.players[i].grenade);
        let p = &mut self.players[i];
        if p.grenade_cooldown > 0.0 {
            return;
        }
        let count = match p.grenade {
            GrenadeKind::Frag => &mut p.frags,
            GrenadeKind::Plasma => &mut p.plasmas,
        };
        if *count == 0 {
            return;
        }
        *count -= 1;
        p.grenade_cooldown = GRENADE_THROW_COOLDOWN;
        let kind = p.grenade;
        // Thrown a little above where the player looks.
        let (f, _, u) = p.basis();
        let dir = (f + u * 0.15).normalize();
        let g = Grenade {
            kind,
            owner: i,
            position: p.eye() + f * 0.25 - Vec3::Z * 0.05,
            velocity: dir * def.speed + p.body.velocity * 0.5,
            fuse: None,
            stuck: None,
            stuck_offset: Vec3::ZERO,
            age: 0.0,
        };
        self.grenades.push(g);
        self.events.push(Event::Thrown { player: i });
    }

    fn grenade_def(&self, kind: GrenadeKind) -> GrenadeDef {
        match kind {
            GrenadeKind::Frag => self.rules.frag,
            GrenadeKind::Plasma => self.rules.plasma,
        }
    }

    fn step_grenades(&mut self, world: &World, dt: f32) {
        let mut exploded = Vec::new();
        for gi in 0..self.grenades.len() {
            let mut g = self.grenades[gi];
            let def = self.grenade_def(g.kind);
            g.age += dt;
            if let Some(j) = g.stuck {
                match self.players.get(j).filter(|p| p.alive) {
                    Some(p) => g.position = p.body.position + g.stuck_offset,
                    None => g.stuck = None,
                }
            } else {
                g.velocity.z -= GRENADE_GRAVITY * dt;
                let step = g.velocity * dt;
                let len = step.length();
                if len > 1e-6 {
                    let dir = step / len;
                    // Plasma grenades stick to players they touch.
                    let victim = def
                        .sticks
                        .then(|| self.trace_players(g.owner, g.position, dir))
                        .flatten()
                        .filter(|&(_, t, _)| t <= len);
                    if let Some((j, _, _)) = victim {
                        g.stuck = Some(j);
                        g.stuck_offset = g.position + dir * len - self.players[j].body.position;
                        g.fuse.get_or_insert(def.fuse);
                    } else if let Some((t, n)) = world.raycast_hit(g.position, dir, len + 0.02) {
                        g.position += dir * (t - 0.02).max(0.0);
                        if def.sticks {
                            g.velocity = Vec3::ZERO;
                        } else {
                            // Bounce, losing most of the speed into the surface.
                            let vn = g.velocity.dot(n) * n;
                            g.velocity = (g.velocity - vn) * 0.6 - vn * 0.35;
                        }
                        g.fuse.get_or_insert(def.fuse);
                    } else {
                        g.position += step;
                    }
                }
            }
            if let Some(f) = g.fuse.as_mut() {
                *f -= dt;
            }
            // Grenades that never land go off anyway.
            if g.fuse.is_some_and(|f| f <= 0.0) || g.age > 6.0 {
                exploded.push(gi);
            }
            self.grenades[gi] = g;
        }
        for &gi in exploded.iter().rev() {
            let g = self.grenades.remove(gi);
            self.explode(world, g);
        }
    }

    fn explode(&mut self, world: &World, g: Grenade) {
        let def = self.grenade_def(g.kind);
        let (inner, outer) = def.radius;
        for j in 0..self.players.len() {
            let p = &self.players[j];
            if !p.alive {
                continue;
            }
            let centre = p.body.position + Vec3::Z * p.body.height() * 0.5;
            let d = centre.distance(g.position);
            if d >= outer {
                continue;
            }
            // Walls shield players from the blast.
            let to = centre - g.position;
            if g.stuck != Some(j)
                && world
                    .raycast(g.position, to / d.max(1e-4), d)
                    .is_some_and(|t| t < d - 0.05)
            {
                continue;
            }
            let falloff = if d <= inner {
                1.0
            } else {
                1.0 - (d - inner) / (outer - inner)
            };
            self.damage(j, Some(g.owner), def.damage * falloff, false);
        }
        self.events.push(Event::Exploded {
            kind: g.kind,
            position: g.position,
        });
    }

    /// Apply damage: shields first, then health.
    pub fn damage(&mut self, victim: usize, attacker: Option<usize>, amount: f32, headshot: bool) {
        let rules_health = self.rules.health;
        let p = &mut self.players[victim];
        if !p.alive || amount <= 0.0 {
            return;
        }
        p.since_damage = 0.0;
        let absorbed = amount.min(p.shield);
        p.shield -= absorbed;
        let rest = amount - absorbed;
        // A headshot kills once the shields are gone.
        let killing_headshot = headshot && p.shield <= 0.0 && absorbed < amount.max(1e-3);
        p.health -= if killing_headshot { rules_health } else { rest };
        self.events.push(Event::Damaged {
            player: victim,
            amount,
        });
        if p.health <= 0.0 {
            self.kill(victim, attacker, killing_headshot);
        }
    }

    fn kill(&mut self, victim: usize, killer: Option<usize>, headshot: bool) {
        let respawn = self.rules.respawn_time;
        let p = &mut self.players[victim];
        p.alive = false;
        p.health = 0.0;
        p.shield = 0.0;
        p.respawn_in = respawn;
        p.deaths += 1;
        p.spree = 0;
        p.multi_kill = 0;
        // Drop the weapon in hand.
        let drop = p.held().cloned();
        let (pos, yaw) = (p.body.position + Vec3::Z * 0.1, p.yaw);
        if let Some(h) = drop {
            self.dropped.push(DroppedWeapon {
                weapon: h.weapon,
                state: h.state,
                position: pos,
                yaw,
                ttl: DROPPED_WEAPON_LIFETIME,
            });
        }
        self.events.push(Event::Killed {
            killer,
            victim,
            headshot,
        });
        if let Some(k) = killer.filter(|&k| k != victim) {
            let leaders = self.leaders();
            let time = self.time;
            let p = &mut self.players[k];
            p.kills += 1;
            p.spree += 1;
            let chained = p.multi_kill > 0 && time - p.last_kill <= MULTI_KILL_WINDOW;
            p.multi_kill = if chained { p.multi_kill + 1 } else { 1 };
            p.last_kill = time;
            let (multi, spree) = (p.multi_kill, p.spree);
            if multi >= 2 {
                self.events.push(Event::Medal {
                    player: k,
                    medal: Medal::MultiKill(multi.min(u8::MAX as u32) as u8),
                });
            }
            if spree % SPREE_STEP == 0 && spree <= SPREE_MAX {
                self.events.push(Event::Medal {
                    player: k,
                    medal: Medal::Spree(spree as u8),
                });
            }
            self.lead_changes(k, &leaders);
            if self.rules.score_to_win > 0 && self.players[k].kills >= self.rules.score_to_win {
                self.winner.get_or_insert(k);
            }
        }
    }

    /// Who has the most kills (no one before the first kill).
    fn leaders(&self) -> Vec<usize> {
        let top = self.players.iter().map(|p| p.kills).max().unwrap_or(0);
        if top == 0 {
            return Vec::new();
        }
        (0..self.players.len())
            .filter(|&i| self.players[i].kills == top)
            .collect()
    }

    /// Announce how `scorer`'s kill changed the lead.
    fn lead_changes(&mut self, scorer: usize, before: &[usize]) {
        let after = self.leaders();
        if after == [scorer] && before != [scorer] {
            self.events.push(Event::Lead {
                player: scorer,
                change: LeadChange::Gained,
            });
        } else if after.len() > 1 && after.contains(&scorer) && !before.contains(&scorer) {
            self.events.push(Event::Lead {
                player: scorer,
                change: LeadChange::Tied,
            });
        }
        for &j in before {
            if j != scorer && !after.contains(&j) {
                self.events.push(Event::Lead {
                    player: j,
                    change: LeadChange::Lost,
                });
            }
        }
    }

    /// Walk over ammo and grenades; hold the action key to take a weapon.
    fn pick_up(&mut self, i: usize, action: bool, dt: f32) {
        let feet = self.players[i].body.position;
        let near = |p: Vec3| {
            Vec2::new(p.x - feet.x, p.y - feet.y).length() < PICKUP_RADIUS
                && (p.z - feet.z).abs() < 0.8
        };
        self.players[i].action_held = if action {
            self.players[i].action_held + dt
        } else {
            0.0
        };

        // Map items.
        for s in 0..self.item_spawns.len() {
            if self.item_timers[s] > 0.0 || !near(self.item_spawns[s].position) {
                continue;
            }
            let spawn = self.item_spawns[s];
            let taken = match spawn.kind {
                ItemKind::FragGrenades | ItemKind::PlasmaGrenades => {
                    let max = self.rules.max_grenades;
                    let p = &mut self.players[i];
                    let count = if spawn.kind == ItemKind::FragGrenades {
                        &mut p.frags
                    } else {
                        &mut p.plasmas
                    };
                    (*count < max).then(|| *count += 1).is_some()
                }
                ItemKind::Weapon(w) => {
                    let fresh = match self.weapons.get(w) {
                        Some(def) => WeaponState::new(def),
                        None => continue,
                    };
                    self.take_weapon(i, w, fresh, action)
                }
            };
            if taken {
                self.item_timers[s] = spawn.respawn.max(1.0);
                self.events.push(Event::PickedUp {
                    player: i,
                    kind: spawn.kind,
                });
            }
        }
        // Dropped weapons.
        let mut d = 0;
        while d < self.dropped.len() {
            if near(self.dropped[d].position) {
                let w = self.dropped[d].weapon;
                let state = self.dropped[d].state.clone();
                if self.take_weapon(i, w, state, action) {
                    self.dropped.remove(d);
                    self.events.push(Event::PickedUp {
                        player: i,
                        kind: ItemKind::Weapon(w),
                    });
                    continue;
                }
            }
            d += 1;
        }
    }

    /// A weapon on the ground: ammo if the player has one already, the
    /// weapon itself if they have a free hand or hold the action key.
    fn take_weapon(&mut self, i: usize, w: usize, state: WeaponState, action: bool) -> bool {
        let Some(def) = self.weapons.get(w).cloned() else {
            return false;
        };
        let p = &mut self.players[i];
        if let Some(h) = p.weapons.iter_mut().find(|h| h.weapon == w) {
            if !def.uses_ammo() {
                return false;
            }
            let room = def.maximum_rounds.saturating_sub(h.state.reserve);
            let take = (state.loaded + state.reserve).min(room);
            if take == 0 {
                return false;
            }
            h.state.reserve += take;
            return true;
        }
        if p.weapons.len() < 2 {
            p.weapons.push(HeldWeapon { weapon: w, state });
            p.current = p.weapons.len() - 1;
            p.readying = ready_time(&self.weapons, p);
            self.events.push(Event::Switched { player: i });
            return true;
        }
        if action && p.action_held >= SWAP_HOLD {
            p.action_held = f32::MIN;
            let old = std::mem::replace(&mut p.weapons[p.current], HeldWeapon { weapon: w, state });
            p.readying = ready_time(&self.weapons, p);
            let (pos, yaw) = (p.body.position + Vec3::Z * 0.1, p.yaw);
            self.dropped.push(DroppedWeapon {
                weapon: old.weapon,
                state: old.state,
                position: pos,
                yaw,
                ttl: DROPPED_WEAPON_LIFETIME,
            });
            self.events.push(Event::Switched { player: i });
            return true;
        }
        false
    }

    fn step_items(&mut self, dt: f32) {
        for t in &mut self.item_timers {
            *t = (*t - dt).max(0.0);
        }
        for d in &mut self.dropped {
            d.ttl -= dt;
        }
        self.dropped.retain(|d| d.ttl > 0.0);
    }

    /// The weapon on the ground a player could swap for (action key prompt).
    pub fn swap_prompt(&self, i: usize) -> Option<usize> {
        let p = self.players.get(i)?;
        if !p.alive || p.weapons.len() < 2 {
            return None;
        }
        let feet = p.body.position;
        let near = |q: Vec3| Vec2::new(q.x - feet.x, q.y - feet.y).length() < PICKUP_RADIUS;
        let held = |w: usize| p.weapons.iter().any(|h| h.weapon == w);
        self.item_spawns
            .iter()
            .zip(&self.item_timers)
            .filter(|(s, t)| **t <= 0.0 && near(s.position))
            .find_map(|(s, _)| match s.kind {
                ItemKind::Weapon(w) if !held(w) => Some(w),
                _ => None,
            })
            .or_else(|| {
                self.dropped
                    .iter()
                    .find(|d| near(d.position) && !held(d.weapon))
                    .map(|d| d.weapon)
            })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) use crate::testing::{floor, game};

    /// Two players facing each other along x.
    fn duel(g: &mut Game) {
        g.add_player();
        g.add_player();
        g.players[0].body.position = Vec3::new(0.0, 0.0, 0.0);
        g.players[1].body.position = Vec3::new(5.0, 0.0, 0.0);
    }

    fn aim_at(g: &Game, from: usize, to: usize, z: f32) -> Command {
        let d = g.players[to].body.position + Vec3::Z * z - g.players[from].eye();
        Command {
            yaw: d.y.atan2(d.x),
            pitch: (d.z / d.truncate().length()).atan(),
            ..Command::default()
        }
    }

    #[test]
    fn shots_strip_shields_then_health() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        let mut fire = aim_at(&g, 0, 1, 0.4);
        // Settle on the floor first.
        for _ in 0..10 {
            g.step(&world, &[aim_at(&g, 0, 1, 0.4), Command::default()]);
        }
        fire.fire = true;
        g.step(&world, &[fire, Command::default()]);
        assert!(g.players[1].shield < 70.0, "shield {}", g.players[1].shield);
        assert_eq!(g.players[1].health, 45.0);
        // Keep shooting (tap the trigger) until dead.
        for t in 0..400 {
            let mut c = aim_at(&g, 0, 1, 0.4);
            c.fire = t % 4 == 0;
            g.step(&world, &[c, Command::default()]);
            if !g.players[1].alive {
                break;
            }
        }
        assert!(!g.players[1].alive);
        assert_eq!(g.players[0].kills, 1);
        assert_eq!(g.players[1].deaths, 1);
        // Dropped their rifle.
        assert_eq!(g.dropped.len(), 1);
    }

    #[test]
    fn headshot_kills_without_shields() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        g.players[1].shield = 0.0;
        let mut c = aim_at(&g, 0, 1, 0.68);
        c.fire = true;
        g.step(&world, &[c, Command::default()]);
        assert!(!g.players[1].alive);
        assert!(g
            .events
            .iter()
            .any(|e| matches!(e, Event::Killed { headshot: true, .. })));
    }

    #[test]
    fn dead_players_respawn() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        g.damage(1, Some(0), 500.0, false);
        assert!(!g.players[1].alive);
        for _ in 0..(5.5 / TICK) as usize {
            g.step(&world, &[Command::default(), Command::default()]);
        }
        assert!(g.players[1].alive);
        assert_eq!(g.players[1].shield, 70.0);
        assert_eq!(g.players[1].deaths, 1);
    }

    #[test]
    fn quick_kills_and_sprees_earn_medals() {
        let mut g = game();
        for _ in 0..3 {
            g.add_player();
        }
        let medals = |g: &mut Game| -> Vec<Medal> {
            g.events
                .drain(..)
                .filter_map(|e| match e {
                    Event::Medal { player: 0, medal } => Some(medal),
                    _ => None,
                })
                .collect()
        };
        g.damage(1, Some(0), 500.0, false);
        assert_eq!(medals(&mut g), vec![]);
        g.damage(2, Some(0), 500.0, false);
        assert_eq!(medals(&mut g), vec![Medal::MultiKill(2)]);
        // Too slow for a triple kill: the chain starts again.
        g.time += 5.0;
        g.respawn(1);
        g.damage(1, Some(0), 500.0, false);
        assert_eq!(medals(&mut g), vec![]);
        g.respawn(1);
        g.respawn(2);
        g.damage(1, Some(0), 500.0, false);
        g.damage(2, Some(0), 500.0, false);
        // Five kills without dying.
        assert_eq!(
            medals(&mut g),
            vec![Medal::MultiKill(2), Medal::MultiKill(3), Medal::Spree(5)]
        );
        // Dying ends the spree.
        g.damage(0, Some(1), 500.0, false);
        assert_eq!(g.players[0].spree, 0);
    }

    #[test]
    fn the_lead_changes_hands() {
        let mut g = game();
        g.add_player();
        g.add_player();
        let leads = |g: &mut Game| -> Vec<(usize, LeadChange)> {
            g.events
                .drain(..)
                .filter_map(|e| match e {
                    Event::Lead { player, change } => Some((player, change)),
                    _ => None,
                })
                .collect()
        };
        g.damage(1, Some(0), 500.0, false);
        assert_eq!(leads(&mut g), vec![(0, LeadChange::Gained)]);
        g.respawn(1);
        g.damage(0, Some(1), 500.0, false);
        assert_eq!(leads(&mut g), vec![(1, LeadChange::Tied)]);
        g.respawn(0);
        g.damage(0, Some(1), 500.0, false);
        assert_eq!(
            leads(&mut g),
            vec![(1, LeadChange::Gained), (0, LeadChange::Lost)]
        );
    }

    #[test]
    fn walking_over_a_weapon_takes_it_into_a_free_hand() {
        let world = floor();
        let mut g = game();
        g.add_player();
        g.players[0].body.position = Vec3::new(0.0, 3.0, 0.0);
        g.step(&world, &[Command::default()]);
        assert_eq!(g.players[0].weapons.len(), 2);
        assert!(g.item_timers[0] > 0.0);
    }

    #[test]
    fn shields_recharge_after_a_delay() {
        let world = floor();
        let mut g = game();
        g.add_player();
        g.damage(0, None, 30.0, false);
        assert_eq!(g.players[0].shield, 40.0);
        for _ in 0..(4.0 / TICK) as usize {
            g.step(&world, &[Command::default()]);
        }
        assert_eq!(g.players[0].shield, 40.0);
        for _ in 0..(3.5 / TICK) as usize {
            g.step(&world, &[Command::default()]);
        }
        assert_eq!(g.players[0].shield, 70.0);
    }

    #[test]
    fn frag_grenades_explode_near_players() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        g.players[1].body.position = Vec3::new(1.5, 0.0, 0.0);
        let throw = Command {
            throw_grenade: true,
            pitch: -0.6,
            ..Command::default()
        };
        g.step(&world, &[throw, Command::default()]);
        assert_eq!(g.players[0].frags, 1);
        for _ in 0..(4.0 / TICK) as usize {
            g.step(&world, &[Command::default(), Command::default()]);
        }
        assert!(g.grenades.is_empty());
        assert!(
            g.events.iter().any(|e| matches!(e, Event::Exploded { .. }))
                || g.players[1].shield < 70.0
                || !g.players[1].alive
        );
    }
}
