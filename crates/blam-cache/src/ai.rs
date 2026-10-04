//! The campaign's AI: the squads a scenario places (who stands where, on
//! which side, with which weapon) and the characters they are made of
//! (`char` tags: which biped, how tough, how far they see, how they
//! fight).

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};

const SCNR_VEHICLE_PALETTE: usize = 0x78;
const SCNR_WEAPON_PALETTE: usize = 0x98;
const SCNR_UNIT_SEATS: usize = 0x228;
const UNIT_SEATS_SIZE: usize = 0x8;
const SCNR_SQUAD_GROUPS: usize = 0x158;
const SCNR_SQUADS: usize = 0x160;
const SQUAD_GROUP_SIZE: usize = 0x24;
const SCNR_CHARACTER_PALETTE: usize = 0x178;
const SQUAD_SIZE: usize = 0x74;
const SQUAD_LOCATIONS: usize = 0x48;
const LOCATION_SIZE: usize = 0x64;
const PALETTE_SIZE: usize = 0x28;
const CHARACTER_PALETTE_SIZE: usize = 0x8;

const CHAR_PARENT: usize = 0x4;
const CHAR_UNIT: usize = 0xC;
/// The character's variants, each with the three-letter designator its
/// mission dialogue is recorded under.
const CHAR_VARIANTS: usize = 0x2C;
const CHAR_GENERAL: usize = 0x34;
const UNIT_DEFAULT_TEAM: usize = 0xC0;
const CHAR_VITALITY: usize = 0x3C;
const CHAR_PERCEPTION: usize = 0x4C;
const CHAR_CHARGE: usize = 0x7C;
const CHAR_WEAPONS: usize = 0xCC;
const CHAR_GRENADES: usize = 0xDC;
const GENERAL_SIZE: usize = 0xC;
const VARIANT_SIZE: usize = 0xC;
const VITALITY_SIZE: usize = 0x70;
const PERCEPTION_SIZE: usize = 0x34;
const CHARGE_SIZE: usize = 0x40;
const WEAPONS_SIZE: usize = 0xCC;
const GRENADES_SIZE: usize = 0x3C;
const FIRING_PATTERNS: usize = 0xBC;
const FIRING_PATTERN_SIZE: usize = 0x40;

/// Squad flags.
const SQUAD_BLIND: u32 = 1 << 8;
const SQUAD_DEAF: u32 = 1 << 9;
const SQUAD_BRAINDEAD: u32 = 1 << 10;
const SQUAD_INITIALLY_PLACED: u32 = 1 << 12;
/// Starting location flags.
const LOCATION_ASLEEP: u32 = 1 << 0;
const LOCATION_ALWAYS_PLACE: u32 = 1 << 3;
const LOCATION_HIDDEN: u32 = 1 << 4;

/// Which side a squad fights on (the scenario's team numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AiTeam {
    /// The character's own.
    #[default]
    Default,
    Player,
    Human,
    Covenant,
    Flood,
    Sentinel,
    Heretic,
    Prophet,
    Other(u16),
}

impl AiTeam {
    pub fn from_number(n: u16) -> AiTeam {
        match n {
            0 => AiTeam::Default,
            1 => AiTeam::Player,
            2 => AiTeam::Human,
            3 => AiTeam::Covenant,
            4 => AiTeam::Flood,
            5 => AiTeam::Sentinel,
            6 => AiTeam::Heretic,
            7 => AiTeam::Prophet,
            n => AiTeam::Other(n),
        }
    }
}

/// A group of actors the scenario places together.
#[derive(Debug, Clone, PartialEq)]
pub struct Squad {
    pub name: String,
    /// The squad group it's in.
    pub group: Option<u16>,
    pub team: AiTeam,
    /// In the level from the start (otherwise a script places it).
    pub initially_placed: bool,
    pub blind: bool,
    pub deaf: bool,
    /// Stands there and does nothing.
    pub braindead: bool,
    /// How many actors on normal and on legendary (heroic is in between);
    /// zero means one per starting location.
    pub counts: (u16, u16),
    /// Defaults for its actors: indices into the scenario's character and
    /// weapon palettes, and its vehicle palette.
    pub character: Option<u16>,
    pub weapon: Option<u16>,
    pub secondary: Option<u16>,
    pub vehicle: Option<u16>,
    /// The variant of its vehicles ("" for the default).
    pub vehicle_variant: String,
    /// The zone and the order (`orders::orders`) it starts with.
    pub zone: Option<u16>,
    pub order: Option<u16>,
    /// The command script its actors run once placed (unless their
    /// starting location has its own), by index.
    pub placement_script: Option<u16>,
    pub locations: Vec<StartingLocation>,
}

/// Where one of a squad's actors starts.
#[derive(Debug, Clone, PartialEq)]
pub struct StartingLocation {
    pub name: String,
    pub position: [f32; 3],
    /// Yaw in radians.
    pub facing: f32,
    pub asleep: bool,
    /// Placed even when the squad has fewer actors than places.
    pub always: bool,
    pub hidden: bool,
    /// Overrides of the squad's defaults.
    pub character: Option<u16>,
    pub weapon: Option<u16>,
    pub secondary: Option<u16>,
    pub vehicle: Option<u16>,
    pub vehicle_variant: String,
    /// Which seat of the vehicle the actor starts in.
    pub seat: SeatType,
    /// The command script the actor runs once placed, by index.
    pub placement_script: Option<u16>,
}

/// Where a squad's actor sits in the vehicle it starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeatType {
    /// Driving (the default).
    #[default]
    Default,
    Passenger,
    Gunner,
    Driver,
    SmallCargo,
    LargeCargo,
    /// In the vehicle, but not driving it.
    NoDriver,
    /// Not in it: the vehicle is placed empty.
    NoVehicle,
}

impl SeatType {
    fn from_number(n: u16) -> SeatType {
        match n {
            1 => SeatType::Passenger,
            2 => SeatType::Gunner,
            3 => SeatType::Driver,
            4 => SeatType::SmallCargo,
            5 => SeatType::LargeCargo,
            6 => SeatType::NoDriver,
            7 => SeatType::NoVehicle,
            _ => SeatType::Default,
        }
    }
}

/// A kind of actor: Grunt, Jackal, Elite, Marine...
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Character {
    pub name: String,
    /// The biped it walks around as.
    pub unit: Option<DatumIndex>,
    pub kind: CharacterKind,
    pub vitality: Vitality,
    pub perception: Perception,
    pub weapon: Option<CharacterWeapon>,
    pub grenades: Option<CharacterGrenades>,
    /// How close it goes to melee, and how likely it is to (per second).
    pub melee_range: f32,
    pub melee_chance: f32,
    /// The designators of the voices it speaks mission dialogue in
    /// ("jon", "nrl"...).
    pub voices: Vec<String>,
    /// The model variants it comes in ("minor_scl"...), for its colours.
    pub model_variants: Vec<String>,
    /// The side its biped is on (heretics against the Covenant...).
    pub team: AiTeam,
}

/// The character's type, from its general properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CharacterKind {
    Elite,
    Jackal,
    Grunt,
    Hunter,
    Engineer,
    Assassin,
    Player,
    Marine,
    Crew,
    CombatForm,
    InfectionForm,
    CarrierForm,
    Monitor,
    Sentinel,
    #[default]
    None,
    MountedWeapon,
    Brute,
    Prophet,
    Bugger,
    Juggernaut,
}

impl CharacterKind {
    fn from_number(n: i16) -> CharacterKind {
        use CharacterKind::*;
        match n {
            0 => Elite,
            1 => Jackal,
            2 => Grunt,
            3 => Hunter,
            4 => Engineer,
            5 => Assassin,
            6 => Player,
            7 => Marine,
            8 => Crew,
            9 => CombatForm,
            10 => InfectionForm,
            11 => CarrierForm,
            12 => Monitor,
            13 => Sentinel,
            15 => MountedWeapon,
            16 => Brute,
            17 => Prophet,
            18 => Bugger,
            19 => Juggernaut,
            _ => None,
        }
    }
}

/// How much body and shield an actor has, on normal and on legendary.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vitality {
    pub body: f32,
    pub shield: f32,
    pub legendary_body: f32,
    pub legendary_shield: f32,
    /// Seconds for shields to come back completely.
    pub shield_recharge_time: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Perception {
    pub vision_distance: f32,
    /// Radians either side of straight ahead seen at range.
    pub vision_angle: f32,
    /// Seen out of the corner of the eye within this distance.
    pub peripheral_distance: f32,
    /// Seconds to notice someone in sight, in a fight and when calm.
    pub combat_time: f32,
    pub calm_time: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CharacterWeapon {
    pub weapon: Option<DatumIndex>,
    /// Fires only at targets within this range (and beyond the minimum).
    pub max_range: f32,
    pub min_range: f32,
    /// The distance it likes to fight at.
    pub combat_range: (f32, f32),
    /// Seconds before its first burst at someone new.
    pub first_burst_delay: (f32, f32),
    /// Accuracy (0-1) over a run of bursts, on normal and legendary.
    pub accuracy: (f32, f32),
    pub legendary_accuracy: (f32, f32),
    /// Trigger pulls a second (zero: held down), from the first firing
    /// pattern.
    pub rate_of_fire: f32,
    /// Seconds a burst lasts and between bursts.
    pub burst_duration: (f32, f32),
    pub burst_separation: (f32, f32),
    /// Scales the damage its shots do (zero: unchanged).
    pub damage_modifier: f32,
    /// Radians of spread added to its shots.
    pub projectile_error: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CharacterGrenades {
    /// 0 frag, 1 plasma.
    pub kind: u16,
    /// Considers throwing at targets within this range.
    pub range: (f32, f32),
    /// Chance of throwing in a second, and seconds between throws.
    pub chance: f32,
    pub delay: f32,
    pub count: (u16, u16),
}

pub(crate) fn scenario_data(set: &mut MapSet) -> Result<Vec<u8>> {
    let map = &mut set.map;
    let scnr = map
        .tag(map.scenario)
        .cloned()
        .ok_or_else(|| Error::Corrupt("scenario tag missing".into()))?;
    map.read_tag_data(&scnr)
}

pub(crate) fn ascii(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

pub(crate) fn index(b: &[u8], o: usize) -> Option<u16> {
    u16::try_from(i16_at(b, o)).ok()
}

fn range(b: &[u8], o: usize) -> (f32, f32) {
    (f32_at(b, o), f32_at(b, o + 4))
}

fn tag_ref(b: &[u8], o: usize) -> Option<DatumIndex> {
    Some(DatumIndex(u32_at(b, o + 4))).filter(|d| *d != DatumIndex::NONE)
}

/// The scenario's squads, in block order (scripts name them by index).
pub fn squads(set: &mut MapSet) -> Result<Vec<Squad>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let squads = map.read_block(meta, &data, SCNR_SQUADS, SQUAD_SIZE)?;
    let mut out = Vec::new();
    for e in squads.as_chunks::<SQUAD_SIZE>().0 {
        let locations = map.read_block(meta, e, SQUAD_LOCATIONS, LOCATION_SIZE)?;
        let locations = locations
            .as_chunks::<LOCATION_SIZE>()
            .0
            .iter()
            .map(|l| {
                let flags = u32_at(l, 0x1C);
                StartingLocation {
                    name: map.string_id(u32_at(l, 0)).unwrap_or_default().to_string(),
                    position: [f32_at(l, 4), f32_at(l, 8), f32_at(l, 0xC)],
                    facing: f32_at(l, 0x14),
                    asleep: flags & LOCATION_ASLEEP != 0,
                    always: flags & LOCATION_ALWAYS_PLACE != 0,
                    hidden: flags & LOCATION_HIDDEN != 0,
                    character: index(l, 0x20),
                    weapon: index(l, 0x22),
                    secondary: index(l, 0x24),
                    vehicle: index(l, 0x28),
                    vehicle_variant: map
                        .string_id(u32_at(l, 0x34))
                        .unwrap_or_default()
                        .to_string(),
                    seat: SeatType::from_number(i16_at(l, 0x2A) as u16),
                    placement_script: index(l, 0x60),
                }
            })
            .collect();
        let flags = u32_at(e, 0x20);
        out.push(Squad {
            name: ascii(&e[..0x20]),
            group: index(e, 0x26),
            team: AiTeam::from_number(i16_at(e, 0x24) as u16),
            initially_placed: flags & SQUAD_INITIALLY_PLACED != 0,
            blind: flags & SQUAD_BLIND != 0,
            deaf: flags & SQUAD_DEAF != 0,
            braindead: flags & SQUAD_BRAINDEAD != 0,
            counts: (index(e, 0x2C).unwrap_or(0), index(e, 0x2E).unwrap_or(0)),
            vehicle: index(e, 0x34),
            vehicle_variant: map
                .string_id(u32_at(e, 0x44))
                .unwrap_or_default()
                .to_string(),
            character: index(e, 0x36),
            weapon: index(e, 0x3C),
            secondary: index(e, 0x3E),
            zone: index(e, 0x38),
            order: index(e, 0x42),
            placement_script: index(e, 0x70),
            locations,
        });
    }
    Ok(out)
}

/// A group of squads (and of other groups) scripts handle together.
#[derive(Debug, Clone, PartialEq)]
pub struct SquadGroup {
    pub name: String,
    pub parent: Option<u16>,
    /// The order its squads start with when they don't have their own.
    pub order: Option<u16>,
}

pub fn squad_groups(set: &mut MapSet) -> Result<Vec<SquadGroup>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let groups = map.read_block(meta, &data, SCNR_SQUAD_GROUPS, SQUAD_GROUP_SIZE)?;
    Ok(groups
        .as_chunks::<SQUAD_GROUP_SIZE>()
        .0
        .iter()
        .map(|g| SquadGroup {
            name: ascii(&g[..0x20]),
            parent: index(g, 0x20),
            order: index(g, 0x22),
        })
        .collect())
}

/// The characters squads are made of (`char` tags).
pub fn character_palette(set: &mut MapSet) -> Result<Vec<DatumIndex>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let palette = map.read_block(meta, &data, SCNR_CHARACTER_PALETTE, CHARACTER_PALETTE_SIZE)?;
    Ok(palette
        .as_chunks::<CHARACTER_PALETTE_SIZE>()
        .0
        .iter()
        .map(|p| DatumIndex(u32_at(p, 4)))
        .collect())
}

/// The weapons squads' weapon indices pick from (the scenario's weapon
/// palette).
pub fn weapon_palette(set: &mut MapSet) -> Result<Vec<DatumIndex>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let palette = map.read_block(meta, &data, SCNR_WEAPON_PALETTE, PALETTE_SIZE)?;
    Ok(palette
        .as_chunks::<PALETTE_SIZE>()
        .0
        .iter()
        .map(|p| DatumIndex(u32_at(p, 4)))
        .collect())
}

/// The vehicles squads' vehicle indices pick from (the scenario's
/// vehicle palette).
pub fn vehicle_palette(set: &mut MapSet) -> Result<Vec<DatumIndex>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let palette = map.read_block(meta, &data, SCNR_VEHICLE_PALETTE, PALETTE_SIZE)?;
    Ok(palette
        .as_chunks::<PALETTE_SIZE>()
        .0
        .iter()
        .map(|p| DatumIndex(u32_at(p, 4)))
        .collect())
}

/// The seats scripts name ("warthog_d", "pelican_g"...): per entry, a
/// unit (`vehi`) and which of its seats, as bits over its seat block.
/// A script's seat value is `count << 16 | first entry`.
pub fn unit_seat_mappings(set: &mut MapSet) -> Result<Vec<(DatumIndex, u32)>> {
    let data = scenario_data(set)?;
    let map = &mut set.map;
    let meta = map.meta_region();
    let block = map.read_block(meta, &data, SCNR_UNIT_SEATS, UNIT_SEATS_SIZE)?;
    Ok(block
        .as_chunks::<UNIT_SEATS_SIZE>()
        .0
        .iter()
        .map(|e| (DatumIndex(u32_at(e, 0)), u32_at(e, 4)))
        .collect())
}

/// One character tag's own blocks (empty where it leaves them to its
/// parent).
struct CharacterBlocks {
    parent: Option<DatumIndex>,
    unit: Option<DatumIndex>,
    variants: Vec<u8>,
    general: Vec<u8>,
    vitality: Vec<u8>,
    perception: Vec<u8>,
    charge: Vec<u8>,
    weapons: Vec<u8>,
    firing: Vec<u8>,
    grenades: Vec<u8>,
}

fn character_blocks(set: &mut MapSet, tag: DatumIndex) -> Result<(String, CharacterBlocks)> {
    let (src, t, d) = set.tag_data(tag)?;
    if d.len() < CHAR_GRENADES + 8 {
        return Err(Error::Corrupt(format!(
            "character tag {} too short",
            t.name
        )));
    }
    let file = set.get(src);
    let region = file.meta_region();
    let weapons = file.read_block(region, &d, CHAR_WEAPONS, WEAPONS_SIZE)?;
    let firing = match weapons.get(..WEAPONS_SIZE) {
        Some(w) => file.read_block(region, w, FIRING_PATTERNS, FIRING_PATTERN_SIZE)?,
        None => Vec::new(),
    };
    Ok((
        t.name.clone(),
        CharacterBlocks {
            parent: tag_ref(&d, CHAR_PARENT),
            unit: tag_ref(&d, CHAR_UNIT),
            variants: file.read_block(region, &d, CHAR_VARIANTS, VARIANT_SIZE)?,
            general: file.read_block(region, &d, CHAR_GENERAL, GENERAL_SIZE)?,
            vitality: file.read_block(region, &d, CHAR_VITALITY, VITALITY_SIZE)?,
            perception: file.read_block(region, &d, CHAR_PERCEPTION, PERCEPTION_SIZE)?,
            charge: file.read_block(region, &d, CHAR_CHARGE, CHARGE_SIZE)?,
            weapons,
            firing,
            grenades: file.read_block(region, &d, CHAR_GRENADES, GRENADES_SIZE)?,
        },
    ))
}

/// A character, with what it leaves out taken from its parents.
pub fn read_character(set: &mut MapSet, tag: DatumIndex) -> Result<Character> {
    let (name, mut own) = character_blocks(set, tag)?;
    // Fill the gaps from up the parent chain.
    let mut parent = own.parent;
    let mut seen = vec![tag];
    while let Some(p) = parent.filter(|p| !seen.contains(p)) {
        seen.push(p);
        let Ok((_, up)) = character_blocks(set, p) else {
            break;
        };
        let fill = |mine: &mut Vec<u8>, theirs: Vec<u8>| {
            if mine.is_empty() {
                *mine = theirs;
            }
        };
        if own.unit.is_none() {
            own.unit = up.unit;
        }
        if own.weapons.is_empty() {
            own.firing = up.firing.clone();
        }
        fill(&mut own.variants, up.variants);
        fill(&mut own.general, up.general);
        fill(&mut own.vitality, up.vitality);
        fill(&mut own.perception, up.perception);
        fill(&mut own.charge, up.charge);
        fill(&mut own.weapons, up.weapons);
        fill(&mut own.grenades, up.grenades);
        parent = up.parent;
    }
    let first = |b: &[u8], size: usize| b.get(..size).map(<[u8]>::to_vec);
    let kind = first(&own.general, GENERAL_SIZE).map_or(CharacterKind::None, |g| {
        CharacterKind::from_number(i16_at(&g, 4))
    });
    let vitality =
        first(&own.vitality, VITALITY_SIZE).map_or_else(Vitality::default, |v| Vitality {
            body: f32_at(&v, 0x4),
            shield: f32_at(&v, 0x8),
            legendary_body: f32_at(&v, 0xC),
            legendary_shield: f32_at(&v, 0x10),
            shield_recharge_time: f32_at(&v, 0x4C),
        });
    let perception =
        first(&own.perception, PERCEPTION_SIZE).map_or_else(Perception::default, |p| Perception {
            vision_distance: f32_at(&p, 0x4),
            vision_angle: f32_at(&p, 0xC),
            peripheral_distance: f32_at(&p, 0x14),
            combat_time: f32_at(&p, 0x24),
            calm_time: f32_at(&p, 0x2C),
        });
    let (melee_range, melee_chance) =
        first(&own.charge, CHARGE_SIZE).map_or((0.0, 0.0), |c| (f32_at(&c, 0x4), f32_at(&c, 0x8)));
    let pattern = first(&own.firing, FIRING_PATTERN_SIZE);
    let weapon = first(&own.weapons, WEAPONS_SIZE).map(|w| CharacterWeapon {
        weapon: tag_ref(&w, 0x4),
        max_range: f32_at(&w, 0xC),
        min_range: f32_at(&w, 0x10),
        combat_range: range(&w, 0x14),
        first_burst_delay: range(&w, 0x48),
        accuracy: range(&w, 0x98),
        legendary_accuracy: range(&w, 0xB0),
        rate_of_fire: pattern.as_ref().map_or(0.0, |p| f32_at(p, 0)),
        burst_duration: pattern.as_ref().map_or((0.0, 0.0), |p| range(p, 0x20)),
        burst_separation: pattern.as_ref().map_or((0.0, 0.0), |p| range(p, 0x28)),
        damage_modifier: pattern.as_ref().map_or(0.0, |p| f32_at(p, 0x30)),
        projectile_error: pattern.as_ref().map_or(0.0, |p| f32_at(p, 0x34)),
    });
    let grenades = first(&own.grenades, GRENADES_SIZE).map(|g| CharacterGrenades {
        kind: i16_at(&g, 4) as u16,
        range: range(&g, 0x18),
        chance: f32_at(&g, 0x24),
        delay: f32_at(&g, 0x28),
        count: (index(&g, 0x34).unwrap_or(0), index(&g, 0x36).unwrap_or(0)),
    });
    // The unit's default team, just after its object part.
    let team = own
        .unit
        .and_then(|u| set.tag_data(u).ok())
        .filter(|(_, _, d)| d.len() >= UNIT_DEFAULT_TEAM + 2)
        .map_or(AiTeam::Default, |(_, _, d)| {
            AiTeam::from_number(i16_at(&d, UNIT_DEFAULT_TEAM) as u16)
        });
    Ok(Character {
        name,
        unit: own.unit,
        team,
        kind,
        vitality,
        perception,
        weapon,
        grenades,
        melee_range,
        melee_chance,
        voices: own
            .variants
            .as_chunks::<VARIANT_SIZE>()
            .0
            .iter()
            .filter_map(|v| set.map.string_id(u32_at(v, 8)))
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .collect(),
        model_variants: own
            .variants
            .as_chunks::<VARIANT_SIZE>()
            .0
            .iter()
            .filter_map(|v| set.map.string_id(u32_at(v, 0)))
            .map(str::to_string)
            .collect(),
    })
}
