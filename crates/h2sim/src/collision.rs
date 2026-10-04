//! Level collision: a triangle soup in a uniform grid, queried with
//! capsules, plus doors that block while shut and movers (lifts) that slide
//! around the level.

use glam::Vec3;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const CELL: f32 = 1.0;
/// Triangles spanning more grid cells than this (a map's floors and walls
/// far below or around it) go in a coarser grid instead, and ones too big
/// even for that in a list tested by every query.
const LARGE: u64 = 4096;
const COARSE_CELL: f32 = 16.0;
/// A mover's grid is at most this many cells across.
const MOVER_CELLS: f32 = 32.0;

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

/// A box that kills whoever enters it (a map's pits and sea).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KillZone {
    corner: Vec3,
    axes: [Vec3; 3],
    extents: Vec3,
}

impl KillZone {
    /// A box from `corner`, `extents` long along `forward`, its left
    /// (`up` x `forward`) and `up`.
    pub fn new(corner: Vec3, forward: Vec3, up: Vec3, extents: Vec3) -> KillZone {
        let forward = forward.normalize_or(Vec3::X);
        let up = up.normalize_or(Vec3::Z);
        KillZone {
            corner,
            axes: [forward, up.cross(forward), up],
            extents,
        }
    }

    pub fn contains(&self, p: Vec3) -> bool {
        let d = p - self.corner;
        self.axes
            .iter()
            .zip(self.extents.to_array())
            .all(|(axis, extent)| (0.0..=extent).contains(&d.dot(*axis)))
    }
}

pub struct World {
    triangles: Vec<Triangle>,
    grid: Grid,
    coarse: Grid,
    large: Vec<u32>,
    pub min: Vec3,
    pub max: Vec3,
    /// Each triangle's door (`LEVEL` for the level's own).
    owner: Vec<u16>,
    /// Each door: shut (it blocks) or open.
    shut: Vec<AtomicBool>,
    movers: Vec<Mover>,
}

/// A part of the level that moves (a lift): its triangles where they were
/// placed, and how far they've moved since.
struct Mover {
    triangles: Vec<Triangle>,
    grid: Grid,
    lo: Vec3,
    hi: Vec3,
    offset: [AtomicU32; 3],
    present: AtomicBool,
}

impl Mover {
    fn offset(&self) -> Vec3 {
        Vec3::from(
            self.offset
                .each_ref()
                .map(|a| f32::from_bits(a.load(Ordering::Relaxed))),
        )
    }

    /// Where a ray from `origin` (relative to the mover where placed)
    /// first hits it.
    fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Hit {
        // Only the stretch of the ray inside its bounds.
        let (mut near, mut far) = (0.0f32, max);
        for k in 0..3 {
            let (o, d) = (origin[k], dir[k]);
            if d.abs() < 1e-9 {
                if o < self.lo[k] || o > self.hi[k] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((self.lo[k] - o) / d, (self.hi[k] - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        if near > far {
            return None;
        }
        let start = origin + dir * near;
        let mut best: Hit = None;
        let test = |id: u32, best: &mut Hit| {
            let t = &self.triangles[id as usize];
            if let Some(d) = ray_triangle(start, dir, t) {
                if d <= far - near && best.is_none_or(|b| d < b.0) {
                    let n = if t.normal.dot(dir) > 0.0 {
                        -t.normal
                    } else {
                        t.normal
                    };
                    *best = Some((d, n));
                }
            }
        };
        self.grid.walk(start, dir, far - near, &mut best, test);
        best.map(|(d, n)| (d + near, n))
    }
}

/// The owner of the level's own triangles.
const LEVEL: u16 = u16::MAX;

/// Triangles by the cubes of space their bounds overlap.
struct Grid {
    size: f32,
    cells: HashMap<(i32, i32, i32), Vec<u32>>,
}

type Hit = Option<(f32, Vec3)>;

impl Grid {
    fn new(size: f32) -> Grid {
        Grid {
            size,
            cells: HashMap::new(),
        }
    }

    fn cell(&self, p: Vec3) -> (i32, i32, i32) {
        (
            (p.x / self.size).floor() as i32,
            (p.y / self.size).floor() as i32,
            (p.z / self.size).floor() as i32,
        )
    }

    /// How many cells a box overlaps.
    fn count(&self, lo: Vec3, hi: Vec3) -> u64 {
        let (l, h) = (self.cell(lo), self.cell(hi));
        [h.0 - l.0, h.1 - l.1, h.2 - l.2]
            .iter()
            .map(|&n| n as u64 + 1)
            .product()
    }

    fn insert(&mut self, id: u32, lo: Vec3, hi: Vec3) {
        let (l, h) = (self.cell(lo), self.cell(hi));
        for x in l.0..=h.0 {
            for y in l.1..=h.1 {
                for z in l.2..=h.2 {
                    self.cells.entry((x, y, z)).or_default().push(id);
                }
            }
        }
    }

    /// The triangles in the cells a box overlaps (some more than once).
    fn overlapping(&self, lo: Vec3, hi: Vec3, out: &mut Vec<u32>) {
        let (l, h) = (self.cell(lo), self.cell(hi));
        for x in l.0..=h.0 {
            for y in l.1..=h.1 {
                for z in l.2..=h.2 {
                    if let Some(ids) = self.cells.get(&(x, y, z)) {
                        out.extend_from_slice(ids);
                    }
                }
            }
        }
    }

    /// Test the triangles in the cells a ray passes through, nearest first
    /// (Amanatides and Woo), until a hit comes before the next cell.
    fn walk(
        &self,
        origin: Vec3,
        dir: Vec3,
        max: f32,
        best: &mut Hit,
        test: impl Fn(u32, &mut Hit),
    ) {
        if self.cells.is_empty() {
            return;
        }
        let size = self.size;
        let mut cell = self.cell(origin);
        let axis = |o: f32, d: f32, c: i32| -> (i32, f32, f32) {
            if d > 0.0 {
                (1, ((c + 1) as f32 * size - o) / d, size / d)
            } else if d < 0.0 {
                (-1, (c as f32 * size - o) / d, -size / d)
            } else {
                (0, f32::INFINITY, f32::INFINITY)
            }
        };
        let (sx, mut tx, dx) = axis(origin.x, dir.x, cell.0);
        let (sy, mut ty, dy) = axis(origin.y, dir.y, cell.1);
        let (sz, mut tz, dz) = axis(origin.z, dir.z, cell.2);
        loop {
            if let Some(ids) = self.cells.get(&cell) {
                for &id in ids {
                    test(id, best);
                }
            }
            let exit = tx.min(ty).min(tz);
            if exit > max || best.is_some_and(|b| b.0 <= exit) {
                break;
            }
            if tx <= ty && tx <= tz {
                cell.0 += sx;
                tx += dx;
            } else if ty <= tz {
                cell.1 += sy;
                ty += dy;
            } else {
                cell.2 += sz;
                tz += dz;
            }
        }
    }
}

impl World {
    pub fn new(positions: &[[f32; 3]], indices: &[u32]) -> World {
        let mut triangles = Vec::with_capacity(indices.len() / 3);
        let mut grid = Grid::new(CELL);
        let mut coarse = Grid::new(COARSE_CELL);
        let mut large = Vec::new();
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
            if grid.count(lo, hi) <= LARGE {
                grid.insert(id, lo, hi);
            } else if coarse.count(lo, hi) <= LARGE {
                coarse.insert(id, lo, hi);
            } else {
                large.push(id);
            }
        }
        World {
            owner: vec![LEVEL; triangles.len()],
            triangles,
            grid,
            coarse,
            large,
            min,
            max,
            shut: Vec::new(),
            movers: Vec::new(),
        }
    }

    /// Add a mover: triangles (where they start) that a lift carries
    /// around with [`World::move_mover`].
    pub fn add_mover(&mut self, triangles: &[[Vec3; 3]]) -> usize {
        let mut kept = Vec::new();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for &[a, b, c] in triangles {
            let n = (b - a).cross(c - a);
            if n.length_squared() < 1e-12 {
                continue;
            }
            lo = lo.min(a.min(b).min(c));
            hi = hi.max(a.max(b).max(c));
            kept.push(Triangle {
                a,
                b,
                c,
                normal: n.normalize(),
            });
        }
        // Big movers (a ship flying by) get bigger cells.
        let mut grid = Grid::new(CELL.max((hi - lo).max_element() / MOVER_CELLS));
        for (id, t) in kept.iter().enumerate() {
            grid.insert(id as u32, t.a.min(t.b).min(t.c), t.a.max(t.b).max(t.c));
        }
        self.movers.push(Mover {
            triangles: kept,
            grid,
            lo,
            hi,
            offset: [0.0f32; 3].map(|v| AtomicU32::new(v.to_bits())),
            present: AtomicBool::new(true),
        });
        self.movers.len() - 1
    }

    /// Put a mover `offset` away from where it started; one not `present`
    /// (not in the level just now) blocks nothing.
    pub fn move_mover(&self, mover: usize, offset: Vec3, present: bool) {
        if let Some(m) = self.movers.get(mover) {
            for (a, v) in m.offset.iter().zip(offset.to_array()) {
                a.store(v.to_bits(), Ordering::Relaxed);
            }
            m.present.store(present, Ordering::Relaxed);
        }
    }

    /// The mover right under `feet` (standing on it), if any is nearer
    /// than the level.
    pub fn mover_under(&self, feet: Vec3, reach: f32) -> Option<usize> {
        const ABOVE: f32 = 0.25;
        let origin = feet + Vec3::Z * ABOVE;
        let max = reach + ABOVE;
        let (k, d) = self
            .movers
            .iter()
            .enumerate()
            .filter(|(_, m)| m.present.load(Ordering::Relaxed))
            .filter_map(|(k, m)| Some((k, m.raycast(origin - m.offset(), Vec3::NEG_Z, max)?.0)))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let level = self.raycast_level(origin, Vec3::NEG_Z, d);
        level.is_none_or(|(l, _)| l >= d - 1e-3).then_some(k)
    }

    /// Add a door (shut): triangles that block until it opens.
    pub fn add_door(&mut self, triangles: &[[Vec3; 3]]) -> usize {
        let door = self.shut.len();
        self.shut.push(AtomicBool::new(true));
        for &[a, b, c] in triangles {
            let n = (b - a).cross(c - a);
            if n.length_squared() < 1e-12 {
                continue;
            }
            let id = self.triangles.len() as u32;
            self.triangles.push(Triangle {
                a,
                b,
                c,
                normal: n.normalize(),
            });
            self.owner.push(door as u16);
            let lo = a.min(b).min(c);
            let hi = a.max(b).max(c);
            if self.grid.count(lo, hi) <= LARGE {
                self.grid.insert(id, lo, hi);
            } else if self.coarse.count(lo, hi) <= LARGE {
                self.coarse.insert(id, lo, hi);
            } else {
                self.large.push(id);
            }
        }
        door
    }

    /// Shut a door (it blocks) or open it.
    pub fn set_door(&self, door: usize, shut: bool) {
        if let Some(d) = self.shut.get(door) {
            d.store(shut, Ordering::Relaxed);
        }
    }

    /// Whether a triangle blocks: the level's do, a door's while shut.
    fn solid(&self, id: u32) -> bool {
        match self.owner[id as usize] {
            LEVEL => true,
            d => self.shut[d as usize].load(Ordering::Relaxed),
        }
    }

    pub fn floors(&self, min_up: f32) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.triangles
            .iter()
            .zip(&self.owner)
            .filter(move |(t, &o)| o == LEVEL && t.normal.z >= min_up)
            .map(|(t, _)| t)
            .map(|t| [t.a, t.b, t.c])
    }

    fn candidates(&self, lo: Vec3, hi: Vec3, out: &mut Vec<u32>) {
        out.clear();
        self.grid.overlapping(lo, hi, out);
        let fine = out.len();
        self.coarse.overlapping(lo, hi, out);
        out.extend_from_slice(&self.large);
        // Big triangles only where their bounds reach the box.
        let mut k = fine;
        for i in fine..out.len() {
            let t = &self.triangles[out[i] as usize];
            if t.a.min(t.b).min(t.c).cmple(hi).all() && t.a.max(t.b).max(t.c).cmpge(lo).all() {
                out[k] = out[i];
                k += 1;
            }
        }
        out.truncate(k);
        out.sort_unstable();
        out.dedup();
    }

    /// All contacts of a capsule (segment `p0`..`p1`, `radius`) with the world.
    pub fn capsule_contacts(&self, p0: Vec3, p1: Vec3, radius: f32, out: &mut Vec<Contact>) {
        out.clear();
        let mut ids = Vec::new();
        let pad = Vec3::splat(radius);
        let (lo, hi) = (p0.min(p1) - pad, p0.max(p1) + pad);
        self.candidates(lo, hi, &mut ids);
        for id in ids {
            if self.solid(id) {
                out.extend(contact(p0, p1, radius, &self.triangles[id as usize]));
            }
        }
        let mut ids = Vec::new();
        for m in &self.movers {
            if !m.present.load(Ordering::Relaxed) {
                continue;
            }
            let off = m.offset();
            let (lo, hi) = (lo - off, hi - off);
            if lo.cmpgt(m.hi).any() || hi.cmplt(m.lo).any() {
                continue;
            }
            ids.clear();
            m.grid.overlapping(lo, hi, &mut ids);
            ids.sort_unstable();
            ids.dedup();
            for &id in &ids {
                out.extend(contact(
                    p0 - off,
                    p1 - off,
                    radius,
                    &m.triangles[id as usize],
                ));
            }
        }
    }

    /// Distance along `dir` (unit) from `origin` to the first triangle hit, up to `max`.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<f32> {
        self.raycast_hit(origin, dir, max).map(|(d, _)| d)
    }

    /// Like [`World::raycast`], also returning the surface normal facing the ray.
    pub fn raycast_hit(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let mut best = self.raycast_level(origin, dir, max);
        for m in &self.movers {
            if !m.present.load(Ordering::Relaxed) {
                continue;
            }
            let reach = best.map_or(max, |b| b.0);
            if let Some(hit) = m.raycast(origin - m.offset(), dir, reach) {
                best = Some(hit);
            }
        }
        best
    }

    /// The same, against the level and its doors only.
    fn raycast_level(&self, origin: Vec3, dir: Vec3, max: f32) -> Hit {
        let mut best: Hit = None;
        let test = |id: u32, best: &mut Hit| {
            if !self.solid(id) {
                return;
            }
            let t = &self.triangles[id as usize];
            if let Some(d) = ray_triangle(origin, dir, t) {
                if d <= max && best.is_none_or(|b| d < b.0) {
                    let n = if t.normal.dot(dir) > 0.0 {
                        -t.normal
                    } else {
                        t.normal
                    };
                    *best = Some((d, n));
                }
            }
        };
        for &id in &self.large {
            test(id, &mut best);
        }
        self.coarse.walk(origin, dir, max, &mut best, test);
        self.grid.walk(origin, dir, max, &mut best, test);
        best
    }

    /// The same, testing every triangle (to check the fast way against).
    #[cfg(test)]
    fn raycast_every_triangle(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<f32> {
        self.triangles
            .iter()
            .filter_map(|t| ray_triangle(origin, dir, t))
            .filter(|&d| d <= max)
            .min_by(f32::total_cmp)
    }
}

/// A capsule's contact with a triangle, if they touch.
fn contact(p0: Vec3, p1: Vec3, radius: f32, t: &Triangle) -> Option<Contact> {
    let (on_seg, on_tri) = closest_segment_triangle(p0, p1, t);
    let d = on_seg - on_tri;
    let dist = d.length();
    if dist >= radius {
        return None;
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
    Some(Contact {
        normal,
        depth: radius - dist,
    })
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
    fn capsules_touch_huge_triangles() {
        let w = World::new(
            &[[-500., -500., 0.], [500., -500., 0.], [0., 500., 0.]],
            &[0, 1, 2],
        );
        assert!(w.grid.cells.is_empty());
        let mut out = Vec::new();
        let p = Vec3::new(3.0, 4.0, 0.3);
        w.capsule_contacts(p, p + Vec3::Z, 0.4, &mut out);
        assert_eq!(out.len(), 1);
        assert!(out[0].normal.z > 0.99);
        let p = Vec3::new(3.0, 4.0, 1.0);
        w.capsule_contacts(p, p + Vec3::Z, 0.4, &mut out);
        assert!(out.is_empty());
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
    fn doors_block_only_while_shut() {
        let mut w = floor();
        // A door across x = 2, from the floor up.
        let (a, b, c, d) = (
            Vec3::new(2.0, -1.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(2.0, 1.0, 2.0),
            Vec3::new(2.0, -1.0, 2.0),
        );
        let door = w.add_door(&[[a, b, c], [a, c, d]]);
        let floors = w.floors(0.7).count();
        let ray = |w: &World| w.raycast(Vec3::new(0.0, 0.0, 1.0), Vec3::X, 5.0);
        let touching = |w: &World| {
            let mut out = Vec::new();
            let p = Vec3::new(1.9, 0.0, 0.5);
            w.capsule_contacts(p, p + Vec3::Z * 0.5, 0.2, &mut out);
            out.iter().any(|c| c.normal.x < -0.9)
        };
        assert_eq!(ray(&w), Some(2.0));
        assert!(touching(&w));
        w.set_door(door, false);
        assert_eq!(ray(&w), None);
        assert!(!touching(&w));
        assert_eq!(floors, w.floors(0.7).count(), "doors aren't floors");
    }

    #[test]
    fn movers_block_where_they_are_and_carry_what_stands_on_them() {
        let w0 = floor();
        let mut w = floor();
        // A platform 1 x 1 at z = 1 around (5, 0).
        let (a, b, c, d) = (
            Vec3::new(4.5, -0.5, 1.0),
            Vec3::new(5.5, -0.5, 1.0),
            Vec3::new(5.5, 0.5, 1.0),
            Vec3::new(4.5, 0.5, 1.0),
        );
        let lift = w.add_mover(&[[a, b, c], [a, c, d]]);
        let down = |w: &World, x: f32| w.raycast(Vec3::new(x, 0.0, 5.0), Vec3::NEG_Z, 10.0);
        assert_eq!(down(&w, 5.0), Some(4.0));
        assert_eq!(
            down(&w, 3.0),
            down(&w0, 3.0),
            "the level as it was elsewhere"
        );
        assert_eq!(w.mover_under(Vec3::new(5.0, 0.0, 1.0), 0.1), Some(lift));
        assert_eq!(w.mover_under(Vec3::new(3.0, 0.0, 0.0), 0.1), None);
        w.move_mover(lift, Vec3::new(0.0, 0.0, 2.0), true);
        assert_eq!(down(&w, 5.0), Some(2.0));
        assert_eq!(w.mover_under(Vec3::new(5.0, 0.0, 3.0), 0.1), Some(lift));
        let mut out = Vec::new();
        let feet = Vec3::new(5.0, 0.0, 2.9);
        w.capsule_contacts(feet + Vec3::Z * 0.2, feet + Vec3::Z * 0.6, 0.2, &mut out);
        assert!(out.iter().any(|c| c.normal.z > 0.9), "standing on it");
        w.move_mover(lift, Vec3::ZERO, false);
        assert_eq!(down(&w, 5.0), down(&w0, 5.0), "gone");
    }

    #[test]
    fn raycast_hit_normal_faces_ray() {
        let w = floor();
        let (_, n) = w
            .raycast_hit(Vec3::new(0.0, 0.0, -2.0), Vec3::Z, 10.0)
            .unwrap();
        assert!(n.z < -0.99);
    }

    #[test]
    fn raycast_down_hits_floor() {
        let w = floor();
        let d = w
            .raycast(Vec3::new(1.0, 2.0, 3.0), Vec3::NEG_Z, 10.0)
            .unwrap();
        assert!((d - 3.0).abs() < 1e-5);
    }

    #[test]
    fn kill_zones_are_turned_boxes() {
        // 2 long, 1 wide, 1 high, turned 90 degrees: it runs along +y.
        let k = KillZone::new(Vec3::ZERO, Vec3::Y, Vec3::Z, Vec3::new(2.0, 1.0, 1.0));
        assert!(k.contains(Vec3::new(-0.5, 1.5, 0.5)));
        assert!(!k.contains(Vec3::new(0.5, 1.5, 0.5)));
        assert!(!k.contains(Vec3::new(-0.5, 2.5, 0.5)));
        assert!(!k.contains(Vec3::new(-0.5, 1.5, 1.5)));
    }

    #[test]
    fn raycasts_find_the_nearest_triangle() {
        // A scatter of triangles, and rays every which way through them.
        let mut rng = 12345u32;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng >> 8) as f32 / (1u32 << 24) as f32 * 20.0 - 10.0
        };
        let mut positions = Vec::new();
        for _ in 0..300 {
            let c = Vec3::new(next(), next(), next() * 0.3);
            for _ in 0..3 {
                let p = c + Vec3::new(next(), next(), next()) * 0.15;
                positions.push(p.into());
            }
        }
        // And a few huge slanted ones, kept out of the fine grid.
        for k in 0..3 {
            let z = k as f32 - 1.0;
            positions.extend([[-60., -60., z - 3.], [60., -50., z], [0., 60., z + 3.]]);
        }
        let indices: Vec<u32> = (0..positions.len() as u32).collect();
        let w = World::new(&positions, &indices);
        let big = |grid: &Grid| {
            let mut ids: Vec<u32> = grid.cells.values().flatten().copied().collect();
            ids.sort_unstable();
            ids.dedup();
            ids.len()
        };
        assert_eq!(big(&w.coarse), 3);
        assert!(w.large.is_empty());
        for _ in 0..500 {
            let o = Vec3::new(next(), next(), next() * 0.3);
            let d = Vec3::new(next(), next(), next() * 0.5).normalize();
            let max = 5.0 + next().abs() * 2.0;
            let fast = w.raycast(o, d, max);
            let slow = w.raycast_every_triangle(o, d, max);
            match (fast, slow) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 1e-4, "{a} vs {b}"),
                (a, b) => assert_eq!(a, b),
            }
        }
    }
}
