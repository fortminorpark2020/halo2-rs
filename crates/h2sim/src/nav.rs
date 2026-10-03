//! A walking graph for computer players: points on the floor (spread over
//! the level's floors, plus spawn points and item spots) joined wherever a
//! Spartan can walk straight from one to the other, up stairs and ramps and
//! down drops.

use crate::collision::{KillZone, World};
use crate::player::GRAVITY;
use glam::Vec3;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Points further apart than this aren't joined directly.
const MAX_LINK: f32 = 10.0;
/// Distance between floor checks along a link.
const STEP: f32 = 0.25;
/// Highest step up between floor checks (stairs, kerbs).
const MAX_RISE: f32 = 0.2;
/// Deepest drop a walker will take.
const MAX_DROP: f32 = 2.0;
/// Height of the clearance check above the floor.
const KNEE: f32 = 0.3;
const CHEST: f32 = 0.5;
/// Distance between the points spread over a level's floors, and between
/// the places tried for them.
const SPACING: f32 = 1.0;
const SAMPLE: f32 = 0.4;
/// Running speed, for how far a Spartan carries off a drop.
const RUN_SPEED: f32 = 2.25;
/// Spread points this close are joined.
const NEAR_LINK: f32 = 3.5;
/// Floors are no steeper than this (cosine of the slope).
const FLOOR_UP: f32 = 0.7;
/// Room a Spartan needs: overhead, and around at waist height.
const HEADROOM: f32 = 0.75;
const ELBOW_ROOM: f32 = 0.3;

#[derive(Debug, Clone, Default)]
pub struct NavGraph {
    /// Feet positions on the floor.
    pub points: Vec<Vec3>,
    /// Outgoing links of each point (links over drops go one way).
    pub links: Vec<Vec<usize>>,
}

/// The floor under `p` (within `depth` below a point `rise` above it).
fn floor_below(world: &World, p: Vec3, rise: f32, depth: f32) -> Option<f32> {
    let top = p + Vec3::Z * rise;
    world
        .raycast(top, Vec3::NEG_Z, rise + depth)
        .map(|t| top.z - t)
}

/// Whether a Spartan can walk in a straight line from `a` to `b`.
pub fn walkable(world: &World, a: Vec3, b: Vec3) -> bool {
    let flat = (b - a).truncate();
    let length = flat.length();
    if length > MAX_LINK {
        return false;
    }
    let steps = (length / STEP).ceil().max(1.0) as usize;
    let mut here = a;
    for k in 1..=steps {
        let t = k as f32 / steps as f32;
        let xy = a.truncate().lerp(b.truncate(), t);
        let probe = xy.extend(here.z);
        // Nothing in the way at knee and chest height.
        let d = probe - here;
        let len = d.length();
        if len > 1e-4 {
            for h in [KNEE, CHEST] {
                if world.raycast(here + Vec3::Z * h, d / len, len).is_some() {
                    return false;
                }
            }
        }
        let Some(z) = floor_below(world, probe, MAX_RISE + 0.05, MAX_DROP) else {
            return false;
        };
        // Running off a drop carries a Spartan on through the air: there
        // must be floor at the bottom where it comes down.
        let drop = here.z - z;
        if drop > 0.5 {
            let carry = RUN_SPEED * (2.0 * drop / GRAVITY).sqrt();
            let dir = (b - a).truncate().normalize_or_zero();
            for k in 1..=4 {
                let land = (xy + dir * carry * k as f32 / 4.0).extend(z);
                if floor_below(world, land, MAX_RISE + 0.05, MAX_DROP).is_none() {
                    return false;
                }
            }
        }
        here = xy.extend(z);
    }
    (here.z - b.z).abs() < 0.3
}

#[derive(PartialEq)]
struct Open(f32, usize);
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Whether a Spartan fits standing at `p` (on the floor).
fn roomy(world: &World, p: Vec3) -> bool {
    if world
        .raycast(p + Vec3::Z * 0.05, Vec3::Z, HEADROOM)
        .is_some()
    {
        return false;
    }
    (0..8).all(|k| {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        let dir = Vec3::new(a.cos(), a.sin(), 0.0);
        [KNEE, CHEST]
            .iter()
            .all(|&h| world.raycast(p + Vec3::Z * h, dir, ELBOW_ROOM).is_none())
    })
}

/// Points spread over the level's floors, about `SPACING` apart, where a
/// Spartan fits, between `lo` and `hi`.
fn floor_points(world: &World, lo: Vec3, hi: Vec3, avoid: &[KillZone]) -> Vec<Vec3> {
    let mut points = Vec::new();
    let mut taken = HashSet::new();
    for [a, b, c] in world.floors(FLOOR_UP) {
        let (tlo, thi) = (a.min(b).min(c), a.max(b).max(c));
        if thi.cmplt(lo).any() || tlo.cmpgt(hi).any() {
            continue;
        }
        // Sampled finely, so even narrow doorways get a point that fits.
        let longest = (b - a).length().max((c - b).length()).max((a - c).length());
        let n = (longest / SAMPLE).ceil().max(1.0) as usize;
        for i in 0..n {
            for j in 0..n - i {
                let (u, v) = (
                    (i as f32 + 1.0 / 3.0) / n as f32,
                    (j as f32 + 1.0 / 3.0) / n as f32,
                );
                let p = a + (b - a) * u + (c - a) * v;
                let cell = (
                    (p.x / SPACING).floor() as i32,
                    (p.y / SPACING).floor() as i32,
                    (p.z / 1.2).floor() as i32,
                );
                if taken.contains(&cell) {
                    continue;
                }
                let Some(z) = floor_below(world, p, 0.3, 0.6) else {
                    continue;
                };
                let p = p.truncate().extend(z);
                let inside = p.cmpge(lo).all() && p.cmple(hi).all();
                if inside && !avoid.iter().any(|k| k.contains(p)) && roomy(world, p) {
                    taken.insert(cell);
                    points.push(p);
                }
            }
        }
    }
    points
}

impl NavGraph {
    /// Points are dropped onto the floor below them; ones over nothing are
    /// left out.
    pub fn build(world: &World, points: &[Vec3]) -> NavGraph {
        let points: Vec<Vec3> = points
            .iter()
            .filter_map(|&p| floor_below(world, p, 0.3, 2.0).map(|z| p.truncate().extend(z)))
            .collect();
        let mut links = vec![Vec::new(); points.len()];
        for (i, &a) in points.iter().enumerate() {
            for (j, &b) in points.iter().enumerate() {
                if i != j && walkable(world, a, b) {
                    links[i].push(j);
                }
            }
        }
        NavGraph { points, links }
    }

    /// A graph over a whole level: points spread over its floors around
    /// `spots` (spawns, items, flags), plus the spots, each joined to its
    /// near neighbours. Places in `avoid`, and ones that can't be reached
    /// from the spots or left again, are left out.
    pub fn for_level(world: &World, spots: &[Vec3], avoid: &[KillZone]) -> NavGraph {
        let spots: Vec<Vec3> = spots
            .iter()
            .filter_map(|&p| floor_below(world, p, 0.3, 2.0).map(|z| p.truncate().extend(z)))
            .collect();
        if spots.is_empty() {
            return NavGraph::default();
        }
        let lo = spots.iter().fold(Vec3::MAX, |m, p| m.min(*p)) - Vec3::new(8.0, 8.0, 4.0);
        let hi = spots.iter().fold(Vec3::MIN, |m, p| m.max(*p)) + Vec3::new(8.0, 8.0, 8.0);
        let mut points = floor_points(world, lo, hi, avoid);
        let first_spot = points.len();
        points.extend(&spots);
        let key = |p: Vec3| {
            (
                (p.x / NEAR_LINK).floor() as i32,
                (p.y / NEAR_LINK).floor() as i32,
            )
        };
        let mut buckets: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, &p) in points.iter().enumerate() {
            buckets.entry(key(p)).or_default().push(i);
        }
        let mut links = vec![Vec::new(); points.len()];
        for (i, &a) in points.iter().enumerate() {
            let (x, y) = key(a);
            for bx in x - 1..=x + 1 {
                for by in y - 1..=y + 1 {
                    for &j in buckets.get(&(bx, by)).map_or(&[][..], Vec::as_slice) {
                        let b = points[j];
                        if i != j
                            && a.truncate().distance(b.truncate()) < NEAR_LINK
                            && (a.z - b.z).abs() < MAX_DROP + 0.5
                            && walkable(world, a, b)
                        {
                            links[i].push(j);
                        }
                    }
                }
            }
        }
        NavGraph { points, links }.keep_reachable(first_spot)
    }

    /// Only the points the spots lead to (points from `first_spot` on)
    /// that can get back to the level's main area (its largest group of
    /// points that all reach each other), so that from anywhere in the
    /// graph a bot can find its way around.
    fn keep_reachable(self, first_spot: usize) -> NavGraph {
        let n = self.points.len();
        let mut back = vec![Vec::new(); n];
        for (i, out) in self.links.iter().enumerate() {
            for &j in out {
                back[j].push(i);
            }
        }
        let flood = |links: &[Vec<usize>], starts: Vec<usize>| {
            let mut seen = vec![false; n];
            let mut stack = starts;
            while let Some(i) = stack.pop() {
                if !std::mem::replace(&mut seen[i], true) {
                    stack.extend(links[i].iter().copied().filter(|&j| !seen[j]));
                }
            }
            seen
        };
        let forward = flood(&self.links, (first_spot..n).collect());
        let returns = flood(&back, self.main_area(&back));
        let mut index = vec![usize::MAX; n];
        let mut points = Vec::new();
        for i in 0..n {
            if forward[i] && returns[i] {
                index[i] = points.len();
                points.push(self.points[i]);
            }
        }
        let links = (0..n)
            .filter(|&i| index[i] != usize::MAX)
            .map(|i| {
                self.links[i]
                    .iter()
                    .map(|&j| index[j])
                    .filter(|&j| j != usize::MAX)
                    .collect()
            })
            .collect();
        NavGraph { points, links }
    }

    /// The largest group of points that can all reach each other (strongly
    /// connected; Kosaraju's method), given the links reversed.
    fn main_area(&self, back: &[Vec<usize>]) -> Vec<usize> {
        let n = self.points.len();
        // Points in the order their searches finish.
        let mut seen = vec![false; n];
        let mut order = Vec::with_capacity(n);
        for start in 0..n {
            if std::mem::replace(&mut seen[start], true) {
                continue;
            }
            let mut stack = vec![(start, 0usize)];
            while let Some(top) = stack.last_mut() {
                let (i, k) = *top;
                if let Some(&j) = self.links[i].get(k) {
                    top.1 += 1;
                    if !std::mem::replace(&mut seen[j], true) {
                        stack.push((j, 0));
                    }
                } else {
                    order.push(i);
                    stack.pop();
                }
            }
        }
        // Groups along the reversed links, latest finished first.
        let mut group = vec![usize::MAX; n];
        let mut sizes = Vec::new();
        for &start in order.iter().rev() {
            if group[start] != usize::MAX {
                continue;
            }
            let g = sizes.len();
            group[start] = g;
            let (mut stack, mut size) = (vec![start], 0);
            while let Some(i) = stack.pop() {
                size += 1;
                for &j in &back[i] {
                    if group[j] == usize::MAX {
                        group[j] = g;
                        stack.push(j);
                    }
                }
            }
            sizes.push(size);
        }
        let biggest = (0..sizes.len()).max_by_key(|&g| sizes[g]);
        (0..n).filter(|&i| Some(group[i]) == biggest).collect()
    }

    /// Cut corners off a route where the way is clear: from each point, on
    /// to the furthest of the next few that can be walked to straight.
    pub fn smooth(&self, world: &World, route: &[usize]) -> Vec<usize> {
        let mut out = Vec::with_capacity(route.len());
        let mut i = 0;
        while i < route.len() {
            out.push(route[i]);
            let from = self.points[route[i]];
            let furthest = (i + 2..route.len().min(i + 7))
                .rev()
                .find(|&j| walkable(world, from, self.points[route[j]]));
            i = furthest.unwrap_or(i + 1);
        }
        out
    }

    /// The closest point reachable in a straight line from `p`.
    pub fn nearest(&self, world: &World, p: Vec3) -> Option<usize> {
        let mut order: Vec<usize> = (0..self.points.len()).collect();
        order.sort_by(|&a, &b| {
            self.points[a]
                .distance_squared(p)
                .total_cmp(&self.points[b].distance_squared(p))
        });
        order
            .iter()
            .take(8)
            .copied()
            .find(|&i| walkable(world, p, self.points[i]))
            .or(order.first().copied())
    }

    /// Shortest route between two points (A*), including both ends.
    pub fn path(&self, from: usize, to: usize) -> Option<Vec<usize>> {
        self.path_avoiding(from, to, &[])
    }

    /// Shortest route that doesn't go through any of the `blocked` points.
    pub fn path_avoiding(&self, from: usize, to: usize, blocked: &[usize]) -> Option<Vec<usize>> {
        let n = self.points.len();
        if from >= n || to >= n {
            return None;
        }
        let h = |i: usize| self.points[i].distance(self.points[to]);
        let mut cost = vec![f32::INFINITY; n];
        let mut came = vec![usize::MAX; n];
        let mut open = BinaryHeap::new();
        cost[from] = 0.0;
        open.push(Open(h(from), from));
        while let Some(Open(_, i)) = open.pop() {
            if i == to {
                let mut route = vec![to];
                let mut k = to;
                while k != from {
                    k = came[k];
                    route.push(k);
                }
                route.reverse();
                return Some(route);
            }
            for &j in &self.links[i] {
                if j != to && blocked.contains(&j) {
                    continue;
                }
                let c = cost[i] + self.points[i].distance(self.points[j]);
                if c < cost[j] {
                    cost[j] = c;
                    came[j] = i;
                    open.push(Open(c + h(j), j));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two floors joined by a ramp, with a wall across the lower floor.
    fn level() -> World {
        let mut p: Vec<[f32; 3]> = Vec::new();
        let mut idx = Vec::new();
        let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| {
            let base = p.len() as u32;
            p.extend_from_slice(&[a, b, c, d]);
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        };
        // Lower floor x 0..10, upper floor x 14..20 at z 2, ramp between.
        quad([0., -5., 0.], [10., -5., 0.], [10., 5., 0.], [0., 5., 0.]);
        quad([10., -5., 0.], [14., -5., 2.], [14., 5., 2.], [10., 5., 0.]);
        quad([14., -5., 2.], [20., -5., 2.], [20., 5., 2.], [14., 5., 2.]);
        // A wall at x = 5 for y > -2.
        quad([5., -2., 0.], [5., 5., 0.], [5., 5., 2.], [5., -2., 2.]);
        World::new(&p, &idx)
    }

    #[test]
    fn links_follow_ramps_and_stop_at_walls() {
        let w = level();
        assert!(walkable(
            &w,
            Vec3::new(8.0, 0.0, 0.0),
            Vec3::new(17.0, 0.0, 2.0)
        ));
        assert!(walkable(
            &w,
            Vec3::new(17.0, 0.0, 2.0),
            Vec3::new(8.0, 0.0, 0.0)
        ));
        assert!(!walkable(
            &w,
            Vec3::new(2.0, 2.0, 0.0),
            Vec3::new(8.0, 2.0, 0.0)
        ));
    }

    #[test]
    fn paths_go_around_walls() {
        let w = level();
        let g = NavGraph::build(
            &w,
            &[
                Vec3::new(2.0, 2.0, 0.1),
                Vec3::new(2.0, -4.0, 0.1),
                Vec3::new(8.0, -4.0, 0.1),
                Vec3::new(8.0, 2.0, 0.1),
                Vec3::new(50.0, 50.0, 0.1),
            ],
        );
        // The point over nothing is dropped.
        assert_eq!(g.points.len(), 4);
        assert!(!g.links[0].contains(&3));
        let route = g.path(0, 3).unwrap();
        assert_eq!(route.first(), Some(&0));
        assert_eq!(route.last(), Some(&3));
        assert!(route.len() >= 3);
    }

    #[test]
    fn level_graphs_cover_the_floors() {
        let w = level();
        let spots = [Vec3::new(2.0, 2.0, 0.1), Vec3::new(17.0, 0.0, 2.1)];
        let g = NavGraph::for_level(&w, &spots, &[]);
        // Points all over both floors and the ramp, and none in the wall.
        assert!(g.points.len() > 50, "{}", g.points.len());
        assert!(g.points.iter().any(|p| p.z > 1.9 && p.x > 15.0));
        assert!(g
            .points
            .iter()
            .all(|p| (p.x - 5.0).abs() > 0.2 || p.y < -2.0));
        // Up the ramp and back, round the wall.
        let low = g.nearest(&w, Vec3::new(2.0, 2.0, 0.0)).unwrap();
        let high = g.nearest(&w, Vec3::new(17.0, 0.0, 2.0)).unwrap();
        assert!(g.path(low, high).is_some());
        assert!(g.path(high, low).is_some());
        // Leaving out a kill zone.
        let pit = KillZone::new(
            Vec3::new(6.0, -5.0, -1.0),
            Vec3::X,
            Vec3::Z,
            Vec3::new(3.0, 10.0, 2.0),
        );
        let g = NavGraph::for_level(&w, &spots, &[pit]);
        assert!(g.points.iter().all(|p| !pit.contains(*p)));
    }

    #[test]
    fn running_off_a_ledge_needs_somewhere_to_land() {
        // A platform at z 1.5 over a strip of floor 1 unit wide, then
        // nothing.
        let world = |quads: &[[[f32; 3]; 4]]| {
            let p: Vec<[f32; 3]> = quads.iter().flatten().copied().collect();
            let idx: Vec<u32> = (0..quads.len() as u32)
                .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|k| q * 4 + k))
                .collect();
            World::new(&p, &idx)
        };
        let platform = [[0., -5., 1.5], [5., -5., 1.5], [5., 5., 1.5], [0., 5., 1.5]];
        let strip = [[5., -5., 0.], [6., -5., 0.], [6., 5., 0.], [5., 5., 0.]];
        let (top, below) = (Vec3::new(4.0, 0.0, 1.5), Vec3::new(5.5, 0.0, 0.0));
        assert!(!walkable(&world(&[platform, strip]), top, below));
        // With a wide floor below it's fine.
        let beyond = [[6., -5., 0.], [12., -5., 0.], [12., 5., 0.], [6., 5., 0.]];
        assert!(walkable(&world(&[platform, strip, beyond]), top, below));
    }
}
