//! Capture the Flag on screen: where a map's flags go, the flags on their
//! stands and in hand with their cloth waving, and the arrows over them.

use crate::gpu::{hud_mode, DrawCall};
use crate::hud::HudBuilder;
use crate::local::{LocalPlayer, TEAM_COLORS};
use crate::scene::{FlagAssets, Scene, Vertex};
use crate::App;
use blam_cache::scenario::NetgameFlagKind;
use glam::{Mat4, Vec3};
use h2sim::game::{FlagEvent, TEAMS};
use h2sim::Game;

/// Places on the map for each team: (team, where).
pub type TeamSpots = Vec<(u8, Vec3)>;

/// Where each team's flag stands, and where each team scores: the map's
/// CTF flag spawns and returns for red and blue.
pub fn flag_spots(scene: &Scene) -> (TeamSpots, TeamSpots) {
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
        match f.kind {
            NetgameFlagKind::CtfFlagSpawn if !homes.iter().any(|h: &(u8, Vec3)| h.0 == team) => {
                homes.push((team, at));
            }
            NetgameFlagKind::CtfFlagReturn => bases.push((team, at)),
            _ => {}
        }
    }
    homes.sort_by_key(|h| h.0);
    (homes, bases)
}

/// What a player at this PC is told when something happens to a flag.
pub fn flag_message(
    me: usize,
    my_team: u8,
    team: u8,
    player: Option<usize>,
    what: FlagEvent,
) -> Option<String> {
    let whose = if team == my_team { "YOUR" } else { "THE ENEMY" };
    let who = player.map(|p| crate::local::player_name(me, p));
    Some(match (what, who) {
        (FlagEvent::Taken, Some(w)) if player == Some(me) => format!("{w} TOOK THE FLAG"),
        (FlagEvent::Taken, Some(w)) => format!("{w} TOOK {whose} FLAG"),
        (FlagEvent::Dropped, Some(w)) if player == Some(me) => format!("{w} DROPPED THE FLAG"),
        (FlagEvent::Dropped, _) => format!("{whose} FLAG WAS DROPPED"),
        (FlagEvent::Returned, _) => format!("{whose} FLAG WAS RETURNED"),
        (FlagEvent::Captured, Some(w)) => format!("{w} CAPTURED {whose} FLAG"),
        (FlagEvent::CaptureFailed, _) if player == Some(me) => {
            "YOUR FLAG MUST BE HOME TO SCORE".into()
        }
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

impl App {
    /// The flags not in anyone's hands, and the stands at their bases.
    pub(crate) fn flag_draws(&self) -> Vec<DrawCall> {
        let (scene, game) = (&self.scene, &self.game);
        let Some(flag) = &scene.flag else {
            return Vec::new();
        };
        if !game.has_flags() {
            return Vec::new();
        }
        let light_at = |p: Vec3| scene.level_light.at(&scene.textures, p + Vec3::Z * 0.2);
        let mut out = Vec::new();
        for f in &game.flags {
            if let Some(stand) = flag.stand {
                out.push(DrawCall {
                    mesh: stand,
                    model: Mat4::from_translation(f.home),
                    light: light_at(f.home),
                    colors: None,
                });
            }
            if f.carrier.is_some() {
                continue;
            }
            let pole = Mat4::from_translation(f.position);
            let light = light_at(f.position);
            if let Some(mesh) = scene.weapons.get(flag.weapon).and_then(|w| w.world_mesh) {
                out.push(DrawCall {
                    mesh,
                    model: pole,
                    light,
                    colors: None,
                });
            }
            out.push(DrawCall {
                mesh: flag.cloth,
                model: pole * Mat4::from_translation(flag.attach),
                light,
                colors: Some(flag_colors(f.team)),
            });
        }
        out
    }
}

/// The cloth on a flag held in third person, given where the pole is drawn.
pub fn carried_cloth(scene: &Scene, game: &Game, player: usize, pole: Mat4) -> Option<DrawCall> {
    let flag = scene.flag.as_ref()?;
    let f = game.flags.iter().find(|f| f.carrier == Some(player))?;
    Some(DrawCall {
        mesh: flag.cloth,
        model: pole * Mat4::from_translation(flag.attach),
        light: None,
        colors: Some(flag_colors(f.team)),
    })
}

impl LocalPlayer {
    /// Arrows over the flags (and, carrying one, over home), in the team's
    /// colour.
    pub fn flag_waypoints(
        &self,
        hb: &mut HudBuilder,
        scene: &Scene,
        game: &Game,
        (w, h): (f32, f32),
    ) {
        let Some(arrow) = scene.waypoint else {
            return;
        };
        if !game.has_flags() {
            return;
        }
        let me = self.me(game);
        let view_proj = self
            .camera
            .view_proj(w / h.max(1.0), self.magnification(scene, game));
        let s = hb.scale();
        let mut marks: Vec<(Vec3, u8, bool)> = Vec::new();
        for f in &game.flags {
            if f.carrier == Some(self.player) {
                continue;
            }
            marks.push((f.position, f.team, true));
        }
        if game.carried_flag(self.player).is_some() {
            if let Some(base) = game.flag_base(me.team) {
                marks.push((base, me.team, false));
            }
        }
        for (at, team, is_flag) in marks {
            let above = at + Vec3::Z * if is_flag { 1.1 } else { 0.9 };
            let clip = view_proj * above.extend(1.0);
            if clip.w <= 0.01 {
                continue;
            }
            let ndc = clip.truncate() / clip.w;
            if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
                continue;
            }
            let (x, y) = ((ndc.x * 0.5 + 0.5) * w, (0.5 - ndc.y * 0.5) * h);
            let [r, g, b] = TEAM_COLORS[team as usize % TEAM_COLORS.len()];
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
            if let (Some(icon), true) = (scene.flag_icon, is_flag) {
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
}
