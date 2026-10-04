//! Drawing the game's vehicles: each posed for its state (wheels turning
//! and riding their suspension, turrets aiming) where the game has it.

use crate::gpu::DrawCall;
use crate::scene::{Scene, Vertex};
use glam::Vec3;
use h2sim::Game;

/// The vehicles this frame: what to draw, and the posed vertices of their
/// meshes.
pub fn draws(scene: &Scene, game: &Game) -> (Vec<DrawCall>, Vec<(usize, Vec<Vertex>)>) {
    let mut draws = Vec::new();
    let mut posed = Vec::new();
    for (k, v) in game.vehicles.iter().enumerate() {
        if v.destroyed {
            continue;
        }
        let (Some(def), Some(kind), Some(&(body, turret))) = (
            game.vehicle_defs.get(v.def),
            scene.vehicles.kinds.get(v.def),
            scene.vehicles.meshes.get(k),
        ) else {
            continue;
        };
        let (vertices, turret_vertices) = kind.pose(def, v);
        let model = v.transform(def);
        let light = scene
            .level_light
            .at(&scene.textures, v.center + Vec3::Z * 0.3);
        posed.push((body, vertices));
        draws.push(DrawCall {
            mesh: body,
            model,
            light,
            colors: None,
        });
        if let (Some(mesh), Some(vertices), Some((_, attach))) =
            (turret, turret_vertices, &kind.turret)
        {
            posed.push((mesh, vertices));
            draws.push(DrawCall {
                mesh,
                model: model * *attach,
                light,
                colors: None,
            });
        }
    }
    (draws, posed)
}
