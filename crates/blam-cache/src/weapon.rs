//! Weapon (`weap`), projectile (`proj`) and damage (`jpt!`) tuning, read from
//! the game's tags so every gun fires like it did in Halo 2.

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};

const WEAP_ZOOM_LEVELS: usize = 0x1FE;
const WEAP_ZOOM_RANGE: usize = 0x200;
const WEAP_AUTOAIM_ANGLE: usize = 0x208;
const WEAP_AUTOAIM_RANGE: usize = 0x20C;
const WEAP_MAGNETISM_ANGLE: usize = 0x210;
const WEAP_MAGNETISM_RANGE: usize = 0x214;
const WEAP_READY_TIME: usize = 0x13C;
const WEAP_FLAGS: usize = 0x12C;
/// The first hit of a melee combo (the player melee damage field is unused).
const WEAP_MELEE_DAMAGE: usize = 0x1BC;
const WEAP_FIRST_PERSON: usize = 0x2A8;
const WEAP_HUD: usize = 0x2B0;
const FIRST_PERSON_SIZE: usize = 0x10;
const WEAP_MAGAZINES: usize = 0x2C0;
const MAGAZINE_SIZE: usize = 0x5C;
const WEAP_TRIGGERS: usize = 0x2C8;
const TRIGGER_SIZE: usize = 0x40;
const WEAP_BARRELS: usize = 0x2D0;
const BARREL_SIZE: usize = 0xEC;

const PROJ_FLAGS: usize = 0xBC;
const PROJ_ARMING_TIME: usize = 0xCC;
const PROJ_TIMER: usize = 0xD4;
const PROJ_MAX_RANGE: usize = 0xE0;
const PROJ_DETONATION_DAMAGE: usize = 0x100;
const PROJ_IMPACT_EFFECT: usize = 0x140;
const PROJ_IMPACT_DAMAGE: usize = 0x148;
const PROJ_AIR_GRAVITY: usize = 0x164;
const PROJ_AIR_DAMAGE_RANGE: usize = 0x168;
const PROJ_INITIAL_VELOCITY: usize = 0x17C;
const PROJ_FINAL_VELOCITY: usize = 0x180;
const PROJ_GUIDED_ANGULAR_VELOCITY: usize = 0x184;
const PROJ_ACCELERATION_RANGE: usize = 0x18C;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Magazine {
    pub rounds_total_initial: i16,
    pub rounds_total_maximum: i16,
    pub rounds_loaded_maximum: i16,
    /// Seconds to load a magazine.
    pub reload_time: f32,
    pub rounds_reloaded: i16,
    pub chamber_time: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerBehavior {
    /// Fires for as long as the trigger is held.
    #[default]
    Spew,
    /// One firing action per pull.
    Latch,
    /// Like latch, but keeps firing when held past the autofire time.
    LatchAutofire,
    Charge,
    LatchZoom,
    LatchRocketLauncher,
}

/// The control that pulls a trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerInput {
    #[default]
    Right,
    /// The left trigger: the Banshee's fuel rod, the Scorpion's machine gun.
    Left,
    Melee,
    /// Fires on its own (AI weapons).
    Automated,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Trigger {
    /// Which control pulls it.
    pub input: TriggerInput,
    pub behavior: TriggerBehavior,
    pub barrel: i16,
    pub autofire_time: f32,
    pub charging_time: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Barrel {
    /// Firing effects per second, from the first shot up to sustained fire.
    pub rounds_per_second: (f32, f32),
    pub acceleration_time: f32,
    pub deceleration_time: f32,
    /// Shots one pull of the trigger fires (the Battle Rifle's burst).
    pub shots_per_fire: (i16, i16),
    /// Seconds after a set of shots before the barrel can fire again.
    pub fire_recovery_time: f32,
    pub magazine: i16,
    pub rounds_per_shot: i16,
    pub projectiles_per_shot: i16,
    /// Radians.
    pub minimum_error: f32,
    /// Radians, from the first shot to sustained fire.
    pub error_angle: (f32, f32),
    pub error_acceleration_time: f32,
    pub error_deceleration_time: f32,
    /// Spread of a shotgun-style fan, radians.
    pub distribution_angle: f32,
    /// +x forward, +y left, +z up, relative to the camera.
    pub first_person_offset: [f32; 3],
    pub projectile: DatumIndex,
    /// Radians, first to last shot of sustained fire.
    pub angle_change_per_shot: (f32, f32),
    /// Spread and damage when the weapon is held in one of two hands.
    pub dual_minimum_error: f32,
    pub dual_error_angle: (f32, f32),
    pub dual_damage_scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Projectile {
    /// World units per second; zero means the shot is a hitscan.
    pub initial_velocity: f32,
    pub final_velocity: f32,
    pub maximum_range: f32,
    pub air_gravity_scale: f32,
    /// Damage falls off between these distances.
    pub air_damage_range: (f32, f32),
    pub impact_damage: DatumIndex,
    pub flags: u32,
    /// Seconds before it can go off.
    pub arming_time: f32,
    /// Seconds before it goes off by itself.
    pub timer: (f32, f32),
    /// The blast when it goes off (`jpt!`).
    pub detonation_damage: DatumIndex,
    /// What hitting something looks and sounds like (`effe`): for rockets,
    /// the explosion.
    pub impact_effect: Option<DatumIndex>,
    /// Radians per second it turns toward its target (rockets locked on to
    /// a vehicle, needles), at the start and the end of its flight.
    pub guided_angular_velocity: (f32, f32),
    /// Distances over which it speeds up from its initial to its final
    /// velocity (rockets).
    pub acceleration_range: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Damage {
    pub radius: (f32, f32),
    pub lower_bound: f32,
    pub upper_bound: (f32, f32),
    pub instantaneous_acceleration: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Weapon {
    pub datum: DatumIndex,
    pub name: String,
    /// First person render model and animations (Spartan entry).
    pub first_person_model: Option<DatumIndex>,
    pub first_person_animations: Option<DatumIndex>,
    /// The weapon's HUD (`nhdt`): crosshair, ammo meter, scope.
    pub hud: Option<DatumIndex>,
    /// The weapon flags (see [`Weapon::CAN_BE_DUAL_WIELDED`]).
    pub flags: u32,
    pub ready_time: f32,
    pub zoom_levels: i16,
    pub zoom_range: (f32, f32),
    /// Radians.
    pub autoaim_angle: f32,
    pub autoaim_range: f32,
    pub magnetism_angle: f32,
    pub magnetism_range: f32,
    /// The player's melee attack with this weapon (`jpt!`).
    pub melee_damage: Option<DatumIndex>,
    pub magazines: Vec<Magazine>,
    pub triggers: Vec<Trigger>,
    pub barrels: Vec<Barrel>,
}

impl Weapon {
    pub const CAN_BE_DUAL_WIELDED: u32 = 1 << 22;

    pub fn can_be_dual_wielded(&self) -> bool {
        self.flags & Self::CAN_BE_DUAL_WIELDED != 0
    }
}

fn range(b: &[u8], o: usize) -> (f32, f32) {
    (f32_at(b, o), f32_at(b, o + 4))
}

fn tag_ref(b: &[u8], o: usize) -> Option<DatumIndex> {
    Some(DatumIndex(u32_at(b, o + 4))).filter(|d| *d != DatumIndex::NONE)
}

pub fn read_weapon(set: &mut MapSet, weap: DatumIndex) -> Result<Weapon> {
    let (src, tag, d) = set.tag_data(weap)?;
    if d.len() < WEAP_BARRELS + 8 {
        return Err(Error::Corrupt(format!("weapon tag {} too short", tag.name)));
    }
    let file = set.get(src);
    let region = file.meta_region();
    let fp = file.read_block(region, &d, WEAP_FIRST_PERSON, FIRST_PERSON_SIZE)?;
    let mags = file.read_block(region, &d, WEAP_MAGAZINES, MAGAZINE_SIZE)?;
    let trigs = file.read_block(region, &d, WEAP_TRIGGERS, TRIGGER_SIZE)?;
    let barrels = file.read_block(region, &d, WEAP_BARRELS, BARREL_SIZE)?;

    let first = fp.as_chunks::<FIRST_PERSON_SIZE>().0.first();
    let magazines = mags
        .as_chunks::<MAGAZINE_SIZE>()
        .0
        .iter()
        .map(|m| Magazine {
            rounds_total_initial: i16_at(m, 0x6),
            rounds_total_maximum: i16_at(m, 0x8),
            rounds_loaded_maximum: i16_at(m, 0xA),
            reload_time: f32_at(m, 0x10),
            rounds_reloaded: i16_at(m, 0x14),
            chamber_time: f32_at(m, 0x18),
        })
        .collect();
    let triggers = trigs
        .as_chunks::<TRIGGER_SIZE>()
        .0
        .iter()
        .map(|t| Trigger {
            input: match i16_at(t, 0x4) {
                1 => TriggerInput::Left,
                2 => TriggerInput::Melee,
                3 => TriggerInput::Automated,
                _ => TriggerInput::Right,
            },
            behavior: match i16_at(t, 0x6) {
                1 => TriggerBehavior::Latch,
                2 => TriggerBehavior::LatchAutofire,
                3 => TriggerBehavior::Charge,
                4 => TriggerBehavior::LatchZoom,
                5 => TriggerBehavior::LatchRocketLauncher,
                _ => TriggerBehavior::Spew,
            },
            barrel: i16_at(t, 0x8),
            autofire_time: f32_at(t, 0x10),
            charging_time: f32_at(t, 0x1C),
        })
        .collect();
    let barrels = barrels
        .as_chunks::<BARREL_SIZE>()
        .0
        .iter()
        .map(|b| Barrel {
            rounds_per_second: range(b, 0x4),
            acceleration_time: f32_at(b, 0xC),
            deceleration_time: f32_at(b, 0x10),
            shots_per_fire: (i16_at(b, 0x1C), i16_at(b, 0x1E)),
            fire_recovery_time: f32_at(b, 0x20),
            magazine: i16_at(b, 0x28),
            rounds_per_shot: i16_at(b, 0x2A),
            error_acceleration_time: f32_at(b, 0x38),
            error_deceleration_time: f32_at(b, 0x3C),
            projectiles_per_shot: i16_at(b, 0x6A),
            distribution_angle: f32_at(b, 0x6C),
            minimum_error: f32_at(b, 0x70),
            error_angle: range(b, 0x74),
            first_person_offset: [f32_at(b, 0x7C), f32_at(b, 0x80), f32_at(b, 0x84)],
            projectile: tag_ref(b, 0x8C).unwrap_or(DatumIndex::NONE),
            angle_change_per_shot: range(b, 0xB0),
            dual_minimum_error: f32_at(b, 0x58),
            dual_error_angle: range(b, 0x5C),
            dual_damage_scale: f32_at(b, 0x64),
        })
        .collect();
    Ok(Weapon {
        datum: weap,
        name: tag.name.clone(),
        first_person_model: first.and_then(|f| tag_ref(f, 0x0)),
        first_person_animations: first.and_then(|f| tag_ref(f, 0x8)),
        hud: tag_ref(&d, WEAP_HUD),
        flags: u32_at(&d, WEAP_FLAGS),
        ready_time: f32_at(&d, WEAP_READY_TIME),
        zoom_levels: i16_at(&d, WEAP_ZOOM_LEVELS),
        zoom_range: range(&d, WEAP_ZOOM_RANGE),
        autoaim_angle: f32_at(&d, WEAP_AUTOAIM_ANGLE),
        autoaim_range: f32_at(&d, WEAP_AUTOAIM_RANGE),
        magnetism_angle: f32_at(&d, WEAP_MAGNETISM_ANGLE),
        magnetism_range: f32_at(&d, WEAP_MAGNETISM_RANGE),
        melee_damage: tag_ref(&d, WEAP_MELEE_DAMAGE),
        magazines,
        triggers,
        barrels,
    })
}

/// Effects a weapon plays, each an `effe` (which may play sounds) or a
/// `snd!` tag.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeaponEffects {
    pub fire: Option<DatumIndex>,
    /// Pulling the trigger with nothing loaded.
    pub empty: Option<DatumIndex>,
    pub reload: Option<DatumIndex>,
    pub ready: Option<DatumIndex>,
    pub pickup: Option<DatumIndex>,
    pub zoom_in: Option<DatumIndex>,
    pub zoom_out: Option<DatumIndex>,
}

const WEAP_READY_EFFECT: usize = 0x140;
const WEAP_PICKUP_SOUND: usize = 0x264;
const WEAP_ZOOM_IN_SOUND: usize = 0x26C;
const WEAP_ZOOM_OUT_SOUND: usize = 0x274;
const MAGAZINE_RELOAD_EFFECT: usize = 0x34;
const BARREL_FIRING_EFFECTS: usize = 0xE4;
const FIRING_EFFECT_SIZE: usize = 0x34;

/// The effects of a weapon firing its barrel `barrel` (and reloading, readying...).
pub fn read_weapon_effects(
    set: &mut MapSet,
    weap: DatumIndex,
    barrel: usize,
) -> Result<WeaponEffects> {
    let (src, tag, d) = set.tag_data(weap)?;
    if d.len() < WEAP_BARRELS + 8 {
        return Err(Error::Corrupt(format!("weapon tag {} too short", tag.name)));
    }
    let file = set.get(src);
    let region = file.meta_region();
    let mags = file.read_block(region, &d, WEAP_MAGAZINES, MAGAZINE_SIZE)?;
    let barrels = file.read_block(region, &d, WEAP_BARRELS, BARREL_SIZE)?;
    let all = barrels.as_chunks::<BARREL_SIZE>().0;
    let firing = match all.get(barrel).or(all.first()) {
        Some(b) => file.read_block(region, b, BARREL_FIRING_EFFECTS, FIRING_EFFECT_SIZE)?,
        None => Vec::new(),
    };
    let first_firing = firing.as_chunks::<FIRING_EFFECT_SIZE>().0.first();
    Ok(WeaponEffects {
        fire: first_firing.and_then(|f| tag_ref(f, 0x4)),
        empty: first_firing.and_then(|f| tag_ref(f, 0x14)),
        reload: mags
            .as_chunks::<MAGAZINE_SIZE>()
            .0
            .first()
            .and_then(|m| tag_ref(m, MAGAZINE_RELOAD_EFFECT)),
        ready: tag_ref(&d, WEAP_READY_EFFECT),
        pickup: tag_ref(&d, WEAP_PICKUP_SOUND),
        zoom_in: tag_ref(&d, WEAP_ZOOM_IN_SOUND),
        zoom_out: tag_ref(&d, WEAP_ZOOM_OUT_SOUND),
    })
}

pub fn read_projectile(set: &mut MapSet, proj: DatumIndex) -> Result<Projectile> {
    let (_, tag, d) = set.tag_data(proj)?;
    if d.len() < PROJ_ACCELERATION_RANGE + 8 {
        return Err(Error::Corrupt(format!(
            "projectile tag {} too short",
            tag.name
        )));
    }
    Ok(Projectile {
        initial_velocity: f32_at(&d, PROJ_INITIAL_VELOCITY),
        final_velocity: f32_at(&d, PROJ_FINAL_VELOCITY),
        maximum_range: f32_at(&d, PROJ_MAX_RANGE),
        air_gravity_scale: f32_at(&d, PROJ_AIR_GRAVITY),
        air_damage_range: range(&d, PROJ_AIR_DAMAGE_RANGE),
        impact_damage: tag_ref(&d, PROJ_IMPACT_DAMAGE).unwrap_or(DatumIndex::NONE),
        flags: u32_at(&d, PROJ_FLAGS),
        arming_time: f32_at(&d, PROJ_ARMING_TIME),
        timer: range(&d, PROJ_TIMER),
        detonation_damage: tag_ref(&d, PROJ_DETONATION_DAMAGE).unwrap_or(DatumIndex::NONE),
        impact_effect: tag_ref(&d, PROJ_IMPACT_EFFECT),
        guided_angular_velocity: range(&d, PROJ_GUIDED_ANGULAR_VELOCITY),
        acceleration_range: range(&d, PROJ_ACCELERATION_RANGE),
    })
}

pub fn read_damage(set: &mut MapSet, jpt: DatumIndex) -> Result<Damage> {
    let (_, tag, d) = set.tag_data(jpt)?;
    if d.len() < 0x44 {
        return Err(Error::Corrupt(format!("damage tag {} too short", tag.name)));
    }
    Ok(Damage {
        radius: range(&d, 0x0),
        lower_bound: f32_at(&d, 0x1C),
        upper_bound: range(&d, 0x20),
        instantaneous_acceleration: f32_at(&d, 0x40),
    })
}
