//! Squads following the mission's orders: each holds the firing positions
//! of its order's areas until one of the order's endings comes true, then
//! takes the next order. Endings test the mission's AI triggers: how many
//! of a squad are left and how hurt, whether a player has come into a
//! volume, what a script says.

use super::{in_group, Ctx};
use blam_cache::orders::{Combine, Condition, Rule, TriggerRef};
use glam::Vec3;
use h2sim::script::Vm;
use std::collections::{HashMap, HashSet};

/// Script ticks between looks at the orders' endings.
const EVERY: u32 = 3;
/// Close enough to its firing position to have arrived.
const ARRIVED: f32 = 1.0;
/// Combat status, as the triggers count it: idle, searching, and with an
/// enemy in clear sight.
const IDLE: i16 = 1;
const SEARCHING: i16 = 4;
const ENGAGED: i16 = 8;

/// A squad's order, and how far along it is.
#[derive(Debug, Clone, Default)]
struct SquadOrder {
    order: Option<u16>,
    /// Seconds since it was given.
    since: f32,
    /// The order's secondary areas are in use.
    secondary: bool,
    /// The ending that came true, and seconds since.
    ending: Option<(usize, f32)>,
}

#[derive(Debug, Default)]
pub(super) struct Orders {
    squads: HashMap<usize, SquadOrder>,
    /// Squads one of whose actors has seen a player.
    sighted_player: HashSet<usize>,
    /// Triggers latched on: for one squad, or for everyone.
    latched: HashSet<(u16, Option<usize>)>,
    /// Static scripts by name.
    scripts: HashMap<String, Option<usize>>,
    ticks: u32,
}

/// Whether a condition says the same whichever squad asks.
fn context_free(c: &Condition) -> bool {
    c.squad.is_some()
        || c.group.is_some()
        || matches!(
            c.rule,
            Rule::PlayerInVolume
                | Rule::AllPlayersInVolume
                | Rule::ScriptTrue
                | Rule::ScriptFalse
                | Rule::PlayerShotLongAgo
                | Rule::SafeToSave
        )
}

impl Ctx<'_> {
    /// The order a squad starts with: its own, or its group's (or a group
    /// above that's).
    fn first_order(&self, squad: usize) -> Option<u16> {
        let ai = &self.scene.ai;
        let s = ai.squads.get(squad)?;
        if s.order.is_some() {
            return s.order;
        }
        let mut group = s.group;
        for _ in 0..64 {
            let g = group? as usize;
            if let Some(o) = ai.group_orders.get(g).copied().flatten() {
                return Some(o);
            }
            group = ai.group_parents.get(g).copied().flatten();
        }
        None
    }

    /// A squad just placed takes up its order (its first, if it has none
    /// yet).
    pub(super) fn start_orders(&mut self, squad: usize) {
        if !self.st.orders.squads.contains_key(&squad) {
            let first = self.first_order(squad);
            self.st.orders.squads.insert(
                squad,
                SquadOrder {
                    order: first,
                    ..SquadOrder::default()
                },
            );
            if self.st.log {
                if let Some(o) = first.and_then(|o| self.scene.ai.orders.get(o as usize)) {
                    let name = &self.scene.ai.squads[squad].name;
                    println!("orders: {name} starts with {}", o.name);
                }
            }
        }
        self.give_areas(squad);
    }

    /// Give squads an order (a script says so).
    pub(super) fn set_order(&mut self, squads: &[usize], order: Option<u16>) {
        for &s in squads {
            self.st.orders.squads.insert(
                s,
                SquadOrder {
                    order,
                    ..SquadOrder::default()
                },
            );
            self.give_areas(s);
        }
    }

    /// Move actors to another squad (`ai_migrate`): they follow its order.
    pub(super) fn migrate(&mut self, actors: &[usize], to: usize) {
        for &i in actors {
            if let Some(a) = &mut self.game.players[i].actor {
                a.squad = to as u16;
            }
        }
        for (i, b) in self.bots.iter_mut() {
            if let (true, Some(mind)) = (actors.contains(i), &mut b.actor) {
                mind.squad = to as u16;
            }
        }
        *self.st.placed.entry(to).or_insert(0) += actors.len() as u32;
        self.start_orders(to);
    }

    /// The firing positions a squad's order gives it, and whether it goes
    /// along with players. `None` without an order (or one with no areas).
    fn areas(&self, squad: usize) -> Option<(Vec<Vec3>, bool)> {
        let ai = &self.scene.ai;
        let so = self.st.orders.squads.get(&squad)?;
        let order = ai.orders.get(so.order? as usize)?;
        let mut sets = order.primary.clone();
        if so.secondary {
            sets.extend(&order.secondary);
        }
        let positions: Vec<Vec3> = sets
            .iter()
            .filter_map(|&(z, a)| Some((ai.zones.get(z as usize)?, a)))
            .flat_map(|(zone, a)| zone.in_area(a))
            .filter(|f| !f.moving)
            .map(|f| Vec3::from(f.position))
            .collect();
        Some((positions, order.follows_player()))
    }

    /// Each of a squad's actors takes up a firing position of its order's
    /// areas (the nearest none of the others has), and fights from them.
    fn give_areas(&mut self, squad: usize) {
        let Some((area, follows)) = self.areas(squad) else {
            return;
        };
        let mut taken: Vec<Vec3> = Vec::new();
        let game = &*self.game;
        for (i, b) in self.bots.iter_mut() {
            let Some(mind) = &mut b.actor else {
                continue;
            };
            if mind.squad as usize != squad || !game.players[*i].alive {
                continue;
            }
            mind.follows = follows;
            if area.is_empty() {
                continue;
            }
            let feet = game.players[*i].body.position;
            let post = area
                .iter()
                .filter(|p| !taken.contains(p))
                .min_by(|p, q| {
                    p.distance_squared(feet)
                        .total_cmp(&q.distance_squared(feet))
                })
                .or_else(|| area.first())
                .copied();
            if let Some(post) = post {
                let to = (post - feet).truncate();
                if to.length() > ARRIVED {
                    mind.facing = to.y.atan2(to.x);
                }
                mind.post = post;
                taken.push(post);
            }
            mind.area = area.clone();
        }
    }

    /// Look at each squad's order: take up its secondary areas or move on
    /// to the next order when the triggers say.
    pub(super) fn follow_orders(&mut self, vm: &mut Vm, dt: f32) {
        for so in self.st.orders.squads.values_mut() {
            so.since += dt;
            if let Some((_, t)) = &mut so.ending {
                *t += dt;
            }
        }
        self.st.orders.ticks += 1;
        if !self.st.orders.ticks.is_multiple_of(EVERY) {
            return;
        }
        for (i, b) in self.bots.iter() {
            let t = b.target().filter(|&t| self.game.players[t].actor.is_none());
            if let (Some(_), Some(a)) = (t, &self.game.players[*i].actor) {
                self.st.orders.sighted_player.insert(a.squad as usize);
            }
        }
        let mut squads: Vec<usize> = self.st.orders.squads.keys().copied().collect();
        squads.sort_unstable();
        let ai = &self.scene.ai;
        for s in squads {
            let Some(so) = self.st.orders.squads.get(&s).cloned() else {
                continue;
            };
            let Some(order) = so.order.and_then(|o| ai.orders.get(o as usize)) else {
                continue;
            };
            if let Some((combine, refs)) = &order.secondary_trigger {
                let on = self.triggers_hold(vm, *combine, refs, s);
                if on != so.secondary {
                    if let Some(x) = self.st.orders.squads.get_mut(&s) {
                        x.secondary = on;
                    }
                    self.give_areas(s);
                }
            }
            let ending = so.ending.or_else(|| {
                let k = (0..order.endings.len()).find(|&k| {
                    let e = &order.endings[k];
                    self.triggers_hold(vm, e.combine, &e.triggers, s)
                })?;
                Some((k, 0.0))
            });
            let Some((k, waited)) = ending else {
                continue;
            };
            let e = &order.endings[k];
            if waited < e.delay {
                if let Some(x) = self.st.orders.squads.get_mut(&s) {
                    x.ending = Some((k, waited));
                }
                continue;
            }
            if self.st.log {
                let next = e.next.and_then(|n| ai.orders.get(n as usize));
                println!(
                    "orders: {} {} -> {}",
                    ai.squads[s].name,
                    order.name,
                    next.map_or("none", |o| o.name.as_str())
                );
            }
            self.set_order(&[s], e.next);
        }
    }

    fn triggers_hold(
        &mut self,
        vm: &mut Vm,
        combine: Combine,
        refs: &[TriggerRef],
        squad: usize,
    ) -> bool {
        if refs.is_empty() {
            return false;
        }
        let values: Vec<bool> = refs
            .iter()
            .map(|r| self.trigger(vm, r.trigger, squad) != r.not)
            .collect();
        combine.holds(values.into_iter())
    }

    /// Whether an AI trigger holds, for a squad.
    pub(super) fn trigger(&mut self, vm: &mut Vm, trigger: u16, squad: usize) -> bool {
        let scene = self.scene;
        let Some(t) = scene.ai.triggers.get(trigger as usize) else {
            return false;
        };
        let key = (
            trigger,
            (!t.conditions.iter().all(context_free)).then_some(squad),
        );
        if self.st.orders.latched.contains(&key) {
            return true;
        }
        let values: Vec<bool> = t
            .conditions
            .iter()
            .map(|c| self.condition(vm, c, squad) != c.not)
            .collect();
        let on = !values.is_empty() && t.combine.holds(values.into_iter());
        if on && t.latch {
            self.st.orders.latched.insert(key);
        }
        on
    }

    /// The squads a condition is about.
    fn about(&self, c: &Condition, squad: usize) -> Vec<usize> {
        let ai = &self.scene.ai;
        match (c.squad, c.group) {
            (Some(s), _) => vec![s as usize],
            (None, Some(g)) => (0..ai.squads.len())
                .filter(|&s| in_group(ai.squads[s].group, g, &ai.group_parents))
                .collect(),
            _ => vec![squad],
        }
    }

    fn condition(&mut self, vm: &mut Vm, c: &Condition, squad: usize) -> bool {
        let who = self.about(c, squad);
        let actors: Vec<usize> = (0..self.game.players.len())
            .filter(|&i| {
                let p = &self.game.players[i];
                p.alive && p.actor.is_some_and(|a| who.contains(&(a.squad as usize)))
            })
            .collect();
        let bots = || {
            self.bots
                .iter()
                .filter(|(i, _)| actors.contains(i))
                .map(|(_, b)| b)
        };
        let status = || {
            bots()
                .map(|b| {
                    if b.target().is_some() {
                        ENGAGED
                    } else if b.fighting() {
                        SEARCHING
                    } else {
                        IDLE
                    }
                })
                .max()
                .unwrap_or(0)
        };
        let fighting = || bots().filter(|b| b.target().is_some()).count() as i16;
        let a = c.a;
        let in_volume = |all: bool| {
            let Some(v) = c.volume.and_then(|v| self.scene.ai.volumes.get(v as usize)) else {
                return false;
            };
            let humans = self.humans();
            let inside = |&i: &usize| {
                let p = &self.game.players[i];
                p.alive && v.contains(p.body.position + Vec3::Z * 0.3)
            };
            if all {
                !humans.is_empty() && humans.iter().all(inside)
            } else {
                humans.iter().any(inside)
            }
        };
        match c.rule {
            Rule::AliveAtLeast => actors.len() as i16 >= a,
            Rule::AliveAtMost => actors.len() as i16 <= a,
            Rule::StrengthAtLeast => self.squads_strength(&who) >= c.x,
            Rule::StrengthAtMost => self.squads_strength(&who) <= c.x,
            Rule::EnemySighted => fighting() > 0,
            Rule::AfterTicks => {
                let since = self.st.orders.squads.get(&squad).map_or(0.0, |s| s.since);
                since * h2sim::script::TICKS_PER_SECOND as f32 >= a as f32
            }
            Rule::ScriptTrue | Rule::ScriptFalse => {
                let k = self.static_script(&c.script);
                let v = k.is_some_and(|k| vm.call(&self.scene.ai.scripts, k, self).truthy());
                v == (c.rule == Rule::ScriptTrue)
            }
            Rule::PlayerInVolume => in_volume(false),
            Rule::AllPlayersInVolume => in_volume(true),
            Rule::CombatStatusAtLeast => status() >= a,
            Rule::CombatStatusAtMost => status() <= a,
            Rule::Arrived => self
                .bots
                .iter()
                .filter(|(i, _)| actors.contains(i))
                .all(|(i, b)| {
                    b.actor.as_ref().is_none_or(|m| {
                        (self.game.players[*i].body.position - m.post)
                            .truncate()
                            .length()
                            < ARRIVED
                    })
                }),
            Rule::InVehicle => actors.iter().any(|&i| self.game.players[i].seat.is_some()),
            Rule::SightedPlayer => who
                .iter()
                .any(|s| self.st.orders.sighted_player.contains(s)),
            Rule::FightingAtLeast => fighting() >= a,
            Rule::FightingAtMost => fighting() <= a,
            Rule::PlayerWithin => self.humans().iter().any(|&h| {
                let at = self.game.players[h].body.position;
                actors
                    .iter()
                    .any(|&i| self.game.players[i].body.position.distance(at) < c.x)
            }),
            Rule::PlayerShotLongAgo | Rule::SafeToSave => true,
            Rule::AlertedBySquad | Rule::Other(_) => false,
        }
    }

    /// A static script, by name.
    fn static_script(&mut self, name: &str) -> Option<usize> {
        if let Some(&k) = self.st.orders.scripts.get(name) {
            return k;
        }
        let k = self
            .scene
            .ai
            .scripts
            .scripts
            .iter()
            .position(|s| s.name == name);
        self.st.orders.scripts.insert(name.to_string(), k);
        k
    }
}
