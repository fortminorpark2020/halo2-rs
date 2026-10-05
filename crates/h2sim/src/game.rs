//! A multiplayer game: Spartans with shields and health, two-weapon
//! inventories, grenades, items lying on the map, damage, deaths, respawns
//! and the score. Rendering and input live elsewhere; each tick takes one
//! command per player, so local splitscreen and networked players drive the
//! game the same way.

use crate::collision::{KillZone, World};
use crate::player::{Input, Player};
use crate::vehicle::{Vehicle, VehicleDef};
use crate::weapon::{ArmorScale, Blast, WeaponDef, WeaponInput, WeaponState};
use blam_cache::physics::{BipedPhysics, PlayerMovement};
use glam::{Vec2, Vec3};

mod campaign;
mod ctf;
mod dual;
mod juggernaut;
mod options;
mod powerups;
mod projectiles;
mod sensor;
mod sync;
mod teleporters;
mod vehicles;
mod zones;
pub use campaign::{ActorSpawn, CharacterDef, Mind, Side};
pub use ctf::{Flag, FlagEvent, NEUTRAL};
pub use options::{MapWeapons, Options};
pub use powerups::Powerup;
pub use projectiles::{Homing, Projectile, StuckRound};
pub use sensor::{Blip, SENSOR_RANGE};
pub use sync::{Malformed, Reader, Writer};
pub use teleporters::Teleporter;
pub use vehicles::{VehicleAction, VehicleSpawn};
pub use zones::{Hill, HillControl, HillEvent, Territory};

/// Simulation step: the game advances in fixed ticks so every machine in a
/// networked game computes the same thing.
pub const TICK: f32 = 1.0 / 60.0;

/// Longest gamertag, in characters (Xbox Live allowed 15).
pub const MAX_NAME: usize = 15;

/// Halo 2 has 18 profile colours (White, Steel, Red ... Tan).
pub const PROFILE_COLORS: u8 = 18;
/// Armour colours players start with, by player number, so everyone looks
/// different: red, blue, green, orange, purple, gold, brown, pink...
pub const DEFAULT_COLORS: [[u8; 2]; 12] = [
    [2, 1],
    [11, 0],
    [6, 1],
    [3, 16],
    [13, 0],
    [4, 1],
    [16, 17],
    [14, 0],
    [0, 2],
    [1, 11],
    [8, 1],
    [15, 1],
];

/// Marks a gamertag can have besides letters, digits and spaces: those the
/// game's font draws, so no two names that look the same differ.
const NAME_MARKS: &str = "/:.-<>'!?,()+";

/// `c` can be in a gamertag (in either case).
pub fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == ' ' || NAME_MARKS.contains(c)
}

/// A name as gamertags are shown here: capitals, digits, spaces and the
/// marks the font has, at most `MAX_NAME` of them, with no spaces at either
/// end. Cleaning a clean name changes nothing.
pub fn clean_name(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|&c| name_char(c))
        .skip_while(|&c| c == ' ')
        .take(MAX_NAME)
        .collect();
    name.trim_end().to_ascii_uppercase()
}

/// A splitscreen guest's name: "NAME(1)", "NAME(2)" and so on after the
/// person signed in (guest 0), as Xbox Live named guests.
pub fn guest_name(name: &str, guest: usize) -> String {
    if guest == 0 {
        return clean_name(name);
    }
    let tag = format!("({guest})");
    let base: String = clean_name(name)
        .chars()
        .take(MAX_NAME - tag.len())
        .collect();
    format!("{}{tag}", base.trim_end())
}

/// Halo 2's emblem pieces: 64 foreground pictures over 32 backgrounds.
pub const EMBLEM_FOREGROUNDS: u8 = 64;
pub const EMBLEM_BACKGROUNDS: u8 = 32;

/// A player's emblem: a foreground picture in two colours over a
/// background pattern in a third.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Emblem {
    pub foreground: u8,
    pub background: u8,
    /// Primary, secondary and background colour (profile colours).
    pub colors: [u8; 3],
}

impl Emblem {
    /// A varied emblem from any number (player numbers, name hashes).
    pub fn from_number(n: u32) -> Emblem {
        let c = PROFILE_COLORS as u32;
        let primary = (n >> 3) % c;
        let mut background = (n >> 9) % c;
        if background == primary {
            background = (primary + c / 2) % c;
        }
        Emblem {
            foreground: (n % EMBLEM_FOREGROUNDS as u32) as u8,
            background: ((n >> 6) % EMBLEM_BACKGROUNDS as u32) as u8,
            colors: [primary as u8, ((n >> 14) % c) as u8, background as u8],
        }
    }

    fn clamped(self) -> Emblem {
        Emblem {
            foreground: self.foreground % EMBLEM_FOREGROUNDS,
            background: self.background % EMBLEM_BACKGROUNDS,
            colors: self.colors.map(|c| c % PROFILE_COLORS),
        }
    }
}

/// How a player looks: a Spartan or an Elite, in two of the profile
/// colours, and their emblem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub elite: bool,
    /// Primary and secondary armour colour, indices into the profile
    /// colours.
    pub colors: [u8; 2],
    pub emblem: Emblem,
}

impl Look {
    /// The look player number `i` starts with.
    pub fn default_for(i: usize) -> Look {
        Look {
            elite: false,
            colors: DEFAULT_COLORS[i % DEFAULT_COLORS.len()],
            emblem: Emblem::from_number((i as u32).wrapping_mul(0x9E37_79B9) >> 7),
        }
    }

    /// A splitscreen guest's look: the same species and emblem in colours
    /// of their own.
    pub fn guest(self, guest: usize) -> Look {
        if guest == 0 {
            return self;
        }
        let shift = (guest * 5) as u8;
        Look {
            colors: self.colors.map(|c| (c + shift) % PROFILE_COLORS),
            ..self
        }
    }

    /// Every value in range.
    pub fn clamped(self) -> Look {
        Look {
            elite: self.elite,
            colors: self.colors.map(|c| c % PROFILE_COLORS),
            emblem: self.emblem.clamped(),
        }
    }

    pub fn write(&self, w: &mut Writer) {
        w.u8(self.elite as u8);
        w.u8(self.colors[0]);
        w.u8(self.colors[1]);
        let e = &self.emblem;
        w.u8(e.foreground);
        w.u8(e.background);
        for c in e.colors {
            w.u8(c);
        }
    }

    pub fn read(r: &mut Reader) -> Result<Look, Malformed> {
        let look = Look {
            elite: r.u8()? != 0,
            colors: [r.u8()?, r.u8()?],
            emblem: Emblem {
                foreground: r.u8()?,
                background: r.u8()?,
                colors: [r.u8()?, r.u8()?, r.u8()?],
            },
        };
        Ok(look.clamped())
    }
}

/// How hard grenade blasts throw people (world units per second).
const GRENADE_PUSH: f32 = 1.5;
/// Gravity for thrown grenades, world units per second squared.
const GRENADE_GRAVITY: f32 = crate::player::GRAVITY;
/// How far in front of the eye a melee reaches.
const MELEE_RANGE: f32 = 0.9;
/// The energy sword's lunge reaches further.
const LUNGE_RANGE: f32 = 2.2;
/// Melee needs the target within this angle of where the player looks.
const MELEE_CONE: f32 = 0.6;
const MELEE_COOLDOWN: f32 = 0.8;
const PICKUP_RADIUS: f32 = 0.45;
/// Hold the action key this long to swap weapons.
const SWAP_HOLD: f32 = 0.25;
const DROPPED_WEAPON_LIFETIME: f32 = 60.0;
const GRENADE_THROW_COOLDOWN: f32 = 0.6;
/// Below this distance from the top of the body, a hit is a headshot.
const HEAD_HEIGHT: f32 = 0.16;
/// Seconds between kills that still chain into a multi-kill.
const MULTI_KILL_WINDOW: f64 = 4.0;
/// A killing spree medal every this many kills without dying.
const SPREE_STEP: u32 = 5;
/// The last spree medal.
const SPREE_MAX: u32 = 25;

/// The kind of game being played.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GameType {
    /// Everyone for themselves; the most kills wins.
    #[default]
    Slayer,
    /// Red against blue; the team with the most kills wins.
    TeamSlayer,
    /// Red against blue; bring the other team's flag to your base.
    Ctf,
    /// Everyone for themselves; stand alone in the hill to score.
    KingOfTheHill,
    /// Red against blue; hold the hill with no enemy in it to score.
    TeamKing,
    /// Everyone for themselves; hold the ball to score.
    Oddball,
    /// Red against blue; a teammate holding the ball scores.
    TeamOddball,
    /// Kill the Juggernaut to become it; only the Juggernaut's kills (and
    /// killing the Juggernaut) score.
    Juggernaut,
    /// Red against blue; take territories and hold them to score.
    Territories,
    /// Red against blue; carry your bomb into the enemy base and arm it.
    Assault,
    /// A campaign mission: the players (alone or side by side) against
    /// the Covenant. Not played over the network.
    Campaign,
}

impl GameType {
    /// Every game type, in the order they're numbered over the network.
    pub const ALL: [GameType; 10] = [
        GameType::Slayer,
        GameType::TeamSlayer,
        GameType::Ctf,
        GameType::KingOfTheHill,
        GameType::TeamKing,
        GameType::Oddball,
        GameType::TeamOddball,
        GameType::Juggernaut,
        GameType::Territories,
        GameType::Assault,
    ];

    pub fn teams(self) -> bool {
        matches!(
            self,
            GameType::TeamSlayer
                | GameType::Ctf
                | GameType::TeamKing
                | GameType::TeamOddball
                | GameType::Territories
                | GameType::Assault
                | GameType::Campaign
        )
    }

    /// Kills score points (and suicides and betrayals cost them).
    pub fn kills_score(self) -> bool {
        matches!(self, GameType::Slayer | GameType::TeamSlayer)
    }

    /// Points are seconds: holding the hill, the ball or territories.
    pub fn timed(self) -> bool {
        matches!(
            self,
            GameType::KingOfTheHill
                | GameType::TeamKing
                | GameType::Oddball
                | GameType::TeamOddball
                | GameType::Territories
        )
    }

    /// Played over hills.
    pub fn king(self) -> bool {
        matches!(self, GameType::KingOfTheHill | GameType::TeamKing)
    }

    /// Played with a ball.
    pub fn oddball(self) -> bool {
        matches!(self, GameType::Oddball | GameType::TeamOddball)
    }
}

/// Teams in team games: red and blue.
pub const TEAMS: u8 = 2;

/// Damage values and timings, from the game's tags where the map has them.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub game_type: GameType,
    /// Teammates can hurt each other (team games).
    pub friendly_fire: bool,
    pub shield: f32,
    pub health: f32,
    /// Seconds without damage before shields start to recharge.
    pub shield_delay: f32,
    /// Seconds for empty shields to fill.
    pub shield_recharge: f32,
    pub respawn_time: f32,
    /// Seconds an overshield takes to drain away, and active camouflage lasts.
    pub overshield_time: f32,
    pub camo_time: f32,
    /// Weapons (indexes into the weapon list) every player spawns with.
    pub starting_weapons: Vec<usize>,
    pub starting_frags: u8,
    pub starting_plasmas: u8,
    pub max_grenades: u8,
    pub melee_damage: f32,
    pub lunge_damage: f32,
    pub frag: GrenadeDef,
    pub plasma: GrenadeDef,
    /// Weapons whose headshots kill once shields are down.
    pub headshot_weapons: Vec<usize>,
    /// Weapons that lunge and kill with melee (the energy sword).
    pub lunge_weapons: Vec<usize>,
    /// Points to win (in Slayer a kill is one, a suicide or betrayal takes
    /// one away; in CTF a capture is one); 0 plays forever.
    pub score_to_win: u32,
    /// Seconds of play before time runs out and the best score wins (a tie
    /// for it is a draw); 0 for no time limit.
    pub time_limit: u32,
    /// The flag, the ball and the bomb, as weapons carried in hand
    /// (indexes into the weapon list).
    pub flag_weapon: Option<usize>,
    pub ball_weapon: Option<usize>,
    pub bomb_weapon: Option<usize>,
    /// Seconds a dropped flag lies before going home by itself.
    pub flag_reset_time: f32,
    /// A team only scores while its own flag is at home.
    pub flag_at_home_to_score: bool,
    /// Touching your own dropped flag sends it home.
    pub flag_touch_return: bool,
    /// Seconds a bomb carrier stands at the enemy base to arm it, and
    /// seconds from then until it goes off.
    pub bomb_arm_time: f32,
    pub bomb_fuse: f32,
    /// Seconds before the hill moves to the next one (0: it stays).
    pub hill_move_time: f32,
    /// Seconds a team stands in a territory alone to take it.
    pub territory_capture_time: f32,
    pub falling: FallingDamage,
    /// Game variant settings every PC needs (see `Game::apply_options`).
    pub options: Options,
}

impl Rules {
    /// What this game type's carried objects are held as in hand.
    pub fn carried_weapon(&self) -> Option<usize> {
        match self.game_type {
            GameType::Oddball | GameType::TeamOddball => self.ball_weapon,
            GameType::Assault => self.bomb_weapon,
            _ => self.flag_weapon,
        }
    }
}

/// How hard landings hurt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FallingDamage {
    /// Falls shorter than the first height are harmless; the damage grows
    /// to `damage` at the second.
    pub harmful: (f32, f32),
    pub damage: f32,
    /// Falls longer than this kill.
    pub deadly: f32,
}

impl FallingDamage {
    /// The damage of landing from a fall of `height`.
    pub fn damage_for(&self, height: f32) -> f32 {
        let (low, high) = self.harmful;
        if height > self.deadly {
            return f32::INFINITY;
        }
        if height <= low {
            return 0.0;
        }
        self.damage * ((height - low) / (high - low).max(1e-3)).min(1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrenadeDef {
    /// Throw speed, world units per second.
    pub speed: f32,
    /// Seconds from the first bounce (or sticking) to the explosion.
    pub fuse: f32,
    pub sticks: bool,
    pub damage: f32,
    /// Full damage inside the first radius, none beyond the second.
    pub radius: (f32, f32),
    pub armor: ArmorScale,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            game_type: GameType::Slayer,
            friendly_fire: true,
            shield: 70.0,
            health: 45.0,
            shield_delay: 5.0,
            shield_recharge: 2.0,
            respawn_time: 5.0,
            // Halo 2's power-up tags.
            overshield_time: 60.0,
            camo_time: 45.0,
            starting_weapons: Vec::new(),
            starting_frags: 2,
            starting_plasmas: 0,
            max_grenades: 4,
            melee_damage: 60.0,
            lunge_damage: 150.0,
            frag: GrenadeDef {
                speed: 7.0,
                fuse: 1.0,
                sticks: false,
                damage: 150.0,
                radius: (0.75, 1.75),
                armor: ArmorScale::EXPLOSION,
            },
            plasma: GrenadeDef {
                speed: 7.0,
                fuse: 1.5,
                sticks: true,
                damage: 120.0,
                radius: (0.75, 1.5),
                armor: ArmorScale::EXPLOSION,
            },
            headshot_weapons: Vec::new(),
            lunge_weapons: Vec::new(),
            score_to_win: 25,
            time_limit: 0,
            flag_weapon: None,
            ball_weapon: None,
            bomb_weapon: None,
            flag_reset_time: 30.0,
            flag_at_home_to_score: false,
            flag_touch_return: false,
            bomb_arm_time: 3.0,
            bomb_fuse: 4.0,
            hill_move_time: 60.0,
            territory_capture_time: 6.0,
            // Halo 2's globals.
            falling: FallingDamage {
                harmful: (6.0, 10.0),
                damage: 125.0,
                deadly: 14.0,
            },
            options: Options::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// Index into the game's weapon list.
    Weapon(usize),
    FragGrenades,
    PlasmaGrenades,
    Powerup(Powerup),
    /// An ammo pack for a weapon: rounds for whoever carries it (0 for a
    /// full load).
    Ammo {
        weapon: usize,
        rounds: u32,
    },
}

/// A place items appear on the map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemSpawn {
    pub kind: ItemKind,
    pub position: Vec3,
    pub respawn: f32,
}

/// One player's controls for a tick.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Command {
    /// x: right, y: forward, each in -1..=1.
    pub movement: Vec2,
    /// Radians: yaw around +z (0 = +x), pitch up from level.
    pub yaw: f32,
    pub pitch: f32,
    pub jump: bool,
    pub crouch: bool,
    pub fire: bool,
    pub zoom: bool,
    pub reload: bool,
    pub melee: bool,
    /// Pick up / swap weapons (held).
    pub action: bool,
    pub switch_weapon: bool,
    pub throw_grenade: bool,
    pub switch_grenade: bool,
    /// The flashlight button: the Arbiter's active camouflage.
    pub vision: bool,
    /// Flying: up (1) or down (-1).
    pub rise: f32,
}

#[derive(Debug, Clone)]
pub struct HeldWeapon {
    pub weapon: usize,
    pub state: WeaponState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrenadeKind {
    Frag,
    Plasma,
}

/// How tough someone is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vitality {
    pub shield: f32,
    pub health: f32,
    /// Seconds for empty shields to charge fully.
    pub recharge: f32,
}

/// What makes a player slot a campaign actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    /// Index into the game's characters.
    pub character: usize,
    /// The scenario squad it was placed with, and at which of its
    /// starting locations.
    pub squad: u16,
    pub location: u16,
    /// Can be hurt but not killed (a script says so).
    pub immortal: bool,
    /// Taken out of the level by a script (not dead: there's no body).
    pub gone: bool,
}

#[derive(Debug, Clone)]
pub struct Spartan {
    /// The gamertag shown in the scoreboard and kill messages.
    pub name: String,
    /// Spartan or Elite, and their armour colours.
    pub look: Look,
    pub body: Player,
    pub yaw: f32,
    pub pitch: f32,
    /// Above the rules' shield while an overshield lasts.
    pub shield: f32,
    pub health: f32,
    /// Shields and health when full, and seconds for shields to recharge.
    pub full: Vitality,
    /// A campaign actor (a Grunt, a Marine...): not a player, and never
    /// respawns.
    pub actor: Option<Actor>,
    since_damage: f32,
    /// Shield an overshield picked up has yet to charge.
    overshield_charge: f32,
    /// Seconds of active camouflage left.
    pub camo: f32,
    /// Seconds until the Arbiter's own camouflage can come on again.
    pub camo_recharge: f32,
    /// How much firing and getting hurt give a camouflaged player away (0-1).
    pub reveal: f32,
    /// Came out of a teleporter and hasn't stepped off its pad yet (so a
    /// two-way one doesn't send them straight back).
    teleported: bool,
    pub alive: bool,
    /// Seconds until respawning, while dead.
    pub respawn_in: f32,
    /// At most two; `current` is in hand.
    pub weapons: Vec<HeldWeapon>,
    pub current: usize,
    /// A second one-handed weapon, in the left hand (dual wielding).
    pub left: Option<HeldWeapon>,
    /// A flag carried in hand in place of the weapons.
    pub objective: Option<HeldWeapon>,
    pub frags: u8,
    pub plasmas: u8,
    pub grenade: GrenadeKind,
    /// In team games: 0 red, 1 blue.
    pub team: u8,
    /// Kills, less suicides and betrayals.
    pub score: i32,
    pub kills: u32,
    pub deaths: u32,
    /// Kills since last spawning.
    pub spree: u32,
    /// Kills in the current multi-kill chain, and when the last one was.
    pub multi_kill: u32,
    pub last_kill: f64,
    /// Seconds left bringing the weapon in hand up.
    pub readying: f32,
    /// Time held toward the next point in timed games.
    hold: f32,
    melee_cooldown: f32,
    grenade_cooldown: f32,
    action_held: f32,
    /// Seconds the switch button has been held (negative once acted on).
    switch_held: f32,
    /// The vehicle and seat ridden in.
    pub seat: Option<(usize, usize)>,
    /// Seconds the action key has been held by a vehicle (negative once
    /// acted on).
    board_held: f32,
    /// Dropped from a dropship: the next landing doesn't hurt.
    soft_landing: bool,
    last: Command,
}

impl Spartan {
    /// Whether the action button was held last tick.
    pub fn holding_action(&self) -> bool {
        self.last.action
    }

    /// What's in hand: a flag being carried, or the current weapon.
    pub fn held(&self) -> Option<&HeldWeapon> {
        self.objective.as_ref().or(self.weapons.get(self.current))
    }

    pub fn eye(&self) -> Vec3 {
        self.body.eye()
    }

    /// Unit view direction.
    pub fn aim(&self) -> Vec3 {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        Vec3::new(self.yaw.cos() * cp, self.yaw.sin() * cp, sp)
    }

    /// Aim basis: forward, right, up.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.aim();
        let r = f.cross(Vec3::Z).normalize_or(Vec3::X);
        (f, r, r.cross(f))
    }

    /// Look where `cmd` aims, and walk, jump and crouch as it says (riders
    /// go where their vehicle takes them). A joined PC moves its own
    /// players this way too, so they answer their controls at once.
    pub fn move_with(&mut self, world: &World, cmd: &Command, dt: f32) {
        self.yaw = cmd.yaw;
        self.pitch = cmd.pitch.clamp(-1.5, 1.5);
        if self.seat.is_none() {
            let input = Input {
                movement: cmd.movement,
                yaw: cmd.yaw,
                jump: cmd.jump,
                crouch: cmd.crouch,
                lift: cmd.rise,
            };
            self.body.update(world, input, dt);
        }
    }
}

/// What walking over a weapon took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Taken {
    Ammo,
    Weapon,
}

/// A weapon on the ground: a map item, or one a player dropped.
#[derive(Debug, Clone)]
pub struct DroppedWeapon {
    pub weapon: usize,
    pub state: WeaponState,
    pub position: Vec3,
    pub yaw: f32,
    pub(crate) ttl: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grenade {
    pub kind: GrenadeKind,
    pub owner: usize,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Seconds to detonation once armed by a bounce.
    pub fuse: Option<f32>,
    /// Stuck to a player.
    pub stuck: Option<usize>,
    stuck_offset: Vec3,
    age: f32,
}

/// Things that happened during a tick, for sounds, effects and animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Shot {
        player: usize,
        /// The weapon fired (index into the game's weapons).
        weapon: usize,
        /// Fired by the left hand's weapon.
        left: bool,
        origin: Vec3,
        direction: Vec3,
        /// Where it hit the level or a player, and the surface normal.
        hit: Option<(Vec3, Vec3)>,
        hit_player: Option<usize>,
    },
    Reloaded {
        player: usize,
        left: bool,
        empty: bool,
    },
    Switched {
        player: usize,
    },
    Melee {
        player: usize,
        hit: Option<usize>,
    },
    Thrown {
        player: usize,
    },
    Exploded {
        kind: GrenadeKind,
        position: Vec3,
    },
    /// A round that flew (plasma, a rocket...) hit something, or went off.
    Impact {
        weapon: usize,
        position: Vec3,
        normal: Vec3,
        hit_player: Option<usize>,
        exploded: bool,
    },
    Damaged {
        player: usize,
        amount: f32,
    },
    Killed {
        killer: Option<usize>,
        victim: usize,
        headshot: bool,
    },
    Spawned {
        player: usize,
        /// The way they face on appearing.
        yaw: f32,
    },
    PickedUp {
        player: usize,
        kind: ItemKind,
    },
    /// Sent by a teleporter from one place to another.
    Teleported {
        player: usize,
        from: Vec3,
        to: Vec3,
    },
    /// Pulled the trigger with no ammo left at all.
    DryFire {
        player: usize,
    },
    /// Earned a medal the announcer calls out.
    Medal {
        player: usize,
        medal: Medal,
    },
    /// Took, lost or tied the lead in kills.
    Lead {
        player: usize,
        change: LeadChange,
    },
    /// Something happened to a team's flag; `player` took, dropped,
    /// returned or captured it (none: it went home by itself).
    Flag {
        team: u8,
        player: Option<usize>,
        what: FlagEvent,
    },
    /// The hill moved, or someone took it or contested it.
    Hill {
        player: Option<usize>,
        what: HillEvent,
    },
    /// A team took a territory (from `from`, if another team held it).
    Territory {
        index: usize,
        team: u8,
        from: Option<u8>,
    },
    /// A new Juggernaut.
    Juggernaut {
        player: usize,
    },
    /// Got into a vehicle's seat.
    Entered {
        player: usize,
        vehicle: usize,
        seat: usize,
    },
    Exited {
        player: usize,
        vehicle: usize,
        seat: usize,
    },
    /// Boarded an enemy's seat, throwing them out (or killing them).
    Hijacked {
        player: usize,
        victim: usize,
        vehicle: usize,
        seat: usize,
    },
    /// Run over by a vehicle.
    Splattered {
        player: usize,
        vehicle: usize,
    },
    VehicleDestroyed {
        vehicle: usize,
        position: Vec3,
    },
    /// A vehicle back at its spawn point.
    VehicleSpawned {
        vehicle: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medal {
    /// Kills chained within a few seconds of each other: 2 (double kill)
    /// up to 7 (Killimanjaro) and beyond.
    MultiKill(u8),
    /// Kills without dying: 5, 10, 15, 20 and 25.
    Spree(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadChange {
    Gained,
    Lost,
    Tied,
}

pub struct Game {
    pub rules: Rules,
    pub weapons: Vec<WeaponDef>,
    pub spawns: Vec<(Vec3, f32)>,
    pub item_spawns: Vec<ItemSpawn>,
    /// Per item spawn: seconds until it is back (0 = lying there).
    pub item_timers: Vec<f32>,
    pub dropped: Vec<DroppedWeapon>,
    pub grenades: Vec<Grenade>,
    /// Rounds in flight (rockets, plasma, needles...).
    pub projectiles: Vec<Projectile>,
    /// Needles stuck in people.
    pub stuck: Vec<StuckRound>,
    pub players: Vec<Spartan>,
    /// Teams that side with each other though they differ (a mission's
    /// Sentinels with the heretics...), as pairs.
    pub allegiances: Vec<(u8, u8)>,
    /// Capture the Flag: each team's flag.
    pub flags: Vec<Flag>,
    /// Where each team brings the enemy flag to score.
    pub flag_bases: Vec<(u8, Vec3)>,
    /// Places that kill whoever enters them.
    pub kill_zones: Vec<KillZone>,
    pub teleporters: Vec<Teleporter>,
    /// King of the Hill: the map's hills, which one is in play and how long
    /// until it moves, and who holds it.
    pub hills: Vec<Hill>,
    pub hill: usize,
    pub hill_moves_in: f32,
    pub hill_control: HillControl,
    /// Territories: the map's territories and who holds them.
    pub territories: Vec<Territory>,
    /// Juggernaut: who it is.
    pub juggernaut: Option<usize>,
    /// The kinds of vehicle on the map, where each vehicle appears, and
    /// the vehicles themselves (one per spawn point).
    pub vehicle_defs: Vec<VehicleDef>,
    pub vehicle_spawns: Vec<VehicleSpawn>,
    pub vehicles: Vec<Vehicle>,
    pub movement: PlayerMovement,
    pub biped: BipedPhysics,
    pub time: f64,
    pub events: Vec<Event>,
    /// The player who won (in team games, whose kill won it, or the team's
    /// best when time ran out). After a draw both stay `None`.
    pub winner: Option<usize>,
    pub winning_team: Option<u8>,
    /// Campaign: the kinds of actor the mission has.
    pub characters: Vec<CharacterDef>,
    /// Campaign: where a player who dies comes back.
    pub checkpoint: Option<(Vec3, f32)>,
    since_checkpoint: f32,
    rng: u32,
}

/// How long the weapon now in a player's hands takes to come up.
fn ready_time(weapons: &[WeaponDef], p: &Spartan) -> f32 {
    p.held()
        .and_then(|h| weapons.get(h.weapon))
        .map_or(crate::weapon::DEFAULT_READY_TIME, |d| d.ready_time)
}

/// Distance along a ray to a vertical capsule (feet at `base`), if it hits.
fn ray_capsule(origin: Vec3, dir: Vec3, base: Vec3, height: f32, radius: f32) -> Option<f32> {
    let a = base + Vec3::Z * radius;
    let b = base + Vec3::Z * (height - radius).max(radius);
    // Closest approach between the ray and the capsule's segment, sampled
    // finely enough for player-sized capsules.
    let mut best: Option<f32> = None;
    let ab = b - a;
    let steps = 8;
    for i in 0..=steps {
        let c = a + ab * (i as f32 / steps as f32);
        // Ray / sphere at c.
        let oc = origin - c;
        let bq = oc.dot(dir);
        let cq = oc.length_squared() - radius * radius;
        let disc = bq * bq - cq;
        if disc < 0.0 {
            continue;
        }
        let t = -bq - disc.sqrt();
        if t >= 0.0 && best.is_none_or(|bt| t < bt) {
            best = Some(t);
        }
    }
    best
}

impl Game {
    pub fn new(
        rules: Rules,
        weapons: Vec<WeaponDef>,
        spawns: Vec<(Vec3, f32)>,
        item_spawns: Vec<ItemSpawn>,
        movement: PlayerMovement,
        biped: BipedPhysics,
    ) -> Game {
        Game {
            item_timers: vec![0.0; item_spawns.len()],
            rules,
            weapons,
            spawns,
            item_spawns,
            dropped: Vec::new(),
            grenades: Vec::new(),
            projectiles: Vec::new(),
            stuck: Vec::new(),
            allegiances: Vec::new(),
            players: Vec::new(),
            flags: Vec::new(),
            flag_bases: Vec::new(),
            kill_zones: Vec::new(),
            teleporters: Vec::new(),
            hills: Vec::new(),
            hill: 0,
            hill_moves_in: 0.0,
            hill_control: HillControl::Empty,
            territories: Vec::new(),
            juggernaut: None,
            vehicle_defs: Vec::new(),
            vehicle_spawns: Vec::new(),
            vehicles: Vec::new(),
            movement,
            biped,
            time: 0.0,
            events: Vec::new(),
            winner: None,
            winning_team: None,
            characters: Vec::new(),
            checkpoint: None,
            since_checkpoint: 0.0,
            rng: 0x2545_F491,
        }
    }

    /// Start the game's randomness (spawn choices) from another seed.
    pub fn reseed(&mut self, seed: u32) {
        self.rng = 0x2545_F491 ^ seed.wrapping_mul(0x9E37_79B9) | 1;
    }

    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Add a player, spawned at once (in team games, on the smaller team);
    /// returns its index.
    pub fn add_player(&mut self) -> usize {
        let team = (0..TEAMS)
            .min_by_key(|&t| self.players.iter().filter(|p| p.team == t).count())
            .unwrap_or(0);
        self.add_player_on(team)
    }

    /// Add a player on a team (ignored outside team games).
    pub fn add_player_on(&mut self, team: u8) -> usize {
        let i = self.players.len();
        let mut spartan = self.fresh_spartan(Vec3::ZERO, 0.0);
        spartan.name = format!("PLAYER {}", i + 1);
        spartan.look = Look::default_for(i);
        spartan.team = if self.rules.game_type.teams() {
            team.min(TEAMS - 1)
        } else {
            0
        };
        self.players.push(spartan);
        self.respawn(i);
        i
    }

    /// Give a player a gamertag (cleaned up; blank names are left alone).
    pub fn set_name(&mut self, player: usize, name: &str) {
        let name = clean_name(name);
        if let (Some(p), false) = (self.players.get_mut(player), name.is_empty()) {
            p.name = name;
        }
    }

    /// Set how a player looks: Elite or Spartan, and armour colours.
    pub fn set_look(&mut self, player: usize, look: Look) {
        if let Some(p) = self.players.get_mut(player) {
            p.look = look.clamped();
        }
    }

    /// A player's gamertag.
    pub fn name(&self, player: usize) -> &str {
        self.players.get(player).map_or("", |p| &p.name)
    }

    /// Players `a` and `b` are on opposite sides.
    pub fn is_enemy(&self, a: usize, b: usize) -> bool {
        let (ta, tb) = (self.players[a].team, self.players[b].team);
        a != b
            && (!self.rules.game_type.teams()
                || ta != tb
                    && !self
                        .allegiances
                        .iter()
                        .any(|&p| p == (ta, tb) || p == (tb, ta)))
    }

    pub fn team_score(&self, team: u8) -> i32 {
        self.players
            .iter()
            .filter(|p| p.team == team)
            .map(|p| p.score)
            .sum()
    }

    /// A player's score in this game type: their team's in team games.
    pub fn side_score(&self, player: usize) -> i32 {
        if self.rules.game_type.teams() {
            self.team_score(self.players[player].team)
        } else {
            self.players[player].score
        }
    }

    /// Seconds of play left before time runs out, in a game with a time
    /// limit.
    pub fn time_left(&self) -> Option<f64> {
        let limit = self.rules.time_limit;
        (limit > 0).then(|| (limit as f64 - self.time).max(0.0))
    }

    /// The game has ended: someone reached the score to win, or time ran
    /// out (a draw if no one was ahead).
    pub fn over(&self) -> bool {
        self.winner.is_some() || self.time_left() == Some(0.0)
    }

    fn fresh_spartan(&self, position: Vec3, yaw: f32) -> Spartan {
        let weapons = self
            .rules
            .starting_weapons
            .iter()
            .filter_map(|&w| {
                Some(HeldWeapon {
                    weapon: w,
                    state: WeaponState::new(self.weapons.get(w)?),
                })
            })
            .take(2)
            .collect();
        Spartan {
            name: String::new(),
            look: Look::default(),
            body: Player::new(position, self.movement, self.biped),
            yaw,
            pitch: 0.0,
            shield: self.rules.shield,
            health: self.rules.health,
            full: Vitality {
                shield: self.rules.shield,
                health: self.rules.health,
                recharge: self.rules.shield_recharge,
            },
            actor: None,
            since_damage: f32::INFINITY,
            overshield_charge: 0.0,
            camo: 0.0,
            camo_recharge: 0.0,
            reveal: 0.0,
            teleported: false,
            alive: true,
            respawn_in: 0.0,
            weapons,
            current: 0,
            left: None,
            objective: None,
            frags: self.rules.starting_frags,
            plasmas: self.rules.starting_plasmas,
            grenade: GrenadeKind::Frag,
            team: 0,
            score: 0,
            kills: 0,
            deaths: 0,
            spree: 0,
            multi_kill: 0,
            last_kill: f64::NEG_INFINITY,
            readying: 0.0,
            hold: 0.0,
            melee_cooldown: 0.0,
            grenade_cooldown: 0.0,
            action_held: 0.0,
            switch_held: f32::MIN,
            seat: None,
            board_held: 0.0,
            soft_landing: false,
            last: Command::default(),
        }
    }

    /// Bring a player back at the spawn point farthest from enemies alive
    /// (in Capture the Flag, on their own side of the map).
    pub fn respawn(&mut self, player: usize) {
        let others: Vec<Vec3> = (0..self.players.len())
            .filter(|&i| self.is_enemy(player, i) && self.players[i].alive)
            .map(|i| self.players[i].body.position)
            .collect();
        let team = self.players[player].team;
        let homes = self.has_flags().then(|| {
            let home = |own: bool| {
                self.flags
                    .iter()
                    .filter(|f| (f.team == team) == own)
                    .map(|f| f.home)
                    .next()
            };
            (home(true), home(false))
        });
        let mut best = (f32::MIN, (Vec3::ZERO, 0.0));
        for k in 0..self.spawns.len() {
            let (pos, yaw) = self.spawns[k];
            let nearest = others
                .iter()
                .map(|o| o.distance(pos))
                .fold(f32::MAX, f32::min);
            let side = match homes {
                Some((Some(own), Some(enemy))) => {
                    (enemy.distance(pos) - own.distance(pos)).clamp(-30.0, 30.0)
                }
                _ => 0.0,
            };
            // A little randomness so spawns vary when no one is around.
            let score = nearest.min(50.0) + side + self.random() * 4.0;
            if score > best.0 {
                best = (score, (pos, yaw));
            }
        }
        let (pos, yaw) = self.checkpoint_spawn().unwrap_or(best.1);
        let old = &mut self.players[player];
        let (team, score, kills, deaths) = (old.team, old.score, old.kills, old.deaths);
        let name = std::mem::take(&mut old.name);
        let look = old.look;
        let mut s = self.fresh_spartan(pos + Vec3::Z * 0.05, yaw);
        s.name = name;
        s.look = look;
        s.team = team;
        s.score = score;
        s.kills = kills;
        s.deaths = deaths;
        self.players[player] = s;
        self.events.push(Event::Spawned { player, yaw });
    }

    /// Advance one tick; `commands[i]` drives player i.
    pub fn step(&mut self, world: &World, commands: &[Command]) {
        let dt = TICK;
        // Once the game is over, everything stands still.
        let over = self.over();
        self.time += dt as f64;
        if over {
            return;
        }
        if self.time_left() == Some(0.0) {
            self.time_up();
            return;
        }
        for i in 0..self.players.len() {
            let cmd = commands.get(i).copied().unwrap_or_default();
            self.step_player(world, i, cmd, dt);
        }
        self.step_vehicles(world, dt);
        self.step_grenades(world, dt);
        self.step_projectiles(world, dt);
        self.step_stuck(dt);
        self.step_items(dt);
        self.step_flags(world, dt);
        self.step_hills(dt);
        self.step_territories(dt);
        self.step_checkpoint(world, dt);
    }

    fn step_player(&mut self, world: &World, i: usize, cmd: Command, dt: f32) {
        if !self.players[i].alive {
            let p = &mut self.players[i];
            p.respawn_in -= dt;
            // Actors stay dead.
            if p.respawn_in <= 0.0 && p.actor.is_none() {
                self.respawn(i);
            }
            return;
        }
        let last = self.players[i].last;
        let pressed = |now: bool, before: bool| now && !before;
        {
            let p = &mut self.players[i];
            p.move_with(world, &cmd, dt);
            // Shields recharge after a while without damage.
            p.since_damage += dt;
            let full = p.full;
            if p.since_damage > self.rules.shield_delay && p.shield < full.shield {
                p.shield = (p.shield + full.shield / full.recharge.max(0.1) * dt).min(full.shield);
            }
            powerups::step(p, &self.rules, dt);
            p.readying = (p.readying - dt).max(0.0);
            p.melee_cooldown = (p.melee_cooldown - dt).max(0.0);
            p.grenade_cooldown = (p.grenade_cooldown - dt).max(0.0);
        }
        // Fell out of the level or into a pit; landed hard.
        let feet = self.players[i].body.position;
        if feet.z < world.min.z - 1.0 || self.kill_zones.iter().any(|z| z.kills(feet)) {
            self.kill(i, None, false);
            return;
        }
        let mut fell = std::mem::take(&mut self.players[i].body.fell);
        if fell > 0.0 && std::mem::take(&mut self.players[i].soft_landing) {
            fell = 0.0;
        }
        let hurt = self.rules.falling.damage_for(fell);
        if hurt > 0.0 {
            self.damage(i, None, hurt, false);
            if !self.players[i].alive {
                return;
            }
        }

        // In a vehicle: driving and gunning; passengers keep their guns.
        let riding = self.players[i].seat.is_some();
        if riding && !self.ride(world, i, cmd, dt) {
            if let Some(p) = self.players.get_mut(i) {
                p.last = cmd;
            }
            return;
        }

        self.press_switch(i, cmd.switch_weapon, last.switch_weapon, dt);
        if pressed(cmd.vision, last.vision) {
            self.toggle_own_camo(i);
        }
        if pressed(cmd.switch_grenade, last.switch_grenade) {
            let p = &mut self.players[i];
            p.grenade = match p.grenade {
                GrenadeKind::Frag if p.plasmas > 0 => GrenadeKind::Plasma,
                GrenadeKind::Plasma if p.frags > 0 => GrenadeKind::Frag,
                g => g,
            };
        }
        let at_vehicle = !riding && self.board(world, i, cmd.action, dt);
        if self.players[i].seat.is_some() {
            self.players[i].last = cmd;
            return;
        }
        self.teleport(i);
        self.pick_up(i, cmd.action && !at_vehicle && !riding, dt);
        self.touch_flags(i, cmd.action && !at_vehicle, dt);

        let lunges = self.players[i]
            .held()
            .is_some_and(|h| self.rules.lunge_weapons.contains(&h.weapon));
        if pressed(cmd.melee, last.melee) && self.players[i].melee_cooldown <= 0.0 && !riding {
            self.melee(i, lunges);
        }
        // Dual wielding, the grenade button is the left trigger.
        let dual = self.players[i].left.is_some();
        if pressed(cmd.throw_grenade, last.throw_grenade) && !dual && !riding {
            self.throw_grenade(i);
        }

        // The weapon in hand. The sword's trigger swings it.
        let (eye, (f, r, u)) = (self.players[i].eye(), self.players[i].basis());
        let ready = self.players[i].readying <= 0.0;
        let mut shots = Vec::new();
        let mut reloaded = Vec::new();
        // A flag in hand only melees.
        let gun = match &self.players[i].objective {
            Some(_) => None,
            None => self.players[i].held().map(|h| h.weapon),
        };
        if let Some(def) = gun.and_then(|w| self.weapons.get(w)) {
            let def = if dual {
                def.dual_wielded()
            } else {
                def.clone()
            };
            let p = &mut self.players[i];
            let held = &mut p.weapons[p.current];
            let was = (held.state.reloading.is_some(), held.state.loaded == 0);
            let dry = def.uses_ammo()
                && held.state.loaded < def.rounds_per_shot
                && held.state.reserve == 0
                && held.state.reloading.is_none();
            if dry && ready && !lunges && pressed(cmd.fire, last.fire) {
                self.events.push(Event::DryFire { player: i });
            }
            let input = WeaponInput {
                fire: cmd.fire && ready && !lunges,
                reload: cmd.reload,
                zoom: cmd.zoom && !dual,
            };
            let w = gun.unwrap_or_default();
            for shot in held.state.update(&def, input, dt) {
                shots.push((shot.direction(f, r, u), def.clone(), w, false));
            }
            if held.state.reloading.is_some() && !was.0 {
                reloaded.push((false, was.1));
            }
        }
        // The left hand's gun, on the left trigger (zoom or grenade).
        let left = self.players[i].left.as_ref().map(|h| h.weapon);
        if let Some(def) = left
            .and_then(|w| self.weapons.get(w))
            .map(|d| d.dual_wielded())
        {
            let held = self.players[i].left.as_mut().expect("a left hand gun");
            let was = (held.state.reloading.is_some(), held.state.loaded == 0);
            let input = WeaponInput {
                fire: (cmd.zoom || cmd.throw_grenade) && ready,
                reload: cmd.reload,
                zoom: false,
            };
            let w = left.unwrap_or_default();
            for shot in held.state.update(&def, input, dt) {
                shots.push((shot.direction(f, r, u), def.clone(), w, true));
            }
            if held.state.reloading.is_some() && !was.0 {
                reloaded.push((true, was.1));
            }
        }
        if lunges && pressed(cmd.fire, last.fire) && ready && self.players[i].melee_cooldown <= 0.0
        {
            self.melee(i, true);
        }
        for (left, empty) in reloaded {
            self.events.push(Event::Reloaded {
                player: i,
                left,
                empty,
            });
        }
        for (dir, def, w, left) in shots {
            self.fire(world, i, eye, dir, &def, w, left);
        }
        self.players[i].last = cmd;
    }

    fn switch_weapon(&mut self, i: usize) {
        let p = &mut self.players[i];
        if p.weapons.len() < 2 {
            return;
        }
        if let Some(h) = p.weapons.get_mut(p.current) {
            h.state.zoom = 0;
            h.state.reloading = None;
        }
        p.current = (p.current + 1) % p.weapons.len();
        p.readying = ready_time(&self.weapons, p);
        self.events.push(Event::Switched { player: i });
    }

    /// The player seen first along a ray (a crosshair) by `viewer` within
    /// `range`, unless the level hides them: (player, distance).
    pub fn player_along(
        &self,
        world: &World,
        viewer: usize,
        origin: Vec3,
        dir: Vec3,
        range: f32,
    ) -> Option<(usize, f32)> {
        let (j, t, _) = self.trace_players(viewer, origin, dir)?;
        (t < range && world.raycast(origin, dir, t).is_none()).then_some((j, t))
    }

    /// Who a ray from `origin` hits first among living players other than
    /// `shooter` (and those riding with them): (player, distance, headshot).
    fn trace_players(&self, shooter: usize, origin: Vec3, dir: Vec3) -> Option<(usize, f32, bool)> {
        let own = self.riding(shooter).map(|(v, _)| v);
        let mut best: Option<(usize, f32, bool)> = None;
        for (j, q) in self.players.iter().enumerate() {
            if j == shooter || !q.alive || !self.exposed(j) {
                continue;
            }
            if own.is_some() && q.seat.map(|(v, _)| v) == own {
                continue;
            }
            let h = q.body.height();
            if let Some(t) = ray_capsule(origin, dir, q.body.position, h, q.body.biped.radius) {
                if best.is_none_or(|b| t < b.1) {
                    let z = origin.z + dir.z * t;
                    let head = z > q.body.position.z + h - HEAD_HEIGHT;
                    best = Some((j, t, head));
                }
            }
        }
        best
    }

    /// Fire one round of `weapon` (`def`) from `eye` along `dir`: it hits
    /// the level, a player or a vehicle, whichever is first.
    #[allow(clippy::too_many_arguments)]
    fn fire(
        &mut self,
        world: &World,
        i: usize,
        eye: Vec3,
        dir: Vec3,
        def: &WeaponDef,
        weapon: usize,
        left: bool,
    ) {
        self.reveal(i, powerups::FIRE_REVEAL);
        // Slow and explosive rounds fly; the rest hit at once.
        if let Some(flight) = def.flight {
            self.launch(i, eye, dir, weapon, flight);
            self.events.push(Event::Shot {
                player: i,
                weapon,
                left,
                origin: eye,
                direction: dir,
                hit: None,
                hit_player: None,
            });
            return;
        }
        let range = def.range;
        let wall = world.raycast_hit(eye, dir, range);
        let wall_t = wall.map_or(f32::MAX, |w| w.0);
        let target = self
            .trace_players(i, eye, dir)
            .filter(|&(_, t, _)| t <= range && t < wall_t);
        let vehicle = self
            .trace_vehicles(i, eye, dir)
            .filter(|&(_, t)| t <= range && t < wall_t);
        let (hit, hit_player) = match (target, vehicle) {
            (Some((j, t, head)), v) if v.is_none_or(|v| t <= v.1) => {
                let damage = WeaponState::damage_at(def, t);
                let headshot = head && self.rules.headshot_weapons.contains(&weapon);
                self.hurt(j, Some(i), damage, headshot, def.armor);
                (Some((eye + dir * t, -dir)), Some(j))
            }
            (_, Some((v, t))) => {
                let damage = WeaponState::damage_at(def, t);
                self.damage_vehicle(v, Some(i), damage);
                (Some((eye + dir * t, -dir)), None)
            }
            _ => (wall.map(|(t, n)| (eye + dir * t, n)), None),
        };
        self.events.push(Event::Shot {
            player: i,
            weapon,
            left,
            origin: eye,
            direction: dir,
            hit,
            hit_player,
        });
    }

    fn melee(&mut self, i: usize, lunge: bool) {
        self.reveal(i, powerups::FIRE_REVEAL);
        let (eye, aim) = (self.players[i].eye(), self.players[i].aim());
        let reach = if lunge { LUNGE_RANGE } else { MELEE_RANGE };
        let mut target: Option<(usize, f32)> = None;
        for (j, q) in self.players.iter().enumerate() {
            // The sword only lunges at enemies.
            if j == i || !q.alive || (lunge && !self.is_enemy(i, j)) {
                continue;
            }
            let centre = q.body.position + Vec3::Z * q.body.height() * 0.6;
            let to = centre - eye;
            let d = to.length();
            if d <= reach
                && aim.dot(to / d.max(1e-4)) > MELEE_CONE.cos()
                && target.is_none_or(|t| d < t.1)
            {
                target = Some((j, d));
            }
        }
        self.players[i].melee_cooldown = MELEE_COOLDOWN;
        if let Some((j, _)) = target {
            if lunge {
                // The lunge carries the attacker to the target.
                let to = self.players[j].body.position - self.players[i].body.position;
                self.players[i].body.position += to * 0.6;
            }
            let damage = if lunge {
                self.rules.lunge_damage
            } else {
                // Some weapons (the flag) hit harder than the usual strike.
                self.players[i]
                    .held()
                    .and_then(|h| self.weapons.get(h.weapon))
                    .and_then(|d| d.melee_damage)
                    .unwrap_or(self.rules.melee_damage)
            };
            self.damage(j, Some(i), damage, false);
        }
        self.events.push(Event::Melee {
            player: i,
            hit: target.map(|t| t.0),
        });
    }

    fn throw_grenade(&mut self, i: usize) {
        let def = self.grenade_def(self.players[i].grenade);
        let p = &mut self.players[i];
        if p.grenade_cooldown > 0.0 || p.objective.is_some() {
            return;
        }
        let count = match p.grenade {
            GrenadeKind::Frag => &mut p.frags,
            GrenadeKind::Plasma => &mut p.plasmas,
        };
        if *count == 0 {
            return;
        }
        *count -= 1;
        p.grenade_cooldown = GRENADE_THROW_COOLDOWN;
        let kind = p.grenade;
        // Thrown a little above where the player looks.
        let (f, _, u) = p.basis();
        let dir = (f + u * 0.15).normalize();
        let g = Grenade {
            kind,
            owner: i,
            position: p.eye() + f * 0.25 - Vec3::Z * 0.05,
            velocity: dir * def.speed + p.body.velocity * 0.5,
            fuse: None,
            stuck: None,
            stuck_offset: Vec3::ZERO,
            age: 0.0,
        };
        self.grenades.push(g);
        self.reveal(i, powerups::FIRE_REVEAL);
        self.events.push(Event::Thrown { player: i });
    }

    fn grenade_def(&self, kind: GrenadeKind) -> GrenadeDef {
        match kind {
            GrenadeKind::Frag => self.rules.frag,
            GrenadeKind::Plasma => self.rules.plasma,
        }
    }

    fn step_grenades(&mut self, world: &World, dt: f32) {
        let mut exploded = Vec::new();
        for gi in 0..self.grenades.len() {
            let mut g = self.grenades[gi];
            let def = self.grenade_def(g.kind);
            g.age += dt;
            if let Some(j) = g.stuck {
                match self.players.get(j).filter(|p| p.alive) {
                    Some(p) => g.position = p.body.position + g.stuck_offset,
                    None => g.stuck = None,
                }
            } else {
                g.velocity.z -= GRENADE_GRAVITY * dt;
                let step = g.velocity * dt;
                let len = step.length();
                if len > 1e-6 {
                    let dir = step / len;
                    // Plasma grenades stick to players they touch.
                    let victim = def
                        .sticks
                        .then(|| self.trace_players(g.owner, g.position, dir))
                        .flatten()
                        .filter(|&(_, t, _)| t <= len);
                    if let Some((j, _, _)) = victim {
                        g.stuck = Some(j);
                        g.stuck_offset = g.position + dir * len - self.players[j].body.position;
                        g.fuse.get_or_insert(def.fuse);
                    } else if let Some((t, n)) = world.raycast_hit(g.position, dir, len + 0.02) {
                        g.position += dir * (t - 0.02).max(0.0);
                        if def.sticks {
                            g.velocity = Vec3::ZERO;
                        } else {
                            // Bounce, losing most of the speed into the surface.
                            let vn = g.velocity.dot(n) * n;
                            g.velocity = (g.velocity - vn) * 0.6 - vn * 0.35;
                        }
                        g.fuse.get_or_insert(def.fuse);
                    } else {
                        g.position += step;
                    }
                }
            }
            if let Some(f) = g.fuse.as_mut() {
                *f -= dt;
            }
            // Grenades that never land go off anyway.
            if g.fuse.is_some_and(|f| f <= 0.0) || g.age > 6.0 {
                exploded.push(gi);
            }
            self.grenades[gi] = g;
        }
        for &gi in exploded.iter().rev() {
            let g = self.grenades.remove(gi);
            self.explode(world, g);
        }
    }

    fn explode(&mut self, world: &World, g: Grenade) {
        let def = self.grenade_def(g.kind);
        let blast = Blast {
            damage: (0.0, def.damage),
            radius: def.radius,
            push: GRENADE_PUSH,
            armor: def.armor,
        };
        self.blast(world, g.position, g.owner, blast, g.stuck);
        self.events.push(Event::Exploded {
            kind: g.kind,
            position: g.position,
        });
    }

    /// Apply damage: shields first, then health.
    pub fn damage(&mut self, victim: usize, attacker: Option<usize>, amount: f32, headshot: bool) {
        self.hurt(victim, attacker, amount, headshot, ArmorScale::default());
    }

    /// Apply damage of a kind that hits shields and bodies harder or softer
    /// (`armor`).
    pub fn hurt(
        &mut self,
        victim: usize,
        attacker: Option<usize>,
        amount: f32,
        headshot: bool,
        armor: ArmorScale,
    ) {
        let teammate = attacker.is_some_and(|a| a != victim && !self.is_enemy(a, victim));
        // Actors never hurt their own side.
        let actor = attacker.is_some_and(|a| self.players[a].actor.is_some());
        if teammate && (!self.rules.friendly_fire || actor) {
            return;
        }
        let amount = self.juggernaut_damage(victim, amount);
        let p = &mut self.players[victim];
        if !p.alive || amount <= 0.0 {
            return;
        }
        p.since_damage = 0.0;
        p.reveal = p.reveal.max(powerups::HURT_REVEAL);
        // What gets past the shields (all of it once they're down).
        let past = if p.shield <= 0.0 {
            amount
        } else if armor.shield <= 0.0 {
            0.0
        } else {
            let absorbed = (amount * armor.shield).min(p.shield);
            p.shield -= absorbed;
            (amount - absorbed / armor.shield).max(0.0)
        };
        let rest = past * armor.body;
        // A headshot kills once the shields are gone.
        let killing_headshot = headshot && p.shield <= 0.0 && past > 1e-3;
        p.health -= if killing_headshot {
            p.full.health
        } else {
            rest
        };
        if p.actor.is_some_and(|a| a.immortal) {
            p.health = p.health.max(1.0);
        }
        self.events.push(Event::Damaged {
            player: victim,
            amount,
        });
        if p.health <= 0.0 {
            self.kill(victim, attacker, killing_headshot);
        }
    }

    fn kill(&mut self, victim: usize, killer: Option<usize>, headshot: bool) {
        let respawn = if self.players[victim].actor.is_some() {
            self.corpse_time()
        } else {
            self.rules.respawn_time
        };
        self.leave_seat(victim);
        if self.players[victim].objective.is_some() {
            self.players[victim].alive = false;
            self.drop_flag(victim);
        }
        let p = &mut self.players[victim];
        p.alive = false;
        p.health = 0.0;
        p.shield = 0.0;
        p.overshield_charge = 0.0;
        p.camo = 0.0;
        p.camo_recharge = 0.0;
        p.respawn_in = respawn;
        p.deaths += 1;
        p.spree = 0;
        p.multi_kill = 0;
        // Drop the weapon in hand.
        let drop = p.weapons.get(p.current).cloned();
        let (pos, yaw) = (p.body.position + Vec3::Z * 0.1, p.yaw);
        if let Some(h) = drop {
            self.dropped.push(DroppedWeapon {
                weapon: h.weapon,
                state: h.state,
                position: pos,
                yaw,
                ttl: DROPPED_WEAPON_LIFETIME,
            });
        }
        self.drop_left(victim);
        self.events.push(Event::Killed {
            killer,
            victim,
            headshot,
        });
        // The campaign keeps no score.
        if self.rules.game_type == GameType::Campaign {
            if let Some(k) = killer.filter(|&k| self.is_enemy(k, victim)) {
                self.players[k].kills += 1;
            }
            return;
        }
        let leaders = self.leaders();
        let kills_score = self.rules.game_type.kills_score();
        // Outside Slayer, kills don't change the score.
        let point = i32::from(kills_score);
        match killer {
            // Killed themselves, or by the level: a point off.
            None => self.players[victim].score -= point,
            Some(k) if k == victim => self.players[victim].score -= point,
            // Betrayed a teammate: a point off.
            Some(k) if !self.is_enemy(k, victim) => self.players[k].score -= point,
            Some(k) => {
                let time = self.time;
                let p = &mut self.players[k];
                p.kills += 1;
                p.score += point;
                p.spree += 1;
                let chained = p.multi_kill > 0 && time - p.last_kill <= MULTI_KILL_WINDOW;
                p.multi_kill = if chained { p.multi_kill + 1 } else { 1 };
                p.last_kill = time;
                let (multi, spree) = (p.multi_kill, p.spree);
                if multi >= 2 {
                    self.events.push(Event::Medal {
                        player: k,
                        medal: Medal::MultiKill(multi.min(u8::MAX as u32) as u8),
                    });
                }
                if spree % SPREE_STEP == 0 && spree <= SPREE_MAX {
                    self.events.push(Event::Medal {
                        player: k,
                        medal: Medal::Spree(spree as u8),
                    });
                }
            }
        }
        if self.rules.game_type == GameType::Juggernaut {
            self.juggernaut_kill(victim, killer);
        }
        self.lead_changes(&leaders);
        if let Some(k) = killer.filter(|&k| kills_score && self.is_enemy(k, victim)) {
            self.check_win(k);
        }
    }

    /// End the game if `player`'s side has reached the score to win.
    fn check_win(&mut self, player: usize) {
        let target = self.rules.score_to_win as i32;
        if target > 0 && self.side_score(player) >= target {
            self.winner.get_or_insert(player);
            if self.rules.game_type.teams() {
                self.winning_team.get_or_insert(self.players[player].team);
            }
        }
    }

    /// Time ran out: the best score wins, a player's or (in team games) a
    /// team's. A tie for the best is a draw, with no winner.
    fn time_up(&mut self) {
        let teams = self.rules.game_type.teams();
        // Teams with someone on them, or players, and their scores.
        let sides: Vec<(usize, i32)> = if teams {
            (0..TEAMS)
                .filter(|&t| self.players.iter().any(|p| p.team == t))
                .map(|t| (t as usize, self.team_score(t)))
                .collect()
        } else {
            self.players.iter().map(|p| p.score).enumerate().collect()
        };
        let top = sides.iter().map(|s| s.1).max();
        let mut best = sides.iter().filter(|s| Some(s.1) == top);
        let (Some(&(side, _)), None) = (best.next(), best.next()) else {
            return;
        };
        if !teams {
            self.winner = Some(side);
            return;
        }
        let team = side as u8;
        self.winning_team = Some(team);
        // The team's best player stands for it, the first of equals.
        self.winner = (0..self.players.len())
            .filter(|&i| self.players[i].team == team)
            .max_by_key(|&i| (self.players[i].score, std::cmp::Reverse(i)));
    }

    /// Who leads: players, or in team games teams (by team number). No one
    /// leads until someone scores.
    fn leaders(&self) -> Vec<usize> {
        let sides: Vec<i32> = if self.rules.game_type.teams() {
            (0..TEAMS).map(|t| self.team_score(t)).collect()
        } else {
            self.players.iter().map(|p| p.score).collect()
        };
        let top = sides.iter().copied().max().unwrap_or(0);
        if top <= 0 {
            return Vec::new();
        }
        (0..sides.len()).filter(|&i| sides[i] == top).collect()
    }

    /// Announce how the last kill or capture changed the lead: to each
    /// player, or to everyone on a team.
    fn lead_changes(&mut self, before: &[usize]) {
        let after = self.leaders();
        let teams = self.rules.game_type.teams();
        let tell = |side: usize, change: LeadChange, events: &mut Vec<Event>| {
            for (i, p) in self.players.iter().enumerate() {
                let on_side = if teams {
                    p.team as usize == side
                } else {
                    i == side
                };
                if on_side {
                    events.push(Event::Lead { player: i, change });
                }
            }
        };
        let mut events = Vec::new();
        for &side in &after {
            if after.len() == 1 && !before.contains(&side) {
                tell(side, LeadChange::Gained, &mut events);
            } else if after.len() > 1 && !before.contains(&side) {
                tell(side, LeadChange::Tied, &mut events);
            } else if after.len() == 1 && before.len() > 1 {
                // Broke a tie.
                tell(side, LeadChange::Gained, &mut events);
            }
        }
        for &side in before {
            if !after.contains(&side) {
                tell(side, LeadChange::Lost, &mut events);
            }
        }
        self.events.extend(events);
    }

    /// Walk over ammo and grenades; hold the action key to take a weapon.
    fn pick_up(&mut self, i: usize, action: bool, dt: f32) {
        let feet = self.players[i].body.position;
        let near = |p: Vec3| {
            Vec2::new(p.x - feet.x, p.y - feet.y).length() < PICKUP_RADIUS
                && (p.z - feet.z).abs() < 0.8
        };
        self.players[i].action_held = if action {
            self.players[i].action_held + dt
        } else {
            0.0
        };

        // Map items.
        for s in 0..self.item_spawns.len() {
            if self.item_timers[s] > 0.0 || !near(self.item_spawns[s].position) {
                continue;
            }
            let spawn = self.item_spawns[s];
            let taken = match spawn.kind {
                ItemKind::FragGrenades | ItemKind::PlasmaGrenades => {
                    let max = self.rules.max_grenades;
                    let p = &mut self.players[i];
                    let count = if spawn.kind == ItemKind::FragGrenades {
                        &mut p.frags
                    } else {
                        &mut p.plasmas
                    };
                    (*count < max).then(|| *count += 1).is_some()
                }
                ItemKind::Powerup(kind) => self.give_powerup(i, kind),
                ItemKind::Ammo { weapon, rounds } => self.take_ammo(i, weapon, rounds),
                ItemKind::Weapon(w) => {
                    let mut state = match self.weapons.get(w) {
                        Some(def) => WeaponState::new(def),
                        None => continue,
                    };
                    match self.take_weapon(i, w, &mut state, action) {
                        // A one-handed gun stays, emptier, for a second hand.
                        Some(Taken::Ammo) if self.one_handed(w) => {
                            self.dropped.push(DroppedWeapon {
                                weapon: w,
                                state,
                                position: spawn.position,
                                yaw: 0.0,
                                ttl: DROPPED_WEAPON_LIFETIME,
                            });
                            true
                        }
                        taken => taken.is_some(),
                    }
                }
            };
            if taken {
                self.item_timers[s] = spawn.respawn.max(1.0);
                self.events.push(Event::PickedUp {
                    player: i,
                    kind: spawn.kind,
                });
            }
        }
        // Dropped weapons.
        let mut d = 0;
        while d < self.dropped.len() {
            if near(self.dropped[d].position) {
                let w = self.dropped[d].weapon;
                let mut state = self.dropped[d].state.clone();
                let taken = self.take_weapon(i, w, &mut state, action);
                if taken.is_some() {
                    self.events.push(Event::PickedUp {
                        player: i,
                        kind: ItemKind::Weapon(w),
                    });
                }
                match taken {
                    Some(Taken::Ammo) if self.one_handed(w) => self.dropped[d].state = state,
                    Some(_) => {
                        self.dropped.remove(d);
                        continue;
                    }
                    None => {}
                }
            }
            d += 1;
        }
    }

    /// A weapon on the ground (`state`): ammo if the player has one
    /// already (drained from `state`), the weapon itself if they have a
    /// free hand or hold the action key.
    fn take_weapon(
        &mut self,
        i: usize,
        w: usize,
        state: &mut WeaponState,
        action: bool,
    ) -> Option<Taken> {
        let def = self.weapons.get(w).cloned()?;
        let p = &mut self.players[i];
        let mut have = p.weapons.iter_mut().chain(p.left.as_mut());
        if let Some(h) = have.find(|h| h.weapon == w) {
            if !def.uses_ammo() {
                return None;
            }
            let room = def.maximum_rounds.saturating_sub(h.state.reserve);
            let take = (state.loaded + state.reserve).min(room);
            if take == 0 {
                return None;
            }
            h.state.reserve += take;
            let spare = take.min(state.reserve);
            state.reserve -= spare;
            state.loaded -= take - spare;
            return Some(Taken::Ammo);
        }
        // Hands full with the flag: ammo only.
        if p.objective.is_some() {
            return None;
        }
        let state = state.clone();
        if p.weapons.len() < 2 {
            p.weapons.push(HeldWeapon { weapon: w, state });
            p.current = p.weapons.len() - 1;
            p.readying = ready_time(&self.weapons, p);
            self.events.push(Event::Switched { player: i });
            return Some(Taken::Weapon);
        }
        if action && p.action_held >= SWAP_HOLD {
            p.action_held = f32::MIN;
            let old = std::mem::replace(&mut p.weapons[p.current], HeldWeapon { weapon: w, state });
            p.readying = ready_time(&self.weapons, p);
            if def.dual.is_none() {
                self.drop_left(i);
            }
            let p = &mut self.players[i];
            let (pos, yaw) = (p.body.position + Vec3::Z * 0.1, p.yaw);
            self.dropped.push(DroppedWeapon {
                weapon: old.weapon,
                state: old.state,
                position: pos,
                yaw,
                ttl: DROPPED_WEAPON_LIFETIME,
            });
            self.events.push(Event::Switched { player: i });
            return Some(Taken::Weapon);
        }
        None
    }

    fn step_items(&mut self, dt: f32) {
        for t in &mut self.item_timers {
            *t = (*t - dt).max(0.0);
        }
        for d in &mut self.dropped {
            d.ttl -= dt;
        }
        self.dropped.retain(|d| d.ttl > 0.0);
    }

    /// The weapon on the ground a player could swap for (action key prompt).
    pub fn swap_prompt(&self, i: usize) -> Option<usize> {
        let p = self.players.get(i)?;
        if !p.alive || p.weapons.len() < 2 || p.objective.is_some() {
            return None;
        }
        let feet = p.body.position;
        let near = |q: Vec3| Vec2::new(q.x - feet.x, q.y - feet.y).length() < PICKUP_RADIUS;
        let held = |w: usize| p.weapons.iter().any(|h| h.weapon == w);
        self.item_spawns
            .iter()
            .zip(&self.item_timers)
            .filter(|(s, t)| **t <= 0.0 && near(s.position))
            .find_map(|(s, _)| match s.kind {
                ItemKind::Weapon(w) if !held(w) => Some(w),
                _ => None,
            })
            .or_else(|| {
                self.dropped
                    .iter()
                    .find(|d| near(d.position) && !held(d.weapon))
                    .map(|d| d.weapon)
            })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) use crate::testing::{floor, game};

    /// Two players facing each other along x.
    fn duel(g: &mut Game) {
        g.add_player();
        g.add_player();
        g.players[0].body.position = Vec3::new(0.0, 0.0, 0.0);
        g.players[1].body.position = Vec3::new(5.0, 0.0, 0.0);
    }

    fn aim_at(g: &Game, from: usize, to: usize, z: f32) -> Command {
        let d = g.players[to].body.position + Vec3::Z * z - g.players[from].eye();
        Command {
            yaw: d.y.atan2(d.x),
            pitch: (d.z / d.truncate().length()).atan(),
            ..Command::default()
        }
    }

    #[test]
    fn shots_strip_shields_then_health() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        let mut fire = aim_at(&g, 0, 1, 0.4);
        // Settle on the floor first.
        for _ in 0..10 {
            g.step(&world, &[aim_at(&g, 0, 1, 0.4), Command::default()]);
        }
        fire.fire = true;
        g.step(&world, &[fire, Command::default()]);
        assert!(g.players[1].shield < 70.0, "shield {}", g.players[1].shield);
        assert_eq!(g.players[1].health, 45.0);
        // Keep shooting (tap the trigger) until dead.
        for t in 0..400 {
            let mut c = aim_at(&g, 0, 1, 0.4);
            c.fire = t % 4 == 0;
            g.step(&world, &[c, Command::default()]);
            if !g.players[1].alive {
                break;
            }
        }
        assert!(!g.players[1].alive);
        assert_eq!(g.players[0].kills, 1);
        assert_eq!(g.players[1].deaths, 1);
        // Dropped their rifle.
        assert_eq!(g.dropped.len(), 1);
    }

    #[test]
    fn headshot_kills_without_shields() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        g.players[1].shield = 0.0;
        let mut c = aim_at(&g, 0, 1, 0.68);
        c.fire = true;
        g.step(&world, &[c, Command::default()]);
        assert!(!g.players[1].alive);
        assert!(g
            .events
            .iter()
            .any(|e| matches!(e, Event::Killed { headshot: true, .. })));
    }

    #[test]
    fn kinds_of_damage_hit_shields_and_bodies_differently() {
        let mut g = game();
        for _ in 0..4 {
            g.add_player();
        }
        // Sniper rounds: twice as hard on shields, so two body shots kill.
        let sniper = ArmorScale {
            shield: 2.0,
            body: 1.0,
        };
        g.hurt(1, Some(0), 45.0, false, sniper);
        assert_eq!(g.players[1].shield, 0.0);
        assert!((g.players[1].health - 35.0).abs() < 1e-3);
        g.hurt(1, Some(0), 45.0, false, sniper);
        assert!(!g.players[1].alive);

        // A grenade at your feet leaves you alive from full shields.
        g.hurt(2, Some(0), 150.0, false, ArmorScale::EXPLOSION);
        assert!(g.players[2].alive);
        assert!((g.players[2].health - 35.0).abs() < 1e-3);

        // Plasma: shields melt, bodies barely suffer.
        let plasma = ArmorScale {
            shield: 1.5,
            body: 0.35,
        };
        g.hurt(3, Some(0), 10.0, false, plasma);
        assert!((g.players[3].shield - 55.0).abs() < 1e-3);
        g.players[3].shield = 0.0;
        g.hurt(3, Some(0), 10.0, false, plasma);
        assert!((g.players[3].health - 41.5).abs() < 1e-3);
    }

    #[test]
    fn dead_players_respawn() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        g.damage(1, Some(0), 500.0, false);
        assert!(!g.players[1].alive);
        for _ in 0..(5.5 / TICK) as usize {
            g.step(&world, &[Command::default(), Command::default()]);
        }
        assert!(g.players[1].alive);
        assert_eq!(g.players[1].shield, 70.0);
        assert_eq!(g.players[1].deaths, 1);
    }

    #[test]
    fn quick_kills_and_sprees_earn_medals() {
        let mut g = game();
        for _ in 0..3 {
            g.add_player();
        }
        let medals = |g: &mut Game| -> Vec<Medal> {
            g.events
                .drain(..)
                .filter_map(|e| match e {
                    Event::Medal { player: 0, medal } => Some(medal),
                    _ => None,
                })
                .collect()
        };
        g.damage(1, Some(0), 500.0, false);
        assert_eq!(medals(&mut g), vec![]);
        g.damage(2, Some(0), 500.0, false);
        assert_eq!(medals(&mut g), vec![Medal::MultiKill(2)]);
        // Too slow for a triple kill: the chain starts again.
        g.time += 5.0;
        g.respawn(1);
        g.damage(1, Some(0), 500.0, false);
        assert_eq!(medals(&mut g), vec![]);
        g.respawn(1);
        g.respawn(2);
        g.damage(1, Some(0), 500.0, false);
        g.damage(2, Some(0), 500.0, false);
        // Five kills without dying.
        assert_eq!(
            medals(&mut g),
            vec![Medal::MultiKill(2), Medal::MultiKill(3), Medal::Spree(5)]
        );
        // Dying ends the spree.
        g.damage(0, Some(1), 500.0, false);
        assert_eq!(g.players[0].spree, 0);
    }

    #[test]
    fn the_game_stands_still_once_won() {
        let world = floor();
        let mut g = game();
        g.rules.score_to_win = 1;
        duel(&mut g);
        g.damage(1, Some(0), 500.0, false);
        assert_eq!(g.winner, Some(0));
        let before = g.players[0].body.position;
        let run = Command {
            movement: glam::Vec2::X,
            ..Command::default()
        };
        for _ in 0..30 {
            g.step(&world, &[run, Command::default()]);
        }
        assert_eq!(g.players[0].body.position, before);
        assert!(!g.players[1].alive, "no respawning after the end");
    }

    #[test]
    fn the_best_score_wins_when_time_runs_out() {
        let world = floor();
        let mut g = game();
        assert_eq!(g.time_left(), None, "no time limit unless set");
        g.rules.time_limit = 2;
        duel(&mut g);
        g.add_player();
        g.players[1].score = 3;
        g.players[2].score = 2;
        let idle = [Command::default(); 3];
        for _ in 0..119 {
            g.step(&world, &idle);
        }
        assert!(!g.over());
        assert!(g.time_left().is_some_and(|t| t > 0.0 && t < 0.1));
        g.step(&world, &idle);
        assert!(g.over());
        assert_eq!(g.time_left(), Some(0.0));
        assert_eq!((g.winner, g.winning_team), (Some(1), None));
    }

    #[test]
    fn a_tie_when_time_runs_out_is_a_draw() {
        let world = floor();
        let mut g = game();
        g.rules.time_limit = 1;
        duel(&mut g);
        g.players[0].score = 4;
        g.players[1].score = 4;
        let run = Command {
            movement: glam::Vec2::X,
            ..Command::default()
        };
        for _ in 0..60 {
            g.step(&world, &[run, Command::default()]);
        }
        assert!(g.over());
        assert_eq!((g.winner, g.winning_team), (None, None));
        // Over with no winner, it stands still all the same.
        let before = g.players[0].body.position;
        for _ in 0..30 {
            g.step(&world, &[run, Command::default()]);
        }
        assert_eq!(g.players[0].body.position, before);
        assert_eq!(g.winner, None);
    }

    #[test]
    fn teams_win_or_draw_on_time() {
        let world = floor();
        // Red is players 0 and 2, blue 1 and 3.
        let finish = |scores: [i32; 4]| {
            let mut g = team_game(4);
            g.rules.time_limit = 1;
            for (p, s) in g.players.iter_mut().zip(scores) {
                p.score = s;
            }
            for _ in 0..60 {
                g.step(&world, &[Command::default(); 4]);
            }
            assert!(g.over());
            (g.winning_team, g.winner)
        };
        // Red wins 3 to 2; its best player stands for it.
        assert_eq!(finish([1, 2, 2, 0]), (Some(0), Some(2)));
        // Blue wins, though a red player has the best score of all.
        assert_eq!(finish([5, 3, -4, 3]), (Some(1), Some(1)));
        // Level at 2 each: a draw.
        assert_eq!(finish([1, 2, 1, 0]), (None, None));
    }

    fn team_game(players: usize) -> Game {
        let mut g = game();
        g.rules.game_type = GameType::TeamSlayer;
        for _ in 0..players {
            g.add_player();
        }
        g
    }

    #[test]
    fn players_split_into_teams() {
        let g = team_game(5);
        let teams: Vec<u8> = g.players.iter().map(|p| p.team).collect();
        assert_eq!(teams, [0, 1, 0, 1, 0]);
        assert!(g.is_enemy(0, 1));
        assert!(!g.is_enemy(0, 2));
        assert!(!g.is_enemy(0, 0));
        // Free for all: everyone is an enemy.
        let mut ffa = game();
        ffa.add_player();
        ffa.add_player();
        assert!(ffa.is_enemy(0, 1));
    }

    #[test]
    fn names_are_gamertags_and_last_through_respawns() {
        assert_eq!(clean_name("  John Morrow\t"), "JOHN MORROW");
        assert_eq!(clean_name("averyveryverylongname"), "AVERYVERYVERYLO");
        // Spaces left at the start once what can't be shown is gone go too.
        assert_eq!(clean_name("\u{e9} Bob"), "BOB");
        // Only what the font draws, so SARGE_ can't pass for SARGE.
        assert_eq!(clean_name("Sarge_#%"), "SARGE");
        assert_eq!(clean_name("(o.o)-<3!?"), "(O.O)-<3!?");
        assert_eq!(
            clean_name("\u{3a9}  Master  Chief \u{3a9}"),
            "MASTER  CHIEF"
        );
        for name in [
            "\u{e9} Bob",
            " a\u{e9}\u{e9} b ",
            "\u{e9}             x  y z",
        ] {
            let clean = clean_name(name);
            assert_eq!(clean_name(&clean), clean, "{name:?}");
        }
        assert_eq!(guest_name("john", 0), "JOHN");
        assert_eq!(guest_name("john", 2), "JOHN(2)");
        assert_eq!(guest_name("averyveryverylongname", 1), "AVERYVERYVER(1)");
        let mut g = game();
        let a = g.add_player();
        assert_eq!(g.name(a), "PLAYER 1");
        g.set_name(a, "fortminorpark");
        g.set_name(a, "   ");
        g.respawn(a);
        assert_eq!(g.name(a), "FORTMINORPARK");
    }

    #[test]
    fn suicides_and_betrayals_cost_a_point() {
        let mut g = team_game(4);
        g.damage(1, Some(0), 500.0, false);
        assert_eq!((g.players[0].score, g.players[0].kills), (1, 1));
        // Player 2 is on player 0's team.
        g.damage(2, Some(0), 500.0, false);
        assert_eq!((g.players[0].score, g.players[0].kills), (0, 1));
        g.damage(3, Some(3), 500.0, false);
        assert_eq!(g.players[3].score, -1);
        g.damage(0, None, 500.0, false);
        assert_eq!(g.players[0].score, -1);
        assert_eq!(g.team_score(0), -1);
        assert_eq!(g.team_score(1), -1);
    }

    #[test]
    fn teammates_are_safe_without_friendly_fire() {
        let mut g = team_game(3);
        g.rules.friendly_fire = false;
        g.damage(2, Some(0), 500.0, false);
        assert!(g.players[2].alive);
        g.damage(1, Some(0), 500.0, false);
        assert!(!g.players[1].alive);
        // Your own grenade still hurts.
        g.damage(0, Some(0), 500.0, false);
        assert!(!g.players[0].alive);
    }

    #[test]
    fn a_team_wins_together() {
        let mut g = team_game(4);
        g.rules.score_to_win = 3;
        let lead = |g: &mut Game| -> Vec<(usize, LeadChange)> {
            g.events
                .drain(..)
                .filter_map(|e| match e {
                    Event::Lead { player, change } => Some((player, change)),
                    _ => None,
                })
                .collect()
        };
        g.damage(1, Some(0), 500.0, false);
        // Both red players hear they took the lead.
        assert_eq!(
            lead(&mut g),
            [(0, LeadChange::Gained), (2, LeadChange::Gained)]
        );
        g.respawn(1);
        g.damage(0, Some(1), 500.0, false);
        assert_eq!(lead(&mut g), [(1, LeadChange::Tied), (3, LeadChange::Tied)]);
        g.respawn(0);
        g.damage(1, Some(2), 500.0, false);
        g.respawn(1);
        assert_eq!(g.winner, None);
        g.damage(3, Some(0), 500.0, false);
        assert_eq!(g.team_score(0), 3);
        assert_eq!(g.winner, Some(0));
        assert_eq!(g.winning_team, Some(0));
    }

    #[test]
    fn the_lead_changes_hands() {
        let mut g = game();
        g.add_player();
        g.add_player();
        let leads = |g: &mut Game| -> Vec<(usize, LeadChange)> {
            g.events
                .drain(..)
                .filter_map(|e| match e {
                    Event::Lead { player, change } => Some((player, change)),
                    _ => None,
                })
                .collect()
        };
        g.damage(1, Some(0), 500.0, false);
        assert_eq!(leads(&mut g), vec![(0, LeadChange::Gained)]);
        g.respawn(1);
        g.damage(0, Some(1), 500.0, false);
        assert_eq!(leads(&mut g), vec![(1, LeadChange::Tied)]);
        g.respawn(0);
        g.damage(0, Some(1), 500.0, false);
        assert_eq!(
            leads(&mut g),
            vec![(1, LeadChange::Gained), (0, LeadChange::Lost)]
        );
    }

    #[test]
    fn walking_over_a_weapon_takes_it_into_a_free_hand() {
        let world = floor();
        let mut g = game();
        g.add_player();
        g.players[0].body.position = Vec3::new(0.0, 3.0, 0.0);
        g.step(&world, &[Command::default()]);
        assert_eq!(g.players[0].weapons.len(), 2);
        assert!(g.item_timers[0] > 0.0);
    }

    #[test]
    fn shields_recharge_after_a_delay() {
        let world = floor();
        let mut g = game();
        g.add_player();
        g.damage(0, None, 30.0, false);
        assert_eq!(g.players[0].shield, 40.0);
        for _ in 0..(4.0 / TICK) as usize {
            g.step(&world, &[Command::default()]);
        }
        assert_eq!(g.players[0].shield, 40.0);
        for _ in 0..(3.5 / TICK) as usize {
            g.step(&world, &[Command::default()]);
        }
        assert_eq!(g.players[0].shield, 70.0);
    }

    #[test]
    fn frag_grenades_explode_near_players() {
        let world = floor();
        let mut g = game();
        duel(&mut g);
        g.players[1].body.position = Vec3::new(1.5, 0.0, 0.0);
        let throw = Command {
            throw_grenade: true,
            pitch: -0.6,
            ..Command::default()
        };
        g.step(&world, &[throw, Command::default()]);
        assert_eq!(g.players[0].frags, 1);
        for _ in 0..(4.0 / TICK) as usize {
            g.step(&world, &[Command::default(), Command::default()]);
        }
        assert!(g.grenades.is_empty());
        assert!(
            g.events.iter().any(|e| matches!(e, Event::Exploded { .. }))
                || g.players[1].shield < 70.0
                || !g.players[1].alive
        );
    }

    /// Player 0 dropped from `height` onto the floor; their health after.
    fn fall_from(height: f32) -> (bool, f32) {
        let world = floor();
        let mut g = game();
        g.add_player();
        g.players[0].body.position = Vec3::new(0.0, 0.0, height);
        for _ in 0..300 {
            g.step(&world, &[Command::default()]);
        }
        (
            g.players[0].alive,
            g.players[0].shield + g.players[0].health,
        )
    }

    #[test]
    fn hard_landings_hurt_and_long_falls_kill() {
        assert_eq!(fall_from(2.0), (true, 115.0));
        let (alive, left) = fall_from(8.0);
        assert!(alive && left < 115.0, "{left}");
        assert!(!fall_from(12.0).0);
        assert!(!fall_from(20.0).0);
    }

    #[test]
    fn kill_zones_kill() {
        let world = floor();
        let mut g = game();
        g.add_player();
        g.players[0].body.position = Vec3::new(3.0, 3.0, 0.0);
        g.kill_zones.push(KillZone::new(
            Vec3::new(2.0, 2.0, -1.0),
            Vec3::X,
            Vec3::Z,
            Vec3::new(2.0, 2.0, 2.0),
        ));
        g.step(&world, &[Command::default()]);
        assert!(!g.players[0].alive);
        assert_eq!(g.players[0].score, -1);
    }
}
