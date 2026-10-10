//! h2launch: hosts MCC's classic Halo 2 engine (`halo2\halo2.dll`) the way
//! MCC's own shell does, through the DLL's three exports, the objects they
//! return, and a host object and event manager of our own. No code in the
//! game's modules is patched or hooked.
//!
//! The modules at the top level are plain Rust with unit tests that run on
//! any platform: the byte layouts the engine reads (game options, player
//! profile, input state), the Steam library parser, the PE reader for
//! `--check`, the map table, the input script and the log summaries.
//! `win` is the Windows program itself.
//!
//! Facts about the engine come from the research behind
//! `docs/notes/launcher/brief.md`; comments say where a value is an
//! estimate.

pub mod cli;
pub mod live;
pub mod maps;
pub mod mccroot;
pub mod net;
pub mod options;
pub mod paths;
pub mod pe;
pub mod profile;
pub mod script;
pub mod session;
pub mod slots;
pub mod util;
pub mod vdf;
pub mod watch;

#[cfg(windows)]
pub mod win;

/// The commit this binary was built from (see build.rs).
pub const BUILD: &str = env!("H2LAUNCH_GIT");

/// halo2.dll 1.3528.0.0, the build all the slot numbers and layouts were
/// checked against (MCC 2025.08.16.178512.1, Steam), as read from the
/// owner's copy.
pub mod expected {
    pub const HALO2_SHA256: &str =
        "DE65B4F4FDBF3F0A5EAB7431FE530DA17DD815599182DFD6AE9B7E21CF171946";
    pub const HALO2_VERSION: [u16; 4] = [1, 3528, 0, 0];
    pub const HALO2_TIMESTAMP: u32 = 0x68A0_F0F2;
    pub const HALO2_SIZE_OF_IMAGE: u32 = 0x2A3_8000;
    pub const HALO2_EXPORTS: [&str; 5] = [
        "CreateDataAccess",
        "CreateGameEngine",
        "SetLibrarySettings",
        "apGpfSetExceptionInfo",
        "apShowErrorDllCall",
    ];
    /// Export RVAs.
    pub const CREATE_GAME_ENGINE_RVA: u32 = 0x54730;
    pub const CREATE_DATA_ACCESS_RVA: u32 = 0x3AF40;
    pub const SET_LIBRARY_SETTINGS_RVA: u32 = 0x54720;
    /// Vtables of the objects the two Create exports return.
    pub const ENGINE_VTABLE_RVA: u32 = 0xBE_3480;
    pub const DATA_ACCESS_VTABLE_RVA: u32 = 0xBE_2038;
    /// The value both Create exports return in this build.
    pub const CREATE_RETURN: u64 = 0x46;
    /// Data read (never written) under `--diag` to see how the engine
    /// polls the keyboard (launch-sequence.verify V24, host-interface.verify
    /// C15/E8): the poller's branch flag and its 256-byte key gate array.
    pub const DIAG_KEY_POLLER_FLAG_RVA: u32 = 0x1E8_CE68;
    pub const DIAG_KEY_GATE_RVA: u32 = 0x15E_A194;
}
