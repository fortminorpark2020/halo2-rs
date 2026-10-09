//! The level's gravity lifts, vents and jump pads: the phantoms of the
//! objects it places that push players (`h2sim::phantom`).

use super::vehicles::{bind_pose, marker};
use super::{placement_matrix, Loader};
use blam_cache::scenario::{self, PlacedKind};
use blam_cache::{model, vehicle, DatumIndex};
use glam::Mat4;
use h2sim::phantom::Phantom;

/// Objects (by tag name) whose phantoms are left out though their tags
/// say they push players. Ascension's jump pad, read our way, throws
/// whoever steps onto it from the ledge just below off the level to their
/// death; in Halo 2 it's a way out, so its push is misread somehow.
const LEFT_OUT: &[&str] = &["ascension_jumppad"];

/// Whether the phantoms of an object the level places are loaded: those
/// of scenery and crates, and of machines that are neither platforms nor
/// gears (`scenario::Machine::kind` 0) and don't move. Relic's watchtower
/// is a platform that moves; standing still, its lift would hold a player
/// in mid-air.
fn loads_phantoms(kind: PlacedKind, machine: Option<&scenario::Machine>, name: &str) -> bool {
    let leaf = name.rsplit('\\').next().unwrap_or(name);
    if LEFT_OUT.contains(&leaf) {
        return false;
    }
    kind != PlacedKind::Machine || machine.is_some_and(|m| m.kind == 0 && m.position_time <= 0.0)
}

impl Loader {
    /// Every phantom that pushes players, of the scenery, crates and
    /// machines that don't move that the level places from the start
    /// (see `loads_phantoms`).
    pub(super) fn phantoms(&mut self) -> Vec<Phantom> {
        let mut out = Vec::new();
        for kind in [PlacedKind::Scenery, PlacedKind::Crate, PlacedKind::Machine] {
            for p in scenario::placements(&mut self.set, kind).unwrap_or_default() {
                let machine = (kind == PlacedKind::Machine)
                    .then(|| scenario::machine(&mut self.set, p.object).ok())
                    .flatten();
                let name = self
                    .set
                    .locate(p.object)
                    .map(|(_, t)| t.name)
                    .unwrap_or_default();
                if !p.automatic || !loads_phantoms(kind, machine.as_ref(), &name) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lifts_that_stay_put_and_land_you_somewhere_load() {
        let machine = |kind, position_time| scenario::Machine {
            kind,
            position_time,
            ..Default::default()
        };
        let multi = |name: &str| format!("scenarios\\objects\\multi\\{name}\\{name}");
        let tube = multi("lockout_lift_tube");
        assert!(loads_phantoms(PlacedKind::Crate, None, &tube));
        // Backwash's lifts are machines that don't move.
        let backwash = multi("backwash_lift");
        assert!(loads_phantoms(
            PlacedKind::Machine,
            Some(&machine(0, 0.0)),
            &backwash
        ));
        // Relic's watchtower and its pod are platforms; Gemini's columns
        // are gears that move.
        let pod = multi("cov_watchtower_pod");
        assert!(!loads_phantoms(
            PlacedKind::Machine,
            Some(&machine(1, 0.0)),
            &pod
        ));
        let column = multi("grav_column_gemini");
        assert!(!loads_phantoms(
            PlacedKind::Machine,
            Some(&machine(2, 20.0)),
            &column
        ));
        assert!(!loads_phantoms(
            PlacedKind::Machine,
            Some(&machine(0, 2.0)),
            &backwash
        ));
        assert!(!loads_phantoms(PlacedKind::Machine, None, &backwash));
        // Ascension's jump pad throws players off the level.
        let pad = multi("ascension_jumppad");
        assert!(!loads_phantoms(PlacedKind::Crate, None, &pad));
    }
}
