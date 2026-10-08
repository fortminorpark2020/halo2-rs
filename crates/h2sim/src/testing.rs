//! A flat test level and a small game on it, for tests here and in other
//! crates.

use crate::game::{Game, ItemKind, ItemSpawn, Rules};
use crate::weapon::WeaponDef;
use crate::World;
use blam_cache::physics::{BipedPhysics, PlayerMovement};
use blam_cache::weapon::TriggerBehavior;
use glam::Vec3;

pub fn floor() -> World {
    let s = 50.0;
    World::new(
        &[[-s, -s, 0.0], [s, -s, 0.0], [s, s, 0.0], [-s, s, 0.0]],
        &[0, 1, 2, 0, 2, 3],
    )
}

fn rifle() -> WeaponDef {
    WeaponDef {
        name: "rifle".into(),
        input: Default::default(),
        behavior: TriggerBehavior::Latch,
        rounds_per_second: (10.0, 10.0),
        rate_acceleration_time: 0.0,
        shots_per_fire: 1,
        fire_recovery_time: 0.0,
        keeps_firing: false,
        soft_recovery: 0.0,
        magazine_size: 12,
        initial_rounds: 36,
        maximum_rounds: 108,
        reload_time: 1.0,
        rounds_per_shot: 1,
        projectiles_per_shot: 1,
        error_angle: (0.0, 0.0),
        minimum_error: 0.0,
        error_only_unzoomed: false,
        error_acceleration_time: 0.0,
        error_deceleration_time: 0.0,
        distribution_angle: 0.0,
        zoom_levels: 0,
        zoom_range: (1.0, 1.0),
        autoaim_angle: 0.0,
        autoaim_range: 0.0,
        magnetism_angle: 0.0,
        magnetism_range: 0.0,
        autoaim_zoomed_only: false,
        range: 100.0,
        velocity: 0.0,
        damage: 20.0,
        damage_range: (0.0, 100.0),
        damage_lower_bound: 20.0,
        ready_time: crate::weapon::DEFAULT_READY_TIME,
        melee_damage: None,
        dual: None,
        flight: None,
        armor: Default::default(),
    }
}

pub fn game() -> Game {
    let rules = Rules {
        starting_weapons: vec![0],
        headshot_weapons: vec![0],
        ..Rules::default()
    };
    let movement = PlayerMovement {
        run_forward: 2.25,
        run_backward: 2.0,
        run_sideways: 2.0,
        run_acceleration: 9.6,
        ..PlayerMovement::default()
    };
    let biped = BipedPhysics {
        jump_velocity: 3.0,
        standing_camera_height: 0.62,
        crouching_camera_height: 0.45,
        height_standing: 0.725,
        height_crouching: 0.5,
        radius: 0.175,
        max_slope: 0.87,
        ..BipedPhysics::default()
    };
    Game::new(
        rules,
        vec![rifle(), rifle()],
        vec![
            (Vec3::new(0.0, 0.0, 0.0), 0.0),
            (Vec3::new(5.0, 0.0, 0.0), 0.0),
        ],
        vec![ItemSpawn {
            kind: ItemKind::Weapon(1),
            position: Vec3::new(0.0, 3.0, 0.0),
            respawn: 30.0,
        }],
        movement,
        biped,
    )
}
