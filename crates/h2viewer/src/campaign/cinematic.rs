//! What the scripts do around cutscenes and in play: teleporting things
//! to cutscene flags and checking whether players look at one, taking the
//! controls away, hiding objects, and the Covenant's active camouflage.

use super::Ctx;
use glam::Vec3;
use h2sim::script::{Obj, Value};

impl Ctx<'_> {
    /// A cutscene flag a script names: where it is and its yaw.
    fn flag(&self, v: &Value) -> Option<(Vec3, f32)> {
        self.scene.ai.flags.get(v.index()? as usize).copied()
    }

    /// The scripts' cutscene-side functions; `None` for any other
    /// function.
    pub(super) fn cinematic_call(&mut self, function: &str, args: &[Value]) -> Option<Value> {
        let arg = |k: usize| args.get(k).cloned().unwrap_or_default();
        let now = self.st.time;
        Some(match function {
            // Shaking the view (the Pelican hit, the carrier's guns).
            "player_effect_set_max_rotation" => {
                self.st.screen.shake_rotation = [0, 1, 2].map(|k| arg(k).num());
                Value::Void
            }
            "player_effect_set_max_translation" | "player_effect_set_max_vibration" => Value::Void,
            "player_effect_start" => {
                self.st.screen.shake.go(now, arg(0).num(), arg(1).num());
                Value::Void
            }
            "player_effect_stop" => {
                self.st.screen.shake.go(now, 0.0, arg(0).num());
                Value::Void
            }
            "object_teleport" => {
                if let Some((at, yaw)) = self.flag(&arg(1)) {
                    for &o in arg(0).objects() {
                        if let Obj::Unit(i) = o {
                            self.game.move_player(i, at + Vec3::Z * 0.05, yaw);
                            if self.game.players[i].actor.is_none() {
                                self.st.turns.push((i, yaw));
                            }
                        }
                    }
                }
                Value::Void
            }
            "objects_can_see_flag" => {
                let cone = arg(2).num().to_radians().cos();
                let seen = self.flag(&arg(1)).is_some_and(|(at, _)| {
                    arg(0).objects().iter().any(|&o| match o {
                        Obj::Unit(i) => self.game.players.get(i).is_some_and(|p| {
                            p.alive && p.aim().dot((at - p.eye()).normalize_or_zero()) >= cone
                        }),
                        Obj::Name(_) | Obj::Vehicle(_) => false,
                    })
                });
                Value::Bool(seen)
            }
            "player_enable_input" => {
                self.st.input_off = !arg(0).truthy();
                if self.st.log {
                    let now = if self.st.input_off { "off" } else { "on" };
                    println!("script: player controls {now} at {:.1}", self.st.time);
                }
                Value::Void
            }
            "object_hide" => {
                let hide = arg(1).truthy();
                for &o in arg(0).objects() {
                    if hide {
                        self.st.hidden.insert(o);
                    } else {
                        self.st.hidden.remove(&o);
                    }
                }
                Value::Void
            }
            "ai_set_active_camo" => {
                let on = arg(1).truthy();
                for i in self.actors(&arg(0)) {
                    self.game.players[i].camo = if on { f32::MAX } else { 0.0 };
                }
                Value::Void
            }
            _ => return None,
        })
    }
}
