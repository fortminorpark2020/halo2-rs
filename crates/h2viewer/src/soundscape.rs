//! What the game sounds like: weapons, grenades, footsteps and shields,
//! placed around whoever is listening (each splitscreen view hears through
//! its own camera; the nearest one wins), and the announcer calling out
//! medals and the lead for the players at this PC.

use crate::audio::{Audio, Clip};
use crate::scene::{Scene, SoundAsset};
use glam::Vec3;
use h2sim::game::{Event, FlagEvent, GrenadeKind, HillEvent, LeadChange, Medal};
use h2sim::{Game, GameType, ItemKind};
use std::collections::VecDeque;
use std::sync::Arc;

/// One view's ears.
pub struct Listener {
    pub position: Vec3,
    /// The camera's right.
    pub right: Vec3,
    pub player: usize,
    /// Looking through the player's eyes: their own sounds play unplaced.
    pub first_person: bool,
}

/// Distance travelled between footsteps, world units.
const STRIDE: f32 = 0.8;
/// Shots closer together than this are one burst (one burst sound).
const BURST_GAP: f64 = 0.15;
/// Announcer lines waiting their turn; beyond this the oldest is dropped.
const ANNOUNCER_QUEUE: usize = 4;
/// Pause between announcer lines, seconds.
const ANNOUNCER_GAP: f32 = 0.15;
/// The announcer speaks up over the fighting.
/// Menu sounds, over the music.
const UI_VOLUME: f32 = 1.6;
const ANNOUNCER_VOLUME: f32 = 1.4;
/// Mission dialogue, and the mission's music.
const DIALOGUE_VOLUME: f32 = 1.5;
const MISSION_MUSIC_VOLUME: f32 = 0.6;
/// Seconds music fades out over when a script stops it.
const MUSIC_FADE: f32 = 2.0;
/// The respawn countdown ticks over its last seconds.
const RESPAWN_TICKS: u32 = 3;

#[derive(Default, Clone)]
struct PlayerSounds {
    last_shot: f64,
    stride: f32,
    grounded: bool,
    falling: f32,
}

#[derive(Default)]
struct ShieldSounds {
    charge: Option<u64>,
    low: Option<u64>,
    last_shield: f32,
    /// The weapon in hand and its zoom level, for zoom sounds.
    zoom: Option<(usize, u32)>,
}

pub struct Soundscape {
    pub audio: Audio,
    players: Vec<PlayerSounds>,
    /// Per local view, by player.
    shields: Vec<(usize, ShieldSounds)>,
    rng: u32,
    announcer: VecDeque<usize>,
    /// Seconds until the announcer is free to speak.
    speaking: f32,
    announced_start: bool,
    announced_winner: bool,
    /// Per local player: the respawn countdown's whole second last ticked.
    respawn_ticks: Vec<(usize, u32)>,
    /// Per vehicle: its engine and boost loops while someone drives.
    engines: Vec<[Option<u64>; 3]>,
    /// Rounds in flight with a sound of their own (a rocket's roar): the
    /// weapon, where the round was last frame, and its loop.
    rounds: Vec<(usize, Vec3, u64)>,
    /// Mission lines playing (sound, voice), and its music (tag, voice).
    lines: Vec<(usize, u64)>,
    loops: Vec<(u32, u64)>,
}

/// Volume at distance `d` for a sound carrying over `range`.
fn falloff(d: f32, (near, far): (f32, f32)) -> f32 {
    if d <= near.max(0.01) {
        return 1.0;
    }
    if d >= far {
        return 0.0;
    }
    let fade = ((far - d) / (far * 0.25)).min(1.0);
    near.max(0.01) / d * fade
}

/// Left/right gains for a sound at `at` heard over `distance`: the loudest
/// any listener hears it (its owner hears it unplaced in first person).
fn placed(
    distance: (f32, f32),
    at: Vec3,
    owner: Option<usize>,
    listeners: &[Listener],
) -> [f32; 2] {
    let mut best = [0.0f32; 2];
    for l in listeners {
        let gains = if l.first_person && owner == Some(l.player) {
            [1.0, 1.0]
        } else {
            let to = at - l.position;
            let d = to.length();
            let g = falloff(d, distance);
            let pan = if d > 1e-3 { to.dot(l.right) / d } else { 0.0 };
            pan_gains(g, pan * 0.8)
        };
        if gains[0] + gains[1] > best[0] + best[1] {
            best = gains;
        }
    }
    best
}

/// Left/right gains for a sound to one side (-1 left .. 1 right).
fn pan_gains(gain: f32, pan: f32) -> [f32; 2] {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    let s = std::f32::consts::SQRT_2;
    [gain * (a.cos() * s).min(1.0), gain * (a.sin() * s).min(1.0)]
}

impl Soundscape {
    pub fn new() -> Soundscape {
        Soundscape {
            audio: Audio::new(),
            players: Vec::new(),
            shields: Vec::new(),
            rng: 0x1234_5678,
            announcer: VecDeque::new(),
            speaking: 0.0,
            announced_start: false,
            announced_winner: false,
            respawn_ticks: Vec::new(),
            engines: Vec::new(),
            rounds: Vec::new(),
            lines: Vec::new(),
            loops: Vec::new(),
        }
    }

    /// Silence everything and start afresh (a new game, or the menus).
    pub fn reset(&mut self) {
        self.audio.stop_all();
        self.players.clear();
        self.shields.clear();
        self.announcer.clear();
        self.speaking = 0.0;
        self.announced_start = false;
        self.announced_winner = false;
        self.respawn_ticks.clear();
        self.engines.clear();
        self.rounds.clear();
        self.lines.clear();
        self.loops.clear();
    }

    /// A mission's dialogue and music, as its scripts call for them.
    pub fn mission(
        &mut self,
        scene: &Scene,
        sound: crate::campaign::MissionSound,
        listeners: &[Listener],
    ) {
        use crate::campaign::MissionSound;
        let clip = |s: Option<usize>| -> Option<(Arc<Clip>, &SoundAsset)> {
            let asset = scene.sounds.get(s?)?;
            Some((asset.clips.first()?.clone(), asset))
        };
        match sound {
            MissionSound::Line { sound, at, gain } => {
                let Some((line, asset)) = clip(Some(sound)) else {
                    return;
                };
                let g = asset.gain * gain * DIALOGUE_VOLUME;
                // Said by someone: heard from where they are, but never
                // too faint to follow.
                let gains = match at {
                    Some(at) => {
                        placed(asset.distance, at, None, listeners).map(|v| v.max(0.35) * g)
                    }
                    None => [g, g],
                };
                let voice = self.audio.play(&line, gains, 1.0, false);
                self.lines.retain(|(s, _)| *s != sound);
                self.lines.push((sound, voice));
            }
            MissionSound::StopLine(sound) => {
                for (_, voice) in self.lines.iter().filter(|(s, _)| *s == sound) {
                    self.audio.stop(*voice);
                }
                self.lines.retain(|(s, _)| *s != sound);
            }
            MissionSound::Hush => {
                for (_, voice) in self.lines.drain(..) {
                    self.audio.stop(voice);
                }
            }
            MissionSound::StartLoop(tag) => {
                if self.loops.iter().any(|(t, _)| *t == tag) {
                    return;
                }
                let Some(music) = scene.ai.loops.get(&tag) else {
                    return;
                };
                let repeat: Vec<Arc<Clip>> = music
                    .repeat
                    .and_then(|s| scene.sounds.get(s))
                    .map(|a| a.clips.clone())
                    .unwrap_or_default();
                let Some(first) = clip(music.start)
                    .map(|c| c.0)
                    .or_else(|| repeat.first().cloned())
                else {
                    return;
                };
                let then = if music.once { &[][..] } else { &repeat[..] };
                let voice = self
                    .audio
                    .play_music(&first, then, music.gain * MISSION_MUSIC_VOLUME);
                self.loops.push((tag, voice));
            }
            MissionSound::StopLoop(tag) => {
                for (_, voice) in self.loops.iter().filter(|(t, _)| *t == tag) {
                    self.audio.fade_out(*voice, MUSIC_FADE);
                }
                self.loops.retain(|(t, _)| *t != tag);
                if let Some((end, _)) = scene.ai.loops.get(&tag).and_then(|m| clip(m.end)) {
                    let g = scene.ai.loops[&tag].gain * MISSION_MUSIC_VOLUME;
                    self.audio.play(&end, [g, g], 1.0, false);
                }
            }
        }
    }

    /// A menu sound.
    pub fn play_ui(&mut self, scene: &Scene, sound: crate::menu::Sound) {
        use crate::menu::Sound;
        let ui = scene.game_sounds.ui;
        let s = match sound {
            Sound::Cursor => ui.cursor,
            Sound::Forward => ui.forward,
            Sound::Back => ui.back,
            Sound::Advance => ui.advance,
        };
        self.play_flat(scene, s, UI_VOLUME);
    }

    /// A sound with no place in the world (interface, announcer).
    fn play_flat(&mut self, scene: &Scene, sound: Option<usize>, volume: f32) -> f32 {
        let Some(asset) = sound.and_then(|s| scene.sounds.get(s)) else {
            return 0.0;
        };
        let clip = asset.clips[self.pick(asset.clips.len())].clone();
        let g = asset.gain * volume;
        self.audio.play(&clip, [g, g], 1.0, false);
        clip.duration()
    }

    /// Queue an announcer line; lines play one after another.
    fn announce(&mut self, line: Option<usize>) {
        if let Some(line) = line {
            if self.announcer.len() >= ANNOUNCER_QUEUE {
                self.announcer.pop_front();
            }
            self.announcer.push_back(line);
        }
    }

    fn speak(&mut self, scene: &Scene, dt: f32) {
        self.speaking -= dt;
        if self.speaking > 0.0 {
            return;
        }
        if let Some(line) = self.announcer.pop_front() {
            self.speaking = self.play_flat(scene, Some(line), ANNOUNCER_VOLUME) + ANNOUNCER_GAP;
        }
    }

    fn pick(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as usize % n.max(1)
    }

    /// Play a sound made at `at` (by `owner`, heard unplaced in their own
    /// first person view).
    fn play(
        &mut self,
        scene: &Scene,
        sound: Option<usize>,
        at: Vec3,
        owner: Option<usize>,
        listeners: &[Listener],
        volume: f32,
    ) {
        let Some(asset) = sound.and_then(|s| scene.sounds.get(s)) else {
            return;
        };
        let best = placed(asset.distance, at, owner, listeners);
        let gain = asset.gain * volume;
        let clip = asset.clips[self.pick(asset.clips.len())].clone();
        self.audio
            .play(&clip, [best[0] * gain, best[1] * gain], 1.0, false);
    }

    /// The sound of something that just happened in the game.
    pub fn event(&mut self, scene: &Scene, game: &Game, listeners: &[Listener], e: &Event) {
        // The gun in a player's right hand, or their left.
        let hand_sounds = |player: usize, left: bool| {
            game.players
                .get(player)
                .and_then(|p| if left { p.left.as_ref() } else { p.held() })
                .and_then(|h| scene.weapons.get(h.weapon))
                .map(|w| (w.sounds, w.def.shots_per_fire > 1))
        };
        let weapon_sounds = |player: usize| hand_sounds(player, false);
        let body = |player: usize| {
            game.players
                .get(player)
                .map_or(Vec3::ZERO, |p| p.body.position + Vec3::Z * 0.5)
        };
        let g = scene.game_sounds;
        let a = g.announcer;
        let local = |player: usize| listeners.iter().any(|l| l.player == player);
        match *e {
            Event::Medal { player, medal } if local(player) => {
                let line = match medal {
                    Medal::MultiKill(n) => a.multi_kill[n.clamp(2, 7) as usize - 2],
                    Medal::Spree(n) => a.spree.get((n / 5).max(1) as usize - 1).copied().flatten(),
                };
                self.announce(line);
            }
            Event::Lead { player, change } if local(player) => {
                self.announce(match change {
                    LeadChange::Gained => a.gained_lead,
                    LeadChange::Lost => a.lost_lead,
                    LeadChange::Tied => a.tied_lead,
                });
            }
            Event::Killed { killer, victim, .. }
                if local(victim) && killer.is_none_or(|k| k == victim) =>
            {
                self.announce(a.suicide);
            }
            // A teammate's kill: the betrayer and the betrayed hear it.
            Event::Killed {
                killer: Some(k),
                victim,
                ..
            } if !game.is_enemy(k, victim) => {
                if local(k) {
                    self.announce(a.betrayal);
                } else if local(victim) {
                    self.announce(a.betrayed);
                }
            }
            Event::Spawned { player, .. } if local(player) => {
                self.play_flat(scene, g.respawn, 1.0);
            }
            Event::Flag { what, .. } if game.rules.game_type.oddball() => match what {
                FlagEvent::Taken => self.announce(a.ball_taken),
                FlagEvent::Returned => self.announce(a.play_ball),
                _ => {}
            },
            Event::Flag { player, what, .. } if game.rules.game_type == GameType::Assault => {
                match what {
                    FlagEvent::Taken if player.is_some_and(local) => {
                        self.play_flat(scene, a.flag_grabbed, 1.0);
                    }
                    FlagEvent::Taken => self.announce(a.bomb_taken),
                    FlagEvent::Dropped => self.announce(a.bomb_dropped),
                    FlagEvent::Returned => self.announce(a.bomb_returned),
                    FlagEvent::Armed => self.announce(a.bomb_armed),
                    FlagEvent::Defused => self.announce(a.bomb_defused),
                    _ => {}
                }
            }
            Event::Flag { player, what, .. } => match what {
                FlagEvent::Taken if player.is_some_and(local) => {
                    self.play_flat(scene, a.flag_grabbed, 1.0);
                }
                FlagEvent::Taken => self.announce(a.flag_taken),
                FlagEvent::Returned => self.announce(a.flag_returned),
                FlagEvent::Captured => self.announce(a.flag_captured),
                FlagEvent::CaptureFailed if player.is_some_and(local) => {
                    self.play_flat(scene, a.flag_failure, 1.0);
                }
                _ => {}
            },
            Event::Hill { player, what } => match what {
                HillEvent::Moved => self.announce(a.hill_moved),
                HillEvent::Contested => self.announce(a.hill_contested),
                // Those taking the hill hear it.
                HillEvent::Controlled
                    if player
                        .is_some_and(|p| listeners.iter().any(|l| !game.is_enemy(l.player, p))) =>
                {
                    self.announce(a.hill_controlled)
                }
                _ => {}
            },
            Event::Territory { team, from, .. } => {
                let on = |t: u8| {
                    listeners
                        .iter()
                        .any(|l| game.players.get(l.player).is_some_and(|p| p.team == t))
                };
                if on(team) {
                    self.announce(if game.holds_every_territory(team) {
                        a.land_grab
                    } else {
                        a.territory_taken
                    });
                } else if from.is_some_and(on) {
                    self.announce(a.territory_lost);
                }
            }
            Event::Juggernaut { .. } => self.announce(a.new_juggernaut),
            Event::Shot {
                player,
                weapon,
                origin,
                hit,
                hit_player,
                ..
            } => {
                if player >= self.players.len() {
                    self.players.resize(player + 1, PlayerSounds::default());
                }
                let fired = scene
                    .weapons
                    .get(weapon)
                    .map(|w| (w.sounds, w.def.shots_per_fire > 1));
                if let Some((sounds, burst)) = fired {
                    let last = self.players[player].last_shot;
                    self.players[player].last_shot = game.time;
                    if !burst || game.time - last > BURST_GAP {
                        self.play(scene, sounds.fire, origin, Some(player), listeners, 1.0);
                    }
                }
                match (hit, hit_player) {
                    (Some((p, _)), Some(j)) => {
                        let shielded = game.players.get(j).is_some_and(|v| v.shield > 0.0);
                        if !shielded {
                            self.play(scene, g.hit_body, p, None, listeners, 0.8);
                        }
                    }
                    (Some((p, _)), None) => self.play(scene, g.impact, p, None, listeners, 0.5),
                    _ => {}
                }
            }
            Event::Reloaded { player, left, .. } => {
                let s = hand_sounds(player, left).and_then(|w| w.0.reload);
                self.play(scene, s, body(player), Some(player), listeners, 1.0);
            }
            Event::Switched { player } => {
                let s = weapon_sounds(player).and_then(|w| w.0.ready);
                self.play(scene, s, body(player), Some(player), listeners, 1.0);
            }
            Event::Melee { player, .. } => {
                let s = weapon_sounds(player).and_then(|w| w.0.melee);
                self.play(scene, s, body(player), Some(player), listeners, 1.0);
            }
            Event::Thrown { player } => {
                self.play(scene, g.throw, body(player), Some(player), listeners, 1.0);
            }
            Event::Entered {
                player,
                vehicle,
                seat,
            }
            | Event::Exited {
                player,
                vehicle,
                seat,
            }
            | Event::Hijacked {
                player,
                vehicle,
                seat,
                ..
            } => {
                let s = game
                    .vehicles
                    .get(vehicle)
                    .and_then(|v| scene.vehicles.kinds.get(v.def))
                    .and_then(|k| {
                        let sounds = match e {
                            Event::Entered { .. } => &k.enter_sounds,
                            Event::Exited { .. } => &k.exit_sounds,
                            _ => &k.board_sounds,
                        };
                        let elite = game.players.get(player).is_some_and(|p| p.look.elite);
                        sounds.get(seat).and_then(|s| s[elite as usize])
                    });
                self.play(scene, s, body(player), Some(player), listeners, 1.0);
            }
            Event::Teleported { from, to, .. } => {
                for at in [from, to] {
                    self.play(scene, g.teleport, at, None, listeners, 1.0);
                }
            }
            Event::VehicleDestroyed { position, .. } => {
                self.play(scene, g.explosion[0], position, None, listeners, 1.0);
            }
            Event::Exploded { kind, position } => {
                let s = g.explosion[(kind == GrenadeKind::Plasma) as usize];
                self.play(scene, s, position, None, listeners, 1.0);
            }
            Event::Impact {
                weapon,
                position,
                exploded,
                ..
            } => {
                let r = scene.weapons.get(weapon).map(|w| w.round);
                let fallback = match (exploded, r.is_some_and(|r| r.fiery)) {
                    (true, true) => g.explosion[0],
                    (true, false) => g.explosion[1],
                    (false, _) => g.impact,
                };
                // Needles all going off together.
                let together = r.and_then(|r| r.supercombine).filter(|_| exploded);
                let s = together.or(r.and_then(|r| r.impact)).or(fallback);
                let gain = if exploded { 1.0 } else { 0.6 };
                self.play(scene, s, position, None, listeners, gain);
            }
            Event::Damaged { player, .. } => {
                // Your own shields taking the hit.
                let shielded = game.players.get(player).is_some_and(|p| p.shield > 0.0);
                if shielded && listeners.iter().any(|l| l.player == player) {
                    self.play(
                        scene,
                        g.shield_hit,
                        body(player),
                        Some(player),
                        listeners,
                        0.7,
                    );
                }
            }
            Event::DryFire { player } => {
                let s = weapon_sounds(player).and_then(|w| w.0.empty);
                self.play(scene, s, body(player), Some(player), listeners, 1.0);
            }
            Event::PickedUp { player, kind } => {
                if !local(player) {
                    return;
                }
                let s = match kind {
                    ItemKind::Weapon(w) => scene.weapons.get(w).and_then(|w| w.sounds.pickup),
                    ItemKind::FragGrenades => g.grenade_pickup[0],
                    ItemKind::PlasmaGrenades => g.grenade_pickup[1],
                    ItemKind::Powerup(_) | ItemKind::Ammo { .. } => scene
                        .item_sounds
                        .iter()
                        .find(|(k, _)| *k == kind)
                        .and_then(|(_, s)| *s),
                };
                self.play(scene, s, body(player), Some(player), listeners, 1.0);
            }
            _ => {}
        }
    }

    /// Ongoing sounds: footsteps, jumps and landings, and the shield
    /// recharge and alarm for each local player.
    pub fn update(&mut self, scene: &Scene, game: &Game, listeners: &[Listener], dt: f32) {
        self.players
            .resize(game.players.len(), PlayerSounds::default());
        let g = scene.game_sounds;
        for (i, p) in game.players.iter().enumerate() {
            let s = &mut self.players[i];
            let was = s.grounded;
            s.grounded = p.body.grounded;
            let feet = p.body.position;
            if !p.alive {
                s.stride = 0.0;
                continue;
            }
            if !p.body.grounded {
                s.falling = s.falling.max(-p.body.velocity.z);
            }
            let mut sound = None;
            let mut volume = 1.0;
            if was && !p.body.grounded && p.body.velocity.z > 1.0 {
                sound = g.jump;
            } else if !was && p.body.grounded {
                if s.falling > 1.5 {
                    sound = g.land;
                }
                s.falling = 0.0;
            } else if p.body.grounded {
                let speed = p.body.velocity.truncate().length();
                // Crouch-walking is silent, as in Halo.
                if speed > 0.6 && p.body.crouch < 0.5 {
                    s.stride += speed * dt;
                    if s.stride > STRIDE {
                        s.stride = 0.0;
                        sound = g.footstep;
                        volume = (speed / 2.25).min(1.0) * 0.8;
                    }
                }
            }
            if sound.is_some() {
                self.play(scene, sound, feet, Some(i), listeners, volume);
            }
        }
        self.shield_alarms(scene, game, listeners);
        self.respawn_countdown(scene, game, listeners);
        self.engines(scene, game, listeners);
        self.flying_rounds(scene, game, listeners, dt);

        // The game type as play begins, and the end of the game.
        let a = g.announcer;
        if !self.announced_start {
            self.announced_start = true;
            let kind = GameType::ALL
                .iter()
                .position(|&t| t == game.rules.game_type)
                .unwrap_or(0);
            self.announce(a.game_names[kind]);
        }
        if game.winner.is_some() != self.announced_winner {
            self.announced_winner = game.winner.is_some();
            if self.announced_winner {
                self.announce(a.game_over);
            }
        }
        self.speak(scene, dt);
    }

    /// Vehicles' engines while someone drives them, rising with speed, and
    /// their boost.
    fn engines(&mut self, scene: &Scene, game: &Game, listeners: &[Listener]) {
        self.engines.resize(game.vehicles.len(), [None; 3]);
        for (k, v) in game.vehicles.iter().enumerate() {
            let (Some(def), Some(kind)) = (
                game.vehicle_defs.get(v.def),
                scene.vehicles.kinds.get(v.def),
            ) else {
                continue;
            };
            let driven = !v.destroyed
                && def
                    .driver_seat()
                    .is_some_and(|d| v.riders.get(d).copied().flatten().is_some());
            let pace = (v.speed() / def.max_forward_speed.max(1.0)).min(1.5);
            for (slot, sound, on) in [
                (0, kind.engine, driven),
                (1, kind.boost, driven && v.controls.boost),
                (2, kind.horn, driven && v.controls.horn),
            ] {
                let voice = &mut self.engines[k][slot];
                let asset = sound.and_then(|s| scene.sounds.get(s));
                match (asset, on, *voice) {
                    (Some(asset), true, playing) => {
                        let g = placed(asset.distance, v.center, None, listeners)
                            .map(|g| g * asset.gain);
                        let pitch = if slot == 2 { 1.0 } else { 0.85 + 0.45 * pace };
                        match playing {
                            Some(id) => self.audio.adjust(id, g, pitch),
                            None => {
                                let clip = asset.clips[0].clone();
                                *voice = Some(self.audio.play_loop(&clip, g, pitch));
                            }
                        }
                    }
                    (_, _, Some(id)) => {
                        self.audio.fade_out(id, 0.4);
                        *voice = None;
                    }
                    _ => {}
                }
            }
        }
    }

    /// The sound rounds make in flight (rockets, Fuel Rod shots), following
    /// each round. Rounds aren't numbered, so each is matched to the nearest
    /// one of its kind last frame.
    fn flying_rounds(&mut self, scene: &Scene, game: &Game, listeners: &[Listener], dt: f32) {
        let mut before = std::mem::take(&mut self.rounds);
        for r in &game.projectiles {
            let Some(asset) = scene
                .weapons
                .get(r.weapon)
                .and_then(|w| w.round.flight)
                .and_then(|s| scene.sounds.get(s))
            else {
                continue;
            };
            let reach = 1.0 + r.velocity.length() * dt * 3.0;
            let was = before
                .iter()
                .enumerate()
                .filter(|(_, b)| b.0 == r.weapon)
                .map(|(k, b)| (k, b.1.distance(r.position)))
                .filter(|&(_, d)| d < reach)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| before.swap_remove(k).2);
            let g = placed(asset.distance, r.position, None, listeners).map(|g| g * asset.gain);
            let voice = match was {
                Some(id) => {
                    self.audio.adjust(id, g, 1.0);
                    id
                }
                None => self.audio.play_loop(&asset.clips[0], g, 1.0),
            };
            self.rounds.push((r.weapon, r.position, voice));
        }
        for (.., id) in before {
            self.audio.fade_out(id, 0.15);
        }
    }

    /// Ticks over the last seconds before a local player respawns.
    fn respawn_countdown(&mut self, scene: &Scene, game: &Game, listeners: &[Listener]) {
        self.respawn_ticks
            .retain(|(p, _)| listeners.iter().any(|l| l.player == *p));
        for l in listeners {
            let Some(p) = game.players.get(l.player) else {
                continue;
            };
            let second = if p.alive {
                0
            } else {
                p.respawn_in.ceil().max(0.0) as u32
            };
            let last = match self.respawn_ticks.iter_mut().find(|t| t.0 == l.player) {
                Some(t) => std::mem::replace(&mut t.1, second),
                None => {
                    self.respawn_ticks.push((l.player, second));
                    second
                }
            };
            if second != last && (1..=RESPAWN_TICKS).contains(&second) {
                self.play_flat(scene, scene.game_sounds.respawn_tick, 1.0);
            }
        }
    }

    fn shield_alarms(&mut self, scene: &Scene, game: &Game, listeners: &[Listener]) {
        let g = scene.game_sounds;
        self.shields
            .retain(|(p, _)| listeners.iter().any(|l| l.player == *p));
        let full = game.rules.shield;
        for l in listeners {
            let Some(p) = game.players.get(l.player) else {
                continue;
            };
            if !self.shields.iter().any(|s| s.0 == l.player) {
                self.shields.push((l.player, ShieldSounds::default()));
            }
            let k = self.shields.iter().position(|s| s.0 == l.player).unwrap();
            // Zooming in and out.
            let zoom = p.held().map(|h| (h.weapon, h.state.zoom));
            if let (Some((w, now)), Some((was_w, was))) = (zoom, self.shields[k].1.zoom) {
                if w == was_w && now != was {
                    let sounds = scene.weapons.get(w).map(|a| a.sounds);
                    let s = sounds.and_then(|s| if now > was { s.zoom_in } else { s.zoom_out });
                    self.play(scene, s, l.position, Some(l.player), listeners, 1.0);
                }
            }
            self.shields[k].1.zoom = zoom;
            let recharging = p.alive && p.shield > self.shields[k].1.last_shield && p.shield < full;
            let low = p.alive && p.shield <= 0.0;
            self.shields[k].1.last_shield = p.shield;
            let quiet = 0.5 / listeners.len() as f32;
            let charge = self.shields[k].1.charge;
            if let Some(id) = self.toggle_loop(scene, charge, recharging, g.shield_charge, quiet) {
                self.shields[k].1.charge = id;
            }
            let alarm = self.shields[k].1.low;
            if let Some(id) = self.toggle_loop(scene, alarm, low, g.shield_low, quiet) {
                self.shields[k].1.low = id;
            }
        }
    }

    /// Start or stop a looping UI sound; returns the new voice when it changed.
    fn toggle_loop(
        &mut self,
        scene: &Scene,
        playing: Option<u64>,
        on: bool,
        sound: Option<usize>,
        volume: f32,
    ) -> Option<Option<u64>> {
        match (playing, on) {
            (None, true) => {
                let asset = sound.and_then(|s| scene.sounds.get(s))?;
                let clip = asset.clips[self.pick(asset.clips.len())].clone();
                let g = asset.gain * volume;
                Some(Some(self.audio.play(&clip, [g, g], 1.0, true)))
            }
            (Some(id), false) => {
                self.audio.stop(id);
                Some(None)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_fade_with_distance_and_pan_to_their_side() {
        assert_eq!(falloff(0.5, (1.0, 10.0)), 1.0);
        assert!(falloff(5.0, (1.0, 10.0)) < 0.25);
        assert_eq!(falloff(12.0, (1.0, 10.0)), 0.0);
        let [l, r] = pan_gains(1.0, 1.0);
        assert!(l < 0.01 && (r - 1.0).abs() < 1e-5);
        let [l, r] = pan_gains(1.0, 0.0);
        assert!((l - 1.0).abs() < 1e-5 && (r - 1.0).abs() < 1e-5);
    }
}
