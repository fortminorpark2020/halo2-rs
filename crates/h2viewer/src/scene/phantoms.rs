//! The level's gravity lifts, vents and jump pads: the phantoms of the
//! objects it places that push players (`h2sim::phantom`).

use super::vehicles::{bind_pose, marker};
use super::{placement_matrix, Loader};
use blam_cache::scenario::{self, PlacedKind};
use blam_cache::{model, vehicle, DatumIndex};
use glam::Mat4;
use h2sim::phantom::Phantom;

impl Loader {
    /// Every phantom that pushes players, of the scenery, crates and
    /// machines that don't move that the level places from the start.
    pub(super) fn phantoms(&mut self) -> Vec<Phantom> {
        let mut out = Vec::new();
        for kind in [PlacedKind::Scenery, PlacedKind::Crate, PlacedKind::Machine] {
            for p in scenario::placements(&mut self.set, kind).unwrap_or_default() {
                let still = kind != PlacedKind::Machine
                    || scenario::machine(&mut self.set, p.object)
                        .is_ok_and(|m| m.position_time <= 0.0);
                if !p.automatic || !still {
                    continue;
                }
                let object = placement_matrix(p.position, p.rotation, p.scale);
                out.extend(self.object_phantoms(p.object, object));
            }
        }
        out
    }

    /// An object's phantoms that push players, the object placed by
    /// `object`.
    fn object_phantoms(&mut self, tag: DatumIndex, object: Mat4) -> Vec<Phantom> {
        let Ok(model) = model::object_model(&mut self.set, tag)
            .and_then(|hlmt| vehicle::read_model(&mut self.set, hlmt))
        else {
            return Vec::new();
        };
        if model.physics_model == DatumIndex::NONE {
            return Vec::new();
        }
        let phantoms = vehicle::read_phantoms(&mut self.set, model.physics_model)
            .unwrap_or_default()
            .into_iter()
            .filter(vehicle::Phantom::pushes_players)
            .collect::<Vec<_>>();
        if phantoms.is_empty() {
            return Vec::new();
        }
        let render = model::read_render_model(&mut self.set, model.render_model).ok();
        let bind = render.as_ref().map(bind_pose).unwrap_or_default();
        phantoms
            .iter()
            .map(|f| {
                let at = render
                    .as_ref()
                    .and_then(|m| marker(m, &bind, &f.marker))
                    .unwrap_or(Mat4::IDENTITY);
                Phantom::placed(f, object, at)
            })
            .collect()
    }
}
