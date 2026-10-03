//! Things carried in hand: Capture the Flag's flags, Oddball's ball and
//! Assault's bombs. A carrier can only melee, and drops what they carry on
//! dying or switching weapons; dropped, it goes home by itself after a
//! while.
//!
//! Capture the Flag: take the other team's flag (the action key) and carry
//! it to your own base. Oddball: whoever holds the ball scores a point a
//! second. Assault: carry your own bomb into the enemy base and stand there
//! to arm it; it goes off a few seconds later unless an enemy defuses it.

use super::{Event, Game, GameType, GrenadeKind, HeldWeapon};
use crate::collision::World;
use crate::weapon::WeaponState;
use glam::{Vec2, Vec3};

/// The team of something no team owns (the ball), as Halo 2 numbers it.
pub const NEUTRAL: u8 = 8;
/// How close (across the floor) a player has to be to take a flag.
const FLAG_REACH: f32 = 0.7;
/// How close a carrier has to get to their base to score.
const CAPTURE_RADIUS: f32 = 1.0;
/// How far above or below a flag or base a player can be and still reach it.
const REACH_HEIGHT: f32 = 1.0;
/// Someone who drops a flag can't take it straight back.
const REGRAB_DELAY: f32 = 1.0;
/// Seconds between "can't score" warnings while standing in the base.
const FAILURE_REPEAT: f64 = 3.0;
/// A bomb going off kills everyone this close.
const BOMB_RADIUS: f32 = 4.0;
const BOMB_DAMAGE: f32 = 500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagEvent {
    Taken,
    Dropped,
    /// Sent home: touched by its team, or left lying too long.
    Returned,
    Captured,
    /// Reached the base, but the team's own flag isn't home.
    CaptureFailed,
    /// A bomb set down and armed at the enemy base.
    Armed,
    /// The bomb went off (and scored).
    Detonated,
    /// An enemy disarmed the bomb (and it went home).
    Defused,
}

impl FlagEvent {
    pub const ALL: [FlagEvent; 8] = [
        FlagEvent::Taken,
        FlagEvent::Dropped,
        FlagEvent::Returned,
        FlagEvent::Captured,
        FlagEvent::CaptureFailed,
        FlagEvent::Armed,
        FlagEvent::Detonated,
        FlagEvent::Defused,
    ];
}

/// A team's flag (or bomb), or the ball.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flag {
    /// The team it belongs to ([`NEUTRAL`] for the ball).
    pub team: u8,
    /// Where it stands at its base.
    pub home: Vec3,
    pub position: Vec3,
    pub carrier: Option<usize>,
    /// While dropped: seconds before it goes home by itself.
    pub reset_in: f32,
    /// The player who dropped it, and how long before they can take it again.
    pub dropped_by: Option<(usize, f32)>,
    /// Seconds spent arming (carried, at the enemy base) or defusing (armed).
    pub arming: f32,
    /// An armed bomb: seconds until it goes off, and who set it.
    pub armed: Option<f32>,
    pub planter: Option<usize>,
    pub(super) failed_at: f64,
}

impl Flag {
    pub fn new(team: u8, home: Vec3) -> Flag {
        Flag {
            team,
            home,
            position: home,
            carrier: None,
            reset_in: 0.0,
            dropped_by: None,
            arming: 0.0,
            armed: None,
            planter: None,
            failed_at: f64::NEG_INFINITY,
        }
    }

    pub fn at_home(&self) -> bool {
        self.carrier.is_none() && self.armed.is_none() && self.position == self.home
    }
}

fn within(a: Vec3, b: Vec3, radius: f32) -> bool {
    Vec2::new(a.x - b.x, a.y - b.y).length() < radius && (a.z - b.z).abs() < REACH_HEIGHT
}

impl Game {
    /// Put each team's flag (or bomb, or the ball) at its base (`homes`),
    /// and set where teams bring them to score (`bases`: in Capture the
    /// Flag a team's own; in Assault, the enemy's; a team without one uses
    /// its flag's home).
    pub fn set_flags(&mut self, homes: &[(u8, Vec3)], bases: &[(u8, Vec3)]) {
        self.flags = homes.iter().map(|&(t, p)| Flag::new(t, p)).collect();
        self.flag_bases = bases.to_vec();
        for f in &self.flags {
            if f.team != NEUTRAL && !self.flag_bases.iter().any(|b| b.0 == f.team) {
                self.flag_bases.push((f.team, f.home));
            }
        }
    }

    /// This game is played with flags, bombs or a ball, and the map has them.
    pub fn has_flags(&self) -> bool {
        match self.rules.game_type {
            GameType::Ctf | GameType::Assault => self.flags.len() >= 2,
            GameType::Oddball | GameType::TeamOddball => !self.flags.is_empty(),
            _ => false,
        }
    }

    /// The flag a player is carrying.
    pub fn carried_flag(&self, player: usize) -> Option<usize> {
        self.flags.iter().position(|f| f.carrier == Some(player))
    }

    /// Where a team brings the enemy flag.
    pub fn flag_base(&self, team: u8) -> Option<Vec3> {
        self.flag_bases.iter().find(|b| b.0 == team).map(|b| b.1)
    }

    /// Where a team's bomb is armed: the enemy bases nearest to `from`.
    pub fn bomb_target(&self, team: u8, from: Vec3) -> Option<Vec3> {
        self.flag_bases
            .iter()
            .filter(|b| b.0 != team)
            .map(|b| b.1)
            .min_by(|a, b| a.distance(from).total_cmp(&b.distance(from)))
    }

    /// Whether `player` can pick up flag `f` (lying there, not armed): the
    /// enemy's flag, their own team's bomb, or the ball.
    pub fn can_take(&self, player: usize, f: usize) -> bool {
        let flag = &self.flags[f];
        if flag.carrier.is_some() || flag.armed.is_some() {
            return false;
        }
        let team = self.players[player].team;
        match self.rules.game_type {
            GameType::Ctf => flag.team != team,
            GameType::Assault => flag.team == team,
            _ => true,
        }
    }

    /// Take, return, score with, arm or defuse flags the player is touching.
    pub(super) fn touch_flags(&mut self, i: usize, action: bool, dt: f32) {
        if !self.has_flags() {
            return;
        }
        let me = self.players[i].body.position;
        let team = self.players[i].team;
        for f in 0..self.flags.len() {
            let flag = self.flags[f];
            if flag.carrier.is_some() || !within(me, flag.position, FLAG_REACH) {
                continue;
            }
            if flag.armed.is_some() {
                if flag.team != team {
                    self.defuse(f, i, action, dt);
                }
                continue;
            }
            if !self.can_take(i, f) {
                if self.rules.flag_touch_return && !flag.at_home() {
                    self.return_flag(f, Some(i));
                }
                continue;
            }
            let barred = flag.dropped_by.is_some_and(|(p, _)| p == i);
            if action && !barred && self.players[i].objective.is_none() {
                self.take_flag(f, i);
            }
        }
        let Some(f) = self.carried_flag(i) else {
            return;
        };
        match self.rules.game_type {
            GameType::Ctf => self.try_capture(f, i),
            GameType::Assault => self.arm_bomb(f, i, dt),
            _ => {}
        }
    }

    /// Scoring: the enemy flag in hand, at the team's own base.
    fn try_capture(&mut self, f: usize, i: usize) {
        let (me, team) = (self.players[i].body.position, self.players[i].team);
        let Some(base) = self.flag_base(team) else {
            return;
        };
        if !within(me, base, CAPTURE_RADIUS) {
            return;
        }
        let own_home = self
            .flags
            .iter()
            .filter(|g| g.team == team)
            .all(|g| g.at_home());
        if self.rules.flag_at_home_to_score && !own_home {
            if self.time - self.flags[f].failed_at > FAILURE_REPEAT {
                self.flags[f].failed_at = self.time;
                self.flag_event(f, Some(i), FlagEvent::CaptureFailed);
            }
            return;
        }
        self.capture(f, i);
    }

    /// A bomb carrier standing at an enemy base arms the bomb there.
    fn arm_bomb(&mut self, f: usize, i: usize, dt: f32) {
        let (me, team) = (self.players[i].body.position, self.players[i].team);
        let post = self
            .flag_bases
            .iter()
            .find(|b| b.0 != team && within(me, b.1, CAPTURE_RADIUS))
            .map(|b| b.1);
        let Some(post) = post else {
            self.flags[f].arming = 0.0;
            return;
        };
        self.flags[f].arming += dt;
        if self.flags[f].arming < self.rules.bomb_arm_time {
            return;
        }
        let p = &mut self.players[i];
        p.objective = None;
        p.readying = super::ready_time(&self.weapons, p);
        self.events.push(Event::Switched { player: i });
        let fuse = self.rules.bomb_fuse;
        let flag = &mut self.flags[f];
        flag.carrier = None;
        flag.position = post;
        flag.arming = 0.0;
        flag.armed = Some(fuse);
        flag.planter = Some(i);
        flag.dropped_by = None;
        self.flag_event(f, Some(i), FlagEvent::Armed);
    }

    /// An enemy holding the action key at an armed bomb disarms it.
    fn defuse(&mut self, f: usize, i: usize, action: bool, dt: f32) {
        let flag = &mut self.flags[f];
        if !action {
            flag.arming = 0.0;
            return;
        }
        flag.arming += dt;
        if flag.arming < self.rules.bomb_arm_time {
            return;
        }
        flag.armed = None;
        flag.planter = None;
        flag.arming = 0.0;
        flag.position = flag.home;
        flag.dropped_by = None;
        self.flag_event(f, Some(i), FlagEvent::Defused);
    }

    /// The bomb goes off: everyone near it dies, and its team scores.
    fn detonate(&mut self, f: usize) {
        let flag = self.flags[f];
        let at = flag.position;
        let planter = flag.planter.filter(|&p| p < self.players.len());
        for j in 0..self.players.len() {
            let p = &self.players[j];
            let centre = p.body.position + Vec3::Z * p.body.height() * 0.5;
            if p.alive && centre.distance(at) < BOMB_RADIUS {
                self.damage(j, planter, BOMB_DAMAGE, false);
            }
        }
        self.events.push(Event::Exploded {
            kind: GrenadeKind::Frag,
            position: at,
        });
        let flag = &mut self.flags[f];
        flag.armed = None;
        flag.planter = None;
        flag.arming = 0.0;
        flag.position = flag.home;
        flag.dropped_by = None;
        if let Some(p) = planter {
            let leaders = self.leaders();
            self.players[p].score += 1;
            self.flag_event(f, Some(p), FlagEvent::Detonated);
            self.lead_changes(&leaders);
            self.check_win(p);
        } else {
            self.flag_event(f, None, FlagEvent::Detonated);
        }
    }

    fn take_flag(&mut self, f: usize, i: usize) {
        let Some(w) = self.rules.carried_weapon() else {
            return;
        };
        let Some(def) = self.weapons.get(w) else {
            return;
        };
        let held = HeldWeapon {
            weapon: w,
            state: WeaponState::new(def),
        };
        let ready = def.ready_time;
        // It takes both hands.
        self.drop_left(i);
        let p = &mut self.players[i];
        if let Some(h) = p.weapons.get_mut(p.current) {
            h.state.zoom = 0;
            h.state.reloading = None;
        }
        p.objective = Some(held);
        p.readying = ready;
        let flag = &mut self.flags[f];
        flag.carrier = Some(i);
        flag.dropped_by = None;
        flag.arming = 0.0;
        self.events.push(Event::Switched { player: i });
        self.flag_event(f, Some(i), FlagEvent::Taken);
    }

    /// Let go of the flag in hand where the player stands.
    pub(super) fn drop_flag(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.objective.take().is_none() {
            return;
        }
        p.readying = super::ready_time(&self.weapons, p);
        let at = p.body.position;
        if p.alive {
            self.events.push(Event::Switched { player: i });
        }
        let reset = self.rules.flag_reset_time;
        let Some(f) = self.carried_flag(i) else {
            return;
        };
        let flag = &mut self.flags[f];
        flag.carrier = None;
        flag.position = at;
        flag.reset_in = reset;
        flag.arming = 0.0;
        flag.dropped_by = Some((i, REGRAB_DELAY));
        self.flag_event(f, Some(i), FlagEvent::Dropped);
    }

    fn return_flag(&mut self, f: usize, by: Option<usize>) {
        let flag = &mut self.flags[f];
        flag.position = flag.home;
        flag.dropped_by = None;
        self.flag_event(f, by, FlagEvent::Returned);
    }

    fn capture(&mut self, f: usize, i: usize) {
        let leaders = self.leaders();
        let p = &mut self.players[i];
        p.objective = None;
        p.readying = super::ready_time(&self.weapons, p);
        p.score += 1;
        self.events.push(Event::Switched { player: i });
        let flag = &mut self.flags[f];
        flag.carrier = None;
        flag.position = flag.home;
        flag.dropped_by = None;
        self.flag_event(f, Some(i), FlagEvent::Captured);
        self.lead_changes(&leaders);
        self.check_win(i);
    }

    fn flag_event(&mut self, f: usize, player: Option<usize>, what: FlagEvent) {
        self.events.push(Event::Flag {
            team: self.flags[f].team,
            player,
            what,
        });
    }

    /// Carried flags follow their carriers (a held ball scores); armed
    /// bombs tick; dropped flags fall to the floor and eventually go home.
    pub(super) fn step_flags(&mut self, world: &World, dt: f32) {
        if !self.has_flags() {
            return;
        }
        let ball = self.rules.game_type.oddball();
        for f in 0..self.flags.len() {
            let flag = &mut self.flags[f];
            if let Some((_, t)) = &mut flag.dropped_by {
                *t -= dt;
                if *t <= 0.0 {
                    flag.dropped_by = None;
                }
            }
            if let Some(c) = flag.carrier {
                flag.position = self.players[c].body.position;
                if ball {
                    self.hold_point(c, dt);
                }
                continue;
            }
            if let Some(t) = &mut flag.armed {
                *t -= dt;
                if *t <= 0.0 {
                    self.detonate(f);
                }
                continue;
            }
            if flag.at_home() {
                continue;
            }
            // Settle onto whatever is below.
            let fall = world.raycast(flag.position + Vec3::Z * 0.1, -Vec3::Z, 50.0);
            match fall {
                Some(t) => flag.position.z += 0.1 - t,
                None => flag.reset_in = 0.0,
            }
            flag.reset_in -= dt;
            let lost = flag.position.z < world.min.z - 1.0
                || self.kill_zones.iter().any(|z| z.contains(flag.position));
            if flag.reset_in <= 0.0 || lost {
                self.return_flag(f, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::{floor, game};
    use crate::game::{Command, GameType};
    use crate::weapon::WeaponDef;

    /// Red (player 0) and blue (player 1), each flag 10 units out along x.
    fn ctf() -> Game {
        let mut g = game();
        g.rules.game_type = GameType::Ctf;
        g.rules.score_to_win = 2;
        let flag = WeaponDef {
            name: "flag".into(),
            melee_damage: Some(120.0),
            ..g.weapons[0].clone()
        };
        g.weapons.push(flag);
        g.rules.flag_weapon = Some(g.weapons.len() - 1);
        g.set_flags(
            &[
                (0, Vec3::new(-10.0, 0.0, 0.0)),
                (1, Vec3::new(10.0, 0.0, 0.0)),
            ],
            &[],
        );
        g.add_player_on(0);
        g.add_player_on(1);
        g
    }

    fn run(g: &mut Game, world: &World, cmds: &[Command], ticks: usize) {
        for _ in 0..ticks {
            g.step(world, cmds);
        }
    }

    fn act() -> Command {
        Command {
            action: true,
            ..Command::default()
        }
    }

    #[test]
    fn take_carry_and_capture() {
        let world = floor();
        let mut g = ctf();
        g.players[0].body.position = Vec3::new(10.2, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 2);
        assert_eq!(g.flags[1].carrier, Some(0));
        assert_eq!(g.players[0].held().map(|h| h.weapon), g.rules.flag_weapon);
        // Carrying the flag, the gun doesn't fire.
        let fire = Command {
            fire: true,
            ..Command::default()
        };
        run(&mut g, &world, &[fire, Command::default()], 30);
        assert!(!g.events.iter().any(|e| matches!(e, Event::Shot { .. })));
        assert!(g.flags[1].position.distance(Vec3::new(10.2, 0.0, 0.0)) < 0.3);
        // Home.
        g.players[0].body.position = Vec3::new(-10.0, 0.3, 0.0);
        run(&mut g, &world, &[Command::default(), Command::default()], 1);
        assert_eq!(g.players[0].score, 1);
        assert_eq!(g.team_score(0), 1);
        assert!(g.flags[1].at_home());
        assert!(g.players[0].objective.is_none());
        assert!(g.events.iter().any(|e| matches!(
            e,
            Event::Flag {
                team: 1,
                what: FlagEvent::Captured,
                ..
            }
        )));
    }

    #[test]
    fn own_flag_is_not_taken() {
        let world = floor();
        let mut g = ctf();
        g.players[0].body.position = Vec3::new(-10.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 5);
        assert!(g.flags[0].at_home());
        assert!(g.players[0].objective.is_none());
    }

    #[test]
    fn dying_drops_the_flag_and_it_goes_home() {
        let world = floor();
        let mut g = ctf();
        g.players[0].body.position = Vec3::new(10.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 1);
        g.players[0].body.position = Vec3::new(3.0, 0.0, 0.0);
        run(&mut g, &world, &[Command::default(), Command::default()], 1);
        g.damage(0, Some(1), 500.0, false);
        assert!(g.flags[1].carrier.is_none());
        assert!(!g.flags[1].at_home());
        assert!(g.flags[1].position.distance(Vec3::new(3.0, 0.0, 0.0)) < 0.3);
        // Kills don't score in CTF.
        assert_eq!(g.players[1].score, 0);
        assert_eq!(g.players[1].kills, 1);
        let ticks = (g.rules.flag_reset_time / crate::game::TICK) as usize + 2;
        run(
            &mut g,
            &world,
            &[Command::default(), Command::default()],
            ticks,
        );
        assert!(g.flags[1].at_home());
    }

    #[test]
    fn switching_weapons_drops_the_flag() {
        let world = floor();
        let mut g = ctf();
        g.players[0].body.position = Vec3::new(10.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 1);
        let switch = Command {
            switch_weapon: true,
            action: true,
            ..Command::default()
        };
        run(&mut g, &world, &[switch, Command::default()], 2);
        // Dropped, and not picked straight back up.
        assert!(g.flags[1].carrier.is_none());
        assert!(g.players[0].objective.is_none());
        let ticks = (REGRAB_DELAY / crate::game::TICK) as usize + 2;
        run(&mut g, &world, &[act(), Command::default()], ticks);
        assert_eq!(g.flags[1].carrier, Some(0));
    }

    #[test]
    fn carriers_pick_up_no_weapons() {
        let world = floor();
        let mut g = ctf();
        g.players[0].body.position = Vec3::new(10.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 1);
        // Over the rifle lying at (0, 3), holding the action key.
        g.players[0].body.position = Vec3::new(0.0, 3.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 60);
        assert_eq!(g.players[0].weapons.len(), 1);
        assert_eq!(g.flags[1].carrier, Some(0));
        assert_eq!(g.swap_prompt(0), None);
    }

    #[test]
    fn touch_return_and_home_to_score() {
        let world = floor();
        let mut g = ctf();
        g.rules.flag_touch_return = true;
        g.rules.flag_at_home_to_score = true;
        // Both carry each other's flag.
        g.players[0].body.position = Vec3::new(10.0, 0.0, 0.0);
        g.players[1].body.position = Vec3::new(-10.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), act()], 1);
        assert_eq!(g.flags[1].carrier, Some(0));
        assert_eq!(g.flags[0].carrier, Some(1));
        // Red can't score while blue holds red's flag.
        g.players[0].body.position = Vec3::new(-10.0, 0.5, 0.0);
        g.players[1].body.position = Vec3::new(0.0, 0.0, 0.0);
        run(&mut g, &world, &[Command::default(), Command::default()], 2);
        assert_eq!(g.team_score(0), 0);
        assert!(g.events.iter().any(|e| matches!(
            e,
            Event::Flag {
                what: FlagEvent::CaptureFailed,
                ..
            }
        )));
        // Blue dies with red's flag; red touches it and it goes home.
        g.damage(1, None, 500.0, false);
        g.players[0].body.position = Vec3::new(0.2, 0.0, 0.0);
        run(&mut g, &world, &[Command::default(), Command::default()], 2);
        assert!(g.flags[0].at_home());
        // Now red scores.
        g.players[0].body.position = Vec3::new(-10.0, 0.5, 0.0);
        run(&mut g, &world, &[Command::default(), Command::default()], 2);
        assert_eq!(g.team_score(0), 1);
    }

    #[test]
    fn the_flag_smashes() {
        let world = floor();
        let mut g = ctf();
        g.players[0].body.position = Vec3::new(10.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 1);
        g.players[1].body.position = Vec3::new(10.6, 0.0, 0.0);
        let melee = Command {
            melee: true,
            ..Command::default()
        };
        // The flag's melee kills outright.
        run(&mut g, &world, &[melee, Command::default()], 1);
        assert!(!g.players[1].alive);
    }

    #[test]
    fn flags_go_over_the_network() {
        let world = floor();
        let mut host = ctf();
        host.players[0].body.position = Vec3::new(10.0, 0.0, 0.0);
        run(&mut host, &world, &[act(), Command::default()], 1);
        let mut w = crate::game::Writer::default();
        host.write_state(&mut w);
        let mut joined = ctf();
        joined
            .read_state(&mut crate::game::Reader::new(&w.0))
            .unwrap();
        assert_eq!(joined.rules.game_type, GameType::Ctf);
        assert_eq!(joined.flags.len(), 2);
        assert_eq!(joined.flags[1].carrier, Some(0));
        assert_eq!(joined.flags[0].home, host.flags[0].home);
        assert_eq!(
            joined.players[0].held().map(|h| h.weapon),
            host.rules.flag_weapon
        );
        assert!(joined.players[1].objective.is_none());
    }

    #[test]
    fn captures_win_the_game() {
        let world = floor();
        let mut g = ctf();
        for _ in 0..2 {
            g.players[0].body.position = Vec3::new(10.0, 0.0, 0.0);
            run(&mut g, &world, &[act(), Command::default()], 1);
            g.players[0].body.position = Vec3::new(-10.0, 0.0, 0.0);
            run(&mut g, &world, &[Command::default(), Command::default()], 1);
        }
        assert_eq!(g.winning_team, Some(0));
    }

    /// A game played with one carried thing per team (or a ball), carried
    /// like the flag.
    fn carried(kind: GameType, homes: &[(u8, Vec3)]) -> Game {
        let mut g = game();
        g.rules.game_type = kind;
        g.rules.score_to_win = 3;
        let thing = WeaponDef {
            name: "carried".into(),
            ..g.weapons[0].clone()
        };
        g.weapons.push(thing);
        let w = Some(g.weapons.len() - 1);
        (
            g.rules.flag_weapon,
            g.rules.ball_weapon,
            g.rules.bomb_weapon,
        ) = (w, w, w);
        g.set_flags(homes, &[]);
        g.add_player_on(0);
        g.add_player_on(1);
        g
    }

    fn bombs() -> Game {
        carried(
            GameType::Assault,
            &[
                (0, Vec3::new(-10.0, 0.0, 0.0)),
                (1, Vec3::new(10.0, 0.0, 0.0)),
            ],
        )
    }

    #[test]
    fn holding_the_ball_scores() {
        let world = floor();
        let mut g = carried(GameType::Oddball, &[(NEUTRAL, Vec3::ZERO)]);
        assert!(g.has_flags());
        g.players[0].body.position = Vec3::new(0.2, 0.0, 0.0);
        g.players[1].body.position = Vec3::new(20.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 1);
        assert_eq!(g.flags[0].carrier, Some(0));
        run(&mut g, &world, &[Command::default(); 2], 60 * 4);
        assert_eq!(g.players[0].score, 3);
        assert_eq!(g.players[1].score, 0);
        assert_eq!(g.winner, Some(0));
    }

    #[test]
    fn bombs_are_armed_in_the_enemy_base_and_go_off() {
        let world = floor();
        let mut g = bombs();
        // Blue can't take red's bomb; red takes its own.
        g.players[1].body.position = Vec3::new(-10.0, 0.3, 0.0);
        g.players[0].body.position = Vec3::new(-10.0, 0.0, 0.0);
        run(&mut g, &world, &[Command::default(), act()], 1);
        assert_eq!(g.flags[0].carrier, None);
        run(&mut g, &world, &[act(), Command::default()], 1);
        assert_eq!(g.flags[0].carrier, Some(0));
        // At blue's base for the arming time.
        g.players[1].body.position = Vec3::new(0.0, 30.0, 0.0);
        let ticks = (g.rules.bomb_arm_time / crate::game::TICK) as usize + 2;
        for _ in 0..ticks {
            g.players[0].body.position = Vec3::new(10.0, 0.3, 0.0);
            run(&mut g, &world, &[Command::default(); 2], 1);
        }
        assert!(g.flags[0].armed.is_some());
        assert!(g.players[0].objective.is_none());
        assert!(g.events.iter().any(|e| matches!(
            e,
            Event::Flag {
                what: FlagEvent::Armed,
                ..
            }
        )));
        // Away from the blast until it goes off.
        g.players[0].body.position = Vec3::new(0.0, -30.0, 0.0);
        let ticks = (g.rules.bomb_fuse / crate::game::TICK) as usize + 2;
        run(&mut g, &world, &[Command::default(); 2], ticks);
        assert_eq!(g.team_score(0), 1);
        assert!(g.flags[0].at_home());
        assert!(g.players.iter().all(|p| p.alive));
    }

    #[test]
    fn defenders_defuse_armed_bombs() {
        let world = floor();
        let mut g = bombs();
        g.players[0].body.position = Vec3::new(-10.0, 0.0, 0.0);
        run(&mut g, &world, &[act(), Command::default()], 1);
        let ticks = (g.rules.bomb_arm_time / crate::game::TICK) as usize + 2;
        for _ in 0..ticks {
            g.players[0].body.position = Vec3::new(10.0, 0.3, 0.0);
            run(&mut g, &world, &[Command::default(); 2], 1);
        }
        assert!(g.flags[0].armed.is_some());
        g.players[0].body.position = Vec3::new(0.0, -30.0, 0.0);
        for _ in 0..ticks {
            g.players[1].body.position = Vec3::new(10.0, -0.3, 0.0);
            run(&mut g, &world, &[Command::default(), act()], 1);
        }
        assert!(g.flags[0].at_home());
        assert_eq!(g.team_score(0), 0);
        assert!(g.events.iter().any(|e| matches!(
            e,
            Event::Flag {
                what: FlagEvent::Defused,
                ..
            }
        )));
    }

    #[test]
    fn balls_and_bombs_go_over_the_network() {
        let world = floor();
        let mut host = carried(GameType::Oddball, &[(NEUTRAL, Vec3::ZERO)]);
        run(&mut host, &world, &[act(), Command::default()], 1);
        let mut w = crate::game::Writer::default();
        host.write_state(&mut w);
        let mut joined = carried(GameType::Slayer, &[]);
        joined
            .read_state(&mut crate::game::Reader::new(&w.0))
            .unwrap();
        assert_eq!(joined.rules.game_type, GameType::Oddball);
        assert_eq!(joined.flags[0].team, NEUTRAL);
        assert_eq!(joined.flags[0].carrier, Some(0));
    }
}
