//! The game in bytes, for LAN play: the host sends everything a joined PC
//! needs to draw the game (players, items, grenades, what happened), and
//! joined PCs send their players' controls.

use super::{
    DroppedWeapon, Event, Game, Grenade, GrenadeKind, HeldWeapon, ItemKind, Spartan,
    DROPPED_WEAPON_LIFETIME,
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
    }
}

fn read_kind(r: &mut Reader, weapons: usize) -> Result<ItemKind, Malformed> {
    Ok(match r.u8()? {
        0 => ItemKind::Weapon(r.index_below(weapons)?),
        1 => ItemKind::FragGrenades,
        2 => ItemKind::PlasmaGrenades,
        _ => return Err(Malformed),
    })
}

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
                origin,
                direction,
                hit,
                hit_player,
            } => {
                w.u8(0);
                w.index(Some(player));
                w.vec3(origin);
                w.vec3(direction);
                w.bool(hit.is_some());
                if let Some((p, n)) = hit {
                    w.vec3(p);
                    w.vec3(n);
                }
                w.index(hit_player);
            }
            Event::Reloaded { player, empty } => {
                w.u8(1);
                w.index(Some(player));
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
        }
    }

    /// An event about `players` players and `weapons` weapon kinds.
    pub fn read(r: &mut Reader, players: usize, weapons: usize) -> Result<Event, Malformed> {
        let opt_player = |r: &mut Reader| -> Result<Option<usize>, Malformed> {
            match r.index()? {
                Some(i) if i >= players => Err(Malformed),
                v => Ok(v),
            }
        };
        Ok(match r.u8()? {
            0 => {
                let player = r.index_below(players)?;
                let origin = r.vec3()?;
                let direction = r.vec3()?;
                let hit = if r.bool()? {
                    Some((r.vec3()?, r.vec3()?))
                } else {
                    None
                };
                Event::Shot {
                    player,
                    origin,
                    direction,
                    hit,
                    hit_player: opt_player(r)?,
                }
            }
            1 => Event::Reloaded {
                player: r.index_below(players)?,
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
            _ => return Err(Malformed),
        })
    }
}

impl Game {
    /// Everything another PC needs to show the game as it is now.
    pub fn write_state(&self, w: &mut Writer) {
        w.f64(self.time);
        w.index(self.winner);
        w.u16(self.item_timers.len() as u16);
        for &t in &self.item_timers {
            w.f32(t);
        }
        w.u16(self.players.len() as u16);
        for p in &self.players {
            w.vec3(p.body.position);
            w.vec3(p.body.velocity);
            w.bool(p.body.grounded);
            w.f32(p.body.crouch);
            w.f32(p.yaw);
            w.f32(p.pitch);
            w.f32(p.shield);
            w.f32(p.health);
            w.f32(p.since_damage);
            w.bool(p.alive);
            w.f32(p.respawn_in);
            w.u8(p.weapons.len().min(2) as u8);
            for h in p.weapons.iter().take(2) {
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
            w.u32(p.kills);
            w.u32(p.deaths);
            w.f32(p.readying);
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
    }

    /// Take on the state another PC sent. Players are added as needed; on
    /// an error the game may be partly updated.
    pub fn read_state(&mut self, r: &mut Reader) -> Result<(), Malformed> {
        let weapons = self.weapons.len();
        self.time = r.f64()?;
        let winner = r.index()?;
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
            p.body.position = r.vec3()?;
            p.body.velocity = r.vec3()?;
            p.body.grounded = r.bool()?;
            p.body.crouch = r.f32()?.clamp(0.0, 1.0);
            p.yaw = r.f32()?;
            p.pitch = r.f32()?;
            p.shield = r.f32()?;
            p.health = r.f32()?;
            p.since_damage = r.f32()?;
            p.alive = r.bool()?;
            p.respawn_in = r.f32()?;
            let n = r.u8()? as usize;
            if n > 2 {
                return Err(Malformed);
            }
            for k in 0..n {
                let weapon = r.index_below(weapons)?;
                // Keep the weapon's own timers when it's the same weapon.
                let mut state = match p.weapons.get(k) {
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
            p.weapons = held;
            p.current = r.u8()? as usize;
            if p.current >= p.weapons.len().max(1) {
                return Err(Malformed);
            }
            p.frags = r.u8()?;
            p.plasmas = r.u8()?;
            p.grenade = grenade_kind(r.u8()?)?;
            p.kills = r.u32()?;
            p.deaths = r.u32()?;
            p.readying = r.f32()?;
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
    fn a_joined_game_matches_the_host() {
        let world = floor();
        let mut host = game();
        let a = host.add_player();
        let b = host.add_player();
        host.players[a].body.position = Vec3::new(0.0, 0.0, 0.0);
        host.players[b].body.position = Vec3::new(5.0, 0.0, 0.0);
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
            .map(|_| Event::read(&mut r, joined.players.len(), joined.weapons.len()).unwrap())
            .collect();
        assert!(r.at_end());
        assert_eq!(read, events);
        assert_eq!(joined.players.len(), 2);
        for (h, j) in host.players.iter().zip(&joined.players) {
            assert_eq!(h.body.position, j.body.position);
            assert_eq!(h.shield, j.shield);
            assert_eq!(h.health, j.health);
            assert_eq!(h.alive, j.alive);
            assert_eq!(h.weapons.len(), j.weapons.len());
            assert_eq!(
                h.held().map(|w| w.state.loaded),
                j.held().map(|w| w.state.loaded)
            );
        }
        assert_eq!(joined.time, host.time);

        // Truncated or garbage data is refused without panicking.
        for cut in [1, 10, w.0.len() / 2] {
            let mut g = game();
            assert!(g.read_state(&mut Reader::new(&w.0[..cut])).is_err());
        }
    }
}
