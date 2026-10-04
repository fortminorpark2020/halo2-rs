//! Campaign actors from the map's tags: each character's body, how tough
//! it is and how it fights (`char`), the squads the mission places
//! (`scnr`), and the weapons they carry.

use super::{Arms2, Body, Loader, MeshData, WeaponAssets};
use blam_cache::ai::{self, Character, CharacterKind, Squad};
use blam_cache::physics::{self, BipedPhysics};
use blam_cache::DatumIndex;
use h2sim::game::{CharacterDef, GrenadeKind, Mind, Side, Vitality};

/// Mesh copies of each actor body: how many of one kind show at once.
pub const ACTOR_BODIES: usize = 16;

/// A mission's actors: its characters (by the scenario's character
/// palette), their bodies, its squads and the weapons squads carry.
#[derive(Default)]
pub struct CampaignAi {
    pub characters: Vec<CharacterDef>,
    /// Each character's body, by index into `bodies`.
    pub body_of: Vec<Option<usize>>,
    pub bodies: Vec<Body>,
    /// Each character's armour colours.
    pub colors: Vec<[[f32; 3]; 2]>,
    /// Each character's accuracy on legendary (the defs have normal's).
    pub legendary_accuracy: Vec<(f32, f32)>,
    pub squads: Vec<Squad>,
    /// The scenario's weapon palette, as weapon indices.
    pub weapons: Vec<Option<usize>>,
}

/// Which side a kind of character fights on.
fn side(kind: CharacterKind) -> Side {
    use CharacterKind::*;
    match kind {
        Marine | Player | Crew => Side::Human,
        CombatForm | InfectionForm | CarrierForm | Juggernaut => Side::Flood,
        Monitor | Sentinel => Side::Sentinel,
        _ => Side::Covenant,
    }
}

/// Armour colours by rank, from the character's name (Elite majors are
/// red, ultras white...).
fn rank_colors(name: &str, kind: CharacterKind) -> [[f32; 3]; 2] {
    let rank = name.rsplit('\\').next().unwrap_or(name);
    let has = |s: &str| rank.contains(s);
    let c = match kind {
        CharacterKind::Elite if has("zealot") => [0.85, 0.65, 0.2],
        CharacterKind::Elite if has("ultra") => [0.85, 0.85, 0.88],
        CharacterKind::Elite if has("major") => [0.75, 0.15, 0.12],
        CharacterKind::Elite if has("specops") || has("stealth") => [0.2, 0.2, 0.24],
        CharacterKind::Elite => [0.2, 0.3, 0.8],
        CharacterKind::Grunt if has("ultra") => [0.75, 0.75, 0.8],
        CharacterKind::Grunt if has("major") || has("heavy") => [0.75, 0.15, 0.1],
        CharacterKind::Grunt if has("specops") => [0.2, 0.2, 0.24],
        CharacterKind::Grunt => [0.9, 0.5, 0.15],
        CharacterKind::Jackal if has("major") => [0.75, 0.15, 0.1],
        CharacterKind::Jackal => [0.25, 0.35, 0.75],
        CharacterKind::Marine => [0.36, 0.42, 0.26],
        _ => [0.5, 0.5, 0.5],
    };
    [c, c.map(|v| v * 0.6)]
}

fn vitality(body: f32, shield: f32, recharge: f32) -> Vitality {
    Vitality {
        shield: shield.max(0.0),
        // Some characters leave it to their biped: about a Spartan's.
        health: if body > 0.0 { body } else { 45.0 },
        recharge: if recharge > 0.0 { recharge } else { 5.0 },
    }
}

/// A character's def for the game: its side, vitality and mind.
fn character_def(
    c: &Character,
    biped: BipedPhysics,
    weapon: Option<usize>,
) -> (CharacterDef, (f32, f32)) {
    let v = &c.vitality;
    let p = &c.perception;
    let w = c.weapon.unwrap_or_default();
    let positive = |x: f32, or: f32| if x > 0.0 { x } else { or };
    let defaults = Mind::default();
    let g = c.grenades.unwrap_or_default();
    let mind = Mind {
        sight: positive(p.vision_distance, defaults.sight),
        fov: positive(p.vision_angle, defaults.fov),
        peripheral: positive(p.peripheral_distance, defaults.peripheral),
        fire_range: positive(w.max_range, defaults.fire_range),
        combat_range: if w.combat_range.1 > 0.0 {
            w.combat_range
        } else {
            defaults.combat_range
        },
        accuracy: if w.accuracy.1 > 0.0 {
            w.accuracy
        } else {
            defaults.accuracy
        },
        melee_range: positive(c.melee_range, defaults.melee_range),
        melee_chance: c.melee_chance.max(0.0),
        grenade_chance: c.grenades.map_or(0.0, |g| g.chance.max(0.0)),
        grenade_delay: positive(g.delay, defaults.grenade_delay),
        grenade_range: if g.range.1 > 0.0 {
            g.range
        } else {
            defaults.grenade_range
        },
        skittish: c.kind == CharacterKind::Grunt,
    };
    let legendary = if w.legendary_accuracy.1 > 0.0 {
        w.legendary_accuracy
    } else {
        mind.accuracy
    };
    let def = CharacterDef {
        name: c.name.rsplit('\\').next().unwrap_or(&c.name).to_uppercase(),
        side: side(c.kind),
        biped,
        vitality: vitality(v.body, v.shield, v.shield_recharge_time),
        legendary: vitality(
            positive(v.legendary_body, v.body),
            positive(v.legendary_shield, v.shield),
            v.shield_recharge_time,
        ),
        mind,
        weapon,
        grenade: if g.kind == 1 {
            GrenadeKind::Plasma
        } else {
            GrenadeKind::Frag
        },
        grenades: (g.count.0 as u32 + g.count.1 as u32).div_ceil(2).min(4) as u8,
    };
    (def, legendary)
}

impl Loader {
    /// A weapon by tag: one already loaded, else loaded now.
    fn weapon_by_tag(
        &mut self,
        tag: DatumIndex,
        weapons: &mut Vec<WeaponAssets>,
        arms: Arms2<'_>,
        meshes: &mut Vec<MeshData>,
    ) -> Option<usize> {
        if let Some(i) = weapons.iter().position(|w| w.tag == tag) {
            return Some(i);
        }
        let name = self.set.locate(tag).map(|(_, t)| t.name)?;
        let w = self.weapon(&name, arms, meshes)?;
        weapons.push(w);
        Some(weapons.len() - 1)
    }

    /// The mission's characters, squads and the weapons they carry.
    pub(super) fn campaign_ai(
        &mut self,
        fallback: BipedPhysics,
        weapons: &mut Vec<WeaponAssets>,
        arms: Arms2<'_>,
        meshes: &mut Vec<MeshData>,
    ) -> CampaignAi {
        let mut out = CampaignAi {
            squads: ai::squads(&mut self.set).unwrap_or_else(|e| {
                println!("warning: squads: {e}");
                Vec::new()
            }),
            ..CampaignAi::default()
        };
        let palette = ai::weapon_palette(&mut self.set).unwrap_or_default();
        out.weapons = palette
            .iter()
            .map(|&w| self.weapon_by_tag(w, weapons, arms, meshes))
            .collect();
        let mut units: Vec<DatumIndex> = Vec::new();
        for tag in ai::character_palette(&mut self.set).unwrap_or_default() {
            let c = match ai::read_character(&mut self.set, tag) {
                Ok(c) => c,
                Err(e) => {
                    println!("warning: character {:08x}: {e}", tag.0);
                    Character::default()
                }
            };
            let biped = c
                .unit
                .and_then(|u| physics::biped_physics_of(&mut self.set, u).ok().flatten())
                .unwrap_or(fallback);
            // Its own weapon, if it's one carried by hand.
            let weapon = c
                .weapon
                .and_then(|w| w.weapon)
                .filter(|&w| {
                    self.set
                        .locate(w)
                        .is_some_and(|(_, t)| t.name.starts_with("objects\\weapons\\"))
                })
                .and_then(|w| self.weapon_by_tag(w, weapons, arms, meshes));
            let (def, legendary) = character_def(&c, biped, weapon);
            out.colors.push(rank_colors(&c.name, c.kind));
            out.legendary_accuracy.push(legendary);
            out.characters.push(def);
            let body = c.unit.and_then(|u| {
                if let Some(k) = units.iter().position(|&x| x == u) {
                    return Some(k);
                }
                let body = self.body_of(u, ["right_hand", "left_hand"], ACTOR_BODIES, meshes)?;
                units.push(u);
                out.bodies.push(body);
                Some(out.bodies.len() - 1)
            });
            out.body_of.push(body);
        }
        out
    }
}
