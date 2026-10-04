//! Movement tuning read from the game's own tags, so the remake moves like Halo 2.

use crate::mapset::MapSet;
use crate::{f32_at, u32_at, DatumIndex, GroupTag, Result};

/// `matg` "Player Information": movement speeds in world units per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerMovement {
    pub walk: f32,
    pub run_forward: f32,
    pub run_backward: f32,
    pub run_sideways: f32,
    pub run_acceleration: f32,
    pub sneak_forward: f32,
    pub sneak_backward: f32,
    pub sneak_sideways: f32,
    pub sneak_acceleration: f32,
    pub airborne_acceleration: f32,
    /// Multiplier on the engine's base gravity.
    pub gravity_scale: f32,
}

/// Biped (`bipd`) physics for the player character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BipedPhysics {
    pub jump_velocity: f32,
    pub standing_camera_height: f32,
    pub crouching_camera_height: f32,
    pub height_standing: f32,
    pub height_crouching: f32,
    pub radius: f32,
    /// Radians; steeper surfaces are walls, not floors.
    pub max_slope: f32,
}

impl Default for PlayerMovement {
    /// Values from Halo 2 PC's globals, used when the tag can't be read.
    fn default() -> Self {
        PlayerMovement {
            walk: 0.512,
            run_forward: 2.25,
            run_backward: 2.0,
            run_sideways: 2.0,
            run_acceleration: 9.6,
            sneak_forward: 0.9,
            sneak_backward: 0.9,
            sneak_sideways: 0.9,
            sneak_acceleration: 9.6,
            airborne_acceleration: 1.05,
            gravity_scale: 1.0,
        }
    }
}

impl Default for BipedPhysics {
    /// Values from masterchief_mp in Halo 2 PC's shared.map.
    fn default() -> Self {
        BipedPhysics {
            jump_velocity: 3.08,
            standing_camera_height: 0.62,
            crouching_camera_height: 0.45,
            height_standing: 0.725,
            height_crouching: 0.5,
            radius: 0.175,
            max_slope: 0.872_664_6,
        }
    }
}

const MATG_PLAYER_CONTROL: usize = 0xF0;
const PLAYER_CONTROL_SIZE: usize = 0x80;
const MATG_PLAYER_INFORMATION: usize = 0x130;
const MATG_FALLING_DAMAGE: usize = 0x140;
const FALLING_DAMAGE_SIZE: usize = 0x68;
const PLAYER_INFORMATION_SIZE: usize = 0x11C;

pub fn player_movement(set: &mut MapSet) -> Result<PlayerMovement> {
    let matg = set.map.globals;
    let (src, _, data) = set.tag_data(matg)?;
    let file = set.get(src);
    let region = file.meta_region();
    let info = file.read_block(
        region,
        &data,
        MATG_PLAYER_INFORMATION,
        PLAYER_INFORMATION_SIZE,
    )?;
    let control = file.read_block(region, &data, MATG_PLAYER_CONTROL, PLAYER_CONTROL_SIZE)?;
    if info.len() < PLAYER_INFORMATION_SIZE {
        return Ok(PlayerMovement::default());
    }
    let gravity_scale = if control.len() >= PLAYER_CONTROL_SIZE {
        f32_at(&control, 0x68)
    } else {
        1.0
    };
    Ok(PlayerMovement {
        walk: f32_at(&info, 0x24),
        run_forward: f32_at(&info, 0x2C),
        run_backward: f32_at(&info, 0x30),
        run_sideways: f32_at(&info, 0x34),
        run_acceleration: f32_at(&info, 0x38),
        sneak_forward: f32_at(&info, 0x3C),
        sneak_backward: f32_at(&info, 0x40),
        sneak_sideways: f32_at(&info, 0x44),
        sneak_acceleration: f32_at(&info, 0x48),
        airborne_acceleration: f32_at(&info, 0x4C),
        gravity_scale: if gravity_scale > 0.0 {
            gravity_scale
        } else {
            1.0
        },
    })
}

/// `matg` "Falling Damage": how far a fall hurts and how far kills.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FallingDamage {
    /// Falls shorter than the first distance don't hurt; the damage grows
    /// to full at the second.
    pub harmful_distance: (f32, f32),
    /// The full damage of a harmful fall (`jpt!`).
    pub falling: DatumIndex,
    /// Falls longer than this kill outright, with the `distance` damage.
    pub maximum_distance: f32,
    pub distance: DatumIndex,
}

pub fn falling_damage(set: &mut MapSet) -> Result<Option<FallingDamage>> {
    let matg = set.map.globals;
    let (src, _, data) = set.tag_data(matg)?;
    let file = set.get(src);
    let region = file.meta_region();
    let block = file.read_block(region, &data, MATG_FALLING_DAMAGE, FALLING_DAMAGE_SIZE)?;
    if block.len() < FALLING_DAMAGE_SIZE {
        return Ok(None);
    }
    let datum = |at: usize| DatumIndex(u32_at(&block, at));
    Ok(Some(FallingDamage {
        harmful_distance: (f32_at(&block, 0x8), f32_at(&block, 0xC)),
        falling: datum(0x14),
        maximum_distance: f32_at(&block, 0x20),
        distance: datum(0x28),
    }))
}

/// Physics of the multiplayer Spartan (falls back to the campaign Master Chief).
pub fn player_biped(set: &mut MapSet) -> Result<BipedPhysics> {
    let found = biped_physics(set, "objects\\characters\\masterchief\\masterchief_mp")?
        .or(biped_physics(
            set,
            "objects\\characters\\masterchief\\masterchief",
        )?)
        .or(biped_physics(set, "objects\\characters\\dervish\\dervish")?);
    Ok(found.unwrap_or_default())
}

/// Physics of the multiplayer Elite, or the Arbiter in his missions.
pub fn elite_biped(set: &mut MapSet) -> Result<Option<BipedPhysics>> {
    Ok(biped_physics(set, "objects\\characters\\elite\\elite_mp")?
        .or(biped_physics(set, "objects\\characters\\dervish\\dervish")?))
}

/// Physics of a biped tag, by name.
pub fn biped_physics(set: &mut MapSet, name: &str) -> Result<Option<BipedPhysics>> {
    let bipd = GroupTag::parse("bipd").expect("valid group");
    let Some(datum) = set
        .map
        .tags
        .iter()
        .find(|t| t.group == bipd && t.name == name)
        .map(|t| t.datum)
    else {
        return Ok(None);
    };
    biped_physics_of(set, datum)
}

/// Physics of a biped tag.
pub fn biped_physics_of(set: &mut MapSet, datum: DatumIndex) -> Result<Option<BipedPhysics>> {
    let (_, _, d) = set.tag_data(datum)?;
    if d.len() < 0x2A0 {
        return Ok(None);
    }
    Ok(Some(BipedPhysics {
        jump_velocity: f32_at(&d, 0x1F8),
        standing_camera_height: f32_at(&d, 0x218),
        crouching_camera_height: f32_at(&d, 0x21C),
        height_standing: f32_at(&d, 0x268),
        height_crouching: f32_at(&d, 0x26C),
        radius: f32_at(&d, 0x270),
        max_slope: f32_at(&d, 0x29C),
    }))
}
