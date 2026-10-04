//! The objective game types on screen: where a map's flags, balls, bombs,
//! hills and territories are; flags on their stands and in hand with their
//! cloth waving; balls and bombs; glowing hill and territory outlines; the
//! arrows over them; and what players are told when something happens.

use crate::effects;
use crate::gpu::{hud_mode, DrawCall, SpriteVertex};
use crate::hud::HudBuilder;
use crate::local::{armor_colors, LocalPlayer, TEAM_COLORS};
use crate::scene::{FlagAssets, Scene, Vertex};
use crate::App;
use blam_cache::scenario::NetgameFlagKind;
use glam::{Mat4, Vec3};
use h2sim::game::{Event, FlagEvent, Hill, HillControl, HillEvent, Territory, NEUTRAL, TEAMS};
use h2sim::{Game, GameType};
use std::collections::BTreeMap;

/// Places on the map for each team: (team, where).
pub type TeamSpots = Vec<(u8, Vec3)>;

/// Where a game type's carried things start (`homes`), and where they're
/// brought (`bases`): Capture the Flag's flags and their returns, Assault's
/// bombs and the bases they're armed at, or Oddball's ball.
pub fn carried_spots(scene: &Scene, game_type: GameType) -> (TeamSpots, TeamSpots) {
    let (spawn, ret) = match game_type {
        GameType::Ctf => (
            NetgameFlagKind::CtfFlagSpawn,
            NetgameFlagKind::CtfFlagReturn,
        ),
        GameType::Assault => (
            NetgameFlagKind::AssaultBombSpawn,
            NetgameFlagKind::AssaultBombReturn,
        ),
        GameType::Oddball | GameType::TeamOddball => {
            let ball = scene
                .netgame_flags
                .iter()
                .find(|f| f.kind == NetgameFlagKind::OddballSpawn)
                .map(|f| (NEUTRAL, Vec3::from(f.position)));
            return (ball.into_iter().collect(), Vec::new());
        }
        _ => return (Vec::new(), Vec::new()),
    };
    let mut homes = Vec::new();
    let mut bases = Vec::new();
    for f in &scene.netgame_flags {
        let Ok(team) = u8::try_from(f.team) else {
            continue;
        };
        if team >= TEAMS {
            continue;
        }
        let at = Vec3::from(f.position);
        if f.kind == spawn && !homes.iter().any(|h: &(u8, Vec3)| h.0 == team) {
            homes.push((team, at));
        } else if f.kind == ret {
            bases.push((team, at));
        }
    }
    homes.sort_by_key(|h| h.0);
    (homes, bases)
}

/// The map's King of the Hill hills, in order.
pub fn hills(scene: &Scene) -> Vec<Hill> {
    let mut corners: BTreeMap<u8, Vec<Vec3>> = BTreeMap::new();
    for f in &scene.netgame_flags {
        if let NetgameFlagKind::KingHill(n) = f.kind {
            corners.entry(n).or_default().push(f.position.into());
        }
    }
    corners
        .values()
        .filter(|c| c.len() >= 3)
        .map(|c| Hill::new(c))
        .collect()
}

/// The map's territories, by their number.
pub fn territories(scene: &Scene) -> Vec<Territory> {
    let mut points: BTreeMap<i16, Vec<Vec3>> = BTreeMap::new();
    for f in &scene.netgame_flags {
        if f.kind == NetgameFlagKind::TerritoriesFlag {
            points
                .entry(f.identifier)
                .or_default()
                .push(f.position.into());
        }
    }
    points.into_values().map(Territory::new).collect()
}

/// Every place an objective game sends players (for the bots' routes).
pub fn objective_points(scene: &Scene) -> Vec<Vec3> {
    scene
        .netgame_flags
        .iter()
        .filter(|f| {
            !matches!(
                f.kind,
                NetgameFlagKind::TeleporterSource
                    | NetgameFlagKind::TeleporterDestination
                    | NetgameFlagKind::Other(_)
            )
        })
        .map(|f| Vec3::from(f.position))
        .chain(hills(scene).iter().map(Hill::centre))
        .collect()
}

/// What a carried thing is called.
fn thing(game_type: GameType) -> &'static str {
    match game_type {
        GameType::Oddball | GameType::TeamOddball => "BALL",
        GameType::Assault => "BOMB",
        _ => "FLAG",
    }
}

/// What a player at this PC is told when something happens in an objective
/// game.
pub fn event_message(me: usize, game: &Game, e: &Event) -> Option<String> {
    let my_team = game.players.get(me)?.team;
    let teams = game.rules.game_type.teams();
    let name = |p: usize| crate::local::player_name(game, me, p);
    let kind = game.rules.game_type;
    Some(match *e {
        Event::Flag { team, player, what } => {
            let whose = if team == NEUTRAL {
                "THE"
            } else if team == my_team {
                "YOUR"
            } else {
                "THE ENEMY"
            };
            let t = thing(kind);
            let who = player.map(name);
            match (what, who) {
                (FlagEvent::Taken, Some(w)) if player == Some(me) => format!("{w} TOOK THE {t}"),
                (FlagEvent::Taken, Some(w)) if team == NEUTRAL => format!("{w} HAS THE BALL"),
                (FlagEvent::Taken, Some(w)) => format!("{w} TOOK {whose} {t}"),
                (FlagEvent::Dropped, Some(w)) if player == Some(me) => {
                    format!("{w} DROPPED THE {t}")
                }
                (FlagEvent::Dropped, _) => format!("{whose} {t} WAS DROPPED"),
                (FlagEvent::Returned, _) if team == NEUTRAL => "THE BALL WAS RESET".into(),
                (FlagEvent::Returned, _) => format!("{whose} {t} WAS RETURNED"),
                (FlagEvent::Captured, Some(w)) => format!("{w} CAPTURED {whose} FLAG"),
                (FlagEvent::CaptureFailed, _) if player == Some(me) => {
                    "YOUR FLAG MUST BE HOME TO SCORE".into()
                }
                (FlagEvent::Armed, _) if team == my_team => "YOUR BOMB IS ARMED".into(),
                (FlagEvent::Armed, _) => "THE ENEMY ARMED A BOMB IN YOUR BASE".into(),
                (FlagEvent::Detonated, _) => format!("{whose} BOMB WENT OFF"),
                (FlagEvent::Defused, Some(w)) => format!("{w} DEFUSED {whose} BOMB"),
                _ => return None,
            }
        }
        Event::Hill { player, what } => match (what, player) {
            (HillEvent::Moved, _) => "HILL MOVED".into(),
            (HillEvent::Contested, _) => "HILL CONTESTED".into(),
            (HillEvent::Controlled, Some(p)) if p == me => "YOU CONTROL THE HILL".into(),
            (HillEvent::Controlled, Some(p)) if teams && !game.is_enemy(me, p) => {
                "YOUR TEAM CONTROLS THE HILL".into()
            }
            (HillEvent::Controlled, Some(_)) if teams => "THE ENEMY CONTROLS THE HILL".into(),
            (HillEvent::Controlled, Some(p)) => format!("{} CONTROLS THE HILL", name(p)),
            _ => return None,
        },
        Event::Territory { team, from, .. } => {
            if team == my_team {
                "YOUR TEAM TOOK A TERRITORY".into()
            } else if from == Some(my_team) {
                "YOUR TEAM LOST A TERRITORY".into()
            } else {
                "THE ENEMY TOOK A TERRITORY".into()
            }
        }
        Event::Juggernaut { player } if player == me => "YOU ARE THE JUGGERNAUT".into(),
        Event::Juggernaut { player } => format!("{} IS THE JUGGERNAUT", name(player)),
        _ => return None,
    })
}

/// A team's flag colours for the change-colour shader.
pub fn flag_colors(team: u8) -> [[f32; 3]; 2] {
    [TEAM_COLORS[team as usize % TEAM_COLORS.len()], [1.0; 3]]
}

/// The cloth this frame. Its points are laid out with x up the pole from
/// the attachment marker and z out from the pole (negative); a wave rolls
/// out to the free edge, which droops a little.
pub fn cloth_vertices(flag: &FlagAssets, time: f32) -> Vec<Vertex> {
    let width = flag
        .rest
        .iter()
        .map(|p| -p.z)
        .fold(0.0f32, f32::max)
        .max(0.01);
    let mut front = Vec::with_capacity(flag.rest.len() * 2);
    let mut back = Vec::with_capacity(flag.rest.len());
    for (k, &p) in flag.rest.iter().enumerate() {
        let out = -p.z;
        let along = (out / width).clamp(0.0, 1.0);
        let phase = time * 7.0 - out * 14.0 + p.x * 4.0;
        let amp = 0.04 * along;
        let y = phase.sin() * amp;
        // How steeply the cloth bends across, for the normal.
        let slope = -phase.cos() * amp * 14.0 + phase.sin() * 0.04 / width;
        let at = Vec3::new(out * (1.0 - 0.06 * along), y, p.x - along * 0.03);
        let n = Vec3::new(-slope, 1.0, 0.0).normalize();
        front.push(Vertex::new(at.into(), n.into(), flag.uvs[k]));
        back.push(Vertex::new(at.into(), (-n).into(), flag.uvs[k]));
    }
    front.extend(back);
    front
}

/// A colour with alpha, from an armour or team colour brightened for light.
fn glow([r, g, b]: [f32; 3], alpha: f32) -> [f32; 4] {
    let m = r.max(g).max(b).max(0.01);
    let k = (1.0 / m).min(2.5);
    [r * k, g * k, b * k, alpha]
}

const NEUTRAL_GLOW: [f32; 3] = [1.0, 0.85, 0.4];

/// The colour of a side holding something: their team's, or in free-for-all
/// the player's armour.
fn side_color(game: &Game, player: usize) -> [f32; 3] {
    match game.players.get(player) {
        Some(p) if game.rules.game_type.teams() => TEAM_COLORS[p.team.min(1) as usize],
        Some(p) => armor_colors(p.look)[0],
        None => NEUTRAL_GLOW,
    }
}

/// Glowing dots along a line on the floor.
fn dotted(
    out: &mut Vec<SpriteVertex>,
    (a, b): (Vec3, Vec3),
    (r, u): (Vec3, Vec3),
    color: [f32; 4],
) {
    let n = ((b - a).length() / 0.3).ceil().max(1.0) as usize;
    for k in 0..n {
        let at = a.lerp(b, k as f32 / n as f32) + Vec3::Z * 0.06;
        effects::quad(out, at, r * 0.12, u * 0.12, color);
        let mut faint = color;
        faint[3] *= 0.35;
        effects::quad(out, at + Vec3::Z * 0.25, r * 0.1, u * 0.1, faint);
    }
}

impl App {
    /// The flags, balls and bombs not in anyone's hands, and the stands at
    /// the flags' bases.
    pub(crate) fn flag_draws(&self) -> Vec<DrawCall> {
        let (scene, game) = (&self.scene, &self.game);
        if !game.has_flags() {
            return Vec::new();
        }
        let ctf = game.rules.game_type == GameType::Ctf;
        let light_at = |p: Vec3| scene.level_light.at(&scene.textures, p + Vec3::Z * 0.2);
        let mesh = game
            .rules
            .carried_weapon()
            .and_then(|w| scene.weapons.get(w))
            .and_then(|w| w.world_mesh);
        let mut out = Vec::new();
        for f in &game.flags {
            if let (Some(stand), true) = (scene.flag.as_ref().and_then(|fl| fl.stand), ctf) {
                out.push(DrawCall {
                    mesh: stand,
                    model: Mat4::from_translation(f.home),
                    light: light_at(f.home),
                    colors: None,
                    emblem: None,
                });
            }
            if f.carrier.is_some() {
                continue;
            }
            let at = if ctf {
                f.position
            } else {
                // Balls and bombs sit a little above the floor.
                f.position + Vec3::Z * 0.1
            };
            let pole = Mat4::from_translation(at);
            let light = light_at(f.position);
            if let Some(mesh) = mesh {
                out.push(DrawCall {
                    mesh,
                    model: pole,
                    light,
                    colors: None,
                    emblem: None,
                });
            }
            if let (Some(flag), true) = (&scene.flag, ctf) {
                out.push(DrawCall {
                    mesh: flag.cloth,
                    model: pole * Mat4::from_translation(flag.attach),
                    light,
                    colors: Some(flag_colors(f.team)),
                    emblem: None,
                });
            }
        }
        out
    }

    /// Glowing outlines: the hill in play, the territories, a ticking bomb.
    pub(crate) fn objective_sprites(&self, r: Vec3, u: Vec3) -> Vec<SpriteVertex> {
        let game = &self.game;
        let mut out = Vec::new();
        let blink = (game.time * 3.0).fract() < 0.5;
        if let Some(hill) = game.current_hill() {
            let color = match game.hill_control {
                HillControl::Empty => glow(NEUTRAL_GLOW, 0.8),
                HillControl::Held(p) => glow(side_color(game, p), 0.9),
                HillControl::Contested if blink => [1.0, 1.0, 1.0, 0.9],
                HillControl::Contested => glow(NEUTRAL_GLOW, 0.5),
            };
            let n = hill.outline.len();
            for k in 0..n {
                dotted(
                    &mut out,
                    (hill.outline[k], hill.outline[(k + 1) % n]),
                    (r, u),
                    color,
                );
            }
        }
        if game.rules.game_type == GameType::Territories {
            for t in &game.territories {
                let color = match (t.owner, t.taking) {
                    (_, Some((team, _))) if blink => glow(TEAM_COLORS[team.min(1) as usize], 0.9),
                    (Some(team), _) => glow(TEAM_COLORS[team.min(1) as usize], 0.8),
                    (None, _) => [0.9, 0.9, 0.9, 0.6],
                };
                // The edge of the ground around its points.
                for (k, &p) in t.points.iter().enumerate() {
                    let steps = 40;
                    for s in 0..steps {
                        let a = s as f32 / steps as f32 * std::f32::consts::TAU;
                        let at = p + Vec3::new(a.cos(), a.sin(), 0.0) * Territory::RADIUS;
                        let covered = t.points.iter().enumerate().any(|(j, q)| {
                            j != k && (at - *q).truncate().length() < Territory::RADIUS - 0.05
                        });
                        if !covered {
                            effects::quad(&mut out, at + Vec3::Z * 0.06, r * 0.12, u * 0.12, color);
                        }
                    }
                }
            }
        }
        for f in game.flags.iter().filter(|f| f.armed.is_some()) {
            if blink {
                let at = f.position + Vec3::Z * 0.4;
                effects::quad(&mut out, at, r * 0.25, u * 0.25, [1.0, 0.2, 0.1, 0.9]);
            }
        }
        out
    }
}

/// The cloth on a flag held in third person, given where the pole is drawn.
pub fn carried_cloth(scene: &Scene, game: &Game, player: usize, pole: Mat4) -> Option<DrawCall> {
    if game.rules.game_type != GameType::Ctf {
        return None;
    }
    let flag = scene.flag.as_ref()?;
    let f = game.flags.iter().find(|f| f.carrier == Some(player))?;
    Some(DrawCall {
        mesh: flag.cloth,
        model: pole * Mat4::from_translation(flag.attach),
        light: None,
        colors: Some(flag_colors(f.team)),
        emblem: None,
    })
}

/// An arrow over something, with an icon above it.
struct Mark {
    at: Vec3,
    color: [f32; 3],
    icon: Option<usize>,
}

impl LocalPlayer {
    /// Arrows over what the game type is about: flags, the ball, bombs and
    /// where to take them, the hill, territories and the Juggernaut.
    pub fn objective_waypoints(
        &self,
        hb: &mut HudBuilder,
        scene: &Scene,
        game: &Game,
        (w, h): (f32, f32),
    ) {
        let Some(arrow) = scene.waypoint else {
            return;
        };
        let me = self.me(game);
        let kind = game.rules.game_type;
        let team_color = |t: u8| TEAM_COLORS[t as usize % TEAM_COLORS.len()];
        let mut marks: Vec<Mark> = Vec::new();
        if game.has_flags() {
            let icon = match kind {
                GameType::Ctf => scene.flag_icon,
                GameType::Assault => scene.bomb_icon,
                _ => scene.ball_icon,
            };
            for f in &game.flags {
                if f.carrier == Some(self.player) {
                    continue;
                }
                let color = match f.carrier {
                    Some(c) if f.team == NEUTRAL => side_color(game, c),
                    _ if f.team == NEUTRAL => NEUTRAL_GLOW,
                    _ => team_color(f.team),
                };
                marks.push(Mark {
                    at: f.position + Vec3::Z * 1.1,
                    color,
                    icon,
                });
            }
            if game.carried_flag(self.player).is_some() {
                let goal = match kind {
                    GameType::Ctf => game.flag_base(me.team),
                    GameType::Assault => game.bomb_target(me.team, me.body.position),
                    _ => None,
                };
                if let Some(at) = goal {
                    marks.push(Mark {
                        at: at + Vec3::Z * 0.9,
                        color: team_color(me.team),
                        icon: None,
                    });
                }
            }
        }
        if let Some(hill) = game.current_hill() {
            let color = match game.hill_control {
                HillControl::Held(p) => side_color(game, p),
                _ => NEUTRAL_GLOW,
            };
            marks.push(Mark {
                at: hill.centre() + Vec3::Z * 1.0,
                color,
                icon: None,
            });
        }
        if kind == GameType::Territories {
            for t in &game.territories {
                marks.push(Mark {
                    at: t.centre() + Vec3::Z * 1.0,
                    color: t.owner.map_or([0.8; 3], team_color),
                    icon: None,
                });
            }
        }
        if kind == GameType::Juggernaut {
            if let Some(j) = game.juggernaut.filter(|&j| j != self.player) {
                let p = &game.players[j];
                if p.alive {
                    marks.push(Mark {
                        at: p.body.position + Vec3::Z * 1.0,
                        color: [0.9, 0.5, 0.1],
                        icon: None,
                    });
                }
            }
        }
        let view_proj = self
            .camera
            .view_proj(w / h.max(1.0), self.magnification(scene, game));
        let s = hb.scale();
        for m in marks {
            let clip = view_proj * m.at.extend(1.0);
            if clip.w <= 0.01 {
                continue;
            }
            let ndc = clip.truncate() / clip.w;
            if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
                continue;
            }
            let (x, y) = ((ndc.x * 0.5 + 0.5) * w, (0.5 - ndc.y * 0.5) * h);
            let [r, g, b] = m.color;
            let color = [r * 1.4, g * 1.4, b * 1.4, 0.9];
            let size = 9.0 * s;
            hb.quad(
                arrow,
                [x - size, y - size * 1.6, x + size, y + size * 0.4],
                [0.0, 0.0, 1.0, 1.0],
                color,
                hud_mode::PLAIN,
                0.0,
            );
            if let Some(icon) = m.icon {
                let size = 11.0 * s;
                let top = y - 9.0 * s * 1.6 - size * 1.8;
                hb.quad(
                    icon,
                    [x - size, top, x + size, top + size * 2.0],
                    [0.0, 0.0, 1.0, 1.0],
                    color,
                    hud_mode::PLAIN,
                    0.0,
                );
            }
        }
    }

    /// What to do about the objective right here: take or drop what's
    /// carried, arm or defuse a bomb, or how a territory's taking is going.
    pub fn objective_prompt(&self, game: &Game) -> Option<String> {
        let me = self.me(game);
        if !me.alive {
            return None;
        }
        let kind = game.rules.game_type;
        let (take, drop) = if self.keyboard {
            ("E", "Q")
        } else {
            ("X", "Y")
        };
        let feet = me.body.position;
        let near = |p: Vec3| (p - feet).truncate().length() < 1.0 && (p.z - feet.z).abs() < 1.0;
        let percent = |done: f32| (done / game.rules.bomb_arm_time * 100.0).min(99.0) as u32;
        if kind == GameType::Territories {
            let taking = game.territories.iter().find_map(|t| match t.taking {
                Some((team, done)) if t.contains(feet) => Some((team, done)),
                _ => None,
            });
            if let Some((team, done)) = taking {
                let p = (done / game.rules.territory_capture_time * 100.0).min(99.0) as u32;
                let whose = if team == me.team { "TAKING" } else { "LOSING" };
                return Some(format!("{whose} TERRITORY {p}%"));
            }
            return None;
        }
        if !game.has_flags() {
            return None;
        }
        if let Some(f) = game.carried_flag(self.player) {
            return Some(match kind {
                GameType::Ctf => format!("TAKE THE FLAG HOME  {drop}: DROP IT"),
                GameType::Assault if game.flags[f].arming > 0.0 => {
                    format!("ARMING THE BOMB {}%", percent(game.flags[f].arming))
                }
                GameType::Assault => format!("ARM THE BOMB IN THE ENEMY BASE  {drop}: DROP IT"),
                _ => format!("HOLD ON TO THE BALL  {drop}: DROP IT"),
            });
        }
        for (k, f) in game.flags.iter().enumerate() {
            if !near(f.position) {
                continue;
            }
            if f.armed.is_some() && f.team != me.team {
                return Some(if f.arming > 0.0 {
                    format!("DEFUSING {}%", percent(f.arming))
                } else {
                    format!("HOLD {take} TO DEFUSE THE BOMB")
                });
            }
            if game.can_take(self.player, k) {
                let t = thing(kind);
                return Some(format!("{take} TO TAKE THE {t}"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use h2sim::testing::game;

    #[test]
    fn objective_messages() {
        let mut g = game();
        g.rules.game_type = GameType::TeamKing;
        let red = g.add_player_on(0);
        let blue = g.add_player_on(1);
        let hill = |player| Event::Hill {
            player: Some(player),
            what: HillEvent::Controlled,
        };
        assert_eq!(
            event_message(red, &g, &hill(red)).as_deref(),
            Some("YOU CONTROL THE HILL")
        );
        assert_eq!(
            event_message(red, &g, &hill(blue)).as_deref(),
            Some("THE ENEMY CONTROLS THE HILL")
        );
        g.rules.game_type = GameType::Oddball;
        let ball = Event::Flag {
            team: NEUTRAL,
            player: Some(blue),
            what: FlagEvent::Taken,
        };
        assert_eq!(
            event_message(red, &g, &ball).as_deref(),
            Some("PLAYER 2 HAS THE BALL")
        );
        g.rules.game_type = GameType::Assault;
        let armed = Event::Flag {
            team: 1,
            player: Some(blue),
            what: FlagEvent::Armed,
        };
        assert_eq!(
            event_message(red, &g, &armed).as_deref(),
            Some("THE ENEMY ARMED A BOMB IN YOUR BASE")
        );
        let territory = Event::Territory {
            index: 0,
            team: 1,
            from: Some(0),
        };
        assert_eq!(
            event_message(red, &g, &territory).as_deref(),
            Some("YOUR TEAM LOST A TERRITORY")
        );
    }
}
