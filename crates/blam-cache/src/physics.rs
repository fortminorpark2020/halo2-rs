//! Movement tuning read from the game's own tags, so the remake moves like Halo 2.

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, GroupTag, Result};

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
    /// Seconds the action button is held to pick up or swap a weapon, or
    /// get in or out of a vehicle (player control's minimum action hold
    /// time).
    pub action_hold: f32,
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
    /// Flies (Sentinels, Drones): no gravity, and it goes up and down.
    pub flying: bool,
    /// Its origin is its middle, not its feet (Sentinels).
    pub centered: bool,
    /// Flying: top speed forward, and sideways or up and down, and how
    /// fast it gets up to speed and slows.
    pub fly_speed: f32,
    pub fly_sidestep: f32,
    pub fly_acceleration: f32,
    pub fly_deceleration: f32,
    /// Landing from a fall: coming down at least `soft_landing_speed`
    /// (world units a second) is a soft landing, at least
    /// `hard_landing_speed` a hard one; each lasts at most its time in
    /// seconds (0 for no limit).
    pub soft_landing_speed: f32,
    pub hard_landing_speed: f32,
    pub soft_landing_time: f32,
    pub hard_landing_time: f32,
    /// The unit's "camera field of view", radians: 70 degrees for every
    /// biped and vehicle in the multiplayer maps. Halo's field of view
    /// is horizontal, across a 4:3 screen.
    pub camera_field_of_view: f32,
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
            action_hold: 0.2333,
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
            flying: false,
            centered: false,
            fly_speed: 0.0,
            fly_sidestep: 0.0,
            fly_acceleration: 0.0,
            fly_deceleration: 0.0,
            // As the multiplayer Spartan's biped tag has them.
            soft_landing_speed: 1.5,
            hard_landing_speed: 7.0,
            soft_landing_time: 0.6,
            hard_landing_time: 0.0,
            camera_field_of_view: 70f32.to_radians(),
        }
    }
}

const MATG_PLAYER_CONTROL: usize = 0xF0;
const PLAYER_CONTROL_SIZE: usize = 0x80;
/// The player control block's look function: a block of reals.
const PLAYER_CONTROL_LOOK_FUNCTION: usize = 0x74;
/// Its look autolevelling scale, minimum autolevelling ticks (a short)
/// and minimum action hold time.
const PLAYER_CONTROL_AUTOLEVEL_SCALE: usize = 0x5C;
const PLAYER_CONTROL_AUTOLEVEL_TICKS: usize = 0x6E;
const PLAYER_CONTROL_ACTION_HOLD: usize = 0x7C;
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
    let whole = control.len() >= PLAYER_CONTROL_SIZE;
    let gravity_scale = if whole { f32_at(&control, 0x68) } else { 1.0 };
    let action_hold = action_hold_from(&control);
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
        action_hold,
    })
}

/// The player control block's minimum action hold time (Halo 2's if
/// the block is short or the time makes no sense).
fn action_hold_from(control: &[u8]) -> f32 {
    Some(control)
        .filter(|c| c.len() >= PLAYER_CONTROL_SIZE)
        .map(|c| f32_at(c, PLAYER_CONTROL_ACTION_HOLD))
        .filter(|&t| t > 0.0 && t < 2.0)
        .unwrap_or(PlayerMovement::default().action_hold)
}

/// `matg` "Player Control": where the crosshair sits, and how a
/// controller's stick looks around and its aim assist pulls.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerControl {
    /// How much the look slows with the crosshair on an enemy (0..1).
    pub magnetism_friction: f32,
    /// How much of an enemy's movement across the view the crosshair
    /// follows (0..1).
    pub magnetism_adhesion: f32,
    /// Where the crosshair sits, from -1 to 1 across and down the view
    /// (0 is the middle; Halo 2's is 0.165, a little below it).
    pub crosshair: [f32; 2],
    /// Turning and looking up and down at full stick, radians per second
    /// (the tag gives degrees).
    pub look_yaw_rate: f32,
    pub look_pitch_rate: f32,
    /// The stick counts as pegged past this (0..1).
    pub look_peg_threshold: f32,
    /// Held pegged, the look speeds up to `scale` times over `time`
    /// seconds: (time, scale), for turning and for looking up and down.
    pub yaw_acceleration: (f32, f32),
    pub pitch_acceleration: (f32, f32),
    /// The look speed (0..1 of the rate) at evenly spaced points of the
    /// stick's travel, from the middle to the edge.
    pub look_function: Vec<f32>,
    /// Automatic Look Centering: how strongly the view levels out, and
    /// how many game ticks (of 30 a second) the player moves forward
    /// without looking up or down before it starts.
    pub autolevel_scale: f32,
    pub autolevel_ticks: u16,
}

impl Default for PlayerControl {
    /// Values from Halo 2 PC's globals (the same in every multiplayer
    /// map), used when the tag can't be read.
    fn default() -> Self {
        PlayerControl {
            magnetism_friction: 0.6,
            magnetism_adhesion: 0.7,
            crosshair: [0.0, 0.165],
            look_yaw_rate: 120f32.to_radians(),
            look_pitch_rate: 60f32.to_radians(),
            look_peg_threshold: 0.85,
            yaw_acceleration: (0.8, 2.5),
            pitch_acceleration: (0.8, 2.5),
            look_function: vec![0.0, 0.05, 0.1, 0.25, 0.58, 1.0],
            autolevel_scale: 0.5,
            autolevel_ticks: 15,
        }
    }
}

pub fn player_control(set: &mut MapSet) -> Result<PlayerControl> {
    let matg = set.map.globals;
    let (src, _, data) = set.tag_data(matg)?;
    let file = set.get(src);
    let region = file.meta_region();
    let control = file.read_block(region, &data, MATG_PLAYER_CONTROL, PLAYER_CONTROL_SIZE)?;
    if control.len() < PLAYER_CONTROL_SIZE {
        return Ok(PlayerControl::default());
    }
    let curve = file.read_block(region, &control, PLAYER_CONTROL_LOOK_FUNCTION, 4)?;
    Ok(player_control_from(&control, &curve))
}

/// Player control from the globals' block and its look function's.
fn player_control_from(control: &[u8], curve: &[u8]) -> PlayerControl {
    let f = |at: usize| f32_at(control, at);
    let mut look_function: Vec<f32> = curve
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    if look_function.len() < 2 {
        look_function = PlayerControl::default().look_function;
    }
    PlayerControl {
        magnetism_friction: f(0x0),
        magnetism_adhesion: f(0x4),
        crosshair: [f(0x18), f(0x1C)],
        look_pitch_rate: f(0x40).to_radians(),
        look_yaw_rate: f(0x44).to_radians(),
        look_peg_threshold: f(0x48),
        yaw_acceleration: (f(0x4C), f(0x50)),
        pitch_acceleration: (f(0x54), f(0x58)),
        look_function,
        autolevel_scale: f(PLAYER_CONTROL_AUTOLEVEL_SCALE),
        autolevel_ticks: i16_at(control, PLAYER_CONTROL_AUTOLEVEL_TICKS).max(0) as u16,
    }
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
        flying: u32_at(&d, 0x264) & (1 << 4) != 0,
        centered: u32_at(&d, 0x264) & 1 != 0,
        fly_speed: f32_at(&d, 0x2DC),
        fly_sidestep: f32_at(&d, 0x2E0),
        fly_acceleration: f32_at(&d, 0x2E4),
        fly_deceleration: f32_at(&d, 0x2E8),
        soft_landing_time: f32_at(&d, 0x1FC),
        hard_landing_time: f32_at(&d, 0x200),
        soft_landing_speed: f32_at(&d, 0x204),
        hard_landing_speed: f32_at(&d, 0x208),
        camera_field_of_view: f32_at(&d, 0xCC),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_control_reads_the_globals_layout() {
        // Halo 2's own block (lockout.map's), with its look function.
        let mut block = vec![0u8; PLAYER_CONTROL_SIZE];
        for (at, v) in [
            (0x0, 0.6f32),
            (0x4, 0.7),
            (0x8, 0.5),
            (0x1C, 0.165),
            (0x40, 60.0),
            (0x44, 120.0),
            (0x48, 0.85),
            (0x4C, 0.8),
            (0x50, 2.5),
            (0x54, 0.8),
            (0x58, 2.5),
            (0x5C, 0.5),
            (0x7C, 0.2333),
        ] {
            block[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        // The minimum autolevelling ticks, a short.
        block[0x6E..0x70].copy_from_slice(&15i16.to_le_bytes());
        let curve: Vec<u8> = [0.0f32, 0.05, 0.1, 0.25, 0.58, 1.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let c = player_control_from(&block, &curve);
        assert_eq!(c, PlayerControl::default());
        assert!((c.look_yaw_rate.to_degrees() - 120.0).abs() < 1e-4);
        // No look function: Halo 2's.
        let c = player_control_from(&block, &[]);
        assert_eq!(c.look_function, PlayerControl::default().look_function);
        // The action button's hold, 7 ticks.
        assert_eq!(action_hold_from(&block), 0.2333);
        assert_eq!(PlayerMovement::default().action_hold, 0.2333);
        block[0x7C..0x80].copy_from_slice(&0f32.to_le_bytes());
        assert_eq!(action_hold_from(&block), 0.2333);
        block[0x7C..0x80].copy_from_slice(&0.5f32.to_le_bytes());
        assert_eq!(action_hold_from(&block), 0.5);
        assert_eq!(action_hold_from(&block[..0x40]), 0.2333);
    }
}
