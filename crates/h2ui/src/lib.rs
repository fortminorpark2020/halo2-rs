//! Halo 2's menus as draw lists, with no GPU and no game files
//! (`docs/notes/launcher/menu.md`, section 2 "Shape of the code").
//!
//! - `layout`: Halo 2's UI space (origin in the middle, +y up, 1,200 units
//!   tall) on a window of any size, and the layout numbers used when no
//!   UI tags are read: where the start screen's and main menu's pieces go,
//!   the header and legend boxes, menu-preview's fallback places.
//! - `anim`: screen and item animations, the focus fades and the pulse.
//! - `text`: lines of text in Halo 2's fonts (`blam_cache::font`), or in a
//!   small built-in font when they aren't there.
//! - `art`: where pictures come from (a trait), a source with none for
//!   the flat look, and pictures read from MCC's or Vista's
//!   `mainmenu.map` (`art::read`, through `blam_cache::ui::Pictures`).
//! - `paint`: the `Painter`, which turns all of that into a `DrawList` of
//!   quads (pictures, fills and glyphs) in window pixels.
//! - `cpu`: draws a `DrawList` into a pixel buffer.
//! - `screens`: the start screen and the main menu.
//! - `tags`: those two screens' layouts from mainmenu.map's UI tags, piece
//!   by piece, falling back to `layout`'s numbers.
//! - `shell`: opens a mainmenu.map (MCC's or Vista's) for both screens,
//!   tags and pictures, logging where each piece came from.
//!
//! Colours are in gamma space throughout, as the tags give them and as
//! the lobby's canvas blends.

pub mod anim;
pub mod art;
pub mod cpu;
pub mod layout;
pub mod paint;
pub mod screens;
pub mod shell;
pub mod tags;
pub mod text;

pub use blam_cache::font::Font;

/// A colour (red, green, blue, alpha; 0 to 1, gamma space, alpha not
/// premultiplied).
pub type Rgba = [f32; 4];

/// A tag colour (red, green, blue) with an alpha.
pub fn rgba([r, g, b]: [f32; 3], alpha: f32) -> Rgba {
    [r, g, b, alpha]
}
