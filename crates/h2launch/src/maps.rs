//! MCC's ids for Halo 2's multiplayer maps (`e_map_id`; Lockout is 44),
//! with the name MCC's enum uses, the name shown in the game and the
//! `.map` file in `halo2\h2_maps_win64_dx11`. The engine picks the map by
//! id; the file name is only for `--check`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapEntry {
    pub id: i32,
    /// libmcc's enum name without its `_map_id_halo2_` prefix.
    pub key: &'static str,
    pub display: &'static str,
    /// File stem in `h2_maps_win64_dx11`.
    pub file: &'static str,
}

const fn m(id: i32, key: &'static str, display: &'static str, file: &'static str) -> MapEntry {
    MapEntry {
        id,
        key,
        display,
        file,
    }
}

pub const LOCKOUT: i32 = 44;

/// The 25 multiplayer maps, ids 44 to 68.
pub const MULTIPLAYER: [MapEntry; 25] = [
    m(44, "lockout", "Lockout", "lockout"),
    m(45, "ascension", "Ascension", "ascension"),
    m(46, "midship", "Midship", "midship"),
    m(47, "ivory_tower", "Ivory Tower", "cyclotron"),
    m(48, "beaver_creek", "Beaver Creek", "beavercreek"),
    m(49, "burial_mounds", "Burial Mounds", "burial_mounds"),
    m(50, "colossus", "Colossus", "colossus"),
    m(51, "zanzibar", "Zanzibar", "zanzibar"),
    m(52, "coagulation", "Coagulation", "coagulation"),
    m(53, "headlong", "Headlong", "headlong"),
    m(54, "waterworks", "Waterworks", "waterworks"),
    m(55, "foundation", "Foundation", "foundation"),
    m(56, "containment", "Containment", "containment"),
    m(57, "warlock", "Warlock", "warlock"),
    m(58, "sanctuary", "Sanctuary", "deltatap"),
    m(59, "turf", "Turf", "turf"),
    m(60, "backwash", "Backwash", "backwash"),
    m(61, "elongation", "Elongation", "elongation"),
    m(62, "gemini", "Gemini", "gemini"),
    m(63, "relic", "Relic", "dune"),
    m(64, "terminal", "Terminal", "triplicate"),
    m(65, "desolation", "Desolation", "derelict"),
    m(66, "tombstone", "Tombstone", "highplains"),
    m(67, "district", "District", "street_sweeper"),
    m(68, "uplift", "Uplift", "needle"),
];

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A map by id, enum name, shown name or file name, any case, with or
/// without spaces, underscores or `.map`.
pub fn find(name: &str) -> Option<&'static MapEntry> {
    let name = name.trim();
    let name = name.strip_suffix(".map").unwrap_or(name);
    if let Some(id) = crate::util::parse_u64(name) {
        return MULTIPLAYER.iter().find(|e| e.id as u64 == id);
    }
    let want = squash(name);
    MULTIPLAYER
        .iter()
        .find(|e| squash(e.key) == want || squash(e.display) == want || squash(e.file) == want)
}

pub fn by_id(id: i32) -> Option<&'static MapEntry> {
    MULTIPLAYER.iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(MULTIPLAYER.len(), 25);
        for (k, e) in MULTIPLAYER.iter().enumerate() {
            assert_eq!(e.id, 44 + k as i32);
        }
        assert_eq!(MULTIPLAYER[0].id, LOCKOUT);
        assert_eq!(MULTIPLAYER[0].file, "lockout");
        let mut files: Vec<_> = MULTIPLAYER.iter().map(|e| e.file).collect();
        files.sort();
        files.dedup();
        assert_eq!(files.len(), 25);
    }

    #[test]
    fn lookups() {
        assert_eq!(find("lockout").unwrap().id, 44);
        assert_eq!(find("Lockout").unwrap().id, 44);
        assert_eq!(find("44").unwrap().display, "Lockout");
        assert_eq!(find("0x2c").unwrap().id, 44);
        assert_eq!(find("Ivory Tower").unwrap().id, 47);
        assert_eq!(find("ivory_tower").unwrap().id, 47);
        assert_eq!(find("cyclotron.map").unwrap().id, 47);
        assert_eq!(find("street_sweeper").unwrap().display, "District");
        assert_eq!(find("beavercreek").unwrap().id, 48);
        assert!(find("pillar_of_autumn").is_none());
        assert!(find("43").is_none());
        assert_eq!(by_id(68).unwrap().file, "needle");
    }
}
