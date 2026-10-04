//! The game in bytes, for LAN play: the host sends everything a joined PC
//! needs to draw the game (players, items, grenades, what happened), and
//! joined PCs send their players' controls.

use super::{
    DroppedWeapon, Event, Flag, FlagEvent, Game, GameType, Grenade, GrenadeKind, HeldWeapon,
    HillControl, HillEvent, ItemKind, LeadChange, Look, Medal, Powerup, Projectile, Spartan,
    StuckRound, DROPPED_WEAPON_LIFETIME, NEUTRAL, TEAMS,
};
use crate::game::Command;
use crate::weapon::WeaponState;
use glam::{Vec2, Vec3};

/// Bytes that don't decode: a different version, or a broken connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("malformed game data")
    }
}

impl std::error::Error for Malformed {}

/// Little-endian encoder.
#[derive(Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    pub fn vec2(&mut self, v: Vec2) {
        self.f32(v.x);
        self.f32(v.y);
    }
    pub fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    pub fn str(&mut self, s: &str) {
        let b = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
        self.u16(b.len() as u16);
        self.0.extend_from_slice(b);
    }
    /// A small index (player, weapon); `None` is 0xFFFF.
    pub fn index(&mut self, v: Option<usize>) {
        self.u16(v.map_or(u16::MAX, |i| i.min(u16::MAX as usize - 1) as u16));
    }
    pub fn opt_f32(&mut self, v: Option<f32>) {
        self.bool(v.is_some());
        if let Some(v) = v {
            self.f32(v);
        }
    }
}

/// Little-endian decoder; every read fails cleanly past the end.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }
    fn take<const N: usize>(&mut self) -> Result<[u8; N], Malformed> {
        let end = self.pos.checked_add(N).ok_or(Malformed)?;
        let b = self.data.get(self.pos..end).ok_or(Malformed)?;
        self.pos = end;
        b.try_into().map_err(|_| Malformed)
    }
    pub fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.take::<1>()?[0])
    }
    pub fn u16(&mut self) -> Result<u16, Malformed> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    pub fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    pub fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    pub fn f32(&mut self) -> Result<f32, Malformed> {
        Ok(f32::from_le_bytes(self.take()?))
    }
    pub fn f64(&mut self) -> Result<f64, Malformed> {
        Ok(f64::from_le_bytes(self.take()?))
    }
    pub fn bool(&mut self) -> Result<bool, Malformed> {
        Ok(self.u8()? != 0)
    }
    pub fn vec2(&mut self) -> Result<Vec2, Malformed> {
        Ok(Vec2::new(self.f32()?, self.f32()?))
    }
    pub fn vec3(&mut self) -> Result<Vec3, Malformed> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    pub fn str(&mut self) -> Result<String, Malformed> {
        let n = self.u16()? as usize;
        let end = self.pos.checked_add(n).ok_or(Malformed)?;
        let b = self.data.get(self.pos..end).ok_or(Malformed)?;
        self.pos = end;
        Ok(String::from_utf8_lossy(b).into_owned())
    }
    pub fn index(&mut self) -> Result<Option<usize>, Malformed> {
        let v = self.u16()?;
        Ok((v != u16::MAX).then_some(v as usize))
    }
    pub fn opt_f32(&mut self) -> Result<Option<f32>, Malformed> {
        Ok(if self.bool()? {
            Some(self.f32()?)
        } else {
            None
        })
    }
    /// An index that must be below `limit`.
    pub fn index_below(&mut self, limit: usize) -> Result<usize, Malformed> {
        self.index()?.filter(|&i| i < limit).ok_or(Malformed)
    }
    pub fn at_end(&self) -> bool {
        self.pos == self.data.len()
    }
}

impl Command {
    pub fn write(&self, w: &mut Writer) {
        w.vec2(self.movement);
        w.f32(self.yaw);
        w.f32(self.pitch);
        let flags = [
            self.jump,
            self.crouch,
            self.fire,
            self.zoom,
            self.reload,
            self.melee,
            self.action,
            self.switch_weapon,
            self.throw_grenade,
            self.switch_grenade,
            self.vision,
        ];
        w.u16(
            flags
                .iter()
                .enumerate()
                .fold(0, |acc, (i, &f)| acc | ((f as u16) << i)),
        );
    }

    pub fn read(r: &mut Reader) -> Result<Command, Malformed> {
        let movement = r.vec2()?;
        let yaw = r.f32()?;
        let pitch = r.f32()?;
        let bits = r.u16()?;
        let f = |i: u32| bits & (1 << i) != 0;
        let finite = movement.is_finite() && yaw.is_finite() && pitch.is_finite();
        if !finite {
            return Err(Malformed);
        }
        Ok(Command {
            movement: movement.clamp(Vec2::NEG_ONE, Vec2::ONE),
            yaw,
            pitch,
            jump: f(0),
            crouch: f(1),
            fire: f(2),
            zoom: f(3),
            reload: f(4),
            melee: f(5),
            action: f(6),
            switch_weapon: f(7),
            throw_grenade: f(8),
            switch_grenade: f(9),
            vision: f(10),
            // Only actors fly, and they aren't sent.
            rise: 0.0,
        })
    }

    /// Buttons pressed in either command (held controls from `self`), so a
    /// quick tap between two ticks still counts.
    pub fn with_presses_from(mut self, other: &Command) -> Command {
        self.jump |= other.jump;
        self.crouch |= other.crouch;
        self.fire |= other.fire;
        self.zoom |= other.zoom;
        self.reload |= other.reload;
        self.melee |= other.melee;
        self.action |= other.action;
        self.switch_weapon |= other.switch_weapon;
        self.throw_grenade |= other.throw_grenade;
        self.switch_grenade |= other.switch_grenade;
        self.vision |= other.vision;
        self
    }
}

fn write_kind(w: &mut Writer, kind: ItemKind) {
    match kind {
        ItemKind::Weapon(i) => {
            w.u8(0);
            w.index(Some(i));
        }
        ItemKind::FragGrenades => w.u8(1),
        ItemKind::PlasmaGrenades => w.u8(2),
        ItemKind::Powerup(Powerup::Overshield) => w.u8(3),
        ItemKind::Powerup(Powerup::Camouflage) => w.u8(4),
        ItemKind::Ammo { weapon, rounds } => {
            w.u8(5);
            w.index(Some(weapon));
            w.u32(rounds);
        }
    }
}

fn read_kind(r: &mut Reader, weapons: usize) -> Result<ItemKind, Malformed> {
    Ok(match r.u8()? {
        0 => ItemKind::Weapon(r.index_below(weapons)?),
        1 => ItemKind::FragGrenades,
        2 => ItemKind::PlasmaGrenades,
        3 => ItemKind::Powerup(Powerup::Overshield),
        4 => ItemKind::Powerup(Powerup::Camouflage),
        5 => ItemKind::Ammo {
            weapon: r.index_below(weapons)?,
            rounds: r.u32()?,
        },
        _ => return Err(Malformed),
    })
}

fn team(t: u8) -> Result<u8, Malformed> {
    if t < TEAMS {
        Ok(t)
    } else {
        Err(Malformed)
    }
}

/// A flag's team: a team's, or no one's (the ball).
fn flag_team(t: u8) -> Result<u8, Malformed> {
    if t == NEUTRAL {
        Ok(t)
    } else {
        team(t)
    }
}

/// Most flags, bombs and balls in a game.
const MAX_FLAGS: usize = 4;
/// Most seats in a vehicle.
const MAX_SEATS: usize = 16;

fn grenade_kind(v: u8) -> Result<GrenadeKind, Malformed> {
    match v {
        0 => Ok(GrenadeKind::Frag),
        1 => Ok(GrenadeKind::Plasma),
        _ => Err(Malformed),
    }
}

impl Event {
    pub fn write(&self, w: &mut Writer) {
        match *self {
            Event::Shot {
                player,
                weapon,
                left,
                origin,
                direction,
                hit,
                hit_player,
            } => {
                w.u8(0);
                w.index(Some(player));
                w.index(Some(weapon));
                w.bool(left);
                w.vec3(origin);
                w.vec3(direction);
                w.bool(hit.is_some());
                if let Some((p, n)) = hit {
                    w.vec3(p);
                    w.vec3(n);
                }
                w.index(hit_player);
            }
            Event::Reloaded {
                player,
                left,
                empty,
            } => {
                w.u8(1);
                w.index(Some(player));
                w.bool(left);
                w.bool(empty);
            }
            Event::Switched { player } => {
                w.u8(2);
                w.index(Some(player));
            }
            Event::Melee { player, hit } => {
                w.u8(3);
                w.index(Some(player));
                w.index(hit);
            }
            Event::Thrown { player } => {
                w.u8(4);
                w.index(Some(player));
            }
            Event::Exploded { kind, position } => {
                w.u8(5);
                w.u8(kind as u8);
                w.vec3(position);
            }
            Event::Damaged { player, amount } => {
                w.u8(6);
                w.index(Some(player));
                w.f32(amount);
            }
            Event::Killed {
                killer,
                victim,
                headshot,
            } => {
                w.u8(7);
                w.index(killer);
                w.index(Some(victim));
                w.bool(headshot);
            }
            Event::Spawned { player, yaw } => {
                w.u8(8);
                w.index(Some(player));
                w.f32(yaw);
            }
            Event::PickedUp { player, kind } => {
                w.u8(9);
                w.index(Some(player));
                write_kind(w, kind);
            }
            Event::DryFire { player } => {
                w.u8(10);
                w.index(Some(player));
            }
            Event::Medal { player, medal } => {
                w.u8(11);
                w.index(Some(player));
                let (kind, count) = match medal {
                    Medal::MultiKill(n) => (0, n),
                    Medal::Spree(n) => (1, n),
                };
                w.u8(kind);
                w.u8(count);
            }
            Event::Lead { player, change } => {
                w.u8(12);
                w.index(Some(player));
                w.u8(change as u8);
            }
            Event::Flag { team, player, what } => {
                w.u8(13);
                w.u8(team);
                w.index(player);
                w.u8(what as u8);
            }
            Event::Hill { player, what } => {
                w.u8(14);
                w.index(player);
                w.u8(what as u8);
            }
            Event::Territory { index, team, from } => {
                w.u8(15);
                w.index(Some(index));
                w.u8(team);
                w.u8(from.unwrap_or(u8::MAX));
            }
            Event::Juggernaut { player } => {
                w.u8(16);
                w.index(Some(player));
            }
            Event::Entered {
                player,
                vehicle,
                seat,
            } => {
                w.u8(17);
                w.index(Some(player));
                w.index(Some(vehicle));
                w.index(Some(seat));
            }
            Event::Exited {
                player,
                vehicle,
                seat,
            } => {
                w.u8(18);
                w.index(Some(player));
                w.index(Some(vehicle));
                w.index(Some(seat));
            }
            Event::Hijacked {
                player,
                victim,
                vehicle,
                seat,
            } => {
                w.u8(23);
                w.index(Some(player));
                w.index(Some(victim));
                w.index(Some(vehicle));
                w.index(Some(seat));
            }
            Event::Splattered { player, vehicle } => {
                w.u8(19);
                w.index(Some(player));
                w.index(Some(vehicle));
            }
            Event::VehicleDestroyed { vehicle, position } => {
                w.u8(20);
                w.index(Some(vehicle));
                w.vec3(position);
            }
            Event::VehicleSpawned { vehicle } => {
                w.u8(21);
                w.index(Some(vehicle));
            }
            Event::Impact {
                weapon,
                position,
                normal,
                hit_player,
                exploded,
            } => {
                w.u8(22);
                w.index(Some(weapon));
                w.vec3(position);
                w.vec3(normal);
                w.index(hit_player);
                w.bool(exploded);
            }
            Event::Teleported { player, from, to } => {
                w.u8(24);
                w.index(Some(player));
                w.vec3(from);
                w.vec3(to);
            }
        }
    }

    /// An event about `players` players, `weapons` weapon kinds and
    /// `vehicles` vehicles.
    pub fn read(
        r: &mut Reader,
        players: usize,
        weapons: usize,
        vehicles: usize,
    ) -> Result<Event, Malformed> {
        let opt_player = |r: &mut Reader| -> Result<Option<usize>, Malformed> {
            match r.index()? {
                Some(i) if i >= players => Err(Malformed),
                v => Ok(v),
            }
        };
        Ok(match r.u8()? {
            0 => {
                let player = r.index_below(players)?;
                let weapon = r.index_below(weapons)?;
                let left = r.bool()?;
                let origin = r.vec3()?;
                let direction = r.vec3()?;
                let hit = if r.bool()? {
                    Some((r.vec3()?, r.vec3()?))
                } else {
                    None
                };
                Event::Shot {
                    player,
                    weapon,
                    left,
                    origin,
                    direction,
                    hit,
                    hit_player: opt_player(r)?,
                }
            }
            1 => Event::Reloaded {
                player: r.index_below(players)?,
                left: r.bool()?,
                empty: r.bool()?,
            },
            2 => Event::Switched {
                player: r.index_below(players)?,
            },
            3 => Event::Melee {
                player: r.index_below(players)?,
                hit: opt_player(r)?,
            },
            4 => Event::Thrown {
                player: r.index_below(players)?,
            },
            5 => Event::Exploded {
                kind: grenade_kind(r.u8()?)?,
                position: r.vec3()?,
            },
            6 => Event::Damaged {
                player: r.index_below(players)?,
                amount: r.f32()?,
            },
            7 => Event::Killed {
                killer: opt_player(r)?,
                victim: r.index_below(players)?,
                headshot: r.bool()?,
            },
            8 => Event::Spawned {
                player: r.index_below(players)?,
                yaw: r.f32()?,
            },
            9 => Event::PickedUp {
                player: r.index_below(players)?,
                kind: read_kind(r, weapons)?,
            },
            10 => Event::DryFire {
                player: r.index_below(players)?,
            },
            11 => Event::Medal {
                player: r.index_below(players)?,
                medal: match (r.u8()?, r.u8()?) {
                    (0, n) => Medal::MultiKill(n),
                    (1, n) => Medal::Spree(n),
                    _ => return Err(Malformed),
                },
            },
            12 => Event::Lead {
                player: r.index_below(players)?,
                change: match r.u8()? {
                    0 => LeadChange::Gained,
                    1 => LeadChange::Lost,
                    2 => LeadChange::Tied,
                    _ => return Err(Malformed),
                },
            },
            13 => Event::Flag {
                team: flag_team(r.u8()?)?,
                player: opt_player(r)?,
                what: *FlagEvent::ALL.get(r.u8()? as usize).ok_or(Malformed)?,
            },
            14 => Event::Hill {
                player: opt_player(r)?,
                what: match r.u8()? {
                    0 => HillEvent::Moved,
                    1 => HillEvent::Controlled,
                    2 => HillEvent::Contested,
                    _ => return Err(Malformed),
                },
            },
            15 => Event::Territory {
                index: r.index()?.ok_or(Malformed)?,
                team: team(r.u8()?)?,
                from: match r.u8()? {
                    u8::MAX => None,
                    t => Some(team(t)?),
                },
            },
            16 => Event::Juggernaut {
                player: r.index_below(players)?,
            },
            17 => Event::Entered {
                player: r.index_below(players)?,
                vehicle: r.index_below(vehicles)?,
                seat: r.index_below(MAX_SEATS)?,
            },
            18 => Event::Exited {
                player: r.index_below(players)?,
                vehicle: r.index_below(vehicles)?,
                seat: r.index_below(MAX_SEATS)?,
            },
            23 => Event::Hijacked {
                player: r.index_below(players)?,
                victim: r.index_below(players)?,
                vehicle: r.index_below(vehicles)?,
                seat: r.index_below(MAX_SEATS)?,
            },
            19 => Event::Splattered {
                player: r.index_below(players)?,
                vehicle: r.index_below(vehicles)?,
            },
            20 => Event::VehicleDestroyed {
                vehicle: r.index_below(vehicles)?,
                position: r.vec3()?,
            },
            21 => Event::VehicleSpawned {
                vehicle: r.index_below(vehicles)?,
            },
            22 => Event::Impact {
                weapon: r.index_below(weapons)?,
                position: r.vec3()?,
                normal: r.vec3()?,
                hit_player: opt_player(r)?,
                exploded: r.bool()?,
            },
            24 => Event::Teleported {
                player: r.index_below(players)?,
                from: r.vec3()?,
                to: r.vec3()?,
            },
            _ => return Err(Malformed),
        })
    }
}

impl Game {
    /// Everything another PC needs to show the game as it is now.
    pub fn write_state(&self, w: &mut Writer) {
        w.f64(self.time);
        w.index(self.winner);
        w.u8(self.rules.game_type as u8);
        w.u32(self.rules.score_to_win);
        self.rules.options.write(w);
        w.u8(self.winning_team.unwrap_or(u8::MAX));
        w.u16(self.item_timers.len() as u16);
        for &t in &self.item_timers {
            w.f32(t);
        }
        w.u16(self.players.len() as u16);
        for p in &self.players {
            w.str(&p.name);
            p.look.write(w);
            w.vec3(p.body.position);
            w.vec3(p.body.velocity);
            w.bool(p.body.grounded);
            w.f32(p.body.crouch);
            w.f32(p.yaw);
            w.f32(p.pitch);
            w.f32(p.shield);
            w.f32(p.health);
            w.f32(p.since_damage);
            w.f32(p.camo);
            w.f32(p.reveal);
            w.bool(p.alive);
            w.f32(p.respawn_in);
            w.u8(p.weapons.len().min(2) as u8 | (p.left.is_some() as u8) << 4);
            for h in p.weapons.iter().take(2).chain(&p.left) {
                w.index(Some(h.weapon));
                w.u32(h.state.loaded);
                w.u32(h.state.reserve);
                w.opt_f32(h.state.reloading);
                w.u8(h.state.zoom.min(255) as u8);
                w.f32(h.state.since_shot);
            }
            w.u8(p.current as u8);
            w.u8(p.frags);
            w.u8(p.plasmas);
            w.u8(p.grenade as u8);
            w.u8(p.team);
            w.u32(p.score as u32);
            w.u32(p.kills);
            w.u32(p.deaths);
            w.u32(p.spree);
            w.u32(p.multi_kill);
            w.f64(p.last_kill);
            w.f32(p.readying);
            w.index(p.seat.map(|s| s.0));
            w.index(p.seat.map(|s| s.1));
        }
        w.u16(self.dropped.len() as u16);
        for d in &self.dropped {
            w.index(Some(d.weapon));
            w.vec3(d.position);
            w.f32(d.yaw);
        }
        w.u16(self.grenades.len() as u16);
        for g in &self.grenades {
            w.u8(g.kind as u8);
            w.index(Some(g.owner));
            w.vec3(g.position);
            w.vec3(g.velocity);
            w.opt_f32(g.fuse);
            w.index(g.stuck);
        }
        w.u16(self.projectiles.len() as u16);
        for p in &self.projectiles {
            w.index(Some(p.weapon));
            w.index(Some(p.owner));
            w.vec3(p.position);
            w.vec3(p.velocity);
        }
        w.u16(self.stuck.len() as u16);
        for r in &self.stuck {
            w.index(Some(r.weapon));
            w.index(Some(r.owner));
            w.index(Some(r.victim));
            w.vec3(r.offset);
        }
        w.u8(self.flags.len().min(MAX_FLAGS) as u8);
        for f in self.flags.iter().take(MAX_FLAGS) {
            w.u8(f.team);
            w.vec3(f.home);
            w.vec3(f.position);
            w.index(f.carrier);
            w.f32(f.reset_in);
            w.f32(f.arming);
            w.opt_f32(f.armed);
        }
        // Hills and territories come from the map; only who holds them is sent.
        w.u8(self.hill.min(255) as u8);
        w.f32(self.hill_moves_in);
        match self.hill_control {
            HillControl::Empty => w.u8(0),
            HillControl::Held(p) => {
                w.u8(1);
                w.index(Some(p));
            }
            HillControl::Contested => w.u8(2),
        }
        w.u8(self.territories.len().min(255) as u8);
        for t in self.territories.iter().take(255) {
            w.u8(t.owner.unwrap_or(u8::MAX));
            let (team, so_far) = t.taking.unwrap_or((u8::MAX, 0.0));
            w.u8(team);
            w.f32(so_far);
        }
        w.index(self.juggernaut);
        w.u16(self.vehicles.len() as u16);
        for v in &self.vehicles {
            w.bool(v.destroyed);
            w.vec3(v.center);
            let q = v.rotation;
            for c in [q.x, q.y, q.z, q.w] {
                w.f32(c);
            }
            w.vec3(v.velocity);
            w.vec3(v.spin);
            w.f32(v.health);
            w.vec2(v.aim);
            w.f32(v.steer);
            w.f32(v.roll);
            w.u8(v.compression.len().min(16) as u8);
            for &c in v.compression.iter().take(16) {
                w.f32(c);
            }
            w.u8(v.riders.len().min(MAX_SEATS) as u8);
            for &r in v.riders.iter().take(MAX_SEATS) {
                w.index(r);
            }
            w.bool(v.controls.boost);
            w.bool(v.controls.horn);
        }
    }

    /// Take on the state another PC sent. Players are added as needed; on
    /// an error the game may be partly updated.
    pub fn read_state(&mut self, r: &mut Reader) -> Result<(), Malformed> {
        let weapons = self.weapons.len();
        self.time = r.f64()?;
        let winner = r.index()?;
        self.rules.game_type = *GameType::ALL.get(r.u8()? as usize).ok_or(Malformed)?;
        self.rules.score_to_win = r.u32()?;
        // The host's options, set up here the first time they arrive.
        let options = super::Options::read(r, weapons)?;
        if options != self.rules.options {
            self.apply_options(options);
        }
        self.winning_team = match r.u8()? {
            u8::MAX => None,
            t if t < TEAMS => Some(t),
            _ => return Err(Malformed),
        };
        let n = r.u16()? as usize;
        if n != self.item_timers.len() {
            return Err(Malformed);
        }
        for t in &mut self.item_timers {
            *t = r.f32()?;
        }
        let count = r.u16()? as usize;
        if count > 255 {
            return Err(Malformed);
        }
        while self.players.len() < count {
            let s = self.fresh_spartan(Vec3::ZERO, 0.0);
            self.players.push(s);
        }
        self.players.truncate(count);
        for i in 0..count {
            let mut held = Vec::new();
            let p = &mut self.players[i];
            p.name = super::clean_name(&r.str()?);
            p.look = Look::read(r)?;
            p.body.position = r.vec3()?;
            p.body.velocity = r.vec3()?;
            p.body.grounded = r.bool()?;
            p.body.crouch = r.f32()?.clamp(0.0, 1.0);
            p.yaw = r.f32()?;
            p.pitch = r.f32()?;
            p.shield = r.f32()?;
            p.health = r.f32()?;
            p.since_damage = r.f32()?;
            p.camo = r.f32()?;
            p.reveal = r.f32()?.clamp(0.0, 1.0);
            p.alive = r.bool()?;
            p.respawn_in = r.f32()?;
            let counts = r.u8()?;
            let (n, left) = ((counts & 0xF) as usize, counts >> 4);
            if n > 2 || left > 1 {
                return Err(Malformed);
            }
            for k in 0..n + left as usize {
                let weapon = r.index_below(weapons)?;
                // Keep the weapon's own timers when it's the same weapon.
                let before = if k < n {
                    p.weapons.get(k)
                } else {
                    p.left.as_ref()
                };
                let mut state = match before {
                    Some(h) if h.weapon == weapon => h.state.clone(),
                    _ => WeaponState::new(&self.weapons[weapon]),
                };
                state.loaded = r.u32()?;
                state.reserve = r.u32()?;
                state.reloading = r.opt_f32()?;
                state.zoom = r.u8()? as u32;
                state.since_shot = r.f32()?;
                held.push(HeldWeapon { weapon, state });
            }
            p.left = (left > 0).then(|| held.pop()).flatten();
            p.weapons = held;
            p.current = r.u8()? as usize;
            if p.current >= p.weapons.len().max(1) {
                return Err(Malformed);
            }
            p.frags = r.u8()?;
            p.plasmas = r.u8()?;
            p.grenade = grenade_kind(r.u8()?)?;
            p.team = team(r.u8()?)?;
            p.score = r.u32()? as i32;
            p.kills = r.u32()?;
            p.deaths = r.u32()?;
            p.spree = r.u32()?;
            p.multi_kill = r.u32()?;
            p.last_kill = r.f64()?;
            p.readying = r.f32()?;
            p.seat = match (r.index()?, r.index()?) {
                (Some(v), Some(s)) => Some((v, s)),
                (None, None) => None,
                _ => return Err(Malformed),
            };
        }
        self.winner = match winner {
            Some(w) if w >= count => return Err(Malformed),
            w => w,
        };
        let n = r.u16()? as usize;
        let mut dropped = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            let weapon = r.index_below(weapons)?;
            dropped.push(DroppedWeapon {
                weapon,
                state: WeaponState::new(&self.weapons[weapon]),
                position: r.vec3()?,
                yaw: r.f32()?,
                ttl: DROPPED_WEAPON_LIFETIME,
            });
        }
        self.dropped = dropped;
        let n = r.u16()? as usize;
        let mut grenades = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            grenades.push(Grenade {
                kind: grenade_kind(r.u8()?)?,
                owner: r.index_below(count)?,
                position: r.vec3()?,
                velocity: r.vec3()?,
                fuse: r.opt_f32()?,
                stuck: match r.index()? {
                    Some(i) if i >= count => return Err(Malformed),
                    v => v,
                },
                stuck_offset: Vec3::ZERO,
                age: 0.0,
            });
        }
        self.grenades = grenades;
        let n = r.u16()? as usize;
        let mut projectiles = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            projectiles.push(Projectile {
                weapon: r.index_below(weapons)?,
                owner: r.index_below(count)?,
                position: r.vec3()?,
                velocity: r.vec3()?,
                travelled: 0.0,
                target: None,
                age: 0.0,
            });
        }
        self.projectiles = projectiles;
        let n = r.u16()? as usize;
        let mut stuck = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            stuck.push(StuckRound {
                weapon: r.index_below(weapons)?,
                owner: r.index_below(count)?,
                victim: r.index_below(count)?,
                offset: r.vec3()?,
                fuse: 1.0,
            });
        }
        self.stuck = stuck;
        let n = r.u8()? as usize;
        if n > MAX_FLAGS {
            return Err(Malformed);
        }
        let mut flags = Vec::with_capacity(n);
        for _ in 0..n {
            let mut f = Flag::new(flag_team(r.u8()?)?, r.vec3()?);
            f.position = r.vec3()?;
            f.carrier = match r.index()? {
                Some(i) if i >= count => return Err(Malformed),
                v => v,
            };
            f.reset_in = r.f32()?;
            f.arming = r.f32()?;
            f.armed = r.opt_f32()?;
            flags.push(f);
        }
        self.flags = flags;
        self.hill = r.u8()? as usize;
        if self.hill > 0 && self.hill >= self.hills.len() {
            return Err(Malformed);
        }
        self.hill_moves_in = r.f32()?;
        self.hill_control = match r.u8()? {
            0 => HillControl::Empty,
            1 => HillControl::Held(r.index_below(count)?),
            2 => HillControl::Contested,
            _ => return Err(Malformed),
        };
        let n = r.u8()? as usize;
        if n != self.territories.len() {
            return Err(Malformed);
        }
        for t in &mut self.territories {
            t.owner = match r.u8()? {
                u8::MAX => None,
                v => Some(team(v)?),
            };
            let (who, so_far) = (r.u8()?, r.f32()?);
            t.taking = match who {
                u8::MAX => None,
                v => Some((team(v)?, so_far)),
            };
        }
        self.juggernaut = match r.index()? {
            Some(i) if i >= count => return Err(Malformed),
            v => v,
        };
        // Vehicles come from the map; their state is sent.
        let n = r.u16()? as usize;
        if n != self.vehicles.len() {
            return Err(Malformed);
        }
        for v in &mut self.vehicles {
            v.destroyed = r.bool()?;
            v.center = r.vec3()?;
            let q = glam::Quat::from_xyzw(r.f32()?, r.f32()?, r.f32()?, r.f32()?);
            if !q.is_finite() || q.length() < 0.5 {
                return Err(Malformed);
            }
            v.rotation = q.normalize();
            v.velocity = r.vec3()?;
            v.spin = r.vec3()?;
            v.health = r.f32()?;
            v.aim = r.vec2()?;
            v.steer = r.f32()?;
            v.roll = r.f32()?;
            let k = r.u8()? as usize;
            if k != v.compression.len().min(16) {
                return Err(Malformed);
            }
            for c in v.compression.iter_mut().take(k) {
                *c = r.f32()?;
            }
            let k = r.u8()? as usize;
            if k != v.riders.len().min(MAX_SEATS) {
                return Err(Malformed);
            }
            for rider in v.riders.iter_mut().take(k) {
                *rider = match r.index()? {
                    Some(i) if i >= count => return Err(Malformed),
                    x => x,
                };
            }
            v.controls.boost = r.bool()?;
            v.controls.horn = r.bool()?;
        }
        for p in &self.players {
            if let Some((v, s)) = p.seat {
                if self.vehicles.get(v).is_none_or(|veh| s >= veh.riders.len()) {
                    return Err(Malformed);
                }
            }
        }
        // Carriers hold the flag in hand.
        for (i, p) in self.players.iter_mut().enumerate() {
            let carrying = self.flags.iter().any(|f| f.carrier == Some(i));
            p.objective = match (carrying, self.rules.carried_weapon()) {
                (true, Some(w)) if w < weapons => match p.objective.take() {
                    Some(h) if h.weapon == w => Some(h),
                    _ => Some(HeldWeapon {
                        weapon: w,
                        state: WeaponState::new(&self.weapons[w]),
                    }),
                },
                _ => None,
            };
        }
        Ok(())
    }

    /// A player as they'd be on joining (for tests and tools).
    pub fn blank_player(&self) -> Spartan {
        self.fresh_spartan(Vec3::ZERO, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::{floor, game};
    use crate::game::Emblem;

    #[test]
    fn commands_survive_the_trip() {
        let c = Command {
            movement: Vec2::new(0.5, -1.0),
            yaw: 1.25,
            pitch: -0.3,
            fire: true,
            switch_grenade: true,
            ..Command::default()
        };
        let mut w = Writer::default();
        c.write(&mut w);
        let mut r = Reader::new(&w.0);
        assert_eq!(Command::read(&mut r), Ok(c));
        assert!(r.at_end());
        // Cut short: an error, not a panic.
        assert!(Command::read(&mut Reader::new(&w.0[..5])).is_err());
    }

    #[test]
    fn announcer_events_survive_the_trip() {
        let events = [
            Event::Medal {
                player: 1,
                medal: Medal::MultiKill(3),
            },
            Event::Medal {
                player: 0,
                medal: Medal::Spree(25),
            },
            Event::Lead {
                player: 1,
                change: LeadChange::Tied,
            },
            Event::Flag {
                team: 1,
                player: Some(0),
                what: FlagEvent::Captured,
            },
            Event::Flag {
                team: 0,
                player: None,
                what: FlagEvent::Returned,
            },
            Event::Flag {
                team: NEUTRAL,
                player: Some(1),
                what: FlagEvent::Defused,
            },
            Event::Hill {
                player: Some(1),
                what: HillEvent::Controlled,
            },
            Event::Territory {
                index: 3,
                team: 1,
                from: Some(0),
            },
            Event::Juggernaut { player: 0 },
        ];
        let mut w = Writer::default();
        for e in &events {
            e.write(&mut w);
        }
        let mut r = Reader::new(&w.0);
        for e in &events {
            assert_eq!(Event::read(&mut r, 2, 0, 0), Ok(*e));
        }
        assert!(r.at_end());
    }

    #[test]
    fn a_joined_game_matches_the_host() {
        let world = floor();
        let mut host = game();
        host.rules.game_type = GameType::TeamSlayer;
        let a = host.add_player();
        let b = host.add_player();
        host.players[b].score = -2;
        host.set_look(
            b,
            Look {
                elite: true,
                colors: [5, 9],
                emblem: Emblem {
                    foreground: 40,
                    background: 7,
                    colors: [1, 2, 3],
                },
            },
        );
        host.players[a].body.position = Vec3::new(0.0, 0.0, 0.0);
        host.players[b].body.position = Vec3::new(5.0, 0.0, 0.0);
        // Player b holds a second gun in the left hand.
        host.players[b].left = Some(HeldWeapon {
            weapon: 1,
            state: WeaponState::new(&host.weapons[1]),
        });
        host.players[b].left.as_mut().unwrap().state.loaded = 5;
        let mut events = Vec::new();
        for _ in 0..30 {
            let fire = Command {
                fire: true,
                ..Command::default()
            };
            host.step(&world, &[fire, Command::default()]);
            events.append(&mut host.events);
        }
        assert!(!events.is_empty());

        let mut w = Writer::default();
        host.write_state(&mut w);
        for e in &events {
            e.write(&mut w);
        }
        let mut joined = game();
        let mut r = Reader::new(&w.0);
        joined.read_state(&mut r).unwrap();
        let read: Vec<Event> = (0..events.len())
            .map(|_| {
                let (p, w, v) = (
                    joined.players.len(),
                    joined.weapons.len(),
                    joined.vehicles.len(),
                );
                Event::read(&mut r, p, w, v).unwrap()
            })
            .collect();
        assert!(r.at_end());
        assert_eq!(read, events);
        assert_eq!(joined.players.len(), 2);
        for (h, j) in host.players.iter().zip(&joined.players) {
            assert_eq!(h.body.position, j.body.position);
            assert_eq!(h.shield, j.shield);
            assert_eq!(h.health, j.health);
            assert_eq!(h.alive, j.alive);
            assert_eq!((h.team, h.score), (j.team, j.score));
            assert_eq!(h.name, j.name);
            assert_eq!(h.look, j.look);
            assert_eq!(h.weapons.len(), j.weapons.len());
            assert_eq!(
                h.held().map(|w| w.state.loaded),
                j.held().map(|w| w.state.loaded)
            );
            assert_eq!(
                h.left.as_ref().map(|w| (w.weapon, w.state.loaded)),
                j.left.as_ref().map(|w| (w.weapon, w.state.loaded))
            );
        }
        assert!(joined.players[1].left.is_some());
        assert_eq!(joined.time, host.time);
        assert_eq!(joined.rules.game_type, GameType::TeamSlayer);
        assert_eq!(joined.players[1].team, 1);

        // The host's game options reach the joined PC with the next state.
        let swat = crate::game::Options {
            map_weapons: crate::game::MapWeapons::None,
            vehicles: false,
            shields: false,
            radar: false,
        };
        host.apply_options(swat);
        let mut w = Writer::default();
        host.write_state(&mut w);
        joined.read_state(&mut Reader::new(&w.0)).unwrap();
        assert_eq!(joined.rules.options, swat);
        assert_eq!(joined.rules.shield, 0.0);
        assert_eq!(joined.item_timers, host.item_timers);

        // Truncated or garbage data is refused without panicking.
        for cut in [1, 10, w.0.len() / 2] {
            let mut g = game();
            assert!(g.read_state(&mut Reader::new(&w.0[..cut])).is_err());
        }
    }
}
