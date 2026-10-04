//! Mission dialogue: the lines scripts have actors say (Johnson calling
//! out, Marines answering) and the ones that come over the radio
//! (Cortana, Miranda), each in the voice of whoever says it. And scenes:
//! a few actors who fit a scene's roles acting it out with a command
//! script, trading lines.

use super::{Ctx, MissionSound};
use h2sim::script::{Obj, Value, TICKS_PER_SECOND};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub(super) struct Scenes {
    /// The scenes that have played, by name.
    played: HashSet<u32>,
    /// Each playing scene's cast (each role's name and actor), by the
    /// actor its command script runs for.
    cast: HashMap<usize, Vec<(u32, usize)>>,
    /// The cast member a scene's script has switched to, by the actor
    /// the script runs for.
    pub switched: HashMap<usize, usize>,
    /// Voices actors took on to play a role.
    voices: HashMap<usize, String>,
    /// Lines command scripts are waiting on (the line and seconds left),
    /// by the actor each script runs for.
    talking: HashMap<usize, (u32, f32)>,
}

impl Ctx<'_> {
    /// The voice an actor speaks in: one of its character's, the same
    /// every time it speaks.
    fn voice(&self, i: usize) -> Option<&str> {
        if let Some(v) = self.st.scenes.voices.get(&i) {
            return Some(v);
        }
        let c = self.game.players.get(i)?.actor?.character;
        let voices = self.scene.ai.voices.get(c)?;
        Some(voices.get(i % voices.len().max(1))?.as_str())
    }

    /// The voices an actor's character has.
    fn voices_of(&self, i: usize) -> &[String] {
        self.game
            .players
            .get(i)
            .and_then(|p| p.actor)
            .and_then(|a| self.scene.ai.voices.get(a.character))
            .map_or(&[], Vec::as_slice)
    }

    /// Which recording of a line an actor (or no one, over the radio)
    /// says: its own voice's, else another of its character's voices',
    /// else the first.
    fn recording(&self, line: u32, speaker: Option<usize>) -> Option<usize> {
        let voices = self.scene.ai.lines.get(&line)?;
        let find = |voice: &str| voices.iter().find(|(d, _)| d == voice).map(|v| v.1);
        let own = speaker.and_then(|i| find(self.voice(i)?));
        let character = || {
            let c = self.game.players.get(speaker?)?.actor?.character;
            self.scene.ai.voices.get(c)?.iter().find_map(|v| find(v))
        };
        own.or_else(character)
            .or_else(|| voices.first().map(|v| v.1))
    }

    /// Of an `ai`'s actors, the one best placed to say a line: one with
    /// a recording in its own voice if there is one.
    fn speaker(&self, ai: &Value, line: u32) -> Option<usize> {
        let actors = self.actors(ai);
        let voices = self.scene.ai.lines.get(&line)?;
        actors
            .iter()
            .copied()
            .find(|&i| {
                self.voice(i)
                    .is_some_and(|own| voices.iter().any(|(d, _)| d == own))
            })
            .or_else(|| actors.first().copied())
    }

    /// Say a line from where `speaker` stands (over the radio when no one
    /// says it); returns how long it lasts, in ticks.
    fn say(&mut self, line: u32, speaker: Option<usize>, at: Option<Obj>) -> Value {
        let Some(sound) = self.recording(line, speaker) else {
            return Value::Real(0.0);
        };
        if self.st.log {
            let name = self
                .scene
                .ai
                .string_ids
                .get(&line)
                .map_or("?", String::as_str);
            let who = speaker
                .and_then(|i| Some((i, self.game.players.get(i)?)))
                .map_or("radio".into(), |(i, p)| format!("{} {i}", p.name));
            println!("script: {who} says line {name}");
        }
        let at = at.and_then(|o| self.position(o));
        self.st.sounds.push(MissionSound::Line {
            sound,
            at,
            gain: 1.0,
        });
        let seconds = self
            .scene
            .sounds
            .get(sound)
            .and_then(|s| s.clips.first())
            .map_or(0.0, |c| c.duration());
        Value::Real((seconds * TICKS_PER_SECOND as f32).round())
    }

    /// Start a scene if its triggers hold and actors from its groups
    /// (`groups`, by role group) can play every role: the first role's
    /// actor runs its command script.
    fn stage(&mut self, name: u32, script: Option<u16>, groups: &[Value]) -> bool {
        let scene = self.scene;
        let Some(def) = scene.ai.scenes.iter().find(|s| s.name == name) else {
            return false;
        };
        let Some(script) = script else {
            return false;
        };
        if def.roles.is_empty() || (!def.repeats && self.st.scenes.played.contains(&name)) {
            return false;
        }
        let mut cast: Vec<(u32, usize)> = Vec::new();
        for role in &def.roles {
            let Some(group) = groups.get(role.group as usize) else {
                return false;
            };
            let fits = |i: usize| {
                !cast.iter().any(|c| c.1 == i)
                    && (role.voices.is_empty()
                        || self.voices_of(i).iter().any(|v| role.voices.contains(v)))
            };
            let Some(i) = self.actors(group).into_iter().find(|&i| fits(i)) else {
                return false;
            };
            cast.push((role.name, i));
        }
        let Some(squad) = self.game.players[cast[0].1].actor.map(|a| a.squad as usize) else {
            return false;
        };
        for (combine, refs) in &def.conditions {
            if !refs.is_empty() && !self.triggers_hold(None, *combine, refs, squad) {
                return false;
            }
        }
        for (role, &(_, i)) in def.roles.iter().zip(&cast) {
            let own = self
                .voice(i)
                .is_some_and(|v| role.voices.iter().any(|r| r == v));
            if !role.voices.is_empty() && !own {
                let voice = self
                    .voices_of(i)
                    .iter()
                    .find(|v| role.voices.contains(v))
                    .cloned();
                if let Some(v) = voice {
                    self.st.scenes.voices.insert(i, v);
                }
            }
        }
        if self.st.log {
            let name = scene.ai.string_ids.get(&name).map_or("?", String::as_str);
            let actors: Vec<usize> = cast.iter().map(|c| c.1).collect();
            println!("scene: {name} with {actors:?}");
        }
        self.st.scenes.played.insert(name);
        let runner = cast[0].1;
        self.st
            .commands
            .starts
            .push((runner, script as usize, false));
        self.st.scenes.switched.remove(&runner);
        self.st.scenes.cast.insert(runner, cast);
        true
    }

    /// After the command scripts: count down the lines they wait on and
    /// forget the scenes whose scripts have ended.
    pub(super) fn run_scenes(&mut self, dt: f32) {
        let scenes = &mut self.st.scenes;
        let commands = &self.st.commands;
        for t in scenes.talking.values_mut() {
            t.1 -= dt;
        }
        scenes.cast.retain(|r, _| commands.commanding(*r));
        scenes.switched.retain(|r, _| commands.commanding(*r));
        scenes.talking.retain(|r, _| commands.commanding(*r));
    }

    /// The scripts' dialogue and scene functions; `None` for any other
    /// function.
    pub(super) fn dialogue_call(&mut self, function: &str, args: &[Value]) -> Option<Value> {
        let arg = |k: usize| args.get(k).cloned().unwrap_or_default();
        let line = || arg(1).handle().unwrap_or(u32::MAX);
        Some(match function {
            "ai_scene" => {
                let name = arg(0).handle().unwrap_or(u32::MAX);
                Value::Bool(self.stage(name, arg(1).index(), args.get(2..).unwrap_or(&[])))
            }
            "cs_switch" => {
                let runner = self.st.commands.runner?;
                let role = arg(0).handle();
                let to = self
                    .st
                    .scenes
                    .cast
                    .get(&runner)
                    .and_then(|c| c.iter().find(|c| Some(c.0) == role))
                    .map(|c| c.1);
                if let Some(to) = to {
                    self.st.scenes.switched.insert(runner, to);
                    self.st.commands.current = Some(to);
                }
                Value::Void
            }
            // Say a line and wait till it's said.
            "cs_play_line" => {
                let runner = self.st.commands.runner?;
                let line = arg(0).handle().unwrap_or(u32::MAX);
                match self.st.scenes.talking.get(&runner) {
                    Some(&(l, left)) if l == line => {
                        if left > 0.0 {
                            self.st.commands.waiting = true;
                        } else {
                            self.st.scenes.talking.remove(&runner);
                        }
                    }
                    _ => {
                        let speaker = self
                            .st
                            .commands
                            .current
                            .filter(|&i| self.game.players.get(i).is_some_and(|p| p.alive));
                        let ticks = self.say(line, speaker, speaker.map(Obj::Unit)).num();
                        let seconds = ticks / TICKS_PER_SECOND as f32;
                        self.st.scenes.talking.insert(runner, (line, seconds));
                        self.st.commands.waiting = true;
                    }
                }
                Value::Void
            }
            "ai_trigger_test" => {
                let trigger = arg(0)
                    .handle()
                    .and_then(|h| self.scene.ai.trigger_names.get(&h).copied());
                let squad = self.squads(&arg(1)).first().copied();
                match (trigger, squad) {
                    (Some(t), Some(s)) => Value::Bool(self.trigger(None, t, s)),
                    _ => Value::Bool(false),
                }
            }
            "ai_play_line" | "ai_play_line_at_player" => {
                let line = line();
                match self.speaker(&arg(0), line) {
                    Some(i) => self.say(line, Some(i), Some(Obj::Unit(i))),
                    None => Value::Real(0.0),
                }
            }
            "ai_play_line_on_object" => {
                let line = line();
                let object = arg(0).objects().first().copied();
                let speaker = object.and_then(|o| self.unit(o));
                self.say(line, speaker, object)
            }
            _ => return None,
        })
    }
}
