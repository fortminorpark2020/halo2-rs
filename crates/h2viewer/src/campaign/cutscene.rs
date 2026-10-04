//! Cutscenes: the camera flying along its animation, the cast (Master
//! Chief, Johnson, Miranda...) and scenery (In Amber Clad) playing theirs,
//! all relative to an anchor, and the subtitles under them.

use super::{Ctx, Mission};
use crate::rig;
use crate::scene::{CinemaBody, Scene, Vertex};
use blam_cache::animation::{Animation, FRAME_RATE};
use glam::{Mat4, Vec3};
use h2sim::script::{Obj, Value, TICKS_PER_SECOND};
use std::collections::HashMap;

/// An animation playing: graph (by tag) and animation, when it started,
/// the anchor it plays relative to, and whether it loops.
#[derive(Debug, Clone, Copy)]
struct Playing {
    graph: u32,
    anim: usize,
    start: f32,
    anchor: Mat4,
    looping: bool,
}

#[derive(Debug, Default)]
pub(super) struct Cutscene {
    /// The scripts have the camera.
    camera_on: bool,
    shot: Option<Playing>,
    /// Horizontal field of view the scripts set, degrees.
    fov: Option<f32>,
    /// Named objects' animations.
    objects: HashMap<u16, Playing>,
    /// A subtitle (by string id) and when it goes.
    subtitle: Option<(u32, f32)>,
    /// In a cutscene the player can skip, and skipping it.
    skippable: bool,
    skipping: bool,
}

impl Cutscene {
    /// The player skipped the cutscene that played last.
    pub(super) fn skipped(&self) -> bool {
        self.skipping
    }

    /// Skip the cutscene playing, if it can be.
    pub(super) fn skip(&mut self) -> bool {
        if self.skippable {
            self.skipping = true;
        }
        self.skippable
    }

    /// A skipped cutscene is still running on.
    pub(super) fn fast_forward(&self) -> bool {
        self.skippable && self.skipping
    }
}

impl Playing {
    fn animation<'a>(&self, scene: &'a Scene) -> Option<&'a Animation> {
        scene
            .ai
            .cinema
            .graphs
            .get(&self.graph)?
            .animations
            .get(self.anim)
    }

    /// The frame it's at, at mission time `now`.
    fn frame(&self, a: &Animation, now: f32) -> f32 {
        let last = a.frame_count.saturating_sub(1).max(1) as f32;
        let frame = (now - self.start).max(0.0) * FRAME_RATE;
        if self.looping {
            frame % last
        } else {
            frame.min(last)
        }
    }

    /// Seconds left to play (none for a loop).
    fn left(&self, scene: &Scene, now: f32) -> f32 {
        match self.animation(scene) {
            Some(a) if !self.looping => (a.duration() - (now - self.start)).max(0.0),
            _ => 0.0,
        }
    }

    /// The root node's place, relative to the anchor.
    fn root(&self, scene: &Scene, now: f32) -> Option<Mat4> {
        let a = self.animation(scene)?;
        let f = self.frame(a, now);
        let t = a.translations.first()?.as_ref().map(|t| t.sample(f));
        let r = a
            .rotations
            .first()?
            .as_ref()
            .map(|r| rig::tag_quat(r.sample(f)));
        let local = Mat4::from_rotation_translation(
            r.unwrap_or_default(),
            Vec3::from(t.unwrap_or_default()),
        );
        Some(self.anchor * local)
    }
}

impl Ctx<'_> {
    /// An animation a script names: graph tag and the animation's index.
    fn animation(&self, graph: &Value, name: &Value) -> Option<(u32, usize)> {
        let tag = graph.handle()?;
        let g = self.scene.ai.cinema.graphs.get(&tag)?;
        let name = self.scene.ai.string_ids.get(&name.handle()?)?;
        Some((tag, g.animations.iter().position(|a| &a.name == name)?))
    }

    /// Where a named object is placed.
    fn placed(&self, v: &Value) -> Mat4 {
        match v.objects().first() {
            Some(&Obj::Name(n)) => self
                .scene
                .ai
                .cinema
                .placed
                .get(n as usize)
                .copied()
                .flatten()
                .unwrap_or(Mat4::IDENTITY),
            _ => Mat4::IDENTITY,
        }
    }

    fn play(&mut self, object: &Value, graph: &Value, name: &Value, anchor: Mat4, looping: bool) {
        let Some((graph, anim)) = self.animation(graph, name) else {
            return;
        };
        let playing = Playing {
            graph,
            anim,
            start: self.st.time,
            anchor,
            looping,
        };
        for &o in object.objects() {
            if let Obj::Name(n) = o {
                self.st.cutscene.objects.insert(n, playing);
            }
        }
    }

    /// The scripts' cutscene camera, animation and subtitle functions;
    /// `None` for any other function.
    pub(super) fn cutscene_call(&mut self, function: &str, args: &[Value]) -> Option<Value> {
        let arg = |k: usize| args.get(k).cloned().unwrap_or_default();
        let ticks = |seconds: f32| Value::Real((seconds * TICKS_PER_SECOND as f32).round());
        let now = self.st.time;
        Some(match function {
            "camera_control" => {
                self.st.cutscene.camera_on = arg(0).truthy();
                if !self.st.cutscene.camera_on {
                    self.st.cutscene.shot = None;
                    self.st.cutscene.fov = None;
                }
                Value::Void
            }
            "camera_set_animation" | "camera_set_animation_relative" => {
                let anchor = if function.ends_with("relative") {
                    let flag = arg(3)
                        .index()
                        .and_then(|k| self.scene.ai.flags.get(k as usize));
                    flag.map_or(Mat4::IDENTITY, |&(at, yaw)| {
                        Mat4::from_translation(at) * Mat4::from_rotation_z(yaw)
                    })
                } else {
                    Mat4::IDENTITY
                };
                if let Some((graph, anim)) = self.animation(&arg(0), &arg(1)) {
                    if self.st.log {
                        let name = self.scene.ai.string_ids.get(&arg(1).handle()?);
                        println!("cutscene: camera {} at {now:.1}", name.map_or("?", |n| n));
                    }
                    self.st.cutscene.shot = Some(Playing {
                        graph,
                        anim,
                        start: now,
                        anchor,
                        looping: false,
                    });
                }
                Value::Void
            }
            "camera_time" => ticks(
                self.st
                    .cutscene
                    .shot
                    .map_or(0.0, |s| s.left(self.scene, now)),
            ),
            "camera_set_field_of_view" => {
                self.st.cutscene.fov = Some(arg(0).num());
                Value::Void
            }
            "custom_animation_relative" | "custom_animation_relative_loop" => {
                let anchor = self.placed(&arg(4));
                let looping = function.ends_with("loop");
                self.play(&arg(0), &arg(1), &arg(2), anchor, looping);
                Value::Void
            }
            "custom_animation" | "custom_animation_loop" => {
                let anchor = self.placed(&arg(0));
                let looping = function.ends_with("loop");
                self.play(&arg(0), &arg(1), &arg(2), anchor, looping);
                Value::Void
            }
            "scenery_animation_start_relative" | "scenery_animation_start_relative_loop" => {
                let anchor = self.placed(&arg(3));
                let looping = function.ends_with("loop");
                self.play(&arg(0), &arg(1), &arg(2), anchor, looping);
                Value::Void
            }
            "scenery_animation_start" | "scenery_animation_start_loop" => {
                let anchor = self.placed(&arg(0));
                let looping = function.ends_with("loop");
                self.play(&arg(0), &arg(1), &arg(2), anchor, looping);
                Value::Void
            }
            "unit_stop_custom_animation" | "scenery_animation_idle" => {
                for &o in arg(0).objects() {
                    if let Obj::Name(n) = o {
                        self.st.cutscene.objects.remove(&n);
                    }
                }
                Value::Void
            }
            "unit_is_playing_custom_animation" => {
                let playing = arg(0).objects().iter().any(|&o| match o {
                    Obj::Name(n) => self
                        .st
                        .cutscene
                        .objects
                        .get(&n)
                        .is_some_and(|p| p.looping || p.left(self.scene, now) > 0.0),
                    _ => false,
                });
                Value::Bool(playing)
            }
            "scenery_get_animation_time" => {
                let left = arg(0).objects().iter().find_map(|&o| match o {
                    Obj::Name(n) => self.st.cutscene.objects.get(&n),
                    _ => None,
                });
                ticks(left.map_or(0.0, |p| p.left(self.scene, now)))
            }
            // A cutscene the player can skip.
            "cinematic_skip_start_internal" => {
                self.st.cutscene.skippable = true;
                self.st.cutscene.skipping = false;
                Value::Void
            }
            "cinematic_skip_stop_internal" => {
                self.st.cutscene.skippable = false;
                Value::Void
            }
            "cinematic_subtitle" => {
                if let Some(id) = arg(0).handle() {
                    self.st.cutscene.subtitle = Some((id, now + arg(1).num()));
                }
                Value::Void
            }
            _ => return None,
        })
    }
}

impl Mission {
    /// Where the cutscene camera is, which way it looks and its horizontal
    /// field of view (degrees), while the scripts have it.
    pub fn cutscene_camera(&self, scene: &Scene) -> Option<(Vec3, Vec3, f32)> {
        let c = &self.state.cutscene;
        if !c.camera_on {
            return None;
        }
        let at = c.shot?.root(scene, self.state.time)?;
        let fov = c.fov.filter(|f| *f > 1.0).unwrap_or(70.0);
        Some((
            at.transform_point3(Vec3::ZERO),
            at.transform_vector3(Vec3::X).normalize_or(Vec3::X),
            fov,
        ))
    }

    /// The cutscene's cast as posed now: mesh, posed vertices, where they
    /// go and the body (for its colours).
    pub fn cutscene_bodies<'a>(
        &self,
        scene: &'a Scene,
    ) -> Vec<(usize, Vec<Vertex>, Mat4, &'a CinemaBody)> {
        let now = self.state.time;
        let mut out = Vec::new();
        for (&name, playing) in &self.state.cutscene.objects {
            let Some(body) = scene.ai.cinema.bodies.get(&name) else {
                continue;
            };
            if !self.state.exists(Some(name), false) || self.state.hidden.contains(&Obj::Name(name))
            {
                continue;
            }
            let (Some(graph), Some(a)) = (
                scene.ai.cinema.graphs.get(&playing.graph),
                playing.animation(scene),
            ) else {
                continue;
            };
            let world = rig::pose_by_name(
                &body.skeleton,
                &body.parents,
                graph,
                a,
                playing.frame(a, now),
            );
            let skin: Vec<Mat4> = world
                .iter()
                .zip(&body.skeleton.inverse_bind)
                .map(|(w, inv)| *w * *inv)
                .collect();
            out.push((body.mesh, body.skin.pose(&skin), playing.anchor, body));
        }
        out
    }

    /// Where a named object a cutscene animates is now (scenery like the
    /// In Amber Clad flying by).
    pub fn cutscene_object(&self, scene: &Scene, name: u16) -> Option<Mat4> {
        self.state
            .cutscene
            .objects
            .get(&name)?
            .root(scene, self.state.time)
    }

    /// The subtitle up now.
    pub(super) fn subtitle(&self, scene: &Scene) -> Option<String> {
        let (id, until) = self.state.cutscene.subtitle?;
        (self.state.time < until)
            .then(|| scene.ai.cinema.subtitles.get(&id).cloned())
            .flatten()
    }
}
