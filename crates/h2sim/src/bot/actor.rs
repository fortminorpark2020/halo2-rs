//! Campaign actors: the same brain as a multiplayer bot, held to its
//! character. An actor stands at its post until it sees an enemy (in front
//! of it, or close by), fights at the range its character likes with the
//! accuracy it has, tells its squad, goes to look where it last saw or
//! heard someone, and otherwise heads back to its post. Allies who have
//! met the player follow them around.

use super::Bot;
use crate::collision::World;
use crate::game::{Event, Game, Mind};
use glam::{Vec2, Vec3};

/// Seconds an actor fighting from a firing position stays there before it
/// thinks about moving, and longest it tries to get to one.
const HOLD: (f32, f32) = (2.0, 5.0);
const GIVE_UP: f32 = 6.0;
/// Firing positions an actor weighs up each time it moves.
const TRIES: usize = 6;
/// What a firing position that can't see the enemy, or can't be walked to
/// straight, costs against one that can (in metres of range or walking).
const BLIND: f32 = 20.0;
const BLOCKED: f32 = 40.0;
/// Seconds an actor keeps looking for an enemy it lost sight of.
const SEARCH_TIME: f32 = 20.0;
/// How far squadmates hear each other call out an enemy, and how far
/// gunfire carries.
const CALL_OUT: f32 = 20.0;
const HEARING: f32 = 15.0;
/// Close enough to the post, or to the player being followed.
const AT_POST: f32 = 0.5;
const FOLLOW_NEAR: f32 = 3.0;
const FOLLOW_FAR: f32 = 5.0;
/// Allies join a player who comes this close (in sight).
const JOIN: f32 = 8.0;
/// Seconds between picking a new spot to aim at off the target.
const WOBBLE: f32 = 0.3;
/// Aim error, in radians, of the least accurate actor.
const WORST_AIM: f32 = 0.12;
/// Seconds of shooting at someone before an actor's aim is at its best.
const ZERO_IN: f32 = 4.0;

/// What a campaign actor knows and is told.
#[derive(Debug, Clone)]
pub struct ActorMind {
    pub mind: Mind,
    /// Where it was placed, and which way it faced.
    pub post: Vec3,
    pub facing: f32,
    /// Its squad, to call out enemies to.
    pub squad: u16,
    /// On the players' side: follows them once it has met them.
    pub ally: bool,
    pub following: Option<usize>,
    /// Where it last knew of an enemy, and how long ago.
    pub alert: Option<(Vec3, f32)>,
    /// The firing positions its orders give it to fight from (none: it
    /// fights wherever it is).
    pub area: Vec<Vec3>,
    /// Goes along with players it meets (unless its orders keep it put).
    pub follows: bool,
    /// The firing position it's fighting from, how long it's been there
    /// or getting there, and how long it stays.
    fight_at: Option<Vec3>,
    held: f32,
    stay: f32,
    wobble: f32,
}

impl ActorMind {
    pub fn new(mind: Mind, post: Vec3, facing: f32, squad: u16, ally: bool) -> ActorMind {
        ActorMind {
            mind,
            post,
            facing,
            squad,
            ally,
            following: None,
            alert: None,
            area: Vec::new(),
            follows: true,
            fight_at: None,
            held: 0.0,
            stay: 0.0,
            wobble: 0.0,
        }
    }
}

impl Bot {
    /// A bot driven as a campaign actor.
    pub fn actor(seed: u32, mind: ActorMind) -> Bot {
        let mut bot = Bot::new(seed);
        bot.riding.reset(false);
        bot.actor = Some(mind);
        bot
    }

    /// An enemy the actor notices: in sight, and in front of it unless
    /// it's already alert or the enemy is close. Keeps to the one it's
    /// fighting while it can see them.
    pub(super) fn actor_target(&self, game: &Game, world: &World, me: usize) -> Option<usize> {
        let a = self.actor.as_ref()?;
        let p = &game.players[me];
        let eye = p.eye();
        let alert = self.target.is_some() || a.alert.is_some();
        let forward = Vec2::from_angle(p.yaw);
        let notices = |j: usize| {
            let q = &game.players[j];
            if !q.alive || !game.is_enemy(me, j) {
                return None;
            }
            let chest = q.eye() - Vec3::Z * 0.1;
            let to = chest - eye;
            let d = to.length();
            let sight = (a.mind.sight * q.visibility()).max(super::CAMO_NOTICE);
            let ahead = forward.angle_to(to.truncate()).abs() <= a.mind.fov;
            let seen = d < sight && (alert || ahead || d < a.mind.peripheral);
            (seen && world.raycast(eye, to / d.max(1e-4), d).is_none()).then_some(d)
        };
        if let Some(t) = self.target.filter(|&t| notices(t).is_some()) {
            return Some(t);
        }
        (0..game.players.len())
            .filter_map(|j| notices(j).map(|d| (j, d)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(j, _)| j)
    }

    /// How well the actor shoots now (0-1), better the longer it's been
    /// at it.
    fn accuracy(&self) -> f32 {
        let Some(a) = &self.actor else {
            return 1.0;
        };
        let (from, to) = a.mind.accuracy;
        from + (to - from) * (self.seen_for / ZERO_IN).min(1.0)
    }

    /// Aim off the target by the actor's (in)accuracy, a fresh spot every
    /// so often.
    pub(super) fn actor_aim(&mut self, dt: f32) {
        let Some(wobble) = self.actor.as_ref().map(|a| a.wobble - dt) else {
            return;
        };
        let accuracy = self.accuracy();
        if wobble <= 0.0 {
            let spread = (1.0 - accuracy) * WORST_AIM * 2.0;
            self.aim_error = Vec2::new(self.random() - 0.5, self.random() - 0.5) * spread;
        }
        if let Some(a) = &mut self.actor {
            a.wobble = if wobble <= 0.0 { WOBBLE } else { wobble };
        }
    }

    /// Fighting `dist` away: forward (1), back (-1) or neither, to stay at
    /// the range the actor likes, or charging in to melee.
    pub(super) fn actor_range(&mut self, dist: f32, dt: f32) -> (f32, bool) {
        let Some(m) = self.actor.as_ref().map(|a| a.mind) else {
            return (0.0, false);
        };
        let charge = dist < m.melee_range.max(1.0) && self.random() < m.melee_chance * dt * 4.0;
        if charge || dist < 1.0 {
            return (1.0, true);
        }
        let fwd = if dist > m.combat_range.1 {
            1.0
        } else if dist < m.combat_range.0 {
            -1.0
        } else {
            0.0
        };
        (fwd, false)
    }

    /// Where an actor fighting `target` goes: a firing position in its area
    /// that sees them from about the range it likes, without far to walk.
    /// It stays a while once there. `None` for an actor with no area.
    pub(super) fn actor_position(
        &mut self,
        game: &Game,
        world: &World,
        me: usize,
        target: usize,
        dt: f32,
    ) -> Option<Vec3> {
        let p = &game.players[me];
        let feet = p.body.position;
        let eye = p.eye() - feet;
        let enemy = game.players[target].eye();
        let n = self.actor.as_ref().map_or(0, |a| a.area.len());
        if n == 0 {
            return None;
        }
        // A few of a big area's positions, or all of a small one's.
        let picks: Vec<usize> = if n <= TRIES {
            (0..n).collect()
        } else {
            (0..TRIES)
                .map(|_| (self.random() * n as f32) as usize % n)
                .collect()
        };
        let stay = HOLD.0 + (HOLD.1 - HOLD.0) * self.random();
        let a = self.actor.as_mut()?;
        a.held += dt;
        if let Some(at) = a.fight_at {
            let there = (at - feet).truncate().length() < AT_POST;
            if there && a.held < a.stay || !there && a.held < GIVE_UP {
                return Some(at);
            }
        }
        let (near, far) = a.mind.combat_range;
        let want = (near + far) / 2.0;
        let cost = |at: Vec3| {
            let from = at + eye;
            let to = enemy - from;
            let d = to.length();
            let blind = world.raycast(from, to / d.max(1e-4), d).is_some();
            let walk = at - feet;
            let w = walk.length();
            let blocked = w > AT_POST && world.raycast(feet + Vec3::Z * 0.3, walk / w, w).is_some();
            (d - want).abs()
                + w * 0.5
                + if blind { BLIND } else { 0.0 }
                + if blocked { BLOCKED } else { 0.0 }
        };
        let best = picks
            .iter()
            .map(|&k| a.area[k])
            .chain(a.fight_at)
            .map(|at| (cost(at), at))
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .map(|(_, at)| at);
        a.fight_at = best;
        a.held = 0.0;
        a.stay = stay;
        best
    }

    /// Whether to throw a grenade at someone `dist` away now.
    pub(super) fn actor_grenade(&mut self, dist: f32, dt: f32) -> bool {
        let Some(m) = self.actor.as_ref().map(|a| a.mind) else {
            return false;
        };
        let (near, far) = m.grenade_range;
        if self.grenade_wait > 0.0 || !(near..far).contains(&dist) {
            return false;
        }
        if self.random() < m.grenade_chance * dt {
            self.grenade_wait = m.grenade_delay * (1.0 + self.random());
            return true;
        }
        false
    }

    /// Where an actor with no one in sight goes: to look where it last
    /// knew of an enemy, after the player it follows, or back to its post.
    /// `None` once there (standing, facing the way it should).
    pub(super) fn actor_goal(
        &mut self,
        game: &Game,
        world: &World,
        me: usize,
        dt: f32,
    ) -> Option<Vec3> {
        let feet = game.players[me].body.position;
        let eye = game.players[me].eye();
        let a = self.actor.as_mut()?;
        if let Some((at, age)) = &mut a.alert {
            *age += dt;
            // Held to an area, it looks from the part of it nearest.
            let look = a
                .area
                .iter()
                .min_by(|p, q| p.distance_squared(*at).total_cmp(&q.distance_squared(*at)))
                .copied()
                .unwrap_or(*at);
            if *age > SEARCH_TIME || (look - feet).truncate().length() < 1.5 {
                a.alert = None;
            } else {
                return Some(look);
            }
        }
        if !a.follows {
            a.following = None;
        }
        // Allies take up with a player who comes close.
        if a.ally && a.follows && a.following.is_none_or(|f| !game.players[f].alive) {
            a.following = (0..game.players.len()).find(|&j| {
                let q = &game.players[j];
                q.actor.is_none()
                    && q.alive
                    && !game.is_enemy(me, j)
                    && q.eye().distance(eye) < JOIN
                    && Bot::visible(world, eye, q.eye())
            });
        }
        if let Some(f) = a.following {
            let q = &game.players[f];
            let d = (q.body.position - feet).truncate().length();
            let moving = self.heading_for.is_some();
            if d > FOLLOW_FAR || (moving && d > FOLLOW_NEAR) {
                return Some(q.body.position);
            }
            a.facing = q.yaw;
            return None;
        }
        ((a.post - feet).truncate().length() > AT_POST).then_some(a.post)
    }

    /// The way an idle actor faces.
    pub(super) fn actor_facing(&self) -> Option<f32> {
        self.actor.as_ref().map(|a| a.facing)
    }
}

/// Actors tell their squad about the enemies they see, and turn toward
/// gunfire and toward whoever shoots them.
pub fn alert_actors(bots: &mut [(usize, Bot)], game: &Game, events: &[Event]) {
    let mut calls: Vec<(u16, Vec3, Vec3)> = Vec::new();
    for (i, bot) in bots.iter() {
        let (Some(a), Some(t)) = (&bot.actor, bot.target) else {
            continue;
        };
        if let Some(q) = game.players.get(t) {
            calls.push((a.squad, game.players[*i].body.position, q.body.position));
        }
    }
    let mut noises: Vec<(Vec3, usize, Option<usize>)> = Vec::new();
    for e in events {
        if let Event::Shot {
            player,
            origin,
            hit_player,
            ..
        } = *e
        {
            noises.push((origin, player, hit_player));
        }
    }
    for (i, bot) in bots.iter_mut() {
        let me = *i;
        let target = bot.target;
        let Some(a) = &mut bot.actor else {
            continue;
        };
        if target.is_some() || !game.players[me].alive {
            continue;
        }
        let feet = game.players[me].body.position;
        let called = calls
            .iter()
            .filter(|(squad, from, _)| *squad == a.squad && from.distance(feet) < CALL_OUT)
            .map(|c| c.2)
            .next();
        let heard = noises
            .iter()
            .filter(|(at, shooter, hit)| {
                game.is_enemy(me, *shooter) && (*hit == Some(me) || at.distance(feet) < HEARING)
            })
            .map(|n| n.0)
            .next();
        if let Some(at) = called.or(heard) {
            a.alert = Some((at, 0.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::{floor, game};
    use crate::game::{ActorSpawn, CharacterDef, Command, GameType, GrenadeKind, Side, Vitality};
    use crate::nav::NavGraph;

    fn grunt_def(g: &Game) -> CharacterDef {
        CharacterDef {
            name: "grunt".into(),
            side: Side::Covenant,
            biped: g.biped,
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
            grenades: 0,
        }
    }

    /// A campaign with the player at the origin and Grunts of squad 0 at
    /// `posts`, facing +x.
    fn mission(posts: &[Vec3]) -> (Game, Vec<(usize, Bot)>) {
        let mut g = game();
        g.rules.game_type = GameType::Campaign;
        let def = grunt_def(&g);
        g.characters.push(def);
        let me = g.add_player();
        g.players[me].body.position = Vec3::ZERO;
        let bots = posts
            .iter()
            .map(|&post| {
                let i = g
                    .spawn_actor(ActorSpawn {
                        character: 0,
                        squad: 0,
                        position: post,
                        yaw: 0.0,
                        weapon: None,
                        secondary: None,
                        difficulty: 1,
                        team: None,
                    })
                    .unwrap();
                let mind = ActorMind::new(Mind::default(), post, 0.0, 0, false);
                (i, Bot::actor(i as u32, mind))
            })
            .collect();
        (g, bots)
    }

    fn run(g: &mut Game, bots: &mut [(usize, Bot)], seconds: f32) -> bool {
        let world = floor();
        let nav = NavGraph::build(&world, &[Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)]);
        let mut fired = false;
        for _ in 0..(seconds * 60.0) as usize {
            let mut commands = vec![Command::default(); g.players.len()];
            for (i, bot) in bots.iter_mut() {
                commands[*i] = bot.think(g, &world, &nav, *i);
                fired |= commands[*i].fire;
            }
            g.step(&world, &commands);
            let events = std::mem::take(&mut g.events);
            alert_actors(bots, g, &events);
        }
        fired
    }

    #[test]
    fn actors_hold_their_post_and_miss_someone_behind_them() {
        // Facing +x, with the player 10 behind.
        let post = Vec3::new(10.0, 0.0, 0.0);
        let (mut g, mut bots) = mission(&[post + Vec3::new(10.0, 0.0, 0.0)]);
        g.players[0].body.position = post;
        let fired = run(&mut g, &mut bots, 3.0);
        let (i, bot) = &bots[0];
        assert!(!fired && bot.target.is_none());
        assert!(g.players[*i].body.position.distance(post + Vec3::X * 10.0) < 0.3);
    }

    #[test]
    fn actors_shoot_who_they_see_in_front_and_tell_their_squad() {
        // Grunts at 10 and 25 facing +x: the player is 6 in front of the
        // first and behind the second.
        let (mut g, mut bots) = mission(&[Vec3::new(10.0, 0.0, 0.0), Vec3::new(25.0, 0.0, 0.0)]);
        g.players[0].body.position = Vec3::new(16.0, 0.0, 0.0);
        g.players[0].health = 1e9;
        let fired = run(&mut g, &mut bots, 2.0);
        assert!(fired, "the first Grunt opens fire");
        assert_eq!(bots[0].1.target, Some(0));
        let far = &bots[1].1;
        assert!(
            far.target.is_some() || far.actor.as_ref().unwrap().alert.is_some(),
            "its squadmate hears about it"
        );
    }

    #[test]
    fn actors_fight_from_the_firing_position_at_the_range_they_like() {
        // Facing +x with the player 10 in front: of its area's firing
        // positions, the one 5 from the player is best.
        let (mut g, mut bots) = mission(&[Vec3::new(10.0, 0.0, 0.0)]);
        g.players[0].body.position = Vec3::new(20.0, 0.0, 0.0);
        g.players[0].health = 1e9;
        let area = [
            Vec3::new(15.0, 0.0, 0.0),
            Vec3::new(10.0, 8.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ];
        bots[0].1.actor.as_mut().unwrap().area = area.to_vec();
        let fired = run(&mut g, &mut bots, 6.0);
        assert!(fired);
        let at = g.players[bots[0].0].body.position;
        assert!(at.distance(area[0]) < 1.0, "at {at}");
    }

    #[test]
    fn allies_follow_the_player_they_meet() {
        let (mut g, mut bots) = mission(&[Vec3::new(3.0, 0.0, 0.0)]);
        let (i, bot) = &mut bots[0];
        g.players[*i].team = 0;
        bot.actor.as_mut().unwrap().ally = true;
        run(&mut g, &mut bots, 1.0);
        assert_eq!(bots[0].1.actor.as_ref().unwrap().following, Some(0));
        g.players[0].body.position = Vec3::new(-10.0, 4.0, 0.0);
        run(&mut g, &mut bots, 8.0);
        let at = g.players[bots[0].0].body.position;
        assert!(
            at.distance(Vec3::new(-10.0, 4.0, 0.0)) < FOLLOW_FAR + 0.5,
            "at {at}"
        );
    }
}
