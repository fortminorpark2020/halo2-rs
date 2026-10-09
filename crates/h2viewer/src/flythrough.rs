//! The camera behind the main menu: mainmenu.map's `mainmenu_flythrough`
//! script, which moves the camera from one of the map's camera points to
//! the next for as long as the menus are up.
//!
//! Each `camera_set` starts the camera towards a point over a number of
//! ticks, and the script's `sleep` lets the next one cut in halfway, so
//! the camera only comes to rest where the loop ends (where it began). A
//! move picks up the speed and turn the camera already has, so it flies
//! smoothly through every cut.

use blam_cache::scenario::CameraPoint;
use blam_cache::script::{value_type, NodeKind, Scripts};
use glam::{DQuat, DVec3, Vec3};

/// Script ticks per second.
const TICKS: f64 = 30.0;
/// The script that flies the camera.
const SCRIPT: &str = "mainmenu_flythrough";
/// Horizontal field of view in degrees: `camera_set` keeps the cutscene
/// default (see `campaign::cutscene`).
pub const FOV: f32 = 70.0;

/// A camera point: where it is and the way it faces (forward is +x, up +z).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    at: DVec3,
    rot: DQuat,
}

impl Pose {
    /// Halo's camera points turn like placed objects: yaw about +z, then
    /// pitch up, then roll about the way they face.
    fn of(c: &CameraPoint) -> Pose {
        let [yaw, pitch, roll] = c.orientation.map(f64::from);
        Pose {
            at: DVec3::from(c.position.map(f64::from)),
            rot: DQuat::from_rotation_z(yaw)
                * DQuat::from_rotation_y(-pitch)
                * DQuat::from_rotation_x(roll),
        }
    }
}

/// The camera on its way: its pose, velocity (units per tick) and spin
/// (a rotation vector per tick, in its own frame).
#[derive(Clone, Copy, Debug)]
struct Motion {
    pose: Pose,
    vel: DVec3,
    spin: DVec3,
}

impl Motion {
    fn at_rest(pose: Pose) -> Motion {
        Motion {
            pose,
            vel: DVec3::ZERO,
            spin: DVec3::ZERO,
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

/// The script's statements in order, inside its `begin`s.
fn statements(scripts: &Scripts, i: u16, out: &mut Vec<u16>) {
    if scripts.function_name(i) == "begin" && out.len() < 4096 {
        for a in scripts.arguments(i) {
            statements(scripts, a, out);
        }
    } else {
        out.push(i);
    }
}

/// The script's moves: each `(camera_set <point> <ticks>)` and the
/// `(sleep <ticks>)` after it.
fn moves(scripts: &Scripts, root: u16) -> Vec<Move> {
    let mut list = Vec::new();
    statements(scripts, root, &mut list);
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

/// The Hermite basis at `u` (start, start tangent, end) and its slopes.
fn hermite(u: f64) -> ([f64; 3], [f64; 3]) {
    let (u2, u3) = (u * u, u * u * u);
    (
        [
            2.0 * u3 - 3.0 * u2 + 1.0,
            u3 - 2.0 * u2 + u,
            3.0 * u2 - 2.0 * u3,
        ],
        [
            6.0 * u2 - 6.0 * u,
            3.0 * u2 - 4.0 * u + 1.0,
            6.0 * u - 6.0 * u2,
        ],
    )
}

/// A rotation as a rotation vector, the short way round.
fn log(q: DQuat) -> DVec3 {
    let q = if q.w < 0.0 { -q } else { q };
    q.to_scaled_axis()
}

/// Turning from `from` by `delta` (in its own frame) over `len` ticks,
/// `u` of the way: starting with its spin, ending still.
fn turned(from: &Motion, delta: DVec3, len: f64, u: f64) -> DQuat {
    let ([_, h10, h01], _) = hermite(u);
    from.pose.rot * DQuat::from_scaled_axis(from.spin * (h10 * len) + delta * h01)
}

/// The camera `s` ticks into a move from `from` to `to` that takes `len`
/// ticks: a curve that starts at the camera's velocity and spin and ends
/// at rest on the point.
fn glide(from: &Motion, to: Pose, len: f64, s: f64) -> Motion {
    if len <= 0.0 || s >= len {
        return Motion::at_rest(to);
    }
    let u = (s / len).max(0.0);
    let ([h00, h10, h01], [d00, d10, d01]) = hermite(u);
    let (p0, m0) = (from.pose.at, from.vel * len);
    let at = p0 * h00 + m0 * h10 + to.at * h01;
    let vel = (p0 * d00 + m0 * d10 + to.at * d01) / len;
    let delta = log(from.pose.rot.inverse() * to.rot);
    let rot = turned(from, delta, len, u);
    // Its spin: how it turns over a moment, in its own frame.
    let e = 1e-3;
    let next = turned(from, delta, len, u + e / len);
    let spin = log(rot.inverse() * next) / e;
    Motion {
        pose: Pose { at, rot },
        vel,
        spin,
    }
}

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
    #[cfg(test)]
    fn seconds(&self) -> f64 {
        self.length / TICKS
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
    }

    #[test]
    fn loops_back_to_where_it_began_at_rest() {
        let fly = Flythrough::from_moves(points(), path()).unwrap();
        assert_eq!(fly.length, 1090.0);
        let start = fly.motion(0.0);
        let end = fly.motion(fly.length - 1e-6);
        assert!(start.pose.at.distance(end.pose.at) < 1e-6);
        assert!(start.pose.rot.angle_between(end.pose.rot) < 1e-6);
        assert!(end.vel.length() < 1e-6 && end.spin.length() < 1e-6);
        // It rests on the first point before it sets off.
        assert!(fly.motion(45.0).pose.at.distance(start.pose.at) < 1e-9);
    }

    #[test]
    fn flies_smoothly_through_each_cut() {
        let fly = Flythrough::from_moves(points(), path()).unwrap();
        // Sixty frames a second, round the loop and on into the next.
        let dt = 1.0 / 60.0;
        let frames = ((fly.seconds() + 5.0) / dt) as usize;
        let at = |k: usize| fly.motion(k as f64 * dt * TICKS);
        let (mut speed, mut turn) = (None::<DVec3>, None::<f64>);
        let (mut fastest, mut sharpest) = (0.0f64, 0.0f64);
        for k in 0..frames {
            let (a, b) = (at(k), at(k + 1));
            // Velocity and turn rate per second from frame to frame.
            let v = (b.pose.at - a.pose.at) / dt;
            let w = a.pose.rot.angle_between(b.pose.rot) / dt;
            if let (Some(s), Some(t)) = (speed, turn) {
                // No sudden change of speed or of turn rate anywhere: a
                // move restarting from rest would jump by the whole speed,
                // where braking for the last point changes it by about 2% of
                // its top speed from one frame to the next.
                let change = (v - s).length();
                assert!(change < 0.05 * fastest.max(1.0), "speed jumps at frame {k}");
                assert!((w - t).abs() < 0.01, "turn jumps at frame {k}");
            }
            fastest = fastest.max(v.length());
            sharpest = sharpest.max(w);
            (speed, turn) = (Some(v), Some(w));
        }
        // It does move, and turn.
        assert!(fastest > 1.0 && sharpest > 0.05, "{fastest} {sharpest}");
    }

    #[test]
    fn banks_with_the_points_roll() {
        let fly = Flythrough::from_moves(points(), path()).unwrap();
        // At rest on the first point (rolled 5 degrees): up leans off +z
        // and stays square to forward.
        let (_, forward, up) = fly.camera(1.0);
        assert!(forward.dot(up).abs() < 1e-5);
        let level = forward.cross(Vec3::Z).cross(forward).normalize();
        let bank = level.angle_between(up).to_degrees();
        assert!((bank - 5.0).abs() < 0.01, "{bank}");
    }
}
