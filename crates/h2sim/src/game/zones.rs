//! Places to hold: King of the Hill's hills (one in play at a time, moving
//! on every so often) and Territories (a team stands in one alone to take
//! it). Holding them scores a point a second.

use super::{Event, Game, TEAMS};
use glam::{Vec2, Vec3};

/// How far below and above a hill's outline a player can stand and be in it.
const HILL_BELOW: f32 = 1.0;
const HILL_ABOVE: f32 = 2.5;
/// How close to one of a territory's points a player has to be to stand in it.
const TERRITORY_RADIUS: f32 = 2.5;
const TERRITORY_HEIGHT: f32 = 1.5;

/// A hill: the area inside an outline on the floor.
#[derive(Debug, Clone, PartialEq)]
pub struct Hill {
    /// Corners of the outline, in order around it.
    pub outline: Vec<Vec3>,
}

impl Hill {
    /// A hill from the map's points for it, put in order around their middle.
    pub fn new(points: &[Vec3]) -> Hill {
        let centre = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
        let mut outline = points.to_vec();
        outline.sort_by(|a, b| {
            let angle = |p: &Vec3| (p.y - centre.y).atan2(p.x - centre.x);
            angle(a).total_cmp(&angle(b))
        });
        Hill { outline }
    }

    pub fn centre(&self) -> Vec3 {
        self.outline.iter().copied().sum::<Vec3>() / self.outline.len().max(1) as f32
    }

    pub fn contains(&self, p: Vec3) -> bool {
        let n = self.outline.len();
        if n < 3 {
            return false;
        }
        let low = self.outline.iter().map(|q| q.z).fold(f32::MAX, f32::min);
        let high = self.outline.iter().map(|q| q.z).fold(f32::MIN, f32::max);
        if p.z < low - HILL_BELOW || p.z > high + HILL_ABOVE {
            return false;
        }
        // Even-odd rule across the floor.
        let at = Vec2::new(p.x, p.y);
        let mut inside = false;
        for k in 0..n {
            let (a, b) = (
                self.outline[k].truncate(),
                self.outline[(k + 1) % n].truncate(),
            );
            if (a.y > at.y) != (b.y > at.y) {
                let x = a.x + (at.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if at.x < x {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// Who holds the hill in play.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HillControl {
    #[default]
    Empty,
    /// One player (or, in Team King, one team: any of its players) alone in it.
    Held(usize),
    /// Enemies in it together.
    Contested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HillEvent {
    Moved,
    Controlled,
    Contested,
}

/// A territory: the ground around its points.
#[derive(Debug, Clone, PartialEq)]
pub struct Territory {
    pub points: Vec<Vec3>,
    pub owner: Option<u8>,
    /// A team taking it, and for how long it has stood there alone.
    pub taking: Option<(u8, f32)>,
    /// Who took it (their points), and time toward its next point.
    pub taken_by: Option<usize>,
    held: f32,
}

impl Territory {
    /// How far around each of its points a territory reaches.
    pub const RADIUS: f32 = TERRITORY_RADIUS;

    pub fn new(points: Vec<Vec3>) -> Territory {
        Territory {
            points,
            owner: None,
            taking: None,
            taken_by: None,
            held: 0.0,
        }
    }

    pub fn centre(&self) -> Vec3 {
        self.points.first().copied().unwrap_or(Vec3::ZERO)
    }

    pub fn contains(&self, p: Vec3) -> bool {
        self.points.iter().any(|q| {
            (p - *q).truncate().length() < TERRITORY_RADIUS && (p.z - q.z).abs() < TERRITORY_HEIGHT
        })
    }
}

impl Game {
    /// The hill in play, in King of the Hill.
    pub fn current_hill(&self) -> Option<&Hill> {
        self.rules
            .game_type
            .king()
            .then(|| self.hills.get(self.hill))
            .flatten()
    }

    /// Who's standing on a side's behalf: the player in free-for-all games,
    /// the team in team games.
    fn side_of(&self, player: usize) -> usize {
        if self.rules.game_type.teams() {
            self.players[player].team as usize
        } else {
            player
        }
    }

    /// A second's point to `player` for holding something, once their held
    /// time comes to a whole second.
    pub(super) fn hold_point(&mut self, player: usize, dt: f32) {
        let p = &mut self.players[player];
        p.hold += dt;
        if p.hold < 1.0 {
            return;
        }
        p.hold -= 1.0;
        let leaders = self.leaders();
        self.players[player].score += 1;
        self.lead_changes(&leaders);
        self.check_win(player);
    }

    pub(super) fn step_hills(&mut self, dt: f32) {
        if !self.rules.game_type.king() || self.hills.is_empty() {
            return;
        }
        let move_time = self.rules.hill_move_time;
        if move_time > 0.0 {
            self.hill_moves_in -= dt;
            if self.hill_moves_in <= 0.0 {
                self.hill_moves_in = move_time;
                if self.time > dt as f64 * 1.5 {
                    self.hill = (self.hill + 1) % self.hills.len();
                    self.hill_control = HillControl::Empty;
                    self.events.push(Event::Hill {
                        player: None,
                        what: HillEvent::Moved,
                    });
                }
            }
        }
        let hill = &self.hills[self.hill];
        let inside: Vec<usize> = (0..self.players.len())
            .filter(|&i| self.players[i].alive && hill.contains(self.players[i].body.position))
            .collect();
        let mut sides: Vec<usize> = inside.iter().map(|&i| self.side_of(i)).collect();
        sides.dedup();
        let control = match (sides.len(), inside.first()) {
            (1, Some(&first)) => {
                // The same team keeps the same holder while they're in it.
                match self.hill_control {
                    HillControl::Held(h) if inside.contains(&h) => HillControl::Held(h),
                    _ => HillControl::Held(first),
                }
            }
            (0, _) => HillControl::Empty,
            _ => HillControl::Contested,
        };
        if control != self.hill_control {
            let was = self.hill_control;
            self.hill_control = control;
            let same_side = match (was, control) {
                (HillControl::Held(a), HillControl::Held(b)) => self.side_of(a) == self.side_of(b),
                _ => false,
            };
            match control {
                HillControl::Held(h) if !same_side => self.events.push(Event::Hill {
                    player: Some(h),
                    what: HillEvent::Controlled,
                }),
                HillControl::Contested => self.events.push(Event::Hill {
                    player: None,
                    what: HillEvent::Contested,
                }),
                _ => {}
            }
        }
        if let HillControl::Held(h) = self.hill_control {
            self.hold_point(h, dt);
        }
    }

    pub(super) fn step_territories(&mut self, dt: f32) {
        if self.rules.game_type != super::GameType::Territories {
            return;
        }
        let take_time = self.rules.territory_capture_time;
        for t in 0..self.territories.len() {
            let inside: Vec<usize> = (0..self.players.len())
                .filter(|&i| {
                    self.players[i].alive
                        && self.territories[t].contains(self.players[i].body.position)
                })
                .collect();
            let mut teams: Vec<u8> = inside.iter().map(|&i| self.players[i].team).collect();
            teams.sort_unstable();
            teams.dedup();
            let ter = &mut self.territories[t];
            match teams.as_slice() {
                [team] if ter.owner != Some(*team) => {
                    let so_far = match ter.taking {
                        Some((t2, s)) if t2 == *team => s,
                        _ => 0.0,
                    } + dt;
                    if so_far >= take_time {
                        let from = ter.owner;
                        ter.owner = Some(*team);
                        ter.taking = None;
                        ter.taken_by = inside.first().copied();
                        ter.held = 0.0;
                        self.events.push(Event::Territory {
                            index: t,
                            team: *team,
                            from,
                        });
                    } else {
                        ter.taking = Some((*team, so_far));
                    }
                }
                // Nobody there: an unfinished take falls away.
                [] => ter.taking = None,
                _ => {}
            }
        }
        // A point a second for each territory held.
        for t in 0..self.territories.len() {
            let Some(team) = self.territories[t].owner else {
                continue;
            };
            let holder = self.territories[t]
                .taken_by
                .filter(|&p| self.players.get(p).is_some_and(|q| q.team == team))
                .or_else(|| self.players.iter().position(|q| q.team == team));
            let Some(holder) = holder else {
                continue;
            };
            self.territories[t].taken_by = Some(holder);
            let ter = &mut self.territories[t];
            ter.held += dt;
            if ter.held >= 1.0 {
                ter.held -= 1.0;
                let leaders = self.leaders();
                self.players[holder].score += 1;
                self.lead_changes(&leaders);
                self.check_win(holder);
            }
        }
    }

    /// Teams holding every territory (for the announcer's "land grab").
    pub fn holds_every_territory(&self, team: u8) -> bool {
        team < TEAMS
            && !self.territories.is_empty()
            && self.territories.iter().all(|t| t.owner == Some(team))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::{floor, game};
    use crate::game::{Command, GameType};

    fn square(x: f32) -> Hill {
        Hill::new(&[
            Vec3::new(x - 1.0, -1.0, 0.0),
            Vec3::new(x + 1.0, 1.0, 0.0),
            Vec3::new(x + 1.0, -1.0, 0.0),
            Vec3::new(x - 1.0, 1.0, 0.0),
        ])
    }

    #[test]
    fn hills_are_their_outlines() {
        let h = square(0.0);
        assert!(h.contains(Vec3::new(0.5, 0.5, 0.0)));
        assert!(!h.contains(Vec3::new(1.5, 0.0, 0.0)));
        assert!(!h.contains(Vec3::new(0.0, 0.0, 5.0)));
        assert_eq!(h.centre(), Vec3::ZERO);
    }

    #[test]
    fn standing_alone_in_the_hill_scores() {
        let world = floor();
        let mut g = game();
        g.rules.game_type = GameType::KingOfTheHill;
        g.rules.score_to_win = 5;
        g.rules.hill_move_time = 0.0;
        g.hills = vec![square(0.0)];
        g.add_player();
        g.add_player();
        g.players[0].body.position = Vec3::ZERO;
        g.players[1].body.position = Vec3::new(10.0, 0.0, 0.0);
        let idle = [Command::default(); 2];
        for _ in 0..60 * 3 {
            g.step(&world, &idle);
            g.players[0].body.position = Vec3::ZERO;
        }
        assert_eq!(g.hill_control, HillControl::Held(0));
        assert!(
            (2..=3).contains(&g.players[0].score),
            "{}",
            g.players[0].score
        );
        // Contested: no one scores.
        let before = g.players[0].score;
        for _ in 0..60 * 2 {
            g.players[1].body.position = Vec3::new(0.3, 0.3, 0.0);
            g.step(&world, &idle);
        }
        assert_eq!(g.hill_control, HillControl::Contested);
        assert_eq!(g.players[0].score, before);
        // Alone again until the score to win.
        for _ in 0..60 * 10 {
            g.players[1].body.position = Vec3::new(10.0, 0.0, 0.0);
            g.step(&world, &idle);
        }
        assert_eq!(g.winner, Some(0));
        assert_eq!(g.players[0].kills, 0);
    }

    #[test]
    fn the_hill_moves_on() {
        let world = floor();
        let mut g = game();
        g.rules.game_type = GameType::TeamKing;
        g.rules.hill_move_time = 1.0;
        g.hills = vec![square(0.0), square(20.0)];
        g.add_player();
        let idle = [Command::default()];
        let mut moved = 0;
        for _ in 0..60 * 3 {
            g.step(&world, &idle);
            moved += g
                .events
                .drain(..)
                .filter(|e| {
                    matches!(
                        e,
                        Event::Hill {
                            what: HillEvent::Moved,
                            ..
                        }
                    )
                })
                .count();
        }
        assert!(moved >= 2, "{moved}");
    }

    #[test]
    fn teams_take_and_hold_territories() {
        let world = floor();
        let mut g = game();
        g.rules.game_type = GameType::Territories;
        g.rules.score_to_win = 0;
        g.territories = vec![
            Territory::new(vec![Vec3::ZERO]),
            Territory::new(vec![Vec3::new(10.0, 0.0, 0.0)]),
        ];
        let red = g.add_player_on(0);
        let blue = g.add_player_on(1);
        let idle = [Command::default(); 2];
        let place = |g: &mut Game, r: Vec3, b: Vec3| {
            g.players[red].body.position = r;
            g.players[blue].body.position = b;
        };
        // Both in the first: nobody takes it.
        for _ in 0..60 * 8 {
            place(&mut g, Vec3::ZERO, Vec3::new(0.5, 0.0, 0.0));
            g.step(&world, &idle);
        }
        assert_eq!(g.territories[0].owner, None);
        // Red alone takes it, then blue takes it back.
        for _ in 0..60 * 7 {
            place(&mut g, Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0));
            g.step(&world, &idle);
        }
        assert_eq!(g.territories[0].owner, Some(0));
        for _ in 0..60 * 7 {
            place(&mut g, Vec3::new(20.0, 0.0, 0.0), Vec3::ZERO);
            g.step(&world, &idle);
        }
        assert_eq!(g.territories[0].owner, Some(1));
        assert!(g.team_score(0) >= 4 && g.team_score(1) >= 0);
        let taken = g
            .events
            .iter()
            .filter(|e| matches!(e, Event::Territory { .. }))
            .count();
        assert!(taken >= 1);
    }
}
