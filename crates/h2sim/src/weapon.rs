//! Weapon firing: triggers, bursts, rate of fire, spread, ammo and reloading,
//! driven by the values in the weapon's tags.

use blam_cache::weapon::{Damage, Projectile, TriggerBehavior, TriggerInput, Weapon};
use glam::Vec3;

/// Halo 2 drives reload length from animations; this stands in until they
/// are played, and matches the Battle Rifle's reload.
const DEFAULT_RELOAD_TIME: f32 = 2.0;
/// Shots that never hit anything vanish after this distance.
const DEFAULT_RANGE: f32 = 1000.0;
/// Rounds at least this fast (world units per second) hit at once; slower
/// ones, and anything that explodes, fly through the air.
const INSTANT_SPEED: f32 = 100.0;
/// Flying rounds with no range of their own go this far.
const FLIGHT_RANGE: f32 = 150.0;

/// Everything about a weapon the simulation needs, flattened from its tags.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponDef {
    pub name: String,
    /// The control that fires it (vehicle guns have a second trigger: the
    /// Scorpion's machine gun, the Banshee's bomb).
    pub input: TriggerInput,
    pub behavior: TriggerBehavior,
    /// Firing effects per second at the start and after sustained fire.
    pub rounds_per_second: (f32, f32),
    pub rate_acceleration_time: f32,
    /// Shots per trigger pull; 0 means keep firing while held.
    pub shots_per_fire: u32,
    pub fire_recovery_time: f32,
    pub magazine_size: u32,
    pub initial_rounds: u32,
    pub maximum_rounds: u32,
    pub reload_time: f32,
    pub rounds_per_shot: u32,
    pub projectiles_per_shot: u32,
    /// Radians, at the start and after sustained fire.
    pub error_angle: (f32, f32),
    pub minimum_error: f32,
    pub error_acceleration_time: f32,
    pub error_deceleration_time: f32,
    /// Radians: projectiles of one shot spread over this fan (shotgun).
    pub distribution_angle: f32,
    pub zoom_levels: u32,
    pub zoom_range: (f32, f32),
    pub range: f32,
    /// World units per second (very fast bullets are treated as instant).
    pub velocity: f32,
    pub damage: f32,
    /// Damage falls to the lower bound over this distance range.
    pub damage_range: (f32, f32),
    pub damage_lower_bound: f32,
    /// Seconds to bring the weapon up after switching to it (the length of
    /// its first person "ready" animation).
    pub ready_time: f32,
    /// Melee damage when it differs from the usual strike (the flag's smash).
    pub melee_damage: Option<f32>,
    /// How it fires held in one of two hands, if it can be.
    pub dual: Option<DualWield>,
    /// How its rounds fly, unless they hit at once.
    pub flight: Option<Flight>,
}

/// A blast: full damage close in, falling off to the edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blast {
    /// Damage at the edge and inside the first radius.
    pub damage: (f32, f32),
    /// Full damage inside the first radius, none beyond the second.
    pub radius: (f32, f32),
    /// How hard it throws people (world units per second at the centre).
    pub push: f32,
}

impl Blast {
    /// The blast a `jpt!` describes, if it reaches anywhere.
    pub fn from_tags(d: &Damage) -> Option<Blast> {
        let outer = d.radius.0.max(d.radius.1);
        let upper = d.upper_bound.0.max(d.upper_bound.1);
        (outer > 0.0 && upper > 0.0).then_some(Blast {
            damage: (d.lower_bound.min(upper), upper),
            radius: (d.radius.0.min(outer), outer),
            push: d.instantaneous_acceleration,
        })
    }

    /// How much of the blast reaches `distance` from its centre (0..1).
    pub fn falloff(&self, distance: f32) -> f32 {
        let (inner, outer) = self.radius;
        if distance >= outer {
            0.0
        } else if distance <= inner {
            1.0
        } else {
            1.0 - (distance - inner) / (outer - inner)
        }
    }

    /// Damage `distance` from the centre.
    pub fn damage_at(&self, distance: f32) -> f32 {
        if distance >= self.radius.1 {
            return 0.0;
        }
        let f = self.falloff(distance);
        self.damage.0 + (self.damage.1 - self.damage.0) * f
    }
}

/// How a weapon's rounds fly: plasma bolts, needles, rockets, shells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flight {
    /// World units per second: leaving the barrel, and once past the
    /// acceleration distances.
    pub speed: (f32, f32),
    pub acceleration_range: (f32, f32),
    /// Times normal gravity.
    pub gravity: f32,
    /// Radians per second it turns toward a target (0: flies straight).
    pub homing: f32,
    /// Only homes in on vehicles (the rocket launcher's lock-on).
    pub homes_on_vehicles: bool,
    pub blast: Option<Blast>,
    /// Rounds that stick in whoever they hit (needles).
    pub sticky: Option<Sticky>,
}

/// Rounds that stick in whoever they hit and go off a moment later; enough
/// in one target at once set off a supercombine (the Needler's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sticky {
    /// Seconds after sticking before one goes off (shortest, longest).
    pub fuse: (f32, f32),
    /// Damage to whoever it's in when it goes off.
    pub damage: f32,
    /// How many at once set off the supercombine.
    pub supercombine: usize,
    /// The supercombine's damage to whoever they're in, and its blast.
    pub super_damage: f32,
    pub super_blast: Option<Blast>,
}

impl Flight {
    /// Speed after travelling `distance`.
    pub fn speed_at(&self, distance: f32) -> f32 {
        let (near, far) = self.acceleration_range;
        let t = if far <= near {
            (distance >= far) as u8 as f32
        } else {
            ((distance - near) / (far - near)).clamp(0.0, 1.0)
        };
        let (a, b) = self.speed;
        let b = if b > 0.0 { b } else { a };
        a + (b - a) * t
    }
}

/// What a barrel fires, read from its tags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rounds<'a> {
    pub projectile: Option<&'a Projectile>,
    /// Damage hitting someone directly.
    pub impact: Option<&'a Damage>,
    /// Damage going off.
    pub detonation: Option<&'a Damage>,
    /// A supercombine's blast, and its damage to whoever the rounds are
    /// stuck in.
    pub super_detonation: Option<&'a Damage>,
    pub attached_super: Option<&'a Damage>,
}

/// A one-handed weapon's spread and damage when dual wielded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualWield {
    /// Radians.
    pub minimum_error: f32,
    /// Radians, from the first shot to sustained fire.
    pub error_angle: (f32, f32),
    pub damage_scale: f32,
}

/// Used when a weapon's ready animation is unknown.
pub const DEFAULT_READY_TIME: f32 = 0.5;

impl WeaponDef {
    /// The barrel trigger `trigger` fires.
    pub fn barrel_of(w: &Weapon, trigger: usize) -> blam_cache::weapon::Barrel {
        let t = w.triggers.get(trigger).or(w.triggers.first());
        t.and_then(|t| usize::try_from(t.barrel).ok())
            .and_then(|b| w.barrels.get(b))
            .or(w.barrels.first())
            .copied()
            .unwrap_or_default()
    }

    /// The weapon as fired by trigger `trigger` (the first for hand-held
    /// guns).
    pub fn from_tags(w: &Weapon, trigger: usize, rounds: Rounds) -> Self {
        let Rounds {
            projectile,
            impact: damage,
            detonation,
            super_detonation,
            attached_super,
        } = rounds;
        let trigger = w
            .triggers
            .get(trigger)
            .or(w.triggers.first())
            .copied()
            .unwrap_or_default();
        let barrel = usize::try_from(trigger.barrel)
            .ok()
            .and_then(|b| w.barrels.get(b))
            .or(w.barrels.first())
            .copied()
            .unwrap_or_default();
        let magazine = usize::try_from(barrel.magazine)
            .ok()
            .and_then(|m| w.magazines.get(m))
            .copied()
            .unwrap_or_default();
        let latch = !matches!(trigger.behavior, TriggerBehavior::Spew);
        let shots_per_fire = match barrel.shots_per_fire.1.max(barrel.shots_per_fire.0) {
            n if n > 0 => n as u32,
            _ if latch => 1,
            _ => 0,
        };
        let rps = (
            barrel.rounds_per_second.0.max(0.1),
            barrel
                .rounds_per_second
                .1
                .max(barrel.rounds_per_second.0)
                .max(0.1),
        );
        let reload = if magazine.reload_time > 0.0 {
            magazine.reload_time
        } else {
            DEFAULT_RELOAD_TIME
        };
        let blast = detonation.and_then(Blast::from_tags);
        let velocity = projectile.map(|p| p.initial_velocity).unwrap_or(0.0);
        let flies = velocity > 0.0 && (velocity < INSTANT_SPEED || blast.is_some());
        let range = projectile
            .map(|p| p.maximum_range)
            .filter(|r| *r > 0.0)
            .unwrap_or(if flies { FLIGHT_RANGE } else { DEFAULT_RANGE });
        let flight = projectile.filter(|_| flies).map(|p| Flight {
            speed: (p.initial_velocity, p.final_velocity),
            acceleration_range: p.acceleration_range,
            gravity: p.air_gravity_scale.max(0.0),
            homing: p.guided_angular_velocity.0.max(p.guided_angular_velocity.1),
            homes_on_vehicles: trigger.behavior == TriggerBehavior::LatchRocketLauncher,
            blast,
            sticky: (p.super_count > 0)
                .then_some(attached_super)
                .flatten()
                .map(|a| Sticky {
                    fuse: if p.timer.1 > 0.0 {
                        (p.timer.0.min(p.timer.1), p.timer.1)
                    } else {
                        (0.5, 0.7)
                    },
                    damage: detonation.map_or(0.0, |d| d.upper_bound.0.max(d.upper_bound.1)),
                    supercombine: p.super_count as usize,
                    super_damage: a.upper_bound.0.max(a.upper_bound.1),
                    super_blast: super_detonation.and_then(Blast::from_tags),
                }),
        });
        let upper = damage.map(|d| d.upper_bound.0.max(d.upper_bound.1));
        let dual = w.can_be_dual_wielded().then_some(DualWield {
            minimum_error: barrel.dual_minimum_error,
            error_angle: barrel.dual_error_angle,
            // Unset (the Magnum): no change.
            damage_scale: if barrel.dual_damage_scale > 0.0 {
                barrel.dual_damage_scale
            } else {
                1.0
            },
        });
        WeaponDef {
            name: w.name.rsplit('\\').next().unwrap_or(&w.name).to_string(),
            input: trigger.input,
            behavior: trigger.behavior,
            rounds_per_second: rps,
            rate_acceleration_time: barrel.acceleration_time,
            shots_per_fire,
            fire_recovery_time: barrel.fire_recovery_time.max(0.0),
            magazine_size: magazine.rounds_loaded_maximum.max(0) as u32,
            initial_rounds: magazine.rounds_total_initial.max(0) as u32,
            maximum_rounds: magazine.rounds_total_maximum.max(0) as u32,
            reload_time: reload,
            rounds_per_shot: barrel.rounds_per_shot.max(0) as u32,
            projectiles_per_shot: barrel.projectiles_per_shot.max(1) as u32,
            error_angle: barrel.error_angle,
            minimum_error: barrel.minimum_error,
            error_acceleration_time: barrel.error_acceleration_time,
            error_deceleration_time: barrel.error_deceleration_time,
            distribution_angle: barrel.distribution_angle,
            zoom_levels: w.zoom_levels.max(0) as u32,
            zoom_range: w.zoom_range,
            range,
            velocity,
            damage: upper.unwrap_or(0.0),
            damage_range: projectile
                .map(|p| p.air_damage_range)
                .unwrap_or((0.0, range)),
            damage_lower_bound: damage.map(|d| d.lower_bound).unwrap_or(0.0),
            ready_time: DEFAULT_READY_TIME,
            melee_damage: None,
            dual,
            flight,
        }
    }

    /// The weapon as it fires held in one of two hands: wider spread and
    /// sometimes less damage. Itself if it can't be dual wielded.
    pub fn dual_wielded(&self) -> WeaponDef {
        let Some(d) = self.dual else {
            return self.clone();
        };
        WeaponDef {
            minimum_error: d.minimum_error.max(self.minimum_error),
            error_angle: (
                d.error_angle.0.max(self.error_angle.0),
                d.error_angle.1.max(self.error_angle.1),
            ),
            damage: self.damage * d.damage_scale,
            damage_lower_bound: self.damage_lower_bound * d.damage_scale,
            zoom_levels: 0,
            ..self.clone()
        }
    }

    /// Energy weapons (no magazine) never run dry here; battery and heat come later.
    pub fn uses_ammo(&self) -> bool {
        self.magazine_size > 0 && self.rounds_per_shot > 0
    }

    /// Magnification for zoom level `level` (1-based); 1.0 when not zoomed.
    pub fn magnification(&self, level: u32) -> f32 {
        if level == 0 || self.zoom_levels == 0 {
            return 1.0;
        }
        let (lo, hi) = self.zoom_range;
        if self.zoom_levels == 1 {
            return lo.max(1.0);
        }
        let t = (level - 1) as f32 / (self.zoom_levels - 1) as f32;
        (lo * (hi / lo.max(1e-3)).powf(t)).max(1.0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WeaponInput {
    pub fire: bool,
    pub reload: bool,
    pub zoom: bool,
}

/// One projectile leaving the barrel: a direction relative to where the
/// player aims, as (right, up) angle offsets in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub yaw: f32,
    pub pitch: f32,
}

impl Shot {
    /// World direction for an aim basis (all unit vectors).
    pub fn direction(&self, forward: Vec3, right: Vec3, up: Vec3) -> Vec3 {
        (forward + right * self.yaw.tan() + up * self.pitch.tan()).normalize()
    }
}

#[derive(Debug, Clone)]
pub struct WeaponState {
    pub loaded: u32,
    pub reserve: u32,
    /// Seconds left on the current reload, if reloading.
    pub reloading: Option<f32>,
    /// 0 = not zoomed, otherwise the zoom level.
    pub zoom: u32,
    /// Seconds until the next shot can leave the barrel.
    cooldown: f32,
    /// Shots still to come from the current trigger pull.
    burst_left: u32,
    fire_held: bool,
    zoom_held: bool,
    /// 0..1: how far sustained fire has pushed spread and rate of fire.
    heat: f32,
    rng: u32,
    /// Seconds since the last shot (for view kick, muzzle flash).
    pub since_shot: f32,
}

impl WeaponState {
    pub fn new(def: &WeaponDef) -> Self {
        let loaded = def.magazine_size.min(def.initial_rounds);
        WeaponState {
            loaded,
            reserve: def.initial_rounds - loaded,
            reloading: None,
            zoom: 0,
            cooldown: 0.0,
            burst_left: 0,
            fire_held: false,
            zoom_held: false,
            heat: 0.0,
            rng: 0x9E37_79B9,
            since_shot: f32::INFINITY,
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift32: deterministic so networked games can replay shots.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn can_reload(&self, def: &WeaponDef) -> bool {
        def.uses_ammo()
            && self.reloading.is_none()
            && self.reserve > 0
            && self.loaded < def.magazine_size
    }

    fn has_round(&self, def: &WeaponDef) -> bool {
        !def.uses_ammo() || self.loaded >= def.rounds_per_shot
    }

    fn start_reload(&mut self, def: &WeaponDef) {
        if self.can_reload(def) {
            self.reloading = Some(def.reload_time);
            self.burst_left = 0;
            self.zoom = 0;
        }
    }

    /// Advance by `dt` seconds; returns the projectiles fired.
    pub fn update(&mut self, def: &WeaponDef, input: WeaponInput, dt: f32) -> Vec<Shot> {
        let mut shots = Vec::new();
        self.since_shot += dt;
        let pressed = input.fire && !self.fire_held;
        self.fire_held = input.fire;

        if input.zoom && !self.zoom_held && def.zoom_levels > 0 && self.reloading.is_none() {
            self.zoom = (self.zoom + 1) % (def.zoom_levels + 1);
        }
        self.zoom_held = input.zoom;

        if let Some(t) = self.reloading.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                let take = (def.magazine_size - self.loaded).min(self.reserve);
                self.loaded += take;
                self.reserve -= take;
                self.reloading = None;
            }
            self.cool(def, dt);
            return shots;
        }
        if input.reload {
            self.start_reload(def);
            return shots;
        }

        // A trigger pull starts a burst (or one shot); spew weapons fire while held.
        if self.burst_left == 0 {
            let wants = if def.shots_per_fire == 0 {
                input.fire
            } else {
                pressed
            };
            if wants && self.cooldown <= 0.0 {
                if !self.has_round(def) {
                    self.start_reload(def);
                    return shots;
                }
                self.burst_left = if def.shots_per_fire == 0 {
                    1
                } else {
                    def.shots_per_fire
                };
            }
        }

        self.cooldown -= dt;
        let mut firing = false;
        while self.burst_left > 0 && self.cooldown <= 0.0 {
            if !self.has_round(def) {
                self.burst_left = 0;
                break;
            }
            firing = true;
            if def.uses_ammo() {
                self.loaded -= def.rounds_per_shot;
            }
            self.burst_left -= 1;
            let rate = def.rounds_per_second.0
                + (def.rounds_per_second.1 - def.rounds_per_second.0) * self.heat;
            self.cooldown += 1.0 / rate;
            if self.burst_left == 0 && def.shots_per_fire > 0 {
                self.cooldown += def.fire_recovery_time;
            }
            let error = (def.error_angle.0 + (def.error_angle.1 - def.error_angle.0) * self.heat)
                .max(def.minimum_error);
            let n = def.projectiles_per_shot.max(1);
            for i in 0..n {
                // Fan weapons spread evenly across the distribution angle.
                let fan = if n > 1 {
                    def.distribution_angle * (i as f32 / (n - 1) as f32 - 0.5)
                } else {
                    0.0
                };
                // Uniform over the error cone's disc.
                let r = error * self.random().sqrt();
                let a = self.random() * std::f32::consts::TAU;
                shots.push(Shot {
                    yaw: fan + r * a.cos(),
                    pitch: r * a.sin(),
                });
            }
            self.since_shot = 0.0;
        }
        if self.cooldown < 0.0 && self.burst_left == 0 {
            self.cooldown = 0.0;
        }
        if firing {
            let t = def.error_acceleration_time.max(1e-3);
            self.heat = (self.heat + dt.max(1.0 / 30.0) / t).min(1.0);
        } else {
            self.cool(def, dt);
        }
        shots
    }

    fn cool(&mut self, def: &WeaponDef, dt: f32) {
        let t = def.error_deceleration_time.max(1e-3);
        self.heat = (self.heat - dt / t).max(0.0);
    }

    /// Damage of a hit at `distance`.
    pub fn damage_at(def: &WeaponDef, distance: f32) -> f32 {
        let (near, far) = def.damage_range;
        if far <= near || distance <= near {
            return def.damage;
        }
        let t = ((distance - near) / (far - near)).clamp(0.0, 1.0);
        def.damage + (def.damage_lower_bound - def.damage) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Battle Rifle values from Halo 2 PC's shared.map.
    fn battle_rifle() -> WeaponDef {
        WeaponDef {
            name: "battle_rifle".into(),
            input: Default::default(),
            behavior: TriggerBehavior::LatchZoom,
            rounds_per_second: (15.0, 15.0),
            rate_acceleration_time: 0.0,
            shots_per_fire: 3,
            fire_recovery_time: 0.26,
            magazine_size: 36,
            initial_rounds: 108,
            maximum_rounds: 144,
            reload_time: 2.0,
            rounds_per_shot: 1,
            projectiles_per_shot: 1,
            error_angle: (0.005_235_988, 0.010_471_975),
            minimum_error: 0.0,
            error_acceleration_time: 0.1,
            error_deceleration_time: 0.1,
            distribution_angle: 0.0,
            zoom_levels: 1,
            zoom_range: (2.0, 2.0),
            range: 40.0,
            velocity: 400.0,
            damage: 6.0,
            damage_range: (0.0, 40.0),
            damage_lower_bound: 6.0,
            ready_time: DEFAULT_READY_TIME,
            melee_damage: None,
            dual: None,
            flight: None,
        }
    }

    fn run(
        def: &WeaponDef,
        w: &mut WeaponState,
        input: WeaponInput,
        seconds: f32,
    ) -> Vec<(f32, Shot)> {
        let dt = 1.0 / 120.0;
        let mut out = Vec::new();
        let mut t = 0.0;
        while t < seconds {
            for s in w.update(def, input, dt) {
                out.push((t, s));
            }
            t += dt;
        }
        out
    }

    #[test]
    fn one_pull_fires_a_three_round_burst() {
        let def = battle_rifle();
        let mut w = WeaponState::new(&def);
        assert_eq!((w.loaded, w.reserve), (36, 72));
        let fire = WeaponInput {
            fire: true,
            ..Default::default()
        };
        let shots = run(&def, &mut w, fire, 1.0);
        assert_eq!(shots.len(), 3);
        // 15 rounds per second inside the burst.
        let gap = shots[1].0 - shots[0].0;
        assert!((gap - 1.0 / 15.0).abs() < 0.01, "gap {gap}");
        assert_eq!(w.loaded, 33);
        for (_, s) in &shots {
            assert!(s.yaw.hypot(s.pitch) <= def.error_angle.1 + 1e-6);
        }
    }

    #[test]
    fn recovery_limits_burst_rate() {
        let def = battle_rifle();
        let mut w = WeaponState::new(&def);
        let mut fired = 0;
        // Pull the trigger every frame-pair for two seconds.
        for i in 0..240 {
            let input = WeaponInput {
                fire: i % 2 == 0,
                ..Default::default()
            };
            fired += w.update(&def, input, 1.0 / 120.0).len();
        }
        // Each burst takes 2/15 s plus 0.26 s recovery: ~5 bursts in 2 s.
        assert!((12..=18).contains(&fired), "fired {fired}");
    }

    #[test]
    fn reloads_from_reserve() {
        let def = battle_rifle();
        let mut w = WeaponState::new(&def);
        w.loaded = 3;
        let reload = WeaponInput {
            reload: true,
            ..Default::default()
        };
        run(&def, &mut w, reload, 0.1);
        assert!(w.reloading.is_some());
        run(&def, &mut w, WeaponInput::default(), 2.0);
        assert_eq!((w.loaded, w.reserve), (36, 39));
    }

    #[test]
    fn empty_magazine_reloads_on_trigger() {
        let def = battle_rifle();
        let mut w = WeaponState::new(&def);
        w.loaded = 0;
        let fire = WeaponInput {
            fire: true,
            ..Default::default()
        };
        let shots = run(&def, &mut w, fire, 0.05);
        assert!(shots.is_empty());
        assert!(w.reloading.is_some());
    }

    #[test]
    fn energy_weapons_fire_without_a_magazine() {
        let mut def = battle_rifle();
        def.magazine_size = 0;
        def.initial_rounds = 0;
        def.shots_per_fire = 0;
        let mut w = WeaponState::new(&def);
        let fire = WeaponInput {
            fire: true,
            ..Default::default()
        };
        assert!(run(&def, &mut w, fire, 1.0).len() >= 14);
    }

    #[test]
    fn zoom_toggles_through_levels() {
        let def = battle_rifle();
        let mut w = WeaponState::new(&def);
        let zoom = WeaponInput {
            zoom: true,
            ..Default::default()
        };
        w.update(&def, zoom, 0.01);
        assert_eq!(w.zoom, 1);
        assert_eq!(def.magnification(w.zoom), 2.0);
        w.update(&def, WeaponInput::default(), 0.01);
        w.update(&def, zoom, 0.01);
        assert_eq!(w.zoom, 0);
    }
}
