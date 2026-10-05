//! For testing (H2_NET_PROBE=1): player one on a joined PC goes through a
//! fixed routine (stand, run, jump, crouch, strafe while turning) over and
//! over, and the game prints how long each start from standing took to
//! show on screen, and where the view was every frame.

use crate::camera::FlyCamera;
use glam::{Vec2, Vec3};
use h2sim::Command;
use std::time::Instant;

/// Seconds the routine takes.
const ROUTINE: f32 = 4.0;
/// How fast the view turns while strafing, radians a second.
const TURN: f32 = 2.0;
/// Farther than this from where a start began, it shows (a run moves 0.002
/// in its first tick).
const MOVED: f32 = 0.001;
/// Frames standing still before a start that's timed.
const STILL: u32 = 10;
/// A start that hasn't shown after this long never will (into a wall).
const GIVE_UP: f32 = 1.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Stand,
    Run,
    Jump,
    Crouch,
    Strafe,
}

pub struct NetProbe {
    /// Seconds into the routine, and since the probe began.
    time: f32,
    clock: Instant,
    /// The part of the routine the controls last came from.
    last: Part,
    /// When the latest start from standing went, and where from, until it
    /// shows; with the frames since.
    start: Option<(Instant, Vec3)>,
    frames: u32,
    /// Where player one was shown last frame, and for how many frames
    /// they've stood still (a start is timed only from standing still).
    shown: Vec3,
    still: u32,
}

impl NetProbe {
    pub fn from_env() -> Option<NetProbe> {
        std::env::var("H2_NET_PROBE").ok().map(|_| NetProbe {
            time: 0.0,
            clock: Instant::now(),
            last: Part::Stand,
            start: None,
            frames: 0,
            shown: Vec3::ZERO,
            still: 0,
        })
    }

    fn part(&self) -> Part {
        match self.time {
            t if t < 1.0 => Part::Stand,
            t if t < 2.0 => Part::Run,
            t if t < 2.6 => Part::Jump,
            t if t < 3.2 => Part::Crouch,
            _ => Part::Strafe,
        }
    }

    /// Move the routine on by a frame, turning the view in its last part.
    pub fn frame(&mut self, dt: f32, camera: &mut FlyCamera) {
        self.time = (self.time + dt) % ROUTINE;
        camera.pitch = 0.0;
        if self.part() == Part::Strafe {
            camera.yaw += TURN * dt;
        }
    }

    /// Player one's controls at this point in the routine (`at`: where
    /// they're shown now).
    pub fn steer(&mut self, cmd: &mut Command, at: Vec3) {
        let part = self.part();
        if part == Part::Run && self.last == Part::Stand {
            if self.still >= STILL {
                self.start = Some((Instant::now(), at));
                self.frames = 0;
            } else {
                println!("probe: not standing still at the start");
            }
        }
        self.last = part;
        let forward = Vec2::new(0.0, 1.0);
        let (movement, jump, crouch) = match part {
            Part::Stand => (Vec2::ZERO, false, false),
            Part::Run => (forward, false, false),
            Part::Jump => (forward, self.time < 2.1, false),
            Part::Crouch => (forward, false, true),
            Part::Strafe => (Vec2::new(1.0, 0.0), false, false),
        };
        *cmd = Command {
            movement,
            jump,
            crouch,
            yaw: cmd.yaw,
            pitch: cmd.pitch,
            ..Command::default()
        };
    }

    /// After a frame: whether the latest start shows yet (`shown`: where
    /// player one is shown), and where the view is.
    pub fn watch(&mut self, shown: Vec3, view: Vec3) {
        let t = self.clock.elapsed().as_secs_f32();
        println!(
            "probe: {t:.4} {:?} {:.4} {:.4} {:.4} view {:.4} {:.4} {:.4}",
            self.part(),
            shown.x,
            shown.y,
            shown.z,
            view.x,
            view.y,
            view.z
        );
        self.still = match shown.distance(self.shown) < MOVED / 10.0 {
            true => self.still + 1,
            false => 0,
        };
        self.shown = shown;
        let Some((sent, from)) = self.start else {
            return;
        };
        self.frames += 1;
        let waited = sent.elapsed().as_secs_f32();
        if shown.distance(from) > MOVED {
            println!(
                "probe: moving {:.0} ms ({} frames) after the controls",
                waited * 1000.0,
                self.frames
            );
            self.start = None;
        } else if waited > GIVE_UP {
            println!("probe: not moving after {GIVE_UP} s (a wall?)");
            self.start = None;
        }
    }
}
