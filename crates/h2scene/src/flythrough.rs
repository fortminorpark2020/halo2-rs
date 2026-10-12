//! The camera behind the main menu: mainmenu.map's `mainmenu_flythrough`
//! script, which moves the camera from one of the map's camera points to
//! the next for as long as the menus are up.
//!
//! Each `camera_set` starts the camera towards a point over a number of
//! ticks, and the script's `sleep` lets the next one cut in halfway, so
//! the camera only comes to rest where the loop ends (where it began). A
//! move picks up the speed and turn the camera already has, and how fast
//! they are changing, so it flies smoothly through every cut.
//!
//! How Halo 2 moves it (decompilation, in our words): `camera_set`
//! (0x16c2f0) aims the observer at the point, and 0x16c840 asks it to get
//! there in the move's ticks. Each frame the observer solves, for its
//! position and its turn, the fifth-order curve (0x16ff10) that starts
//! from where it is with its velocity and acceleration and arrives with
//! neither, and follows it (0x1701f0, 0x170630). The code that counts a
//! move's ticks down hasn't been found; one curve per move assumes it
//! does, as `camera_time` (the ticks left) and the script's last move
//! (300 ticks, then a 300-tick sleep before the loop starts again on the
//! same point at rest) suggest. Solving again each frame with the ticks
//! counting down gives the same curve. Halo 2 turns by a rotation vector
//! in the world's frame; this turns by one in the camera's frame at the
//! start of the move, which is close.

use crate::camera::halo_rotation;
use blam_cache::scenario::CameraPoint;
use blam_cache::script::{value_type, NodeKind, Scripts};
use glam::{DQuat, DVec3, Vec3};

/// Script ticks per second.
const TICKS: f64 = 30.0;
/// The script that flies the camera.
const SCRIPT: &str = "mainmenu_flythrough";
/// Horizontal field of view in degrees: `camera_set` keeps the cutscene
/// default (h2viewer's `campaign::cutscene`). Unverified: not yet checked
/// against Halo 2's own menu on screen.
pub const FOV: f32 = 70.0;

/// A camera point: where it is and the way it faces (forward is +x, up +z).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    at: DVec3,
    rot: DQuat,
}

impl Pose {
    /// A camera point, turned as Halo 2's `camera_set` turns it
    /// (`camera::halo_rotation`).
    fn of(c: &CameraPoint) -> Pose {
        Pose {
            at: DVec3::from(c.position.map(f64::from)),
            rot: halo_rotation(c.orientation.map(f64::from)),
        }
    }
}

/// The camera on its way: its pose, velocity (units per tick),
/// acceleration (units per tick squared), spin (a rotation vector per
/// tick, in its own frame) and how fast the spin changes (per tick).
#[derive(Clone, Copy, Debug)]
struct Motion {
    pose: Pose,
    vel: DVec3,
    acc: DVec3,
    spin: DVec3,
    spin_rate: DVec3,
}

impl Motion {
    fn at_rest(pose: Pose) -> Motion {
        Motion {
            pose,
            vel: DVec3::ZERO,
            acc: DVec3::ZERO,
            spin: DVec3::ZERO,
            spin_rate: DVec3::ZERO,
        }
    }
}

/// A `camera_set` and the ticks until the script's next one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Move {
    /// The camera point it goes to.
    to: usize,
    /// Ticks the move takes.
    ticks: f64,
    /// Ticks before the next move (or the loop) starts.
    wait: f64,
}

/// A literal number in a script.
fn number(scripts: &Scripts, i: u16) -> Option<f64> {
    let e = scripts
        .expression(i)
        .filter(|e| e.kind == NodeKind::Value)?;
    match e.value_type {
        value_type::SHORT => Some(e.as_i16() as f64),
        value_type::LONG => Some(e.value as i32 as f64),
        value_type::REAL => Some(e.as_f32() as f64),
        _ => None,
    }
}

/// The most expressions `statements` looks at, and the deepest `begin`
/// it goes into. mainmenu.map's script has a few dozen statements; the
/// limits only stop a damaged script (whose `begin`s lead back into
/// themselves, or fan out) from running on or overflowing the stack,
/// which would end the process rather than panic.
const MAX_EXPRESSIONS: u32 = 4096;
const MAX_DEPTH: u32 = 64;

/// The script's statements in order, inside its `begin`s, looking at no
/// more than `budget` more expressions.
fn statements(scripts: &Scripts, i: u16, depth: u32, budget: &mut u32, out: &mut Vec<u16>) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    if depth < MAX_DEPTH && scripts.function_name(i) == "begin" {
        for a in scripts.arguments(i) {
            if *budget == 0 {
                break;
            }
            statements(scripts, a, depth + 1, budget, out);
        }
    } else {
        out.push(i);
    }
}

/// The script's moves: each `(camera_set <point> <ticks>)` and the
/// `(sleep <ticks>)` after it.
fn moves(scripts: &Scripts, root: u16) -> Vec<Move> {
    let (mut list, mut budget) = (Vec::new(), MAX_EXPRESSIONS);
    statements(scripts, root, 0, &mut budget, &mut list);
    let mut out: Vec<Move> = Vec::new();
    for s in list {
        let args = scripts.arguments(s);
        let arg = |k: usize| args.get(k).copied();
        match scripts.function_name(s) {
            "camera_set" => {
                let to = arg(0).and_then(|a| scripts.expression(a));
                let ticks = arg(1).and_then(|a| number(scripts, a));
                if let (Some(to), Some(ticks)) = (to, ticks) {
                    out.push(Move {
                        to: to.as_i16().max(0) as usize,
                        ticks: ticks.max(0.0),
                        wait: 0.0,
                    });
                }
            }
            "sleep" => {
                let ticks = arg(0).and_then(|a| number(scripts, a)).unwrap_or(0.0);
                if let Some(m) = out.last_mut() {
                    m.wait += ticks.max(0.0);
                }
            }
            _ => {}
        }
    }
    out
}

/// Halo 2's camera curve over a move (0x16ff10) at `u` of the way: the
/// weights of the start, its velocity (times the move's ticks), its
/// acceleration (times the ticks squared) and the end; then their first
/// and second slopes. It leaves with the camera's velocity and
/// acceleration and arrives at rest: of the curves that do, the one with
/// the least jerk.
fn quintic(u: f64) -> [[f64; 4]; 3] {
    let (u2, u3, u4, u5) = (u * u, u * u * u, u * u * u * u, u * u * u * u * u);
    [
        [
            1.0 - 10.0 * u3 + 15.0 * u4 - 6.0 * u5,
            u - 6.0 * u3 + 8.0 * u4 - 3.0 * u5,
            0.5 * u2 - 1.5 * u3 + 1.5 * u4 - 0.5 * u5,
            10.0 * u3 - 15.0 * u4 + 6.0 * u5,
        ],
        [
            -30.0 * u2 + 60.0 * u3 - 30.0 * u4,
            1.0 - 18.0 * u2 + 32.0 * u3 - 15.0 * u4,
            u - 4.5 * u2 + 6.0 * u3 - 2.5 * u4,
            30.0 * u2 - 60.0 * u3 + 30.0 * u4,
        ],
        [
            -60.0 * u + 180.0 * u2 - 120.0 * u3,
            -36.0 * u + 96.0 * u2 - 60.0 * u3,
            1.0 - 9.0 * u + 18.0 * u2 - 10.0 * u3,
            60.0 * u - 180.0 * u2 + 120.0 * u3,
        ],
    ]
}

/// A rotation as a rotation vector, the short way round.
fn log(q: DQuat) -> DVec3 {
    let q = if q.w < 0.0 { -q } else { q };
    q.to_scaled_axis()
}

/// Turning from `from` by `delta` (in its own frame) over `len` ticks,
/// `u` of the way: starting with its spin and the spin's rate of change,
/// ending still.
fn turned(from: &Motion, delta: DVec3, len: f64, u: f64) -> DQuat {
    let [[_, w_spin, w_rate, w_end], _, _] = quintic(u);
    let turn = from.spin * (w_spin * len) + from.spin_rate * (w_rate * len * len) + delta * w_end;
    from.pose.rot * DQuat::from_scaled_axis(turn)
}

/// The camera `s` ticks into a move from `from` to `to` that takes `len`
/// ticks: Halo 2's curve from the camera's velocity and acceleration (and
/// its spin and the spin's rate of change) to rest on the point.
fn glide(from: &Motion, to: Pose, len: f64, s: f64) -> Motion {
    if len <= 0.0 || s >= len {
        return Motion::at_rest(to);
    }
    let u = (s / len).max(0.0);
    let curve = |w: [f64; 4]| {
        from.pose.at * w[0] + from.vel * (w[1] * len) + from.acc * (w[2] * len * len) + to.at * w[3]
    };
    let [at, slope, bend] = quintic(u);
    let delta = log(from.pose.rot.inverse() * to.rot);
    let rot = turned(from, delta, len, u);
    // Its spin and the spin's rate of change, in its own frame: how it
    // turns over a moment before and a moment after.
    let h = 1e-2;
    let before = turned(from, delta, len, u - h / len);
    let after = turned(from, delta, len, u + h / len);
    let (early, late) = (
        log(before.inverse() * rot) / h,
        log(rot.inverse() * after) / h,
    );
    Motion {
        pose: Pose { at: curve(at), rot },
        vel: curve(slope) / len,
        acc: curve(bend) / (len * len),
        spin: (early + late) * 0.5,
        spin_rate: (late - early) / h,
    }
}

/// The main menu's camera path, read from its script and camera points.
#[derive(Clone, Debug)]
pub struct Flythrough {
    points: Vec<Pose>,
    moves: Vec<Move>,
    /// Ticks in one loop of the script.
    length: f64,
}

impl Flythrough {
    /// The map's flythrough, if it has the script and its camera points.
    pub fn new(scripts: &Scripts, points: &[CameraPoint]) -> Option<Flythrough> {
        let script = scripts.scripts.iter().find(|s| s.name == SCRIPT)?;
        let moves = moves(scripts, script.root?);
        Flythrough::from_moves(points.iter().map(Pose::of).collect(), moves)
    }

    fn from_moves(points: Vec<Pose>, mut moves: Vec<Move>) -> Option<Flythrough> {
        moves.retain(|m| m.to < points.len());
        let length: f64 = moves.iter().map(|m| m.wait).sum();
        (length > 0.0).then_some(Flythrough {
            points,
            moves,
            length,
        })
    }

    /// The camera `tick` ticks into the script (looping): each move from
    /// the start of the loop in turn, from where the last one had got to.
    fn motion(&self, tick: f64) -> Motion {
        let t = tick.rem_euclid(self.length);
        let mut m = Motion::at_rest(self.points[self.moves[0].to]);
        let mut clock = 0.0;
        for mv in &self.moves {
            let to = self.points[mv.to];
            if t < clock + mv.wait {
                return glide(&m, to, mv.ticks, t - clock);
            }
            m = glide(&m, to, mv.ticks, mv.wait);
            clock += mv.wait;
        }
        m
    }

    /// Where the camera is `seconds` into the menus: its position, the way
    /// it faces, and its up (it banks).
    pub fn camera(&self, seconds: f32) -> (Vec3, Vec3, Vec3) {
        let m = self.motion(seconds as f64 * TICKS);
        let rot = m.pose.rot;
        (
            m.pose.at.as_vec3(),
            (rot * DVec3::X).as_vec3(),
            (rot * DVec3::Z).as_vec3(),
        )
    }

    /// Seconds in one loop.
    pub fn seconds(&self) -> f64 {
        self.length / TICKS
    }

    /// Script ticks in one loop.
    pub fn ticks(&self) -> f64 {
        self.length
    }

    /// The map's camera points (the script may use only some).
    pub fn points(&self) -> usize {
        self.points.len()
    }

    /// The camera points the script flies to.
    pub fn points_used(&self) -> usize {
        let mut used: Vec<usize> = self.moves.iter().map(|m| m.to).collect();
        used.sort_unstable();
        used.dedup();
        used.len()
    }

    /// The `camera_set`s in one loop.
    pub fn moves(&self) -> usize {
        self.moves.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_cache::script::{Expression, Script, ScriptKind};

    /// Builds a script's expression tree.
    #[derive(Default)]
    struct Builder(Scripts);

    impl Builder {
        fn node(&mut self, kind: NodeKind, value_type: u16, text: &str, value: u32) -> u16 {
            let at = self.0.strings.len() as u32;
            self.0.strings.extend(text.bytes());
            self.0.strings.push(0);
            self.0.expressions.push(Expression {
                opcode: 0,
                value_type,
                kind,
                next: None,
                text: at,
                value,
            });
            (self.0.expressions.len() - 1) as u16
        }

        fn short(&mut self, v: i16) -> u16 {
            self.node(NodeKind::Value, value_type::SHORT, "", v as u16 as u32)
        }

        fn call(&mut self, name: &str, args: &[u16]) -> u16 {
            let f = self.node(NodeKind::Value, value_type::VOID, name, 0);
            let mut prev = f;
            for &a in args {
                self.0.expressions[prev as usize].next = Some(a);
                prev = a;
            }
            self.node(NodeKind::Call, value_type::VOID, "", f as u32)
        }
    }

    /// Four camera points round a square, each turned and banked its own
    /// way.
    fn points() -> Vec<Pose> {
        let p = |x, y, z, yaw: f32, pitch: f32, roll: f32| {
            Pose::of(&CameraPoint {
                name: String::new(),
                position: [x, y, z],
                orientation: [yaw, pitch, roll].map(f32::to_radians),
            })
        };
        vec![
            p(0.0, 0.0, 200.0, 105.0, -1.0, 5.0),
            p(-8.0, 26.0, 207.0, 110.0, 18.0, -41.0),
            p(-34.0, 57.0, 210.0, 112.0, -13.0, 12.0),
            p(-50.0, 70.0, 245.0, 81.0, -6.0, -34.0),
        ]
    }

    /// Like mainmenu.map's: to the first point at once, then on round,
    /// each move cut off halfway, and back to rest where it began.
    fn path() -> Vec<Move> {
        let m = |to, ticks, wait| Move { to, ticks, wait };
        vec![
            m(0, 0.0, 90.0),
            m(1, 500.0, 250.0),
            m(2, 500.0, 250.0),
            m(3, 400.0, 200.0),
            m(0, 300.0, 300.0),
        ]
    }

    #[test]
    fn reads_the_script() {
        let mut b = Builder::default();
        let (p1, t1) = (b.short(0), b.short(0));
        let first = b.call("camera_set", &[p1, t1]);
        let nap = b.short(90);
        let sleep1 = b.call("sleep", &[nap]);
        let control = b.node(NodeKind::Value, value_type::BOOLEAN, "true", 1);
        let on = b.call("camera_control", &[control]);
        let (p2, t2) = (b.short(1), b.short(500));
        let second = b.call("camera_set", &[p2, t2]);
        let nap = b.short(250);
        let sleep2 = b.call("sleep", &[nap]);
        let root = b.call("begin", &[on, first, sleep1, second, sleep2]);
        let mut scripts = b.0;
        scripts.scripts.push(Script {
            name: SCRIPT.into(),
            kind: ScriptKind::Continuous,
            return_type: value_type::VOID,
            root: Some(root),
        });
        let found = moves(&scripts, root);
        let m = |to, ticks, wait| Move { to, ticks, wait };
        assert_eq!(found, [m(0, 0.0, 90.0), m(1, 500.0, 250.0)]);
        let cams: Vec<CameraPoint> = (0..2)
            .map(|k| CameraPoint {
                name: format!("ui_path_0{}", k + 1),
                position: [k as f32, 0.0, 0.0],
                orientation: [0.0; 3],
            })
            .collect();
        let fly = Flythrough::new(&scripts, &cams).unwrap();
        assert!((fly.seconds() - 340.0 / TICKS).abs() < 1e-9);
        assert_eq!((fly.points(), fly.points_used(), fly.moves()), (2, 2, 2));
    }

    #[test]
    fn a_damaged_script_cannot_run_on() {
        // A `begin` whose name's next argument is the call itself, and
        // whose own next is itself again: it reads as a `begin` of 4,097
        // copies of itself, without end.
        let mut b = Builder::default();
        let name = b.node(NodeKind::Value, value_type::VOID, "begin", 0);
        let looped = b.node(NodeKind::Call, value_type::VOID, "", name as u32);
        b.0.expressions[name as usize].next = Some(looped);
        b.0.expressions[looped as usize].next = Some(looped);
        assert!(moves(&b.0, looped).is_empty());
        // Empty `begin`s fanned out (each level a `begin` of 4,097 of the
        // level below): 4,097 to the fifth power calls without the budget.
        let mut b = Builder::default();
        let name = b.node(NodeKind::Value, value_type::VOID, "begin", 0);
        let mut level = b.node(NodeKind::Call, value_type::VOID, "", name as u32);
        for _ in 0..5 {
            let name = b.node(NodeKind::Value, value_type::VOID, "begin", 0);
            b.0.expressions[name as usize].next = Some(level);
            b.0.expressions[level as usize].next = Some(level);
            level = b.node(NodeKind::Call, value_type::VOID, "", name as u32);
        }
        assert!(moves(&b.0, level).is_empty());
        // A move in one `begin` is read; in `begin`s nested 1,000 deep it
        // is past the depth followed (a damaged script nested deep enough
        // would overflow the stack), so nothing is.
        let mut b = Builder::default();
        let (p, t) = (b.short(0), b.short(0));
        let mut inner = b.call("camera_set", &[p, t]);
        let shallow = b.call("begin", &[inner]);
        assert_eq!(moves(&b.0, shallow).len(), 1);
        for _ in 0..1000 {
            inner = b.call("begin", &[inner]);
        }
        assert!(moves(&b.0, inner).is_empty());
    }

    #[test]
    fn moves_to_missing_points_are_dropped() {
        let m = |to, ticks, wait| Move { to, ticks, wait };
        // Only a move to a point the map doesn't have: nothing to fly.
        assert!(Flythrough::from_moves(points(), vec![m(7, 100.0, 50.0)]).is_none());
        // Among good ones, it's left out and the rest fly.
        let fly = Flythrough::from_moves(
            points(),
            vec![m(0, 0.0, 90.0), m(9, 100.0, 50.0), m(1, 500.0, 250.0)],
        )
        .unwrap();
        assert_eq!(fly.moves(), 2);
        for seconds in [0.0, 3.5, 7.0, 20.0, 1000.0] {
            let (at, forward, _) = fly.camera(seconds);
            assert!(at.is_finite() && forward.is_finite());
        }
        // Moves that never wait loop over nothing.
        assert!(Flythrough::from_moves(points(), vec![m(0, 100.0, 0.0)]).is_none());
        assert!(Flythrough::from_moves(points(), Vec::new()).is_none());
        // No flythrough script, or one with nothing in it.
        let cams = [CameraPoint {
            name: String::new(),
            position: [0.0; 3],
            orientation: [0.0; 3],
        }];
        assert!(Flythrough::new(&Scripts::default(), &cams).is_none());
        let mut scripts = Scripts::default();
        scripts.scripts.push(Script {
            name: SCRIPT.into(),
            kind: ScriptKind::Continuous,
            return_type: value_type::VOID,
            root: None,
        });
        assert!(Flythrough::new(&scripts, &cams).is_none());
    }

    #[test]
    fn loops_back_to_where_it_began_at_rest() {
        let fly = Flythrough::from_moves(points(), path()).unwrap();
        assert_eq!(fly.length, 1090.0);
        // Five moves over four points, the last back to the first.
        assert_eq!((fly.points_used(), fly.moves()), (4, 5));
        let start = fly.motion(0.0);
        let end = fly.motion(fly.length - 1e-6);
        assert!(start.pose.at.distance(end.pose.at) < 1e-6);
        assert!(start.pose.rot.angle_between(end.pose.rot) < 1e-6);
        assert!(end.vel.length() < 1e-6 && end.spin.length() < 1e-6);
        assert!(end.acc.length() < 1e-6 && end.spin_rate.length() < 1e-6);
        // It rests on the first point before it sets off.
        assert!(fly.motion(45.0).pose.at.distance(start.pose.at) < 1e-9);
    }

    #[test]
    fn flies_smoothly_through_each_cut() {
        let fly = Flythrough::from_moves(points(), path()).unwrap();
        // Sixty frames a second, round the loop and on into the next.
        let dt = 1.0 / 60.0;
        let frames = ((fly.seconds() + 5.0) / dt) as usize;
        let poses: Vec<Pose> = (0..=frames)
            .map(|k| fly.motion(k as f64 * dt * TICKS).pose)
            .collect();
        // Velocity and spin per second from frame to frame, the turn rate,
        // and how the velocity and spin change (per second squared).
        let vel: Vec<DVec3> = poses.windows(2).map(|p| (p[1].at - p[0].at) / dt).collect();
        let spin: Vec<DVec3> = poses
            .windows(2)
            .map(|p| log(p[0].rot.inverse() * p[1].rot) / dt)
            .collect();
        let turn: Vec<f64> = spin.iter().map(|w| w.length()).collect();
        let acc: Vec<DVec3> = vel.windows(2).map(|v| (v[1] - v[0]) / dt).collect();
        let spin_rate: Vec<DVec3> = spin.windows(2).map(|w| (w[1] - w[0]) / dt).collect();
        let most = |v: &[DVec3]| v.iter().map(|x| x.length()).fold(0.0, f64::max);
        let (fastest, sharpest) = (most(&vel), turn.iter().copied().fold(0.0, f64::max));
        let (hardest, sharpest_change) = (most(&acc), most(&spin_rate));
        let mut fastest_yet = 0.0f64;
        for k in 1..vel.len() {
            // No sudden change of speed or of turn rate anywhere: a move
            // restarting from rest would jump by the whole speed, where
            // braking for the last point changes it by about 2% of its top
            // speed from one frame to the next.
            fastest_yet = fastest_yet.max(vel[k - 1].length());
            let change = (vel[k] - vel[k - 1]).length();
            assert!(
                change < 0.05 * fastest_yet.max(1.0),
                "speed jumps at frame {k}"
            );
            assert!(
                (turn[k] - turn[k - 1]).abs() < 0.01,
                "turn jumps at frame {k}"
            );
        }
        for k in 1..acc.len() {
            // Nor of acceleration, in moving or turning: Halo 2's curve
            // starts each move with the acceleration the last one had and
            // changes it by under 2% of the strongest a frame. A curve that
            // starts it afresh (the cubic used before) jumps at the cuts by
            // 5 to 48% (moving) and 16 to 54% (turning).
            let jump = (acc[k] - acc[k - 1]).length() / hardest;
            assert!(jump < 0.03, "acceleration jumps {jump} at frame {k}");
            let jump = (spin_rate[k] - spin_rate[k - 1]).length() / sharpest_change;
            assert!(
                jump < 0.03,
                "turning acceleration jumps {jump} at frame {k}"
            );
        }
        // It does move, and turn.
        assert!(fastest > 1.0 && sharpest > 0.05, "{fastest} {sharpest}");
    }

    #[test]
    fn the_menu_view_looks_where_the_camera_faces() {
        let fly = Flythrough::from_moves(points(), path()).unwrap();
        let (at, view_proj) = crate::camera::menu_view_proj(&fly, 12.0, 16.0 / 9.0);
        let (eye, forward, up) = fly.camera(12.0);
        assert_eq!(at, eye);
        // Straight ahead is the middle of the view; the camera's up is up.
        let ndc = |p: Vec3| {
            let c = view_proj * p.extend(1.0);
            c.truncate() / c.w
        };
        let ahead = ndc(at + forward * 50.0);
        assert!(ahead.x.abs() < 1e-3 && ahead.y.abs() < 1e-3, "{ahead}");
        assert!(ahead.z > 0.0 && ahead.z < 1.0);
        let above = ndc(at + forward * 50.0 + up * 5.0);
        assert!(above.x.abs() < 1e-3 && above.y > 0.05, "{above}");
    }

    #[test]
    fn a_level_point_stays_level() {
        // Turned as Halo 2 turns it, the menu's seventh path point (its
        // angles rounded) is a level camera looking 46 degrees down at the
        // city. Rolling about the camera's own forward instead would bank
        // it 45 degrees and point it 10 degrees up.
        let point = CameraPoint {
            name: String::new(),
            position: [0.0, 0.0, 200.0],
            orientation: [-100.2f32, 10.4, 45.2].map(f32::to_radians),
        };
        let fly = Flythrough::from_moves(
            vec![Pose::of(&point)],
            vec![Move {
                to: 0,
                ticks: 0.0,
                wait: 90.0,
            }],
        )
        .unwrap();
        let (_, forward, up) = fly.camera(1.0);
        assert!(forward.dot(up).abs() < 1e-5);
        let pitch = forward.z.asin().to_degrees();
        assert!((pitch + 46.1).abs() < 0.1, "{pitch}");
        let level = forward.cross(Vec3::Z).cross(forward).normalize();
        let bank = level.angle_between(up).to_degrees();
        assert!(bank < 0.1, "{bank}");
        // The way placed scenery turns, too.
        let placed = crate::load::placement_matrix([0.0; 3], point.orientation, 1.0);
        assert!(placed.transform_vector3(Vec3::X).distance(forward) < 1e-5);
        assert!(placed.transform_vector3(Vec3::Z).distance(up) < 1e-5);
    }
}
