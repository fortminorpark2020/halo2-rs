//! `s_game_options`: the 0x2BF30-byte block the host hands to the engine's
//! `initialize_game(host, options)`. It is packed to one byte, except the
//! 0x298-byte player block at 0x60, which keeps natural alignment inside
//! (so the player entries start at 0xF0 and the peer count is at 0x2F0).
//!
//! The buffer is built with small writers and one named constant per
//! offset, and the tests pin every offset against the research tables
//! (game-options.md section 2 with its verify file, host-interface.verify
//! A1/A2). Everything starts at zero; nothing gets libmcc's -1 defaults.

use std::fmt;

/// Size of the whole block.
pub const SIZE: usize = 0x2BF30;

/// Byte offsets of the fields.
pub mod off {
    /// u16 flag bits (see `flags`).
    pub const FLAGS: usize = 0x000;
    /// u8, no known reader.
    pub const OPTION_1: usize = 0x002;
    /// u8 "option_2": maximum players (the engine checks (v - 1) < 16;
    /// MCC's own set-up writes 16).
    pub const MAX_PLAYERS: usize = 0x003;
    /// u8 "option_3": team count (MCC writes 3).
    pub const TEAM_COUNT: usize = 0x004;
    /// u8 x 2, probably the multiplayer and single-player simulation
    /// client counts.
    pub const NETWORK_OPTION_1: usize = 0x008;
    pub const NETWORK_OPTION_2: usize = 0x009;
    /// u8 ticks per second.
    pub const GAME_TICK: usize = 0x00A;
    /// u8 "un_0", compared with the peer count by halo2.
    pub const UN_0: usize = 0x00B;
    /// i32 `game_mode` (see `mode`).
    pub const GAME_MODE: usize = 0x00C;
    /// i32 MCC map id as a plain number.
    pub const LEGACY_MAP_ID: usize = 0x010;
    /// 16-byte `s_scenario_map_id`.
    pub const MAP_ID: usize = 0x014;
    pub const CUSTOM_CAMPAIGN_MAP_ID: usize = 0x024;
    pub const DIFFICULTY: usize = 0x034;
    pub const INSERTION_POINT: usize = 0x03C;
    /// u32 scripted-event permission bits.
    pub const SCRIPTED_EVENT_PERMISSION: usize = 0x040;
    /// i16 per-engine variant slot id (HaloX leaves 0; -1 is the fallback
    /// to try if a load stalls).
    pub const VARIANT_SLOT_ID: usize = 0x044;
    /// u64 skull bits.
    pub const SKULLS: usize = 0x048;
    /// u64 "un_2": the host's secure address.
    pub const HOST_SECURE_ADDRESS: usize = 0x050;
    /// u64 host address; must not be zero for halo2.
    pub const HOST_ADDRESS: usize = 0x058;
    /// Start of `s_player_options` (0x298 bytes, natural alignment).
    pub const PLAYER_OPTIONS: usize = 0x060;
    /// u64[17] peer (machine) addresses.
    pub const PEER_ADDRESSES: usize = 0x060;
    /// i32 player count.
    pub const PLAYER_COUNT: usize = 0x0E8;
    /// 16 player entries of 0x20 bytes.
    pub const PLAYERS: usize = 0x0F0;
    /// i32 peer count.
    pub const PEER_COUNT: usize = 0x2F0;
    /// i32 after the peer count (libmcc's initialiser writes -1; we keep 0).
    pub const PLAYER_OPTIONS_TAIL: usize = 0x2F4;
    /// u64 "un_3": this machine's network id.
    pub const LOCAL_NETWORK_ID: usize = 0x2F8;
    /// The engine's own game variant, written by the variant object.
    pub const GAME_VARIANT: usize = 0x300;
    /// 0x408-byte custom data ("msf_" block) after the game variant.
    pub const GAME_VARIANT_CUSTOM: usize = 0x1CF00;
    /// The map variant (none for Halo 2).
    pub const MAP_VARIANT: usize = 0x1D308;
    pub const MAP_VARIANT_CUSTOM: usize = 0x2BB08;
    /// u32 size of the saved game state.
    pub const SAVED_GAME_SIZE: usize = 0x2BF10;
    /// Pointers: saved game state, saved film path (non-null = theater),
    /// theater root folder.
    pub const SAVED_GAME_STATE: usize = 0x2BF18;
    pub const SAVED_FILM_PATH: usize = 0x2BF20;
    pub const FILM_ROOT: usize = 0x2BF28;
}

pub const PEER_SLOTS: usize = 17;
pub const PLAYER_SLOTS: usize = 16;
pub const PLAYER_STRIDE: usize = 0x20;
pub const PLAYER_OPTIONS_SIZE: usize = 0x298;
pub const GAME_VARIANT_SIZE: usize = 0x1CC00;
pub const MAP_VARIANT_SIZE: usize = 0xE800;
pub const CUSTOM_DATA_SIZE: usize = 0x408;

/// Fields of a player entry, relative to its start.
pub mod player {
    /// u64 XUID.
    pub const XUID: usize = 0x00;
    /// u64 address of the player's machine (same value as its peer slot).
    pub const ADDRESS: usize = 0x08;
    /// i32 preferred team.
    pub const TEAM: usize = 0x10;
    /// u8 local players on that machine (no engine reader found).
    pub const LOCAL_COUNT: usize = 0x14;
    /// i32 peer index; equal to the local peer's index for local players.
    pub const PEER_INDEX: usize = 0x18;
    /// i32 controller index on its machine.
    pub const CONTROLLER: usize = 0x1C;
}

/// Bits of the u16 at 0x00.
pub mod flags {
    /// "Multiplayer" (set by HaloX for every mode; halo2 needs it).
    pub const MULTIPLAYER: u16 = 1 << 3;
    /// Local multiplayer / splitscreen.
    pub const LOCAL_MULTIPLAYER: u16 = 1 << 4;
    /// Input locked.
    pub const INPUT_LOCKED: u16 = 1 << 5;
    /// Listen server (with bit 3: online host).
    pub const LISTEN_SERVER: u16 = 1 << 6;
    /// Enables the theater root path.
    pub const FILM_ROOT: u16 = 1 << 7;
    /// Debug: reads init.txt and enables the terminal.
    pub const DEBUG: u16 = 1 << 9;
}

/// `e_game_mode`, the MCC numbering (not the engine's internal one).
pub mod mode {
    pub const NONE: i32 = 0;
    pub const CAMPAIGN: i32 = 1;
    pub const SPARTAN_OPS: i32 = 2;
    pub const MULTIPLAYER: i32 = 3;
    pub const UI_SHELL: i32 = 4;
    pub const FIREFIGHT: i32 = 5;
}

/// `s_scenario_map_id.flags` of a built-in map.
pub const BUILTIN_MAP_FLAGS: u16 = 0x8888;
/// The address HaloX gives the one machine of an offline game. Any
/// non-zero value works; zero freezes halo2's loading.
pub const HOST_ADDRESS_STUB: u64 = 123;

/// What goes into the block for an offline match on one machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub map_id: i32,
    pub xuid: u64,
    pub flags: u16,
    pub game_tick: u8,
    /// 0x03 and 0x04. MCC itself always writes 16 and 3; HaloX leaves 0
    /// and 0 (game-options.verify A13 recommends MCC's values first).
    pub max_players: u8,
    pub team_count: u8,
}

impl Launch {
    pub fn offline(map_id: i32, xuid: u64) -> Launch {
        Launch {
            map_id,
            xuid,
            flags: flags::MULTIPLAYER,
            game_tick: 60,
            max_players: 16,
            team_count: 3,
        }
    }
}

/// The block, kept 8-byte aligned and on the heap (it is 176 KB).
pub struct GameOptions {
    words: Vec<u64>,
}

impl Default for GameOptions {
    fn default() -> Self {
        GameOptions::new()
    }
}

impl fmt::Debug for GameOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GameOptions({SIZE:#x} bytes)")
    }
}

impl GameOptions {
    /// All zero.
    pub fn new() -> GameOptions {
        const _: () = assert!(SIZE.is_multiple_of(8));
        GameOptions {
            words: vec![0; SIZE / 8],
        }
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the Vec holds SIZE bytes of plain integers.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast(), SIZE) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above, and we hold the only reference.
        unsafe { std::slice::from_raw_parts_mut(self.words.as_mut_ptr().cast(), SIZE) }
    }

    /// The start of the 0x2BF30-byte buffer: what the variant object's
    /// copy and the engine's `initialize_game` read and write. (The
    /// `GameOptions` value itself is only the Vec's 24-byte header.)
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.words.as_mut_ptr().cast()
    }

    /// Hands the buffer over for the rest of the run and returns the
    /// pointer `initialize_game` gets: the buffer itself, never the address
    /// of this struct. The memory is never freed (the engine may keep
    /// reading it; the process ends with TerminateProcess).
    pub fn leak_for_engine(self: Box<Self>) -> *mut u8 {
        Box::leak(self).as_mut_ptr()
    }

    pub fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes_mut()[at..at + bytes.len()].copy_from_slice(bytes);
    }
    pub fn put_u8(&mut self, at: usize, v: u8) {
        self.put(at, &[v]);
    }
    pub fn put_u16(&mut self, at: usize, v: u16) {
        self.put(at, &v.to_le_bytes());
    }
    pub fn put_i16(&mut self, at: usize, v: i16) {
        self.put(at, &v.to_le_bytes());
    }
    pub fn put_i32(&mut self, at: usize, v: i32) {
        self.put(at, &v.to_le_bytes());
    }
    pub fn put_u32(&mut self, at: usize, v: u32) {
        self.put(at, &v.to_le_bytes());
    }
    pub fn put_u64(&mut self, at: usize, v: u64) {
        self.put(at, &v.to_le_bytes());
    }

    pub fn get_u16(&self, at: usize) -> u16 {
        u16::from_le_bytes(self.bytes()[at..at + 2].try_into().unwrap())
    }
    pub fn get_i32(&self, at: usize) -> i32 {
        i32::from_le_bytes(self.bytes()[at..at + 4].try_into().unwrap())
    }
    pub fn get_u64(&self, at: usize) -> u64 {
        u64::from_le_bytes(self.bytes()[at..at + 8].try_into().unwrap())
    }

    /// A built-in `s_scenario_map_id`: id, part1 0, flags 0x8888, part2 0.
    pub fn put_builtin_map_id(&mut self, at: usize, id: i32) {
        self.put_i32(at, id);
        self.put_u16(at + 4, 0);
        self.put_u16(at + 6, BUILTIN_MAP_FLAGS);
        self.put_u64(at + 8, 0);
    }

    pub fn put_peer(&mut self, slot: usize, address: u64) {
        assert!(slot < PEER_SLOTS);
        self.put_u64(off::PEER_ADDRESSES + 8 * slot, address);
    }

    /// Player entry `slot`: XUID and machine address; the team, local
    /// count, peer index and controller stay 0 (what HaloX sends, and peer 0
    /// is this machine).
    pub fn put_player(&mut self, slot: usize, xuid: u64, address: u64) {
        assert!(slot < PLAYER_SLOTS);
        let at = off::PLAYERS + PLAYER_STRIDE * slot;
        self.put_u64(at + player::XUID, xuid);
        self.put_u64(at + player::ADDRESS, address);
    }

    /// The fields HaloX sets before the variant copies itself in: flags,
    /// player limit and team count, tick and mode.
    pub fn apply_base(&mut self, l: &Launch) {
        self.put_u16(off::FLAGS, l.flags);
        self.put_u8(off::MAX_PLAYERS, l.max_players);
        self.put_u8(off::TEAM_COUNT, l.team_count);
        self.put_u8(off::GAME_TICK, l.game_tick);
        self.put_i32(off::GAME_MODE, mode::MULTIPLAYER);
    }

    /// The fields written after the variant copy, so they win: the map,
    /// the host address, one peer (us) and one player (us).
    pub fn apply_match(&mut self, l: &Launch) {
        self.put_i32(off::LEGACY_MAP_ID, l.map_id);
        self.put_builtin_map_id(off::MAP_ID, l.map_id);
        self.put_u64(off::HOST_ADDRESS, HOST_ADDRESS_STUB);
        self.put_peer(0, HOST_ADDRESS_STUB);
        self.put_i32(off::PLAYER_COUNT, 1);
        self.put_i32(off::PEER_COUNT, 1);
        self.put_player(0, l.xuid, HOST_ADDRESS_STUB);
    }
}

/// A raw write the user asks for with `--set-option` or `--set-profile`,
/// for trying another value without a rebuild: `<offset>=<type>:<value>`,
/// e.g. `0x44=i16:-1` or `0x03=u8:0`.
#[derive(Clone, Debug, PartialEq)]
pub struct RawWrite {
    pub offset: usize,
    pub bytes: Vec<u8>,
    pub text: String,
}

impl RawWrite {
    pub fn parse(s: &str) -> Result<RawWrite, String> {
        let (o, rest) = s
            .split_once('=')
            .ok_or_else(|| format!("{s:?}: expected <offset>=<type>:<value>"))?;
        let (ty, v) = rest
            .split_once(':')
            .ok_or_else(|| format!("{s:?}: expected <type>:<value>"))?;
        let offset = crate::util::parse_u64(o.trim())
            .ok_or_else(|| format!("{s:?}: bad offset {o:?}"))? as usize;
        let v = v.trim();
        let int = || -> Result<i128, String> {
            crate::util::parse_i128(v).ok_or_else(|| format!("{s:?}: bad number {v:?}"))
        };
        let fits = |n: i128, lo: i128, hi: i128| -> Result<i128, String> {
            if n < lo || n > hi {
                Err(format!("{s:?}: {n} does not fit in {ty}"))
            } else {
                Ok(n)
            }
        };
        let bytes = match ty.trim() {
            "u8" => vec![fits(int()?, 0, 0xFF)? as u8],
            "i8" => vec![fits(int()?, -0x80, 0x7F)? as i8 as u8],
            "u16" => (fits(int()?, 0, 0xFFFF)? as u16).to_le_bytes().to_vec(),
            "i16" => (fits(int()?, -0x8000, 0x7FFF)? as i16)
                .to_le_bytes()
                .to_vec(),
            "u32" => (fits(int()?, 0, 0xFFFF_FFFF)? as u32)
                .to_le_bytes()
                .to_vec(),
            "i32" => (fits(int()?, i32::MIN as i128, i32::MAX as i128)? as i32)
                .to_le_bytes()
                .to_vec(),
            "u64" => (fits(int()?, 0, u64::MAX as i128)? as u64)
                .to_le_bytes()
                .to_vec(),
            "i64" => (fits(int()?, i64::MIN as i128, i64::MAX as i128)? as i64)
                .to_le_bytes()
                .to_vec(),
            "f32" => v
                .parse::<f32>()
                .map_err(|_| format!("{s:?}: bad float {v:?}"))?
                .to_le_bytes()
                .to_vec(),
            "hex" => crate::util::parse_hex_bytes(v)
                .ok_or_else(|| format!("{s:?}: bad hex bytes {v:?}"))?,
            other => {
                return Err(format!(
                    "{s:?}: unknown type {other:?} (u8 i8 u16 i16 u32 i32 u64 i64 f32 hex)"
                ))
            }
        };
        Ok(RawWrite {
            offset,
            bytes,
            text: s.to_string(),
        })
    }

    /// Whether it fits in a block of `size` bytes.
    pub fn fits(&self, size: usize) -> bool {
        self.offset
            .checked_add(self.bytes.len())
            .is_some_and(|end| end <= size)
    }

    pub fn apply(&self, buf: &mut [u8]) -> Result<(), String> {
        if !self.fits(buf.len()) {
            return Err(format!(
                "{}: offset {:#x} + {} bytes is past the end ({:#x})",
                self.text,
                self.offset,
                self.bytes.len(),
                buf.len()
            ));
        }
        buf[self.offset..self.offset + self.bytes.len()].copy_from_slice(&self.bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_match_the_research_tables() {
        // game-options.md 2.1/2.2 as corrected by host-interface.verify A1.
        assert_eq!(SIZE, 0x2BF30);
        assert_eq!(off::FLAGS, 0x00);
        assert_eq!(off::MAX_PLAYERS, 0x03);
        assert_eq!(off::TEAM_COUNT, 0x04);
        assert_eq!(off::NETWORK_OPTION_1, 0x08);
        assert_eq!(off::GAME_TICK, 0x0A);
        assert_eq!(off::UN_0, 0x0B);
        assert_eq!(off::GAME_MODE, 0x0C);
        assert_eq!(off::LEGACY_MAP_ID, 0x10);
        assert_eq!(off::MAP_ID, 0x14);
        assert_eq!(off::CUSTOM_CAMPAIGN_MAP_ID, 0x24);
        assert_eq!(off::DIFFICULTY, 0x34);
        assert_eq!(off::INSERTION_POINT, 0x3C);
        assert_eq!(off::SCRIPTED_EVENT_PERMISSION, 0x40);
        assert_eq!(off::VARIANT_SLOT_ID, 0x44);
        assert_eq!(off::SKULLS, 0x48);
        assert_eq!(off::HOST_SECURE_ADDRESS, 0x50);
        assert_eq!(off::HOST_ADDRESS, 0x58);
        assert_eq!(off::PLAYER_OPTIONS, 0x60);
        assert_eq!(off::PEER_ADDRESSES, 0x60);
        // 17 u64 addresses end at 0xE8: the player count follows.
        assert_eq!(off::PEER_ADDRESSES + 8 * PEER_SLOTS, off::PLAYER_COUNT);
        assert_eq!(off::PLAYER_COUNT, 0xE8);
        // The entries start 8-aligned (0xEC is padding), not at 0xEC.
        assert_eq!(off::PLAYERS, 0xF0);
        assert_eq!(PLAYER_STRIDE, 0x20);
        assert_eq!(off::PLAYERS + PLAYER_STRIDE * PLAYER_SLOTS, off::PEER_COUNT);
        assert_eq!(off::PEER_COUNT, 0x2F0);
        assert_eq!(off::PLAYER_OPTIONS_TAIL, 0x2F4);
        assert_eq!(
            off::PLAYER_OPTIONS + PLAYER_OPTIONS_SIZE,
            off::LOCAL_NETWORK_ID
        );
        assert_eq!(off::LOCAL_NETWORK_ID, 0x2F8);
        assert_eq!(off::GAME_VARIANT, 0x300);
        assert_eq!(
            off::GAME_VARIANT + GAME_VARIANT_SIZE,
            off::GAME_VARIANT_CUSTOM
        );
        assert_eq!(off::GAME_VARIANT_CUSTOM, 0x1CF00);
        assert_eq!(
            off::GAME_VARIANT_CUSTOM + CUSTOM_DATA_SIZE,
            off::MAP_VARIANT
        );
        assert_eq!(off::MAP_VARIANT, 0x1D308);
        assert_eq!(off::MAP_VARIANT + MAP_VARIANT_SIZE, off::MAP_VARIANT_CUSTOM);
        assert_eq!(off::MAP_VARIANT_CUSTOM, 0x2BB08);
        assert_eq!(
            off::MAP_VARIANT_CUSTOM + CUSTOM_DATA_SIZE,
            off::SAVED_GAME_SIZE
        );
        assert_eq!(off::SAVED_GAME_SIZE, 0x2BF10);
        assert_eq!(off::SAVED_GAME_STATE, 0x2BF18);
        assert_eq!(off::SAVED_FILM_PATH, 0x2BF20);
        assert_eq!(off::FILM_ROOT, 0x2BF28);
        assert_eq!(off::FILM_ROOT + 8, SIZE);
    }

    #[test]
    fn player_entry_fields() {
        assert_eq!(player::XUID, 0x00);
        assert_eq!(player::ADDRESS, 0x08);
        assert_eq!(player::TEAM, 0x10);
        assert_eq!(player::LOCAL_COUNT, 0x14);
        assert_eq!(player::PEER_INDEX, 0x18);
        assert_eq!(player::CONTROLLER, 0x1C);
        // Player 0's absolute offsets (host-interface.verify A1).
        assert_eq!(off::PLAYERS + player::ADDRESS, 0xF8);
        assert_eq!(off::PLAYERS + player::TEAM, 0x100);
        assert_eq!(off::PLAYERS + player::PEER_INDEX, 0x108);
        assert_eq!(off::PLAYERS + player::CONTROLLER, 0x10C);
        // Player 15 ends at 0x2EF.
        assert_eq!(off::PLAYERS + PLAYER_STRIDE * 15 + PLAYER_STRIDE - 1, 0x2EF);
    }

    #[test]
    fn flags_and_modes() {
        assert_eq!(flags::MULTIPLAYER, 0x0008);
        assert_eq!(flags::LOCAL_MULTIPLAYER, 0x0010);
        assert_eq!(flags::LISTEN_SERVER, 0x0040);
        assert_eq!(flags::DEBUG, 0x0200);
        // MCC numbering: 3 is multiplayer (2 would be spartan ops).
        assert_eq!(mode::MULTIPLAYER, 3);
        assert_eq!(mode::SPARTAN_OPS, 2);
        assert_eq!(mode::UI_SHELL, 4);
    }

    /// The golden bytes of recipe (a) for Lockout, before any variant copy
    /// (game-options.verify C5, with A13's 16/3 at 0x03/0x04).
    #[test]
    fn recipe_a_golden_bytes() {
        let xuid = 0x0123_4567_89AB_CDEFu64;
        let l = Launch::offline(44, xuid);
        let mut o = GameOptions::new();
        o.apply_base(&l);
        o.apply_match(&l);
        let b = o.bytes();
        assert_eq!(b.len(), SIZE);
        let mut want = vec![0u8; SIZE];
        let mut set = |at: usize, v: &[u8]| want[at..at + v.len()].copy_from_slice(v);
        set(0x00, &[0x08, 0x00]);
        set(0x03, &[0x10, 0x03]);
        set(0x0A, &[0x3C]);
        set(0x0C, &[3, 0, 0, 0]);
        set(0x10, &[0x2C, 0, 0, 0]);
        set(
            0x14,
            &[0x2C, 0, 0, 0, 0, 0, 0x88, 0x88, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        set(0x58, &[0x7B, 0, 0, 0, 0, 0, 0, 0]);
        set(0x60, &[0x7B, 0, 0, 0, 0, 0, 0, 0]);
        set(0xE8, &[1, 0, 0, 0]);
        set(0xF0, &xuid.to_le_bytes());
        set(0xF8, &[0x7B, 0, 0, 0, 0, 0, 0, 0]);
        set(0x2F0, &[1, 0, 0, 0]);
        assert!(b == &want[..], "recipe bytes differ from the golden table");
        // Nothing past the header block.
        assert!(b[0x300..].iter().all(|&x| x == 0));
        assert_eq!(o.get_i32(off::PLAYER_COUNT), 1);
        assert_eq!(o.get_i32(off::PEER_COUNT), 1);
        assert_eq!(o.get_u64(off::PLAYERS), xuid);
        // The padding and the tail int stay 0, never -1.
        assert_eq!(o.get_i32(0xEC), 0);
        assert_eq!(o.get_i32(off::PLAYER_OPTIONS_TAIL), 0);
    }

    #[test]
    fn match_fields_win_over_a_variant_copy() {
        // A variant copy that scribbled on the header must not leave its
        // map or host values behind.
        let l = Launch::offline(44, 7);
        let mut o = GameOptions::new();
        o.apply_base(&l);
        o.bytes_mut()[0x10..0x300].fill(0xEE);
        o.apply_match(&l);
        assert_eq!(o.get_i32(off::LEGACY_MAP_ID), 44);
        assert_eq!(o.get_u16(off::MAP_ID + 6), BUILTIN_MAP_FLAGS);
        assert_eq!(o.get_u64(off::HOST_ADDRESS), 123);
        assert_eq!(o.get_u64(off::PLAYERS + player::ADDRESS), 123);
    }

    #[test]
    fn buffer_is_eight_byte_aligned() {
        let mut o = GameOptions::new();
        assert_eq!(o.as_mut_ptr() as usize % 8, 0);
    }

    #[test]
    fn raw_writes() {
        let w = RawWrite::parse("0x44=i16:-1").unwrap();
        assert_eq!(w.offset, 0x44);
        assert_eq!(w.bytes, vec![0xFF, 0xFF]);
        let w = RawWrite::parse("3=u8:0").unwrap();
        assert_eq!((w.offset, w.bytes.clone()), (3, vec![0]));
        let w = RawWrite::parse("0x1C0=f32:1.0").unwrap();
        assert_eq!(w.bytes, 1.0f32.to_le_bytes().to_vec());
        let w = RawWrite::parse("0x10=u32:0x2C").unwrap();
        assert_eq!(w.bytes, vec![0x2C, 0, 0, 0]);
        let w = RawWrite::parse("0x20=hex:01 02 ff").unwrap();
        assert_eq!(w.bytes, vec![1, 2, 0xFF]);
        assert!(RawWrite::parse("0x10=u8:256").is_err());
        assert!(RawWrite::parse("0x10=q8:1").is_err());
        assert!(RawWrite::parse("0x10").is_err());
        let mut buf = vec![0u8; 4];
        assert!(RawWrite::parse("3=u16:1").unwrap().apply(&mut buf).is_err());
        RawWrite::parse("2=u16:0x0102")
            .unwrap()
            .apply(&mut buf)
            .unwrap();
        assert_eq!(buf, vec![0, 0, 2, 1]);
    }

    #[test]
    fn the_engine_gets_the_buffer_not_the_struct() {
        let mut o = Box::new(GameOptions::new());
        o.apply_base(&Launch::offline(44, 7));
        let data = o.bytes().as_ptr();
        let header = o.as_ref() as *const GameOptions as *const u8;
        let p = o.leak_for_engine();
        assert_eq!(p as *const u8, data);
        assert_ne!(p as *const u8, header);
        // SAFETY: p is the leaked buffer of SIZE bytes.
        let b = unsafe { std::slice::from_raw_parts(p, SIZE) };
        assert_eq!(b[off::GAME_TICK], 60);
        assert_eq!(b[off::GAME_MODE], mode::MULTIPLAYER as u8);
        assert_eq!(p as usize % 8, 0);
    }
}
