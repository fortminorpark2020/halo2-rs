//! What a player feels through their view: the camera kicked when they fire
//! or are hit, shaken by blasts nearby, and the screen flashed when they
//! take damage. The sizes, lengths and colours are the `jpt!` tags' (see
//! `blam_cache::weapon::DamageFeedback`). It moves only the picture, never
//! where the player aims.

use blam_cache::weapon::{CameraImpulse, CameraShake, DamageFeedback, ScreenFlash};
use glam::Vec3;

/// The shake and jitter pick new random offsets this many times a second.
/// The tags give how far the view shakes, not how fast: an estimate.
const SHAKE_RATE: f32 = 30.0;

/// A kick under way.
struct Kick {
    impulse: CameraImpulse,
    /// Which way the view turns (yaw, pitch; together unit length or zero),
    /// and the world direction it is pushed.
    turn: [f32; 2],
    push: Vec3,
    /// How strongly (1 at the source of a blast, less further off).
    scale: f32,
    /// World units of jitter, picked in the tag's range.
    jitter: f32,
    age: f32,
}

struct Shake {
    shake: CameraShake,
    scale: f32,
    age: f32,
}

struct Flash {
    flash: ScreenFlash,
    age: f32,
}

/// How far the picture is moved this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ViewOffset {
    /// Radians (yaw to the left, pitch up).
    pub yaw: f32,
    pub pitch: f32,
    /// World units.
    pub position: Vec3,
}

/// One player's camera kicks, shakes and screen flashes.
pub struct ViewFeedback {
    kicks: Vec<Kick>,
    shakes: Vec<Shake>,
    flashes: Vec<Flash>,
    rng: u32,
    /// Seconds until the random offsets change, and the offsets: a unit
    /// direction for the jitter and shake, and a turn (yaw, pitch).
    next_noise: f32,
    noise: (Vec3, [f32; 2]),
}

impl Default for ViewFeedback {
    fn default() -> ViewFeedback {
        ViewFeedback {
            kicks: Vec::new(),
            shakes: Vec::new(),
            flashes: Vec::new(),
            rng: 0x2545_f491,
            next_noise: 0.0,
            noise: (Vec3::ZERO, [0.0; 2]),
        }
    }
}

impl ViewFeedback {
    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn signed(&mut self) -> f32 {
        self.random() * 2.0 - 1.0
    }

    /// The kick of firing: the view tips up and is pushed back along
    /// `forward`, as the firing effect's `jpt!` says.
    pub fn fired(&mut self, feedback: &DamageFeedback, forward: Vec3) {
        if let Some(impulse) = feedback.impulse {
            self.kick(impulse, [0.0, 1.0], -forward, 1.0);
        }
    }

    /// Hit by damage travelling along `direction` (world, unit length),
    /// `scale` as strongly as the tag says (less further from a blast),
    /// seen along `forward`/`right`/`up`. The view turns the way the damage
    /// travels (a hit from in front tips it up), is pushed along it, and
    /// the screen flashes the shielded or unshielded colour. The tags don't
    /// say which way the view turns: an estimate.
    pub fn hit(
        &mut self,
        feedback: &DamageFeedback,
        direction: Vec3,
        (forward, right, up): (Vec3, Vec3, Vec3),
        shielded: bool,
        scale: f32,
    ) {
        if let Some(impulse) = feedback.impulse {
            // Yaw grows to the left: damage pushing right turns it right.
            let turn = glam::vec2(
                -direction.dot(right),
                direction.dot(up) - direction.dot(forward),
            )
            .normalize_or_zero();
            self.kick(impulse, turn.into(), direction, scale);
        }
        if let Some(&flash) = feedback.flash(shielded) {
            self.flashes.push(Flash { flash, age: 0.0 });
        }
    }

    /// A blast `distance` away: its shake, weaker further off, out to the
    /// tag's shake radius. How it weakens isn't in the tags; it fades
    /// evenly to nothing at the radius here (an estimate).
    pub fn blast(&mut self, feedback: &DamageFeedback, distance: f32) {
        let scale = blast_scale(feedback.shake_radius, distance);
        if let (Some(shake), true) = (feedback.shake, scale > 0.0) {
            self.shakes.push(Shake {
                shake,
                scale,
                age: 0.0,
            });
        }
    }

    fn kick(&mut self, impulse: CameraImpulse, turn: [f32; 2], push: Vec3, scale: f32) {
        let (lo, hi) = impulse.jitter;
        let jitter = lo + (hi - lo) * self.random();
        self.kicks.push(Kick {
            impulse,
            turn,
            push,
            scale,
            jitter,
            age: 0.0,
        });
    }

    /// Nothing more to feel (the player died or respawned).
    pub fn clear(&mut self) {
        self.kicks.clear();
        self.shakes.clear();
        self.flashes.clear();
    }

    /// Age everything by `dt` seconds, dropping what has run its course.
    pub fn update(&mut self, dt: f32) {
        for k in &mut self.kicks {
            k.age += dt;
        }
        for s in &mut self.shakes {
            s.age += dt;
        }
        for f in &mut self.flashes {
            f.age += dt;
        }
        self.kicks.retain(|k| k.age < k.impulse.duration);
        self.shakes.retain(|s| s.age < s.shake.duration);
        self.flashes.retain(|f| f.age < f.flash.duration);
        self.next_noise -= dt;
        if self.next_noise <= 0.0 {
            self.next_noise += 1.0 / SHAKE_RATE;
            self.next_noise = self.next_noise.max(0.0);
            let dir = Vec3::new(self.signed(), self.signed(), self.signed());
            let turn = [self.signed(), self.signed()];
            self.noise = (dir.normalize_or_zero(), turn);
        }
    }

    /// How far the picture is moved now. Kicks overlapping (an SMG's hits,
    /// fifteen a second) add up, but never past the strongest of them at
    /// full strength: the tags give each kick's size, not how Halo 2
    /// combines them, and summed freely a stream of hits carried the view
    /// a metre or two. The cap is an estimate.
    pub fn offset(&self) -> ViewOffset {
        let mut out = ViewOffset::default();
        let (dir, [nyaw, npitch]) = self.noise;
        let (mut turn, mut moved) = (glam::Vec2::ZERO, Vec3::ZERO);
        let (mut most_turn, mut most_moved) = (0.0f32, 0.0f32);
        for k in &self.kicks {
            let i = &k.impulse;
            let left = i.fade.remaining(k.age / i.duration) * k.scale;
            turn += glam::Vec2::from(k.turn) * i.rotation * left;
            moved += (k.push * i.pushback + dir * k.jitter) * left;
            most_turn = most_turn.max(i.rotation * k.scale);
            most_moved = most_moved.max((i.pushback + k.jitter) * k.scale);
        }
        let turn = turn.clamp_length_max(most_turn);
        (out.yaw, out.pitch) = (turn.x, turn.y);
        out.position = moved.clamp_length_max(most_moved);
        for s in &self.shakes {
            let sh = &s.shake;
            let left = sh.falloff.remaining(s.age / sh.duration) * s.scale;
            out.yaw += nyaw * sh.rotation * left;
            out.pitch += npitch * sh.rotation * left;
            out.position += dir * sh.translation * left;
        }
        out
    }

    /// The screen flash now, as a colour to draw over the view (red,
    /// green, blue, opacity), if any. Each hit flashes the tag's colour at
    /// full strength, fading by its function; hits add up to the tag's
    /// maximum intensity. Here the flash adds its colour and dims the view
    /// by its alpha, as one see-through layer; how Halo 2 blends a
    /// "lighten" flash isn't in the tags, so that is an estimate. The tag's
    /// colours are gamma-encoded and the HUD blends in linear light, so
    /// the colour is converted (as `gpu.rs` does the tags' tints): drawn
    /// as stored, a shielded hit's light blue swamped the view.
    pub fn flash(&self) -> Option<[f32; 4]> {
        let mut sum = [0.0f32; 4];
        let mut total = 0.0;
        let mut most = 0.0f32;
        for f in &self.flashes {
            let i = f.flash.fade.remaining(f.age / f.flash.duration);
            for (s, c) in sum.iter_mut().zip(f.flash.color) {
                *s += c * i;
            }
            total += i;
            most = most.max(f.flash.max_intensity);
        }
        if total <= 1e-4 {
            return None;
        }
        let k = total.min(most) / total;
        let [r, g, b, a] = sum.map(|v| v * k);
        let [r, g, b] = [r, g, b].map(|v| v.powf(2.2));
        let alpha = a.max(r).max(g).max(b).min(1.0);
        (alpha > 1e-3).then(|| [r / alpha, g / alpha, b / alpha, alpha])
    }
}

/// Seconds shields flare after a hit. The shield shaders' own animation
/// would say, in data not read here: an estimate.
pub const SHIELD_FLARE_TIME: f32 = 0.5;

/// Every player's shields flaring from their latest hit.
#[derive(Default)]
pub struct ShieldFlares {
    /// Seconds since each player's shields last took a hit.
    since: Vec<f32>,
    /// Each player's shields before the latest damage.
    before: Vec<f32>,
}

impl ShieldFlares {
    /// Player `i`'s shields were up when the latest damage came (`now`,
    /// their shields after it, when that isn't known).
    pub fn were_up(&self, i: usize, now: f32) -> bool {
        self.before.get(i).map_or(now > 0.0, |&b| b > 0.0)
    }

    pub fn hit(&mut self, i: usize) {
        if self.since.len() <= i {
            self.since.resize(i + 1, f32::INFINITY);
        }
        self.since[i] = 0.0;
    }

    pub fn update(&mut self, dt: f32) {
        for s in &mut self.since {
            *s += dt;
        }
    }

    /// How brightly player `i`'s shields flare now, 0-1.
    pub fn flare(&self, i: usize) -> f32 {
        self.since
            .get(i)
            .map_or(0.0, |&s| (1.0 - s / SHIELD_FLARE_TIME).clamp(0.0, 1.0))
    }

    /// Everyone's shields after the latest damage, for the next.
    pub fn remember(&mut self, shields: impl Iterator<Item = f32>) {
        self.before.clear();
        self.before.extend(shields);
    }
}

/// How strongly a blast `distance` away shakes the view, out to `radius`.
fn blast_scale(radius: f32, distance: f32) -> f32 {
    match radius > 0.0 {
        true => (1.0 - distance / radius).clamp(0.0, 1.0),
        false => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_cache::weapon::{Fade, ResponseKind};

    /// The Battle Rifle's firing kick, from its `jpt!`.
    fn rifle_kick() -> DamageFeedback {
        DamageFeedback {
            impulse: Some(CameraImpulse {
                duration: 0.5,
                fade: Fade::VeryEarly,
                rotation: 0.5f32.to_radians(),
                pushback: 0.03,
                jitter: (0.0, 0.0),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_kick_tips_the_view_up_then_lets_go() {
        let mut v = ViewFeedback::default();
        v.fired(&rifle_kick(), Vec3::X);
        let o = v.offset();
        assert!((o.pitch - 0.5f32.to_radians()).abs() < 1e-6, "{o:?}");
        assert!((o.position - Vec3::new(-0.03, 0.0, 0.0)).length() < 1e-6);
        v.update(0.25);
        // Halfway, a very early fade has (1 - 0.5)^4 of it left.
        let half = v.offset().pitch / 0.5f32.to_radians();
        assert!((half - 0.0625).abs() < 1e-4, "{half}");
        v.update(0.3);
        assert_eq!(v.offset(), ViewOffset::default());
        assert!(v.kicks.is_empty());
    }

    #[test]
    fn hits_flash_their_colour_up_to_the_maximum() {
        let flash = |kind, color| ScreenFlash {
            kind,
            flash_type: 1,
            duration: 0.5,
            fade: Fade::Linear,
            max_intensity: 0.5,
            color,
        };
        let hit = DamageFeedback {
            flashes: vec![
                flash(ResponseKind::Shielded, [0.0, 0.5, 0.75, 0.25]),
                flash(ResponseKind::Unshielded, [0.25, 0.0, 0.0, 0.25]),
            ],
            ..Default::default()
        };
        let basis = (Vec3::X, -Vec3::Y, Vec3::Z);
        let mut v = ViewFeedback::default();
        assert_eq!(v.flash(), None);
        v.hit(&hit, -Vec3::X, basis, true, 1.0);
        v.hit(&hit, -Vec3::X, basis, true, 1.0);
        // Two hits at full strength are held to half: blue at 0.375, in
        // linear light, over the view dimmed by the tag's alpha (0.125).
        let [r, g, b, a] = v.flash().unwrap();
        assert!((a - 0.125).abs() < 1e-5 && r == 0.0, "{a}");
        let (lb, lg) = (0.375f32.powf(2.2), 0.25f32.powf(2.2));
        assert!((b * a - lb).abs() < 1e-5 && (g * a - lg).abs() < 1e-5);
        v.update(0.6);
        assert_eq!(v.flash(), None);
        // Shields down, the flash is (a dark) red.
        v.hit(&hit, -Vec3::X, basis, false, 1.0);
        let [r, g, b, a] = v.flash().unwrap();
        assert!(r > 0.0 && g == 0.0 && b == 0.0 && (a - 0.125).abs() < 1e-5);
    }

    #[test]
    fn a_stream_of_hits_moves_the_view_no_more_than_one() {
        let hit = DamageFeedback {
            impulse: Some(CameraImpulse {
                jitter: (0.01, 0.025),
                ..rifle_kick().impulse.unwrap()
            }),
            ..Default::default()
        };
        let mut v = ViewFeedback::default();
        let basis = (Vec3::X, -Vec3::Y, Vec3::Z);
        // Two SMGs' worth from in front: thirty hits a second for a second.
        for _ in 0..30 {
            v.hit(&hit, -Vec3::X, basis, true, 1.0);
            v.update(1.0 / 30.0);
            let o = v.offset();
            assert!(o.yaw.hypot(o.pitch) <= 0.5f32.to_radians() + 1e-6, "{o:?}");
            assert!(o.position.length() <= 0.03 + 0.025 + 1e-6, "{o:?}");
        }
        // Though it still moves.
        assert!(v.offset().pitch > 0.0);
    }

    #[test]
    fn a_hit_from_the_right_turns_the_view_left() {
        let hit = rifle_kick();
        let mut v = ViewFeedback::default();
        // Facing +x, right is -y: damage travelling +y comes from the right.
        v.hit(&hit, Vec3::Y, (Vec3::X, -Vec3::Y, Vec3::Z), true, 1.0);
        let o = v.offset();
        assert!(o.yaw > 0.0 && o.pitch.abs() < 1e-6, "{o:?}");
    }

    #[test]
    fn shields_flare_then_fade() {
        let mut f = ShieldFlares::default();
        assert_eq!(f.flare(3), 0.0);
        assert!(f.were_up(3, 1.0) && !f.were_up(3, 0.0));
        f.remember([70.0, 0.0].into_iter());
        assert!(f.were_up(0, 0.0) && !f.were_up(1, 70.0));
        f.hit(1);
        assert_eq!((f.flare(0), f.flare(1)), (0.0, 1.0));
        f.update(SHIELD_FLARE_TIME / 2.0);
        assert!((f.flare(1) - 0.5).abs() < 1e-6);
        f.update(SHIELD_FLARE_TIME);
        assert_eq!(f.flare(1), 0.0);
    }

    #[test]
    fn blasts_shake_less_further_off() {
        assert_eq!(blast_scale(12.0, 0.0), 1.0);
        assert_eq!(blast_scale(12.0, 6.0), 0.5);
        assert_eq!(blast_scale(12.0, 20.0), 0.0);
        assert_eq!(blast_scale(0.0, 1.0), 0.0);
        let rocket = DamageFeedback {
            shake: Some(CameraShake {
                duration: 1.25,
                falloff: Fade::Early,
                translation: 0.075,
                rotation: 0.0,
            }),
            shake_radius: 12.0,
            ..Default::default()
        };
        let mut v = ViewFeedback::default();
        v.blast(&rocket, 30.0);
        assert!(v.shakes.is_empty());
        v.blast(&rocket, 3.0);
        v.update(0.01);
        let moved = v.offset().position.length();
        assert!(moved > 0.0 && moved <= 0.075 * 0.75, "{moved}");
        v.update(1.3);
        assert_eq!(v.offset(), ViewOffset::default());
    }
}
