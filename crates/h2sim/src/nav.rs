//! A walking graph for computer players: points on the floor (spawn points,
//! item spots) joined wherever a Spartan can walk straight from one to the
//! other, up stairs and ramps and down drops.

use crate::collision::World;
use glam::Vec3;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

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
}
