//! Halo 2's menus: the screens (`wgit`) of mainmenu.map, their list skins
//! (`skin`) and the globals they share (`wigl`), found through the main
//! menu's screen collection (`wgtz`). Field layouts follow Assembly's Halo 2
//! plugins.
//!
//! Menu coordinates put the origin at the middle of the screen, +x right
//! and +y up, with the screen about 1200 units tall (a 4:3 screen's safe
//! area is about ±800 by ±600). A bitmap is a unit to its pixel, times its
//! scale. Screens place bitmaps by their top-left corner; list skins by
//! their bottom-left, from each list item's corner.

use crate::font::Font;
use crate::mapset::{MapSet, Source};
use crate::text;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, GroupTag, Result};

/// wgtz: the shared globals (a tag reference) and the screens.
const WGTZ_GLOBALS: usize = 0x0;
const WGTZ_SCREENS: usize = 0x8;
const TAG_REF_SIZE: usize = 0x8;

/// wigl: the overlay behind dialogs, the menu sounds, screen animations,
/// list skins, button legends, header fonts and bounds, and the music.
const WIGL_OVERLAY_ALPHA_MOD: usize = 0x44;
const WIGL_OVERLAY_COLOR: usize = 0x6C;
const WIGL_SOUNDS: usize = 0x90;
const WIGL_ANIMATIONS: usize = 0x120;
const ANIMATION_SIZE: usize = 0x2C;
const WIGL_SKINS: usize = 0x138;
const WIGL_BUTTON_KEYS: usize = 0x140;
const WIGL_HEADER_FONTS: usize = 0x160;
const WIGL_TEXT_COLOR: usize = 0x168;
const WIGL_BOUNDS: usize = 0x178;
const WIGL_MUSIC: usize = 0x1B8;
const WIGL_MUSIC_FADE: usize = 0x1C0;
const WIGL_SIZE: usize = 0x1C4;
const KEYFRAME_SIZE: usize = 0x14;

/// wgit: a screen.
const WGIT_STRINGS: usize = 0x18;
const WGIT_PANES: usize = 0x20;
const WGIT_HEADER: usize = 0x2C;
const WGIT_SIZE: usize = 0x70;
const PANE_SIZE: usize = 0x4C;
const BUTTON_SIZE: usize = 0x3C;
const LIST_SIZE: usize = 0x18;
const TEXT_SIZE: usize = 0x2C;
const BITMAP_SIZE: usize = 0x38;
const MODEL_SCENE_SIZE: usize = 0x4C;
const PLAYERS_SIZE: usize = 0x18;
const NAME_SIZE: usize = 0x20;

/// skin: a list's look.
const SKIN_ITEM_ANIMATIONS: usize = 0x14;
const ITEM_ANIMATION_SIZE: usize = 0x10;
const SKIN_SIZE: usize = 0x3C;

/// Text flags: where text sits in its bounds (centred unless one is set),
/// and whether it pulses.
pub const LEFT_JUSTIFY: u32 = 1;
pub const RIGHT_JUSTIFY: u32 = 2;
pub const PULSATING: u32 = 4;
/// Bitmap flags: a list skin's bitmap doesn't count towards an item's
/// height.
pub const IGNORE_FOR_LIST_SIZE: u32 = 1;
/// Screen flags: a quarter, half or large dialog (otherwise full screen),
/// and no header.
pub const QUARTER_DIALOG: u32 = 1;
pub const NO_HEADER: u32 = 4;
pub const HALF_DIALOG: u32 = 8;
pub const LARGE_DIALOG: u32 = 0x10;
/// List flags: the list wraps round.
pub const LIST_WRAPS: u32 = 1;

/// A box in menu units: top and bottom (+y up), left and right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

/// A tag a menu element names.
#[derive(Clone, Debug, PartialEq)]
pub struct TagRef {
    pub datum: DatumIndex,
    pub name: String,
}

/// One step of an animation: how opaque, and how far from its place, an
/// element is. A period's keyframes are evenly spaced through it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub alpha: f32,
    pub position: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    pub period_ms: i32,
    pub keyframes: Vec<Keyframe>,
}

/// How an element comes onto a screen and leaves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScreenAnimation {
    pub intro: Animation,
    pub outro: Animation,
    pub ambient: Animation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub flags: u32,
    /// In `Globals::animations`.
    pub animation: Option<usize>,
    pub delay_ms: i16,
    pub font: Font,
    /// RGB: the tags' alpha is 0 for most text that shows, so it's left
    /// out.
    pub color: [f32; 3],
    pub bounds: Rect,
    /// Its string id's name, and its words from the screen's strings.
    pub string: String,
    pub text: Option<String>,
    pub depth: i16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub flags: u32,
    pub animation: Option<usize>,
    pub delay_ms: i16,
    /// Multiplied over what's behind, rather than blended.
    pub multiply: bool,
    /// Which of the bitmap tag's images.
    pub frame: i16,
    /// Its top-left corner on a screen; its bottom-left in a list skin.
    pub corner: [i16; 2],
    /// The image scrolls this many times its width (and height) a second.
    pub wraps_per_second: [f32; 2],
    pub bitmap: Option<TagRef>,
    pub depth: i16,
    /// 0 is 1.
    pub scale: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct List {
    pub flags: u32,
    /// In `Globals::skins`.
    pub skin: i16,
    pub visible: i16,
    /// The first item's corner; the rest stack below it.
    pub corner: [i16; 2],
    pub animation: Option<usize>,
    pub delay_ms: i16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub text: Text,
    pub bitmap: Option<TagRef>,
    /// Where the bitmap's top-left sits from the text's top-left.
    pub bitmap_offset: [i16; 2],
}

/// Scenario objects of mainmenu.map drawn into a box of the screen (a
/// player's model), seen from `camera`.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelScene {
    pub animation: Option<usize>,
    pub delay_ms: i16,
    pub depth: i16,
    pub objects: Vec<String>,
    pub lights: Vec<String>,
    pub camera: [f32; 3],
    pub fov: f32,
    pub viewport: Rect,
}

/// A list of players drawn with a player skin.
#[derive(Clone, Debug, PartialEq)]
pub struct Players {
    pub skin: Option<TagRef>,
    pub corner: [i16; 2],
    pub max: u8,
    pub rows: u8,
    pub columns: u8,
    pub row_height: i16,
    pub column_width: i16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pane {
    pub animation: Option<usize>,
    pub buttons: Vec<Button>,
    pub lists: Vec<List>,
    pub texts: Vec<Text>,
    pub bitmaps: Vec<Bitmap>,
    pub model_scenes: Vec<ModelScene>,
    pub players: Vec<Players>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub flags: u32,
    pub screen_id: u16,
    /// Which button legend shows (Halo 2's `button_key` names, in
    /// `Globals::button_keys` order); 0 is none.
    pub button_key: u16,
    /// The header's string id's name, and its words.
    pub header: String,
    pub header_text: Option<String>,
    pub panes: Vec<Pane>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListSkin {
    pub name: String,
    pub arrows: Option<TagRef>,
    /// Where the scroll arrows go, from the list's corner.
    pub arrow_up: [i16; 2],
    pub arrow_down: [i16; 2],
    /// An item gaining focus, losing it, sitting unfocused, and hovered.
    pub item_animations: Vec<Animation>,
    pub texts: Vec<Text>,
    pub bitmaps: Vec<Bitmap>,
}

/// The menu sounds, by tag name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sounds {
    pub cursor: Option<String>,
    pub select: Option<String>,
    pub error: Option<String>,
    pub advance: Option<String>,
    pub retreat: Option<String>,
}

/// A dialog's size: its header and button legend go in its size's bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogSize {
    Full,
    Large,
    Half,
    Quarter,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Globals {
    /// RGBA of the shade over the screens under a dialog, and how much
    /// those screens fade.
    pub overlay_color: [f32; 4],
    pub overlay_alpha_mod: f32,
    pub sounds: Sounds,
    pub animations: Vec<ScreenAnimation>,
    pub skins: Vec<ListSkin>,
    /// Button legends by string id name ("a_select_b_back"), with the
    /// controller's button glyphs (U+E100 A, U+E101 B, U+E102 X, U+E103
    /// Y).
    pub button_keys: Vec<(String, String)>,
    /// By `DialogSize`.
    pub header_fonts: [Font; 4],
    pub header_bounds: [Rect; 4],
    pub button_key_bounds: [Rect; 4],
    /// Text with no colour of its own.
    pub text_color: [f32; 3],
    pub music: Option<String>,
    pub music_fade_ms: i32,
}

/// mainmenu.map's menus.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ui {
    pub globals: Globals,
    pub screens: Vec<Screen>,
}

impl Ui {
    /// A screen by its tag's name, such as
    /// `ui\screens\game_shell\main_menu_screen\main_menu`.
    pub fn screen(&self, name: &str) -> Option<&Screen> {
        self.screens.iter().find(|s| s.name == name)
    }
}

impl Screen {
    pub fn dialog_size(&self) -> DialogSize {
        if self.flags & QUARTER_DIALOG != 0 {
            DialogSize::Quarter
        } else if self.flags & HALF_DIALOG != 0 {
            DialogSize::Half
        } else if self.flags & LARGE_DIALOG != 0 {
            DialogSize::Large
        } else {
            DialogSize::Full
        }
    }
}

impl Globals {
    /// A button legend by its string id's name.
    pub fn button_key(&self, name: &str) -> Option<&str> {
        self.button_keys
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.as_str())
    }
}

impl ListSkin {
    /// How far apart its items are: the height of what its texts and the
    /// bitmaps that count cover, given each bitmap's height (its pixels
    /// times its scale).
    pub fn item_height(&self, bitmap_height: impl Fn(&Bitmap) -> f32) -> f32 {
        let texts = self
            .texts
            .iter()
            .map(|t| (t.bounds.bottom as f32, t.bounds.top as f32));
        let bitmaps = self
            .bitmaps
            .iter()
            .filter(|b| b.flags & IGNORE_FOR_LIST_SIZE == 0)
            .map(|b| (b.corner[1] as f32, b.corner[1] as f32 + bitmap_height(b)));
        let (bottom, top) = texts
            .chain(bitmaps)
            .fold((f32::MAX, f32::MIN), |(lo, hi), (b, t)| {
                (lo.min(b), hi.max(t))
            });
        (top - bottom).max(1.0)
    }
}

/// Where a tag's blocks, string ids' names and tags' names come from.
pub trait Reader {
    /// The elements of the block whose header (count, address) is at
    /// `at` in `parent`, each `size` bytes.
    fn block(&mut self, parent: &[u8], at: usize, size: usize) -> Vec<Vec<u8>>;
    fn string_id(&self, id: u32) -> String;
    fn tag(&self, datum: u32) -> Option<TagRef>;
}

/// Reads the blocks of a tag stored in one file of a map set.
struct MapReader<'a> {
    set: &'a mut MapSet,
    source: Source,
}

impl Reader for MapReader<'_> {
    fn block(&mut self, parent: &[u8], at: usize, size: usize) -> Vec<Vec<u8>> {
        if parent.len() < at + 8 {
            return Vec::new();
        }
        let file = self.set.get(self.source);
        let region = file.meta_region();
        let raw = file
            .read_block(region, parent, at, size)
            .unwrap_or_default();
        raw.chunks_exact(size).map(<[u8]>::to_vec).collect()
    }

    fn string_id(&self, id: u32) -> String {
        self.set.map.string_id(id).unwrap_or_default().to_string()
    }

    fn tag(&self, datum: u32) -> Option<TagRef> {
        let t = self.set.map.tag(DatumIndex(datum))?;
        Some(TagRef {
            datum: t.datum,
            name: t.name.clone(),
        })
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn point(b: &[u8], o: usize) -> [i16; 2] {
    [i16_at(b, o), i16_at(b, o + 2)]
}

fn rect(b: &[u8], o: usize) -> Rect {
    Rect {
        top: i16_at(b, o),
        left: i16_at(b, o + 2),
        bottom: i16_at(b, o + 4),
        right: i16_at(b, o + 6),
    }
}

/// An animation index: 0 is none, then the globals' animations from 1.
fn animation(b: &[u8], o: usize) -> Option<usize> {
    (u16_at(b, o) as usize).checked_sub(1)
}

/// A tag reference's datum (after its group) as the tag it names.
fn tag_ref(r: &dyn Reader, b: &[u8], o: usize) -> Option<TagRef> {
    match u32_at(b, o + 4) {
        u32::MAX => None,
        datum => r.tag(datum),
    }
}

fn ascii(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// A string id's name and its words in `strings` (id, words).
fn string(r: &dyn Reader, strings: &[(u32, String)], id: u32) -> (String, Option<String>) {
    if id == 0 {
        return (String::new(), None);
    }
    let words = strings.iter().find(|s| s.0 == id).map(|s| s.1.clone());
    (r.string_id(id), words)
}

fn keyframes(r: &mut dyn Reader, b: &[u8], at: usize) -> Vec<Keyframe> {
    r.block(b, at, KEYFRAME_SIZE)
        .iter()
        .map(|k| Keyframe {
            alpha: f32_at(k, 4),
            position: [f32_at(k, 8), f32_at(k, 12), f32_at(k, 16)],
        })
        .collect()
}

fn text_block(r: &dyn Reader, t: &[u8], strings: &[(u32, String)]) -> Text {
    text_at(r, t, strings, 0x24, 0x28)
}

/// A text block's fields, with its string id and depth at `string_at` and
/// `depth_at` (a button's come after its bitmap).
fn text_at(
    r: &dyn Reader,
    t: &[u8],
    strings: &[(u32, String)],
    string_at: usize,
    depth_at: usize,
) -> Text {
    let (string, text) = string(r, strings, u32_at(t, string_at));
    Text {
        flags: u32_at(t, 0),
        animation: animation(t, 4),
        delay_ms: i16_at(t, 6),
        font: Font::from_index(u16_at(t, 0xA)).unwrap_or(Font::Body),
        color: [f32_at(t, 0x10), f32_at(t, 0x14), f32_at(t, 0x18)],
        bounds: rect(t, 0x1C),
        string,
        text,
        depth: i16_at(t, depth_at),
    }
}

fn bitmap_block(r: &dyn Reader, b: &[u8]) -> Bitmap {
    Bitmap {
        flags: u32_at(b, 0),
        animation: animation(b, 4),
        delay_ms: i16_at(b, 6),
        multiply: u16_at(b, 8) == 1,
        frame: i16_at(b, 0xA),
        corner: point(b, 0xC),
        wraps_per_second: [f32_at(b, 0x10), f32_at(b, 0x14)],
        bitmap: tag_ref(r, b, 0x18),
        depth: i16_at(b, 0x20),
        scale: [f32_at(b, 0x30), f32_at(b, 0x34)],
    }
}

fn texts(r: &mut dyn Reader, b: &[u8], at: usize, strings: &[(u32, String)]) -> Vec<Text> {
    let raw = r.block(b, at, TEXT_SIZE);
    raw.iter().map(|t| text_block(r, t, strings)).collect()
}

fn bitmaps(r: &mut dyn Reader, b: &[u8], at: usize) -> Vec<Bitmap> {
    let raw = r.block(b, at, BITMAP_SIZE);
    raw.iter().map(|e| bitmap_block(r, e)).collect()
}

fn pane(r: &mut dyn Reader, p: &[u8], strings: &[(u32, String)]) -> Pane {
    let buttons = r.block(p, 0x4, BUTTON_SIZE);
    let buttons = buttons
        .iter()
        .map(|b| Button {
            text: text_at(r, b, strings, 0x30, 0x34),
            bitmap: tag_ref(r, b, 0x24),
            bitmap_offset: point(b, 0x2C),
        })
        .collect();
    let lists = r
        .block(p, 0xC, LIST_SIZE)
        .iter()
        .map(|l| List {
            flags: u32_at(l, 0),
            skin: i16_at(l, 4),
            visible: i16_at(l, 6),
            corner: point(l, 8),
            animation: animation(l, 0xC),
            delay_ms: i16_at(l, 0xE),
        })
        .collect();
    let scenes = r.block(p, 0x2C, MODEL_SCENE_SIZE);
    let model_scenes = scenes
        .iter()
        .map(|m| {
            let names = |r: &mut dyn Reader, at| -> Vec<String> {
                r.block(m, at, NAME_SIZE).iter().map(|n| ascii(n)).collect()
            };
            ModelScene {
                animation: animation(m, 4),
                delay_ms: i16_at(m, 6),
                depth: i16_at(m, 8),
                objects: names(r, 0xC),
                lights: names(r, 0x14),
                camera: [f32_at(m, 0x28), f32_at(m, 0x2C), f32_at(m, 0x30)],
                fov: f32_at(m, 0x34),
                viewport: rect(m, 0x38),
            }
        })
        .collect();
    let raw = r.block(p, 0x44, PLAYERS_SIZE);
    let players = raw.iter().map(|e| players(r, e)).collect();
    Pane {
        animation: animation(p, 2),
        buttons,
        lists,
        texts: texts(r, p, 0x1C, strings),
        bitmaps: bitmaps(r, p, 0x24),
        model_scenes,
        players,
    }
}

fn players(r: &dyn Reader, e: &[u8]) -> Players {
    Players {
        skin: tag_ref(r, e, 0x4),
        corner: point(e, 0xC),
        max: e[0x11],
        rows: e[0x12],
        columns: e[0x13],
        row_height: i16_at(e, 0x14),
        column_width: i16_at(e, 0x16),
    }
}

/// A screen from its tag's data and its string list's strings (id, words).
pub fn parse_screen(
    r: &mut dyn Reader,
    name: &str,
    d: &[u8],
    strings: &[(u32, String)],
) -> Result<Screen> {
    if d.len() < WGIT_SIZE {
        return Err(Error::Corrupt(format!("screen {name} too small")));
    }
    let (header, header_text) = string(r, strings, u32_at(d, WGIT_HEADER));
    let raw = r.block(d, WGIT_PANES, PANE_SIZE);
    Ok(Screen {
        name: name.to_string(),
        flags: u32_at(d, 0),
        screen_id: u16_at(d, 4),
        button_key: u16_at(d, 6),
        header,
        header_text,
        panes: raw.iter().map(|p| pane(r, p, strings)).collect(),
    })
}

/// A list skin from its tag's data.
pub fn parse_skin(r: &mut dyn Reader, name: &str, d: &[u8]) -> Result<ListSkin> {
    if d.len() < SKIN_SIZE {
        return Err(Error::Corrupt(format!("list skin {name} too small")));
    }
    let item_animations = r
        .block(d, SKIN_ITEM_ANIMATIONS, ITEM_ANIMATION_SIZE)
        .iter()
        .map(|a| Animation {
            period_ms: u32_at(a, 4) as i32,
            keyframes: keyframes(r, a, 8),
        })
        .collect();
    Ok(ListSkin {
        name: name.to_string(),
        arrows: tag_ref(r, d, 0x4),
        arrow_up: point(d, 0xC),
        arrow_down: point(d, 0x10),
        item_animations,
        texts: texts(r, d, 0x1C, &[]),
        bitmaps: bitmaps(r, d, 0x24),
    })
}

/// The globals from their tag's data, the list skins they name (parsed
/// already) and the button legends' strings (id, words).
pub fn parse_globals(
    r: &mut dyn Reader,
    d: &[u8],
    skins: Vec<ListSkin>,
    button_keys: &[(u32, String)],
) -> Result<Globals> {
    if d.len() < WIGL_SIZE {
        return Err(Error::Corrupt("menu globals too small".into()));
    }
    let sound = |at: usize| tag_ref(r, d, WIGL_SOUNDS + at * TAG_REF_SIZE).map(|t| t.name);
    let sounds = Sounds {
        cursor: sound(0),
        select: sound(1),
        error: sound(2),
        advance: sound(3),
        retreat: sound(4),
    };
    let raw = r.block(d, WIGL_ANIMATIONS, ANIMATION_SIZE);
    let animations = raw
        .iter()
        .map(|a| {
            let mut part = |period: usize, frames: usize| Animation {
                period_ms: u32_at(a, period) as i32,
                keyframes: keyframes(r, a, frames),
            };
            ScreenAnimation {
                intro: part(0x4, 0x8),
                outro: part(0x10, 0x14),
                ambient: part(0x1C, 0x24),
            }
        })
        .collect();
    let argb = |o: usize| {
        [
            f32_at(d, o + 4),
            f32_at(d, o + 8),
            f32_at(d, o + 12),
            f32_at(d, o),
        ]
    };
    let font =
        |k: usize| Font::from_index(u16_at(d, WIGL_HEADER_FONTS + 2 * k)).unwrap_or(Font::Title);
    let bounds = |k: usize| rect(d, WIGL_BOUNDS + 8 * k);
    let mut keys: Vec<(String, String)> = button_keys
        .iter()
        .map(|(id, words)| (r.string_id(*id), words.clone()))
        .collect();
    keys.sort();
    Ok(Globals {
        overlay_color: argb(WIGL_OVERLAY_COLOR),
        overlay_alpha_mod: f32_at(d, WIGL_OVERLAY_ALPHA_MOD),
        sounds,
        animations,
        skins,
        button_keys: keys,
        header_fonts: [font(0), font(1), font(2), font(3)],
        header_bounds: [bounds(0), bounds(2), bounds(4), bounds(6)],
        button_key_bounds: [bounds(1), bounds(3), bounds(5), bounds(7)],
        text_color: [
            f32_at(d, WIGL_TEXT_COLOR + 4),
            f32_at(d, WIGL_TEXT_COLOR + 8),
            f32_at(d, WIGL_TEXT_COLOR + 12),
        ],
        music: tag_ref(r, d, WIGL_MUSIC).map(|t| t.name),
        music_fade_ms: u32_at(d, WIGL_MUSIC_FADE) as i32,
    })
}

/// A tag's data, and a reader for its blocks.
fn tag(set: &mut MapSet, datum: DatumIndex) -> Result<(Source, Vec<u8>)> {
    let (source, _, data) = set.tag_data(datum)?;
    Ok((source, data))
}

/// A string list's strings, or none.
fn strings(set: &mut MapSet, table: &[(u32, String)], datum: u32) -> Vec<(u32, String)> {
    match datum {
        u32::MAX => Vec::new(),
        d => text::unicode_strings(set, table, DatumIndex(d)).unwrap_or_default(),
    }
}

/// Read the menus of mainmenu.map (`set`): its screen collection's globals,
/// list skins and screens. Screens that don't read are left out.
pub fn read(set: &mut MapSet) -> Result<Ui> {
    let wgtz = GroupTag::parse("wgtz").expect("a group tag");
    let collection = set
        .map
        .tags
        .iter()
        .find(|t| t.group == wgtz)
        .map(|t| t.datum)
        .ok_or_else(|| Error::Corrupt("no menus in this map".into()))?;
    let table = text::language_table(set).unwrap_or_default();
    let (source, z) = tag(set, collection)?;
    if z.len() < WGTZ_SCREENS + 8 {
        return Err(Error::Corrupt("screen collection too small".into()));
    }
    let screens: Vec<u32> = MapReader { set, source }
        .block(&z, WGTZ_SCREENS, TAG_REF_SIZE)
        .iter()
        .map(|s| u32_at(s, 4))
        .collect();
    let globals = DatumIndex(u32_at(&z, WGTZ_GLOBALS + 4));
    let (source, g) = tag(set, globals)?;
    let skin_refs: Vec<u32> = MapReader { set, source }
        .block(&g, WIGL_SKINS, TAG_REF_SIZE)
        .iter()
        .map(|s| u32_at(s, 4))
        .collect();
    let mut skins = Vec::new();
    for datum in skin_refs {
        let name = MapReader { set, source }
            .tag(datum)
            .map(|t| t.name)
            .unwrap_or_default();
        let skin = tag(set, DatumIndex(datum))
            .and_then(|(source, d)| parse_skin(&mut MapReader { set, source }, &name, &d));
        // An empty skin keeps the others at their indices.
        skins.push(skin.unwrap_or_else(|_| ListSkin {
            name,
            arrows: None,
            arrow_up: [0; 2],
            arrow_down: [0; 2],
            item_animations: Vec::new(),
            texts: Vec::new(),
            bitmaps: Vec::new(),
        }));
    }
    let button_keys = if g.len() >= WIGL_SIZE {
        strings(set, &table, u32_at(&g, WIGL_BUTTON_KEYS + 4))
    } else {
        Vec::new()
    };
    let globals = parse_globals(&mut MapReader { set, source }, &g, skins, &button_keys)?;
    let mut ui = Ui {
        globals,
        screens: Vec::new(),
    };
    for datum in screens {
        let Some(name) = set.map.tag(DatumIndex(datum)).map(|t| t.name.clone()) else {
            continue;
        };
        let Ok((source, d)) = tag(set, DatumIndex(datum)) else {
            continue;
        };
        let list = if d.len() >= WGIT_SIZE {
            strings(set, &table, u32_at(&d, WGIT_STRINGS + 4))
        } else {
            Vec::new()
        };
        if let Ok(s) = parse_screen(&mut MapReader { set, source }, &name, &d, &list) {
            ui.screens.push(s);
        }
    }
    Ok(ui)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Blocks at made-up addresses, and names for string ids and tags.
    #[derive(Default)]
    struct Fake {
        blocks: HashMap<u32, Vec<u8>>,
        strings: HashMap<u32, String>,
        tags: HashMap<u32, String>,
        next: u32,
    }

    impl Fake {
        /// Put `elements` in a block whose header goes at `at` in `parent`.
        fn put(&mut self, parent: &mut [u8], at: usize, elements: &[Vec<u8>]) {
            self.next += 0x1000;
            parent[at..at + 4].copy_from_slice(&(elements.len() as u32).to_le_bytes());
            parent[at + 4..at + 8].copy_from_slice(&self.next.to_le_bytes());
            self.blocks.insert(self.next, elements.concat());
        }
    }

    impl Reader for Fake {
        fn block(&mut self, parent: &[u8], at: usize, size: usize) -> Vec<Vec<u8>> {
            let count = u32_at(parent, at) as usize;
            let address = u32_at(parent, at + 4);
            let data = self.blocks.get(&address).cloned().unwrap_or_default();
            data.chunks_exact(size)
                .take(count)
                .map(<[u8]>::to_vec)
                .collect()
        }

        fn string_id(&self, id: u32) -> String {
            self.strings.get(&id).cloned().unwrap_or_default()
        }

        fn tag(&self, datum: u32) -> Option<TagRef> {
            Some(TagRef {
                datum: DatumIndex(datum),
                name: self.tags.get(&datum)?.clone(),
            })
        }
    }

    fn put16(b: &mut [u8], o: usize, v: i16) {
        b[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn put32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn putf(b: &mut [u8], o: usize, v: f32) {
        put32(b, o, v.to_bits());
    }

    /// The main menu's chrome logo: top-left (-511, 90), fading in with
    /// animation 29.
    fn logo() -> Vec<u8> {
        let mut b = vec![0u8; BITMAP_SIZE];
        put16(&mut b, 4, 30);
        put16(&mut b, 0xC, -511);
        put16(&mut b, 0xE, 90);
        put32(&mut b, 0x1C, 0xE001_0002);
        put16(&mut b, 0x20, 1);
        b
    }

    #[test]
    fn a_screen_reads_its_panes_lists_texts_and_bitmaps() {
        let mut r = Fake::default();
        r.tags.insert(0xE001_0002, "ui\\start_screen".into());
        r.strings.insert(0x77, "press_start".into());
        let mut text = vec![0u8; TEXT_SIZE];
        put32(&mut text, 0, PULSATING);
        put16(&mut text, 0xA, 2);
        putf(&mut text, 0x10, 0.64);
        putf(&mut text, 0x14, 0.72);
        putf(&mut text, 0x18, 0.87);
        put16(&mut text, 0x1C, -65);
        put16(&mut text, 0x1E, -400);
        put16(&mut text, 0x20, -105);
        put16(&mut text, 0x22, 400);
        put32(&mut text, 0x24, 0x77);
        let mut list = vec![0u8; LIST_SIZE];
        put32(&mut list, 0, 3);
        put16(&mut list, 4, 11);
        put16(&mut list, 6, 6);
        put16(&mut list, 8, -178);
        put16(&mut list, 0xA, -80);
        put16(&mut list, 0xC, 35);
        let mut pane_data = vec![0u8; PANE_SIZE];
        put16(&mut pane_data, 2, 30);
        r.put(&mut pane_data, 0xC, &[list]);
        r.put(&mut pane_data, 0x1C, &[text]);
        r.put(&mut pane_data, 0x24, &[logo()]);
        let mut d = vec![0u8; WGIT_SIZE];
        put32(&mut d, 0, NO_HEADER);
        put16(&mut d, 4, 6);
        r.put(&mut d, WGIT_PANES, &[pane_data]);
        let strings = [(0x77, "PRESS ANY KEY TO CONTINUE".to_string())];
        let s = parse_screen(&mut r, "ui\\main_menu", &d, &strings).unwrap();
        assert_eq!(s.screen_id, 6);
        assert_eq!(s.dialog_size(), DialogSize::Full);
        assert_eq!(s.panes.len(), 1);
        let p = &s.panes[0];
        assert_eq!(p.animation, Some(29));
        assert_eq!(
            p.lists,
            [List {
                flags: 3,
                skin: 11,
                visible: 6,
                corner: [-178, -80],
                animation: Some(34),
                delay_ms: 0,
            }]
        );
        let t = &p.texts[0];
        assert_eq!(t.font, Font::Title);
        assert_eq!(t.flags & PULSATING, PULSATING);
        assert_eq!(
            t.bounds,
            Rect {
                top: -65,
                left: -400,
                bottom: -105,
                right: 400
            }
        );
        assert_eq!(t.string, "press_start");
        assert_eq!(t.text.as_deref(), Some("PRESS ANY KEY TO CONTINUE"));
        assert_eq!(t.color, [0.64, 0.72, 0.87]);
        let b = &p.bitmaps[0];
        assert_eq!(b.corner, [-511, 90]);
        assert_eq!(b.animation, Some(29));
        assert_eq!(b.bitmap.as_ref().unwrap().name, "ui\\start_screen");
        assert!(!b.multiply);
    }

    #[test]
    fn a_list_skin_reads_its_items_look_and_height() {
        let mut r = Fake::default();
        r.tags.insert(0xE002_0003, "ui\\list_bkd".into());
        // Focus: 0.5 to 1 alpha in 120 ms.
        let mut focus = vec![0u8; ITEM_ANIMATION_SIZE];
        put32(&mut focus, 4, 120);
        let frame = |alpha: f32| {
            let mut k = vec![0u8; KEYFRAME_SIZE];
            putf(&mut k, 4, alpha);
            k
        };
        r.put(&mut focus, 8, &[frame(0.5), frame(1.0)]);
        let mut text = vec![0u8; TEXT_SIZE];
        put16(&mut text, 0xA, 10);
        put16(&mut text, 0x1C, 10);
        put16(&mut text, 0x1E, -62);
        put16(&mut text, 0x20, -40);
        put16(&mut text, 0x22, 418);
        let mut bar = vec![0u8; BITMAP_SIZE];
        put32(&mut bar, 0, IGNORE_FOR_LIST_SIZE);
        put16(&mut bar, 0xC, -60);
        put16(&mut bar, 0xE, -40);
        put32(&mut bar, 0x1C, 0xE002_0003);
        let mut sheen = bar.clone();
        put16(&mut sheen, 8, 1);
        putf(&mut sheen, 0x10, 0.2);
        let mut d = vec![0u8; SKIN_SIZE];
        put32(&mut d, 0x8, u32::MAX);
        r.put(&mut d, SKIN_ITEM_ANIMATIONS, &[focus]);
        r.put(&mut d, 0x1C, &[text]);
        r.put(&mut d, 0x24, &[bar, sheen]);
        let s = parse_skin(&mut r, "ui\\list_skins\\main_menu", &d).unwrap();
        assert_eq!(s.arrows, None);
        assert_eq!(s.item_animations[0].period_ms, 120);
        let alphas: Vec<f32> = s.item_animations[0]
            .keyframes
            .iter()
            .map(|k| k.alpha)
            .collect();
        assert_eq!(alphas, [0.5, 1.0]);
        assert_eq!(s.texts[0].font, Font::MainMenu);
        assert_eq!(s.bitmaps[0].corner, [-60, -40]);
        assert!(s.bitmaps[1].multiply);
        assert_eq!(s.bitmaps[1].wraps_per_second, [0.2, 0.0]);
        // The text's 50 units: its bitmaps don't count.
        assert_eq!(s.item_height(|_| 62.0), 50.0);
    }

    #[test]
    fn the_globals_read_sounds_bounds_legends_and_animations() {
        let mut r = Fake::default();
        r.tags.insert(0xE003_0001, "sound\\ui\\cursor1".into());
        r.tags.insert(0xE003_0002, "sound\\ui\\flag_fail".into());
        r.strings.insert(0x10, "a_select_b_back".into());
        let mut d = vec![0u8; WIGL_SIZE];
        for k in 0..32 {
            put32(&mut d, WIGL_SOUNDS + 8 * k + 4, u32::MAX);
        }
        put32(&mut d, WIGL_SOUNDS + 4, 0xE003_0001);
        put32(&mut d, WIGL_SOUNDS + 2 * 8 + 4, 0xE003_0002);
        put32(&mut d, WIGL_MUSIC + 4, u32::MAX);
        // The full screen header's bounds and the fade animation.
        put16(&mut d, WIGL_BOUNDS, 567);
        put16(&mut d, WIGL_BOUNDS + 2, -730);
        put16(&mut d, WIGL_BOUNDS + 4, 520);
        put16(&mut d, WIGL_BOUNDS + 6, 50);
        put16(&mut d, WIGL_HEADER_FONTS, 2);
        putf(&mut d, WIGL_OVERLAY_COLOR, 0.85);
        putf(&mut d, WIGL_OVERLAY_COLOR + 12, 0.17);
        let mut fade = vec![0u8; ANIMATION_SIZE];
        put32(&mut fade, 4, 250);
        let frame = |alpha: f32, x: f32| {
            let mut k = vec![0u8; KEYFRAME_SIZE];
            putf(&mut k, 4, alpha);
            putf(&mut k, 8, x);
            k
        };
        r.put(&mut fade, 8, &[frame(0.0, 1024.0), frame(1.0, 0.0)]);
        r.put(&mut d, WIGL_ANIMATIONS, &[fade]);
        let keys = [(0x10, "\u{e100} SELECT \u{e101} BACK".to_string())];
        let g = parse_globals(&mut r, &d, Vec::new(), &keys).unwrap();
        assert_eq!(g.sounds.cursor.as_deref(), Some("sound\\ui\\cursor1"));
        assert_eq!(g.sounds.error.as_deref(), Some("sound\\ui\\flag_fail"));
        assert_eq!(g.sounds.select, None);
        assert_eq!(g.music, None);
        assert_eq!(g.header_fonts[0], Font::Title);
        assert_eq!(g.header_bounds[0].left, -730);
        assert_eq!(g.overlay_color, [0.0, 0.0, 0.17, 0.85]);
        assert_eq!(g.animations[0].intro.period_ms, 250);
        assert_eq!(g.animations[0].intro.keyframes[0].position[0], 1024.0);
        assert_eq!(
            g.button_key("a_select_b_back"),
            Some("\u{e100} SELECT \u{e101} BACK")
        );
    }

    /// The real menus, from the mainmenu.map in `H2_MAPS`.
    #[test]
    #[ignore]
    fn mainmenu_maps_menus_read() {
        let dir = std::env::var("H2_MAPS").expect("H2_MAPS");
        let mut set = MapSet::open(std::path::Path::new(&dir).join("mainmenu.map")).unwrap();
        let ui = read(&mut set).unwrap();
        let main = ui
            .screen("ui\\screens\\game_shell\\main_menu_screen\\main_menu")
            .unwrap();
        let logo = main.panes[0]
            .bitmaps
            .iter()
            .find(|b| {
                b.bitmap.as_ref().map(|t| t.name.as_str())
                    == Some("ui\\screens\\game_shell\\start_screen\\start_screen")
            })
            .unwrap();
        assert_eq!(logo.corner, [-511, 90]);
        let list = &main.panes[0].lists[0];
        assert_eq!((list.skin, list.corner), (11, [-178, -80]));
        assert_eq!(
            ui.globals.skins[11].name,
            "ui\\list_skins\\main_menu\\main_menu"
        );
        assert_eq!(ui.globals.skins[11].item_height(|_| 62.0), 50.0);
        let shell = ui
            .screen("ui\\screens\\game_shell\\main_menu_screen\\game_shell_background")
            .unwrap();
        let framing = &shell.panes[0].bitmaps[0];
        assert_eq!(
            framing.bitmap.as_ref().unwrap().name,
            "ui\\global_bitmaps\\framing_center"
        );
        assert_eq!(framing.corner, [-1070, 654]);
        // 1.078, which covers a 16:9 screen.
        assert!((framing.scale[0] - 1.08).abs() < 0.005);
        assert_eq!(
            ui.globals.sounds.error.as_deref(),
            Some("sound\\ui\\flag_fail")
        );
        assert!(ui.globals.button_key("a_select_b_back").is_some());
    }
}
