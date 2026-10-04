//! Campaign missions: the actors (Grunts, Elites, Marines...) a mission
//! places, who fight but never respawn, and checkpoints, where a player who
//! dies comes back.

use super::{Actor, Game, GameType, GrenadeKind, HeldWeapon, Look, Vitality};
use crate::weapon::WeaponState;
use blam_cache::physics::BipedPhysics;
use glam::Vec3;

/// Seconds a dead actor lies there before its slot can take a new one.
const CORPSE_TIME: f32 = 30.0;
/// Seconds between checkpoints, at the most.
const CHECKPOINT_EVERY: f32 = 8.0;
/// No checkpoint with an enemy actor closer than this.
const CHECKPOINT_CLEAR: f32 = 12.0;

/// Which side of the war a character is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    #[default]
    Human,
    Covenant,
    Flood,
    Sentinel,
}

impl Side {
    /// Its team number in a campaign game.
    pub fn team(self) -> u8 {
        match self {
            Side::Human => 0,
            Side::Covenant => 1,
            Side::Flood => 2,
            Side::Sentinel => 3,
        }
    }
}

/// A kind of actor: how tough it is, how it moves and sees, and how it
/// fights.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterDef {
    pub name: String,
    pub side: Side,
    pub biped: BipedPhysics,
    /// On normal and on legendary (heroic is in between).
    pub vitality: Vitality,
    pub legendary: Vitality,
    pub mind: Mind,
    /// The weapon it carries when its squad doesn't say (a weapon index).
    pub weapon: Option<usize>,
    pub grenade: GrenadeKind,
    pub grenades: u8,
}

/// How an actor perceives and fights, from its character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mind {
    /// How far it sees, and the half-angle it sees in (radians).
    pub sight: f32,
    pub fov: f32,
    /// Notices anyone this close, whichever way it faces.
    pub peripheral: f32,
    /// Shoots at targets no farther than this.
    pub fire_range: f32,
    /// The distance it likes to fight at.
    pub combat_range: (f32, f32),
    /// 0 (wild) to 1 (dead on), over a run of shooting.
    pub accuracy: (f32, f32),
    /// Charges in to melee within this distance, this likely per second.
    pub melee_range: f32,
    pub melee_chance: f32,
    /// Grenades: likely per second, seconds between, and the range thrown
    /// at.
    pub grenade_chance: f32,
    pub grenade_delay: f32,
    pub grenade_range: (f32, f32),
    /// Runs from danger (Grunts) rather than holding its ground.
    pub skittish: bool,
}

impl Default for Mind {
    fn default() -> Mind {
        Mind {
            sight: 30.0,
            fov: 0.87,
            peripheral: 4.0,
            fire_range: 24.0,
            combat_range: (2.0, 7.0),
            accuracy: (0.5, 1.0),
            melee_range: 1.0,
            melee_chance: 0.5,
            grenade_chance: 0.25,
            grenade_delay: 6.0,
            grenade_range: (2.0, 15.0),
            skittish: false,
        }
    }
}

/// Where and as what an actor comes into the level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorSpawn {
    pub character: usize,
    pub squad: u16,
    pub position: Vec3,
    pub yaw: f32,
    /// Weapon indices, or the character's own.
    pub weapon: Option<usize>,
    pub secondary: Option<usize>,
    /// 0 easy ... 3 legendary.
    pub difficulty: u8,
    /// Its side (a team number), when the squad says.
    pub team: Option<u8>,
}

impl Game {
    /// Bring an actor into the level; returns its player index (a dead
    /// actor's slot, once it has lain there a while).
    pub fn spawn_actor(&mut self, spawn: ActorSpawn) -> Option<usize> {
        let def = self.characters.get(spawn.character)?.clone();
        let full = match spawn.difficulty {
            0 | 1 => def.vitality,
            2 => Vitality {
                shield: (def.vitality.shield + def.legendary.shield) / 2.0,
                health: (def.vitality.health + def.legendary.health) / 2.0,
                ..def.vitality
            },
            _ => def.legendary,
        };
        let mut s = self.fresh_spartan(spawn.position + Vec3::Z * 0.05, spawn.yaw);
        s.body.biped = def.biped;
        s.name = def.name.clone();
        s.look = Look::default();
        s.team = spawn.team.unwrap_or(def.side.team());
        s.full = full;
        s.shield = full.shield;
        s.health = full.health;
        s.actor = Some(Actor {
            character: spawn.character,
            squad: spawn.squad,
        });
        s.weapons = [spawn.weapon.or(def.weapon), spawn.secondary]
            .into_iter()
            .flatten()
            .filter_map(|w| {
                Some(HeldWeapon {
                    weapon: w,
                    state: WeaponState::new(self.weapons.get(w)?),
                })
            })
            .collect();
        s.frags = 0;
        s.plasmas = 0;
        match def.grenade {
            GrenadeKind::Frag => s.frags = def.grenades,
            GrenadeKind::Plasma => s.plasmas = def.grenades,
        }
        s.grenade = def.grenade;
        let slot = (0..self.players.len()).find(|&i| {
            let p = &self.players[i];
            p.actor.is_some() && !p.alive && p.respawn_in <= 0.0
        });
        let i = match slot {
            Some(i) => {
                self.players[i] = s;
                i
            }
            None => {
                self.players.push(s);
                self.players.len() - 1
            }
        };
        self.events.push(super::Event::Spawned {
            player: i,
            yaw: spawn.yaw,
        });
        Some(i)
    }

    /// Actors of a squad still alive.
    pub fn squad_alive(&self, squad: u16) -> usize {
        self.players
            .iter()
            .filter(|p| p.alive && p.actor.is_some_and(|a| a.squad == squad))
            .count()
    }

    /// How long a dead actor lies before its slot is reused.
    pub(super) fn corpse_time(&self) -> f32 {
        CORPSE_TIME
    }

    /// Note where the players are safe (alive, on the ground, no enemy
    /// actor close by), to bring them back there when they die.
    pub(super) fn step_checkpoint(&mut self, dt: f32) {
        if self.rules.game_type != GameType::Campaign {
            return;
        }
        self.since_checkpoint += dt;
        if self.since_checkpoint < CHECKPOINT_EVERY {
            return;
        }
        let people: Vec<usize> = (0..self.players.len())
            .filter(|&i| self.players[i].actor.is_none())
            .collect();
        let safe = |i: usize| {
            let p = &self.players[i];
            p.alive
                && p.body.grounded
                && p.seat.is_none()
                && !self.players.iter().enumerate().any(|(j, q)| {
                    q.alive
                        && q.actor.is_some()
                        && self.is_enemy(i, j)
                        && q.body.position.distance(p.body.position) < CHECKPOINT_CLEAR
                })
        };
        if let Some(&first) = people.first() {
            if people.iter().all(|&i| safe(i)) {
                let p = &self.players[first];
                self.checkpoint = Some((p.body.position, p.yaw));
                self.since_checkpoint = 0.0;
            }
        }
    }

    /// Where a campaign player comes back: the last checkpoint.
    pub(super) fn checkpoint_spawn(&self) -> Option<(Vec3, f32)> {
        if self.rules.game_type != GameType::Campaign {
            return None;
        }
        self.checkpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Command;
    use crate::testing::{floor, game};

    fn grunt() -> CharacterDef {
        CharacterDef {
            name: "grunt".into(),
            side: Side::Covenant,
            biped: BipedPhysics::default(),
            vitality: Vitality {
                shield: 0.0,
                health: 25.0,
                recharge: 1.0,
            },
            legendary: Vitality {
                shield: 0.0,
                health: 40.0,
                recharge: 1.0,
            },
            mind: Mind::default(),
            weapon: Some(0),
            grenade: GrenadeKind::Plasma,
            grenades: 2,
        }
    }

    fn campaign() -> Game {
        let mut g = game();
        g.rules.game_type = GameType::Campaign;
        let biped = g.biped;
        g.characters.push(CharacterDef { biped, ..grunt() });
        g
    }

    fn spawn(g: &mut Game, at: Vec3) -> usize {
        g.spawn_actor(ActorSpawn {
            character: 0,
            squad: 0,
            position: at,
            yaw: 0.0,
            weapon: None,
            secondary: None,
            difficulty: 1,
            team: None,
        })
        .unwrap()
    }

    #[test]
    fn actors_stay_dead_and_their_slots_are_reused() {
        let world = floor();
        let mut g = campaign();
        let me = g.add_player();
        let grunt = spawn(&mut g, Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(g.players[grunt].team, 1);
        assert_eq!(g.players[grunt].health, 25.0);
        assert_eq!(g.players[grunt].plasmas, 2);
        assert!(g.is_enemy(me, grunt));
        g.damage(grunt, Some(me), 30.0, false);
        assert!(!g.players[grunt].alive);
        for _ in 0..60 * 10 {
            g.step(&world, &[Command::default()]);
        }
        assert!(!g.players[grunt].alive, "actors don't respawn");
        assert_eq!(spawn(&mut g, Vec3::ZERO), grunt + 1, "corpse still there");
        g.players[grunt].respawn_in = 0.0;
        assert_eq!(spawn(&mut g, Vec3::ZERO), grunt, "slot reused");
    }

    #[test]
    fn actors_do_not_hurt_their_own_side_but_players_can() {
        let mut g = campaign();
        let me = g.add_player();
        let a = spawn(&mut g, Vec3::new(5.0, 0.0, 0.0));
        let b = spawn(&mut g, Vec3::new(6.0, 0.0, 0.0));
        g.rules.friendly_fire = true;
        g.damage(b, Some(a), 10.0, false);
        assert_eq!(g.players[b].health, 25.0);
        g.players[me].team = 1;
        g.damage(b, Some(me), 10.0, false);
        assert_eq!(g.players[b].health, 15.0);
    }

    #[test]
    fn dead_players_come_back_at_the_last_checkpoint() {
        let world = floor();
        let mut g = campaign();
        let me = g.add_player();
        let safe = Vec3::new(3.0, 2.0, 0.0);
        g.players[me].body.position = safe;
        for _ in 0..60 * 10 {
            g.step(&world, &[Command::default()]);
        }
        let (at, _) = g.checkpoint.expect("a checkpoint");
        assert!(at.truncate().distance(safe.truncate()) < 0.1);
        g.damage(me, None, 1000.0, false);
        for _ in 0..60 * 10 {
            g.step(&world, &[Command::default()]);
        }
        assert!(g.players[me].alive);
        assert!(
            g.players[me]
                .body
                .position
                .truncate()
                .distance(safe.truncate())
                < 0.2
        );
    }
}
