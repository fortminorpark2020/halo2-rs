//! Static level collision: a triangle soup in a uniform grid, queried with capsules.

use glam::Vec3;
use std::collections::HashMap;

const CELL: f32 = 1.0;

#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub normal: Vec3,
}

/// A contact between a capsule and the world.
#[derive(Debug, Clone, Copy)]
pub struct Contact {
    /// Direction to push the capsule out (unit length).
    pub normal: Vec3,
    pub depth: f32,
}

pub struct World {
    triangles: Vec<Triangle>,
    grid: HashMap<(i32, i32, i32), Vec<u32>>,
    pub min: Vec3,
    pub max: Vec3,
}

fn cell_of(p: Vec3) -> (i32, i32, i32) {
    (
        (p.x / CELL).floor() as i32,
        (p.y / CELL).floor() as i32,
        (p.z / CELL).floor() as i32,
    )
}

impl World {
    pub fn new(positions: &[[f32; 3]], indices: &[u32]) -> World {
        let mut triangles = Vec::with_capacity(indices.len() / 3);
        let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for t in indices.as_chunks::<3>().0 {
            let [a, b, c] = t.map(|i| Vec3::from(positions[i as usize]));
            let n = (b - a).cross(c - a);
            if n.length_squared() < 1e-12 {
                continue;
            }
            let id = triangles.len() as u32;
            triangles.push(Triangle {
                a,
                b,
                c,
                normal: n.normalize(),
            });
            let lo = a.min(b).min(c);
            let hi = a.max(b).max(c);
            min = min.min(lo);
            max = max.max(hi);
            let (l, h) = (cell_of(lo), cell_of(hi));
            for x in l.0..=h.0 {
                for y in l.1..=h.1 {
                    for z in l.2..=h.2 {
                        grid.entry((x, y, z)).or_default().push(id);
                    }
                }
            }
        }
        World {
            triangles,
            grid,
            min,
            max,
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    fn candidates(&self, lo: Vec3, hi: Vec3, out: &mut Vec<u32>) {
        out.clear();
        let (l, h) = (cell_of(lo), cell_of(hi));
        for x in l.0..=h.0 {
            for y in l.1..=h.1 {
                for z in l.2..=h.2 {
                    if let Some(ids) = self.grid.get(&(x, y, z)) {
                        out.extend_from_slice(ids);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }

    /// All contacts of a capsule (segment `p0`..`p1`, `radius`) with the world.
    pub fn capsule_contacts(&self, p0: Vec3, p1: Vec3, radius: f32, out: &mut Vec<Contact>) {
        out.clear();
        let mut ids = Vec::new();
        let pad = Vec3::splat(radius);
        self.candidates(p0.min(p1) - pad, p0.max(p1) + pad, &mut ids);
        for id in ids {
            let t = &self.triangles[id as usize];
            let (on_seg, on_tri) = closest_segment_triangle(p0, p1, t);
            let d = on_seg - on_tri;
            let dist = d.length();
            if dist >= radius {
                continue;
            }
            // Push out along the separating direction; for (near-)touching or
            // intersecting cases use the face normal, toward the capsule.
            let normal = if dist > 1e-4 {
                d / dist
            } else {
                let mid = (p0 + p1) * 0.5;
                if (mid - t.a).dot(t.normal) >= 0.0 {
                    t.normal
                } else {
                    -t.normal
                }
            };
            out.push(Contact {
                normal,
                depth: radius - dist,
            });
        }
    }

    /// Distance along `dir` (unit) from `origin` to the first triangle hit, up to `max`.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        let mut ids = Vec::new();
        // March cell by cell is overkill for short rays; gather along the ray's box.
        let end = origin + dir * max;
        self.candidates(origin.min(end), origin.max(end), &mut ids);
        for id in ids {
            let t = &self.triangles[id as usize];
            if let Some(d) = ray_triangle(origin, dir, t) {
                if d <= max && best.is_none_or(|b| d < b) {
                    best = Some(d);
                }
            }
        }
        best
    }
}

/// Möller–Trumbore; hits from either side.
fn ray_triangle(o: Vec3, d: Vec3, t: &Triangle) -> Option<f32> {
    let e1 = t.b - t.a;
    let e2 = t.c - t.a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t.a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let dist = e2.dot(q) * inv;
    (dist >= 0.0).then_some(dist)
}

/// Closest point on triangle to `p` (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_point_triangle(p: Vec3, t: &Triangle) -> Vec3 {
    let (a, b, c) = (t.a, t.b, t.c);
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Closest points between segments `p1`..`q1` and `p2`..`q2`.
fn closest_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= 1e-12 && e <= 1e-12 {
        return (p1, p2);
    }
    if a <= 1e-12 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom != 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// Closest points between a segment and a triangle.
pub fn closest_segment_triangle(p0: Vec3, p1: Vec3, t: &Triangle) -> (Vec3, Vec3) {
    // Segment crossing the triangle: zero distance at the crossing point.
    let d0 = (p0 - t.a).dot(t.normal);
    let d1 = (p1 - t.a).dot(t.normal);
    if d0 * d1 < 0.0 {
        let x = p0 + (p1 - p0) * (d0 / (d0 - d1));
        if closest_point_triangle(x, t).distance_squared(x) < 1e-10 {
            return (x, x);
        }
    }
    let mut best = (p0, closest_point_triangle(p0, t));
    let mut best_d = best.0.distance_squared(best.1);
    let mut consider = |s: Vec3, q: Vec3| {
        let d = s.distance_squared(q);
        if d < best_d {
            best_d = d;
            best = (s, q);
        }
    };
    consider(p1, closest_point_triangle(p1, t));
    for (ea, eb) in [(t.a, t.b), (t.b, t.c), (t.c, t.a)] {
        let (s, q) = closest_segments(p0, p1, ea, eb);
        consider(s, q);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> World {
        // 20x20 quad at z = 0, facing up.
        let p = [
            [-10.0, -10.0, 0.0],
            [10.0, -10.0, 0.0],
            [10.0, 10.0, 0.0],
            [-10.0, 10.0, 0.0],
        ];
        World::new(&p, &[0, 1, 2, 0, 2, 3])
    }

    #[test]
    fn capsule_resting_on_floor_touches_with_up_normal() {
        let w = floor();
        let mut c = Vec::new();
        w.capsule_contacts(
            Vec3::new(0.0, 0.0, 0.15),
            Vec3::new(0.0, 0.0, 0.5),
            0.175,
            &mut c,
        );
        assert!(!c.is_empty());
        assert!(c
            .iter()
            .all(|c| c.normal.z > 0.99 && (c.depth - 0.025).abs() < 1e-4));
    }

    #[test]
    fn capsule_above_floor_has_no_contacts() {
        let w = floor();
        let mut c = Vec::new();
        w.capsule_contacts(
            Vec3::new(0.0, 0.0, 0.3),
            Vec3::new(0.0, 0.0, 0.6),
            0.175,
            &mut c,
        );
        assert!(c.is_empty());
    }

    #[test]
    fn raycast_down_hits_floor() {
        let w = floor();
        let d = w
            .raycast(Vec3::new(1.0, 2.0, 3.0), Vec3::NEG_Z, 10.0)
            .unwrap();
        assert!((d - 3.0).abs() < 1e-5);
    }
}
