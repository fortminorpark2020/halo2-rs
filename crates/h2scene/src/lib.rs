//! Halo 2's main menu scene: mainmenu.map's level, sky, fog and scenery
//! (the carrier over New Mombasa), their materials and textures, and the
//! camera path its `mainmenu_flythrough` script flies, read into plain
//! structs on any thread with no GPU (`load`), then drawn offscreen with
//! wgpu (`render`) on a thread of its own (`backdrop`). The launcher's
//! lobby copies the frames in behind the start screen and main menu
//! (docs/notes/launcher/menu.md, Phase 2.4 and 2.5).
//!
//! Only Halo 2 Vista's maps (cache format 8, as Project Cartographer
//! installs them) are read; MCC's format 13 has no geometry reader yet.
//! Nothing from the maps is kept in this crate or its tests.

pub mod backdrop;
pub mod camera;
pub mod data;
pub mod flythrough;
pub mod light;
pub mod load;
pub mod render;

pub use backdrop::{Backdrop, Frame, Mailbox, Options, SceneSource, SceneState};
pub use blam_cache::model::{Atmosphere, Fog};
pub use blam_cache::shader::Blend;
pub use camera::{look_to, menu_view_proj, Lens};
pub use data::{
    mip_chain, AuxKind, Batch, Material, MeshData, Object, Pass, SceneData, TextureData, Vertex,
};
pub use flythrough::{Flythrough, FOV};
pub use light::LevelLight;
pub use load::{load, placement_matrix};
pub use render::{Eye, Parts, Renderer, Veil};
