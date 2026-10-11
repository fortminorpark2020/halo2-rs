//! Halo 2's menus: the screens (`wgit`) of mainmenu.map, their list skins
//! (`skin`) and the globals they share (`wigl`), found through the main
//! menu's screen collection (`wgtz`), and the pictures they name. Field
//! layouts follow Assembly's Halo 2 plugins.
//!
//! The parsers read through `Reader`, so the same code reads Halo 2 Vista's
//! maps (`Vista`, `read`) and MCC's format-13 maps (`Mcc`, `read_mcc`).
//! MCC's UI tags are expected to keep Vista's layouts; until the probe
//! (`examples/mcc_ui_probe.rs`) has run on a real MCC map that is a
//! prediction. `Pictures` gives a bitmap tag's images, decoded, and its
//! sequences by name from either kind of map; `Menus::open` reads both
//! from a mainmenu.map of either kind.
//!
//! Menu coordinates put the origin at the middle of the screen, +x right
//! and +y up, with the screen about 1200 units tall (a 4:3 screen's safe
//! area is about ±800 by ±600). A bitmap is a unit to its pixel, times its
//! scale. Screens place bitmaps by their top-left corner; list skins by
//! their bottom-left, from each list item's corner.

use crate::bitmap::{self, Image, ImageInfo, Sequence};
use crate::font::Font;
use crate::mapset::{MapSet, Source};
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, GroupTag, Result};
use crate::{mcc, text};
use std::io::{Read, Seek};
use std::path::Path;

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

/// unic: where its English strings are in the language table (start,
/// count), as in Vista.
const UNIC_ENGLISH: usize = 0x10;

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

/// Where a tag's blocks, string ids' names and tags' names come from. The
/// parsers read through it, so they run on Halo 2 Vista's maps (`Vista`)
/// and MCC's format-13 maps (`Mcc`) alike.
pub trait Reader {
    /// The elements of the block whose header (count, address) is at
    /// `at` in `parent`, each `size` bytes; none when it doesn't read.
    /// `parent` is part of the tag last opened (`MenuMap::open`).
    fn block(&mut self, parent: &[u8], at: usize, size: usize) -> Vec<Vec<u8>>;
    /// A string id's name; empty when it isn't known.
    fn string_id(&self, id: u32) -> String;
    fn tag(&self, datum: u32) -> Option<TagRef>;
}

/// A whole map whose menus can be read: its tags, their meta, and its
/// string lists' English strings.
pub trait MenuMap: Reader {
    /// Every tag of `group`, in index order.
    fn tags_of(&self, group: GroupTag) -> Vec<TagRef>;
    /// A tag's meta. `block` then reads the blocks nested in it.
    fn open(&mut self, datum: DatumIndex) -> Result<Vec<u8>>;
    /// A string list's (`unic`) English strings: each one's string id and
    /// words; none when they don't read. The tag last opened stays so.
    fn unicode_strings(&mut self, unic: DatumIndex) -> Vec<(u32, String)>;
}

/// Halo 2 Vista's maps (cache format 8): a map set, whose blocks are read
/// from the file that holds the tag last opened.
pub struct Vista<'a> {
    set: &'a mut MapSet,
    source: Source,
    /// The language table, read on first use.
    table: Option<Vec<(u32, String)>>,
}

impl<'a> Vista<'a> {
    pub fn new(set: &'a mut MapSet) -> Vista<'a> {
        Vista {
            set,
            source: Source::Map,
            table: None,
        }
    }
}

impl Reader for Vista<'_> {
    fn block(&mut self, parent: &[u8], at: usize, size: usize) -> Vec<Vec<u8>> {
        if parent.len() < at + 8 || size == 0 {
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

impl MenuMap for Vista<'_> {
    fn tags_of(&self, group: GroupTag) -> Vec<TagRef> {
        self.set
            .map
            .tags
            .iter()
            .filter(|t| t.group == group)
            .map(|t| TagRef {
                datum: t.datum,
                name: t.name.clone(),
            })
            .collect()
    }

    fn open(&mut self, datum: DatumIndex) -> Result<Vec<u8>> {
        let (source, _, data) = self.set.tag_data(datum)?;
        self.source = source;
        Ok(data)
    }

    fn unicode_strings(&mut self, unic: DatumIndex) -> Vec<(u32, String)> {
        let table = self
            .table
            .get_or_insert_with(|| text::language_table(self.set).unwrap_or_default());
        text::unicode_strings(self.set, table, unic).unwrap_or_default()
    }
}

/// What didn't read through an `Mcc`, kept for the probe's report.
#[derive(Clone, Debug, Default)]
pub struct Problems {
    /// Blocks that didn't read, and why the first one didn't.
    pub bad_blocks: usize,
    pub first_bad_block: Option<String>,
    /// Blocks that read but lie outside the meta of the tag they're in
    /// (Vista keeps a tag's blocks inside its meta).
    pub outside: usize,
    pub first_outside: Option<String>,
    /// Why the string-id table, or the English strings, didn't read.
    pub string_ids: Option<String>,
    pub language: Option<String>,
}

/// MCC's Halo 2 maps (cache format 13), with the shared.map beside them
/// when one is given: a tag with no meta in the map is looked up there by
/// group and name. The UI tags are expected to keep Vista's layouts in
/// format 13 (`docs/notes/launcher/menu.md`, section 3), so the parsers
/// run on them unchanged; the probe checks that on the owner's PC.
pub struct Mcc<'a, R> {
    map: &'a mut mcc::Map<R>,
    shared: Option<&'a mut mcc::Map<R>>,
    /// The tag last opened, and whether it is in the shared map.
    current: Option<(bool, mcc::Tag)>,
    /// The English strings, read on first use.
    table: Option<Vec<(u32, String)>>,
    pub problems: Problems,
}

impl<'a, R: Read + Seek> Mcc<'a, R> {
    /// Reads the string-id tables first, so names resolve.
    pub fn new(map: &'a mut mcc::Map<R>, mut shared: Option<&'a mut mcc::Map<R>>) -> Mcc<'a, R> {
        let mut problems = Problems::default();
        if let Err(e) = map.read_string_ids() {
            problems.string_ids = Some(e.to_string());
        }
        if let Some(s) = shared.as_deref_mut() {
            // The map's own table matters; the shared one only fills gaps.
            let _ = s.read_string_ids();
        }
        Mcc {
            map,
            shared,
            current: None,
            table: None,
            problems,
        }
    }

    /// A tag's meta, from the map or else the shared map, with the tag as
    /// that file lists it and whether it is the shared map.
    fn meta(&mut self, datum: DatumIndex) -> Result<(bool, mcc::Tag, Vec<u8>)> {
        let tag = self
            .map
            .tag(datum)
            .cloned()
            .ok_or_else(|| Error::Corrupt(format!("no tag {:08x} in the map", datum.0)))?;
        if tag.has_data() {
            let meta = self.map.tag_meta(&tag)?;
            return Ok((false, tag, meta));
        }
        let Some(shared) = self.shared.as_deref_mut() else {
            return Err(Error::Corrupt(format!(
                "tag {} has no meta in the map, and no shared.map was given",
                tag.name
            )));
        };
        let there = shared
            .find_tag(tag.group, &tag.name)
            .filter(|t| t.has_data())
            .cloned()
            .ok_or_else(|| Error::Corrupt(format!("tag {} has no meta in either map", tag.name)))?;
        let meta = shared.tag_meta(&there)?;
        Ok((true, there, meta))
    }
}

impl<R: Read + Seek> Reader for Mcc<'_, R> {
    fn block(&mut self, parent: &[u8], at: usize, size: usize) -> Vec<Vec<u8>> {
        let in_shared = matches!(self.current, Some((true, _)));
        let file: &mut mcc::Map<R> = match (in_shared, self.shared.as_deref_mut()) {
            (true, Some(s)) => s,
            _ => &mut *self.map,
        };
        let name = self.current.as_ref().map_or("", |(_, t)| t.name.as_str());
        match file.block(parent, at, size) {
            Ok(raw) => {
                if let (Ok((_, address)), Some((_, tag))) =
                    (mcc::block_header(parent, at), &self.current)
                {
                    if !raw.is_empty() && !tag.holds(address, raw.len()) {
                        self.problems.outside += 1;
                        self.problems
                            .first_outside
                            .get_or_insert_with(|| format!("{name}: a block at {address:#x}"));
                    }
                }
                raw.chunks_exact(size.max(1)).map(<[u8]>::to_vec).collect()
            }
            Err(e) => {
                self.problems.bad_blocks += 1;
                self.problems
                    .first_bad_block
                    .get_or_insert_with(|| format!("{name}: {e}"));
                Vec::new()
            }
        }
    }

    fn string_id(&self, id: u32) -> String {
        let shared = self.shared.as_deref().and_then(|s| s.string_id(id));
        self.map
            .string_id(id)
            .or(shared)
            .unwrap_or_default()
            .to_string()
    }

    fn tag(&self, datum: u32) -> Option<TagRef> {
        let datum = DatumIndex(datum);
        let in_shared = matches!(self.current, Some((true, _)));
        let shared = self.shared.as_deref().and_then(|s| s.tag(datum));
        let t = match in_shared {
            true => shared.or_else(|| self.map.tag(datum)),
            false => self.map.tag(datum).or(shared),
        }?;
        Some(TagRef {
            datum: t.datum,
            name: t.name.clone(),
        })
    }
}

impl<R: Read + Seek> MenuMap for Mcc<'_, R> {
    fn tags_of(&self, group: GroupTag) -> Vec<TagRef> {
        self.map
            .tags
            .iter()
            .filter(|t| t.group == group)
            .map(|t| TagRef {
                datum: t.datum,
                name: t.name.clone(),
            })
            .collect()
    }

    fn open(&mut self, datum: DatumIndex) -> Result<Vec<u8>> {
        let (in_shared, tag, meta) = self.meta(datum)?;
        self.current = Some((in_shared, tag));
        Ok(meta)
    }

    fn unicode_strings(&mut self, unic: DatumIndex) -> Vec<(u32, String)> {
        if self.table.is_none() {
            let mut found = self.map.language_table();
            if let (Err(_), Some(s)) = (&found, self.shared.as_deref_mut()) {
                found = s.language_table().map_err(|e| {
                    let own = found.as_ref().err().map(|e| e.to_string());
                    Error::Corrupt(format!("{}; in shared.map: {e}", own.unwrap_or_default()))
                });
            }
            self.table = Some(match found {
                Ok(l) => l.strings,
                Err(e) => {
                    self.problems.language = Some(e.to_string());
                    Vec::new()
                }
            });
        }
        let Ok((_, _, meta)) = self.meta(unic) else {
            return Vec::new();
        };
        let table = self.table.as_deref().unwrap_or_default();
        english_range(&meta, table)
    }
}

/// A string list's English strings: the range of the language table its
/// meta names at 0x10 (start, count; as in Vista).
fn english_range(unic: &[u8], table: &[(u32, String)]) -> Vec<(u32, String)> {
    if unic.len() < UNIC_ENGLISH + 4 {
        return Vec::new();
    }
    let start = usize::from(u16_at(unic, UNIC_ENGLISH));
    let count = usize::from(u16_at(unic, UNIC_ENGLISH + 2));
    table
        .get(start..(start + count).min(table.len()))
        .unwrap_or_default()
        .to_vec()
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

/// A string list's strings, or none.
fn strings(m: &mut dyn MenuMap, datum: u32) -> Vec<(u32, String)> {
    match datum {
        u32::MAX => Vec::new(),
        d => m.unicode_strings(DatumIndex(d)),
    }
}

/// Read the menus of a mainmenu.map: its screen collection's globals, list
/// skins and screens. Screens that don't read are left out.
pub fn read_from(m: &mut dyn MenuMap) -> Result<Ui> {
    let wgtz = GroupTag::parse("wgtz").expect("a group tag");
    let collection = m
        .tags_of(wgtz)
        .first()
        .map(|t| t.datum)
        .ok_or_else(|| Error::Corrupt("no menus in this map".into()))?;
    let z = m.open(collection)?;
    if z.len() < WGTZ_SCREENS + 8 {
        return Err(Error::Corrupt("screen collection too small".into()));
    }
    let screens: Vec<u32> = m
        .block(&z, WGTZ_SCREENS, TAG_REF_SIZE)
        .iter()
        .map(|s| u32_at(s, 4))
        .collect();
    let globals = DatumIndex(u32_at(&z, WGTZ_GLOBALS + 4));
    let g = m.open(globals)?;
    let skin_refs: Vec<u32> = m
        .block(&g, WIGL_SKINS, TAG_REF_SIZE)
        .iter()
        .map(|s| u32_at(s, 4))
        .collect();
    let mut skins = Vec::new();
    for datum in skin_refs {
        let name = m.tag(datum).map(|t| t.name).unwrap_or_default();
        let skin = m
            .open(DatumIndex(datum))
            .and_then(|d| parse_skin(m, &name, &d));
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
        strings(m, u32_at(&g, WIGL_BUTTON_KEYS + 4))
    } else {
        Vec::new()
    };
    // Back to the globals, whose blocks the skins' have replaced.
    m.open(globals)?;
    let globals = parse_globals(m, &g, skins, &button_keys)?;
    let mut ui = Ui {
        globals,
        screens: Vec::new(),
    };
    for datum in screens {
        let Some(name) = m.tag(datum).map(|t| t.name) else {
            continue;
        };
        let Ok(d) = m.open(DatumIndex(datum)) else {
            continue;
        };
        let list = if d.len() >= WGIT_SIZE {
            strings(m, u32_at(&d, WGIT_STRINGS + 4))
        } else {
            Vec::new()
        };
        if let Ok(s) = parse_screen(m, &name, &d, &list) {
            ui.screens.push(s);
        }
    }
    Ok(ui)
}

/// Read the menus of Halo 2 Vista's mainmenu.map (`set`).
pub fn read(set: &mut MapSet) -> Result<Ui> {
    read_from(&mut Vista::new(set))
}

/// Read the menus of MCC's format-13 mainmenu.map, with the shared.map
/// beside it if it was opened.
pub fn read_mcc<R: Read + Seek>(
    map: &mut mcc::Map<R>,
    shared: Option<&mut mcc::Map<R>>,
) -> Result<Ui> {
    read_from(&mut Mcc::new(map, shared))
}

/// One frame of a bitmap tag: which image, and the rectangle of it in
/// pixels (left, top, width, height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub image: usize,
    pub rect: [u32; 4],
}

/// The frame a menu bitmap's `frame` number shows: sequence `frame`'s
/// first sprite (its image and rectangle), or with no sprites its first
/// image whole; with no sequences, image `frame` whole. This is how
/// menu-preview read the field; how Halo 2 itself uses it is unverified.
fn frame_of(sizes: &[(u32, u32)], sequences: &[Sequence], frame: i16) -> Option<Frame> {
    let n = usize::try_from(frame.max(0)).ok()?;
    let whole = |image: usize| {
        let (w, h) = *sizes.get(image)?;
        Some(Frame {
            image,
            rect: [0, 0, w, h],
        })
    };
    let Some(s) = sequences.get(n) else {
        return whole(n);
    };
    match s.sprites.first() {
        Some(p) => {
            let image = usize::try_from(p.bitmap.max(0)).ok()?;
            let (w, h) = *sizes.get(image)?;
            Some(Frame {
                image,
                rect: p.pixels(w, h),
            })
        }
        None => whole(usize::try_from(s.first_bitmap.max(0)).ok()?),
    }
}

/// A bitmap tag's images (size and format, without pixels) and sequences.
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapInfo {
    pub name: String,
    pub images: Vec<ImageInfo>,
    pub sequences: Vec<Sequence>,
}

impl BitmapInfo {
    /// The image and rectangle a menu bitmap's `frame` shows.
    pub fn frame(&self, frame: i16) -> Option<Frame> {
        let sizes: Vec<(u32, u32)> = self.images.iter().map(|i| (i.width, i.height)).collect();
        frame_of(&sizes, &self.sequences, frame)
    }
}

/// A bitmap tag's images decoded to RGBA8 (top level), with its sequences.
#[derive(Clone, Debug)]
pub struct Picture {
    pub name: String,
    pub images: Vec<Image>,
    pub sequences: Vec<Sequence>,
}

impl Picture {
    /// The image and rectangle a menu bitmap's `frame` shows.
    pub fn frame(&self, frame: i16) -> Option<Frame> {
        let sizes: Vec<(u32, u32)> = self.images.iter().map(|i| (i.width, i.height)).collect();
        frame_of(&sizes, &self.sequences, frame)
    }

    /// A frame's pixels, cut out of its image.
    pub fn crop(&self, frame: Frame) -> Option<Image> {
        let im = self.images.get(frame.image)?;
        let [x, y, w, h] = frame.rect.map(|v| v as usize);
        let (iw, ih) = (im.width as usize, im.height as usize);
        if x + w > iw || y + h > ih {
            return None;
        }
        let rgba = (y..y + h)
            .flat_map(|row| &im.rgba[(row * iw + x) * 4..(row * iw + x + w) * 4])
            .copied()
            .collect();
        Some(Image {
            width: w as u32,
            height: h as u32,
            rgba,
        })
    }
}

/// Where the menus' pictures come from, by bitmap tag name: Halo 2 Vista's
/// map set, or MCC's map with its textures.dat (`MccPictures`).
pub trait Pictures {
    /// A bitmap tag's images and sequences, without decoding pixels.
    fn bitmap(&mut self, name: &str) -> Result<BitmapInfo>;
    /// Image `index` of a bitmap tag, decoded to RGBA8 (its top level).
    fn image(&mut self, name: &str, index: usize) -> Result<Image>;
    /// Every image of a bitmap tag decoded, with its sequences.
    fn picture(&mut self, name: &str) -> Result<Picture> {
        let info = self.bitmap(name)?;
        let images = (0..info.images.len())
            .map(|i| self.image(name, i))
            .collect::<Result<_>>()?;
        Ok(Picture {
            name: info.name,
            images,
            sequences: info.sequences,
        })
    }
}

impl<P: Pictures + ?Sized> Pictures for &mut P {
    fn bitmap(&mut self, name: &str) -> Result<BitmapInfo> {
        (**self).bitmap(name)
    }

    fn image(&mut self, name: &str, index: usize) -> Result<Image> {
        (**self).image(name, index)
    }
}

impl Pictures for MapSet {
    fn bitmap(&mut self, name: &str) -> Result<BitmapInfo> {
        let tag = vista_bitmap(self, name)?;
        Ok(BitmapInfo {
            name: tag.name,
            images: bitmap::image_infos(self, tag.datum)?,
            sequences: bitmap::read_sequences(self, tag.datum)?,
        })
    }

    fn image(&mut self, name: &str, index: usize) -> Result<Image> {
        let tag = vista_bitmap(self, name)?;
        bitmap::read_bitmap_at(self, tag.datum, index)
    }
}

/// The bitm tag named `name` in a Vista map set (letter case ignored).
fn vista_bitmap(set: &MapSet, name: &str) -> Result<crate::Tag> {
    let bitm = GroupTag::parse("bitm").expect("a group tag");
    set.map
        .tags
        .iter()
        .find(|t| t.group == bitm && t.name.eq_ignore_ascii_case(name))
        .cloned()
        .ok_or_else(|| Error::Corrupt(format!("no bitmap {name}")))
}

/// MCC's pictures: the bitmap tags of a format-13 map (or of its shared
/// map, by name, when the map has no meta for one) and their pixels in
/// textures.dat.
pub struct MccPictures<R, T> {
    pub map: mcc::Map<R>,
    pub shared: Option<mcc::Map<R>>,
    pub textures: Option<mcc::Textures<T>>,
}

impl<R: Read + Seek, T: Read + Seek> MccPictures<R, T> {
    /// The map (or shared map) that holds bitmap `name`'s meta, and its tag.
    fn find(&mut self, name: &str) -> Result<(&mut mcc::Map<R>, mcc::Tag)> {
        let bitm = GroupTag::parse("bitm").expect("a group tag");
        if let Some(t) = self.map.find_tag(bitm, name).filter(|t| t.has_data()) {
            let t = t.clone();
            return Ok((&mut self.map, t));
        }
        if let Some(shared) = self.shared.as_mut() {
            if let Some(t) = shared.find_tag(bitm, name).filter(|t| t.has_data()) {
                let t = t.clone();
                return Ok((shared, t));
            }
        }
        Err(Error::Corrupt(format!("no bitmap {name} with meta")))
    }
}

impl<R: Read + Seek, T: Read + Seek> Pictures for MccPictures<R, T> {
    fn bitmap(&mut self, name: &str) -> Result<BitmapInfo> {
        let (file, tag) = self.find(name)?;
        let images = file
            .bitmaps(&tag)?
            .iter()
            .map(|e| ImageInfo {
                width: e.width.into(),
                height: e.height.into(),
                format: e.format,
            })
            .collect();
        // Sequences are at a predicted place; images are worth having
        // without them.
        let sequences = file.sequences(&tag).unwrap_or_default();
        Ok(BitmapInfo {
            name: tag.name,
            images,
            sequences,
        })
    }

    fn image(&mut self, name: &str, index: usize) -> Result<Image> {
        let (file, tag) = self.find(name)?;
        let entries = file.bitmaps(&tag)?;
        let e = entries
            .get(index)
            .ok_or_else(|| Error::Corrupt(format!("bitmap {name} has no image {index}")))?;
        let textures = self
            .textures
            .as_mut()
            .ok_or_else(|| Error::Corrupt("no textures.dat beside the map".into()))?;
        textures.read(e)
    }
}

/// A mainmenu.map's menus and pictures, from MCC's (format 13, with the
/// textures.dat and shared.map beside it) or Halo 2 Vista's map, told
/// apart by the version in its first bytes.
pub struct Menus {
    /// The UI tags, or why they didn't read (the pictures may still).
    pub ui: Result<Ui>,
    pub pictures: Box<dyn Pictures>,
    /// Read from MCC's map rather than Vista's.
    pub mcc: bool,
    /// What else didn't read, a line each: a file missing beside the map,
    /// blocks or string ids of its UI tags.
    pub notes: Vec<String>,
}

impl Menus {
    /// Fails only when the map itself can't be opened.
    pub fn open(path: &Path) -> Result<Menus> {
        let mut start = [0u8; 8];
        std::fs::File::open(path)?.read_exact(&mut start)?;
        if crate::cache_version(&start) != Some(mcc::VERSION) {
            let mut set = MapSet::open(path)?;
            return Ok(Menus {
                ui: read(&mut set),
                pictures: Box::new(set),
                mcc: false,
                notes: Vec::new(),
            });
        }
        let mut notes = Vec::new();
        let beside = |name: &str| path.with_file_name(name);
        let mut map = mcc::Map::open(path)?;
        let same = path
            .file_name()
            .is_some_and(|f| f.eq_ignore_ascii_case("shared.map"));
        let mut shared = match same {
            true => None,
            false => match mcc::Map::open(&beside("shared.map")) {
                Ok(s) => Some(s),
                Err(e) => {
                    notes.push(format!("no shared.map beside it ({e})"));
                    None
                }
            },
        };
        let mut reader = Mcc::new(&mut map, shared.as_mut());
        let ui = read_from(&mut reader);
        let p = reader.problems;
        if p.bad_blocks > 0 {
            let first = p.first_bad_block.as_deref().unwrap_or_default();
            notes.push(format!(
                "{} blocks of its UI tags didn't read (the first: {first})",
                p.bad_blocks
            ));
        }
        if let Some(e) = &p.string_ids {
            notes.push(format!("its string ids didn't read: {e}"));
        }
        let textures = match mcc::Textures::open(&beside("textures.dat")) {
            Ok(t) => Some(t),
            Err(e) => {
                notes.push(format!("no textures.dat beside it ({e}): no pictures"));
                None
            }
        };
        Ok(Menus {
            ui,
            pictures: Box::new(MccPictures {
                map,
                shared,
                textures,
            }),
            mcc: true,
            notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitmap::{Format, Sprite};
    use crate::mcc::synthetic::{string_id, MapBuilder, Meta, Struct, TexturesDat};
    use std::collections::HashMap;
    use std::fmt::Debug;
    use std::io::Cursor;

    fn put32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// The same made-up tags as Halo 2 Vista lays them out to a reader:
    /// blocks at made-up memory addresses, names for string ids and tags.
    #[derive(Default)]
    struct Fake {
        metas: HashMap<u32, Vec<u8>>,
        blocks: HashMap<u32, Vec<u8>>,
        strings: Vec<String>,
        tags: Vec<(GroupTag, String)>,
        language: Vec<(u32, String)>,
        next: u32,
    }

    impl Fake {
        fn new(b: &MapBuilder) -> Fake {
            let mut f = Fake {
                strings: b.string_ids(),
                tags: b.tags.iter().map(|(g, n, _)| (*g, n.clone())).collect(),
                language: b.language_table(),
                next: 0x8000_0000,
                ..Fake::default()
            };
            for (i, (_, _, meta)) in b.tags.iter().enumerate() {
                let bytes = f.lay_out(b, &meta.to_struct());
                f.metas.insert(MapBuilder::datum(i), bytes);
            }
            f
        }

        fn lay_out(&mut self, b: &MapBuilder, s: &Struct) -> Vec<u8> {
            let mut bytes = s.bytes.clone();
            for (o, group, name) in &s.refs {
                let datum = b.datum_of(*group, name);
                let group = if datum == u32::MAX { u32::MAX } else { group.0 };
                put32(&mut bytes, *o, group);
                put32(&mut bytes, o + 4, datum);
            }
            for (o, name) in &s.ids {
                put32(&mut bytes, *o, string_id(&self.strings, name));
            }
            for (o, elements) in &s.blocks {
                let mut data = Vec::new();
                for e in elements {
                    data.extend(self.lay_out(b, e));
                }
                self.next += 0x1000;
                put32(&mut bytes, *o, elements.len() as u32);
                put32(&mut bytes, o + 4, self.next);
                self.blocks.insert(self.next, data);
            }
            bytes
        }

        fn index(&self, datum: u32) -> Option<usize> {
            let i = usize::from(DatumIndex(datum).index());
            (datum == MapBuilder::datum(i) && i < self.tags.len()).then_some(i)
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
            let i = (id & 0x00FF_FFFF) as usize;
            self.strings.get(i).cloned().unwrap_or_default()
        }

        fn tag(&self, datum: u32) -> Option<TagRef> {
            Some(TagRef {
                datum: DatumIndex(datum),
                name: self.tags[self.index(datum)?].1.clone(),
            })
        }
    }

    impl MenuMap for Fake {
        fn tags_of(&self, group: GroupTag) -> Vec<TagRef> {
            (0..self.tags.len())
                .filter(|&i| self.tags[i].0 == group)
                .map(|i| TagRef {
                    datum: DatumIndex(MapBuilder::datum(i)),
                    name: self.tags[i].1.clone(),
                })
                .collect()
        }

        fn open(&mut self, datum: DatumIndex) -> Result<Vec<u8>> {
            self.metas
                .get(&datum.0)
                .cloned()
                .ok_or_else(|| Error::Corrupt("no such tag".into()))
        }

        fn unicode_strings(&mut self, unic: DatumIndex) -> Vec<(u32, String)> {
            let meta = self.metas.get(&unic.0).cloned().unwrap_or_default();
            english_range(&meta, &self.language)
        }
    }

    /// The format-13 map `b` builds, in small chunks so tags cross them.
    fn mcc_map(b: &MapBuilder) -> mcc::Map<Cursor<Vec<u8>>> {
        let mut b2 = MapBuilder::new(64);
        b2.pad = 5;
        b2.tags = b
            .tags
            .iter()
            .map(|(g, n, m)| (*g, n.clone(), Meta::Struct(m.to_struct())))
            .collect();
        b2.language = b.language.clone();
        mcc::Map::from_reader(Cursor::new(b2.build())).unwrap()
    }

    /// Runs `parse` on the tag named `name` through both readers, checks
    /// they agree and that the format-13 one read every block inside its
    /// tag, and returns what they read.
    fn both<T: PartialEq + Debug>(
        b: &MapBuilder,
        name: &str,
        parse: impl Fn(&mut dyn MenuMap, &[u8]) -> T,
    ) -> T {
        let i = b.tags.iter().position(|t| t.1 == name).unwrap();
        let datum = DatumIndex(MapBuilder::datum(i));
        let mut fake = Fake::new(b);
        let d = fake.open(datum).unwrap();
        let vista = parse(&mut fake, &d);
        let mut map = mcc_map(b);
        let mut m = Mcc::new(&mut map, None);
        let d = m.open(datum).unwrap();
        let format13 = parse(&mut m, &d);
        assert_eq!(m.problems.bad_blocks, 0, "{:?}", m.problems);
        assert_eq!(m.problems.outside, 0, "{:?}", m.problems);
        assert!(m.problems.string_ids.is_none(), "{:?}", m.problems);
        assert_eq!(vista, format13);
        format13
    }

    /// A text block: font, colour, bounds and string id.
    fn text(flags: u32, font: i16, bounds: [i16; 4], id: &str) -> Struct {
        Struct::new(TEXT_SIZE)
            .u32(0, flags)
            .i16(0xA, font)
            .f32(0x10, 0.64)
            .f32(0x14, 0.72)
            .f32(0x18, 0.87)
            .i16(0x1C, bounds[0])
            .i16(0x1E, bounds[1])
            .i16(0x20, bounds[2])
            .i16(0x22, bounds[3])
            .string_id(0x24, id)
    }

    /// The main menu's chrome logo: top-left (-511, 90), fading in with
    /// animation 29.
    fn logo() -> Struct {
        Struct::new(BITMAP_SIZE)
            .i16(4, 30)
            .i16(0xC, -511)
            .i16(0xE, 90)
            .tag_ref(0x18, "bitm", r"ui\start_screen")
            .i16(0x20, 1)
    }

    fn keyframe(alpha: f32, x: f32) -> Struct {
        Struct::new(KEYFRAME_SIZE).f32(4, alpha).f32(8, x)
    }

    /// A screen like the main menu, with one of every pane element.
    fn main_menu_screen() -> Struct {
        let list = Struct::new(LIST_SIZE)
            .u32(0, 3)
            .i16(4, 11)
            .i16(6, 6)
            .i16(8, -178)
            .i16(0xA, -80)
            .i16(0xC, 35);
        let button = Struct::new(BUTTON_SIZE)
            .i16(0xA, 10)
            .tag_ref(0x24, "bitm", r"ui\start_screen")
            .i16(0x2C, 4)
            .i16(0x2E, -2)
            .string_id(0x30, "press_start")
            .i16(0x34, 7);
        let scene = Struct::new(MODEL_SCENE_SIZE)
            .i16(4, 2)
            .block(0xC, vec![Struct::new(NAME_SIZE).text(0, "ui_player1")])
            .block(0x14, Vec::new())
            .f32(0x34, 70.0)
            .i16(0x38, 10);
        let players = Struct::new(PLAYERS_SIZE)
            .tag_ref(4, "skin", r"ui\player_skin")
            .i16(0xC, -300)
            .u8(0x11, 16)
            .u8(0x12, 8)
            .u8(0x13, 2)
            .i16(0x14, 40);
        let pane = Struct::new(PANE_SIZE)
            .i16(2, 30)
            .block(0x4, vec![button])
            .block(0xC, vec![list])
            .block(
                0x1C,
                vec![text(PULSATING, 2, [-65, -400, -105, 400], "press_start")],
            )
            .block(0x24, vec![logo()])
            .block(0x2C, vec![scene])
            .block(0x44, vec![players]);
        Struct::new(WGIT_SIZE)
            .u32(0, NO_HEADER)
            .i16(4, 6)
            .tag_ref(WGIT_STRINGS, "unic", r"ui\main_menu_strings")
            .block(WGIT_PANES, vec![pane])
    }

    /// The tags of a small mainmenu.map.
    fn menus() -> MapBuilder {
        let wgtz = Struct::new(0x20)
            .tag_ref(WGTZ_GLOBALS, "wigl", r"ui\ui_shared_globals")
            .block(
                WGTZ_SCREENS,
                [r"ui\main_menu", r"ui\missing", r"ui\settings"]
                    .iter()
                    .map(|n| Struct::new(TAG_REF_SIZE).tag_ref(0, "wgit", n))
                    .collect(),
            )
            .tag_ref(0x10, "goof", "")
            .tag_ref(0x18, "unic", r"ui\button_keys");
        let settings = Struct::new(WGIT_SIZE)
            .u32(0, HALF_DIALOG)
            .i16(4, 0x13)
            .i16(6, 1)
            .tag_ref(WGIT_STRINGS, "unic", r"ui\settings_strings")
            .string_id(WGIT_HEADER, "settings_header");
        let range = |start: i16, count: i16| Struct::new(0x50).i16(0x10, start).i16(0x12, count);
        let mut b = MapBuilder::new(64)
            .tag("matg", r"globals\globals", Meta::Raw(vec![0; 0x200]))
            .tag("bitm", r"ui\start_screen", Meta::Bitmaps(Vec::new()))
            .tag("bitm", r"ui\list_bkd", Meta::Bitmaps(Vec::new()))
            .tag("unic", r"ui\main_menu_strings", Meta::Struct(range(0, 1)))
            .tag("unic", r"ui\settings_strings", Meta::Struct(range(1, 1)))
            .tag("unic", r"ui\button_keys", Meta::Struct(range(2, 1)))
            .tag("wgit", r"ui\main_menu", Meta::Struct(main_menu_screen()))
            .tag("wgit", r"ui\settings", Meta::Struct(settings))
            .tag("skin", r"ui\list_skins\main_menu", Meta::Struct(skin()))
            .tag("wigl", r"ui\ui_shared_globals", Meta::Struct(globals()))
            .tag("wgtz", r"ui\main_menu", Meta::Struct(wgtz));
        b.language = vec![
            ("press_start".into(), "PRESS ANY KEY TO CONTINUE".into()),
            ("settings_header".into(), "SETTINGS".into()),
            (
                "a_select_b_back".into(),
                "\u{e100} SELECT \u{e101} BACK".into(),
            ),
        ];
        b
    }

    /// A list skin: focus fades 0.5 to 1 in 120 ms; a bar that doesn't
    /// count towards the item's height, and a multiplied sheen.
    fn skin() -> Struct {
        let focus = Struct::new(ITEM_ANIMATION_SIZE)
            .u32(4, 120)
            .block(8, vec![keyframe(0.5, 0.0), keyframe(1.0, 0.0)]);
        let bar = Struct::new(BITMAP_SIZE)
            .u32(0, IGNORE_FOR_LIST_SIZE)
            .i16(0xC, -60)
            .i16(0xE, -40)
            .tag_ref(0x18, "bitm", r"ui\list_bkd");
        let sheen = bar.clone().i16(8, 1).f32(0x10, 0.2);
        Struct::new(SKIN_SIZE)
            .tag_ref(0x4, "bitm", "")
            .i16(0xC, 5)
            .i16(0x10, -5)
            .block(SKIN_ITEM_ANIMATIONS, vec![focus])
            .block(0x1C, vec![text(0, 10, [10, -62, -40, 418], "")])
            .block(0x24, vec![bar, sheen])
    }

    /// The shared globals: two sounds, the full screen header's bounds, a
    /// fade animation, the button legends' strings and one skin.
    fn globals() -> Struct {
        let mut d = Struct::new(WIGL_SIZE);
        for k in 0..5 {
            d = d.tag_ref(WIGL_SOUNDS + 8 * k, "snd!", "");
        }
        let fade = Struct::new(ANIMATION_SIZE)
            .u32(4, 250)
            .block(8, vec![keyframe(0.0, 1024.0), keyframe(1.0, 0.0)])
            .block(0x14, Vec::new())
            .block(0x24, Vec::new());
        d.tag_ref(WIGL_SOUNDS, "bitm", r"ui\start_screen")
            .tag_ref(WIGL_SOUNDS + 2 * 8, "bitm", r"ui\list_bkd")
            .tag_ref(WIGL_MUSIC, "lsnd", "")
            .tag_ref(WIGL_BUTTON_KEYS, "unic", r"ui\button_keys")
            .i16(WIGL_BOUNDS, 567)
            .i16(WIGL_BOUNDS + 2, -730)
            .i16(WIGL_BOUNDS + 4, 520)
            .i16(WIGL_BOUNDS + 6, 50)
            .i16(WIGL_HEADER_FONTS, 2)
            .f32(WIGL_OVERLAY_ALPHA_MOD, 0.1)
            .f32(WIGL_OVERLAY_COLOR, 0.85)
            .f32(WIGL_OVERLAY_COLOR + 12, 0.17)
            .u32(WIGL_MUSIC_FADE, 5500)
            .block(WIGL_ANIMATIONS, vec![fade])
            .block(
                WIGL_SKINS,
                vec![Struct::new(TAG_REF_SIZE).tag_ref(0, "skin", r"ui\list_skins\main_menu")],
            )
    }

    #[test]
    fn a_screen_reads_its_panes_lists_texts_and_bitmaps() {
        let s = both(&menus(), r"ui\main_menu", |r, d| {
            let strings = r.unicode_strings(DatumIndex(u32_at(d, WGIT_STRINGS + 4)));
            parse_screen(r, r"ui\main_menu", d, &strings).unwrap()
        });
        assert_eq!(s.screen_id, 6);
        assert_eq!(s.dialog_size(), DialogSize::Full);
        assert_eq!(s.flags & NO_HEADER, NO_HEADER);
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
        assert_eq!(b.bitmap.as_ref().unwrap().name, r"ui\start_screen");
        assert!(!b.multiply);
        let button = &p.buttons[0];
        assert_eq!(button.bitmap.as_ref().unwrap().name, r"ui\start_screen");
        assert_eq!(button.bitmap_offset, [4, -2]);
        assert_eq!(button.text.font, Font::MainMenu);
        assert_eq!(button.text.depth, 7);
        assert_eq!(
            button.text.text.as_deref(),
            Some("PRESS ANY KEY TO CONTINUE")
        );
        let scene = &p.model_scenes[0];
        assert_eq!(scene.objects, ["ui_player1"]);
        assert!(scene.lights.is_empty());
        assert_eq!((scene.fov, scene.viewport.top), (70.0, 10));
        let players = &p.players[0];
        assert_eq!(players.skin, None, "no skin tag of that name");
        assert_eq!(
            (players.corner, players.max, players.rows, players.columns),
            ([-300, 0], 16, 8, 2)
        );
        assert_eq!(players.row_height, 40);
    }

    #[test]
    fn a_list_skin_reads_its_items_look_and_height() {
        let s = both(&menus(), r"ui\list_skins\main_menu", |r, d| {
            parse_skin(r, r"ui\list_skins\main_menu", d).unwrap()
        });
        assert_eq!(s.arrows, None);
        assert_eq!((s.arrow_up, s.arrow_down), ([5, 0], [-5, 0]));
        assert_eq!(s.item_animations[0].period_ms, 120);
        let alphas: Vec<f32> = s.item_animations[0]
            .keyframes
            .iter()
            .map(|k| k.alpha)
            .collect();
        assert_eq!(alphas, [0.5, 1.0]);
        assert_eq!(s.texts[0].font, Font::MainMenu);
        assert_eq!(s.texts[0].string, "");
        assert_eq!(s.bitmaps[0].corner, [-60, -40]);
        assert_eq!(s.bitmaps[0].bitmap.as_ref().unwrap().name, r"ui\list_bkd");
        assert!(s.bitmaps[1].multiply);
        assert_eq!(s.bitmaps[1].wraps_per_second, [0.2, 0.0]);
        // The text's 50 units: its bitmaps don't count.
        assert_eq!(s.item_height(|_| 62.0), 50.0);
    }

    #[test]
    fn the_globals_read_sounds_bounds_legends_and_animations() {
        let g = both(&menus(), r"ui\ui_shared_globals", |r, d| {
            let keys = r.unicode_strings(DatumIndex(u32_at(d, WIGL_BUTTON_KEYS + 4)));
            parse_globals(r, d, Vec::new(), &keys).unwrap()
        });
        assert_eq!(g.sounds.cursor.as_deref(), Some(r"ui\start_screen"));
        assert_eq!(g.sounds.error.as_deref(), Some(r"ui\list_bkd"));
        assert_eq!(g.sounds.select, None);
        assert_eq!(g.music, None);
        assert_eq!(g.music_fade_ms, 5500);
        assert_eq!(g.header_fonts[0], Font::Title);
        assert_eq!(g.header_bounds[0].left, -730);
        assert_eq!(g.overlay_color, [0.0, 0.0, 0.17, 0.85]);
        assert_eq!(g.overlay_alpha_mod, 0.1);
        assert_eq!(g.animations[0].intro.period_ms, 250);
        assert_eq!(g.animations[0].intro.keyframes[0].position[0], 1024.0);
        assert!(g.animations[0].outro.keyframes.is_empty());
        assert_eq!(
            g.button_key("a_select_b_back"),
            Some("\u{e100} SELECT \u{e101} BACK")
        );
    }

    #[test]
    fn the_whole_menus_read_through_the_screen_collection() {
        let b = menus();
        let vista = read_from(&mut Fake::new(&b)).unwrap();
        let mut map = mcc_map(&b);
        let mut m = Mcc::new(&mut map, None);
        let format13 = read_from(&mut m).unwrap();
        assert_eq!(m.problems.bad_blocks, 0, "{:?}", m.problems);
        assert!(m.problems.language.is_none(), "{:?}", m.problems);
        assert_eq!(vista, format13);
        let ui = format13;
        // The screen that names no tag is left out.
        let names: Vec<&str> = ui.screens.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [r"ui\main_menu", r"ui\settings"]);
        let settings = ui.screen(r"ui\settings").unwrap();
        assert_eq!(settings.dialog_size(), DialogSize::Half);
        assert_eq!(settings.header, "settings_header");
        assert_eq!(settings.header_text.as_deref(), Some("SETTINGS"));
        assert_eq!(ui.globals.skins.len(), 1);
        assert_eq!(ui.globals.skins[0].name, r"ui\list_skins\main_menu");
        assert_eq!(ui.globals.skins[0].item_animations[0].period_ms, 120);
        assert!(ui.globals.button_key("a_select_b_back").is_some());
        assert_eq!(
            ui.globals.sounds.cursor.as_deref(),
            Some(r"ui\start_screen")
        );
        let main = ui.screen(r"ui\main_menu").unwrap();
        assert_eq!(
            main.panes[0].texts[0].text.as_deref(),
            Some("PRESS ANY KEY TO CONTINUE")
        );
        // The same through read_mcc.
        let mut map = mcc_map(&b);
        assert_eq!(read_mcc(&mut map, None).unwrap(), ui);
        // No collection: an error, not a panic.
        let empty = MapBuilder::new(64).tag("matg", "g", Meta::Raw(vec![0; 8]));
        assert!(read_from(&mut Fake::new(&empty)).is_err());
        let mut map = mcc_map(&empty);
        assert!(read_mcc(&mut map, None).is_err());
    }

    #[test]
    fn a_tag_without_meta_is_read_from_the_shared_map() {
        // The map lists the skin with no meta; the shared map holds it.
        let full = menus();
        let mut local = mcc_map(&full);
        let mut shared = mcc_map(&full);
        let i = local
            .tags
            .iter()
            .position(|t| t.name == r"ui\list_skins\main_menu")
            .unwrap();
        local.tags[i].address = 0;
        let alone = read_mcc(&mut local, None).unwrap();
        assert!(alone.globals.skins[0].item_animations.is_empty());
        let ui = read_mcc(&mut local, Some(&mut shared)).unwrap();
        assert_eq!(ui.globals.skins[0].item_animations[0].period_ms, 120);
        assert_eq!(
            ui.globals.skins[0].bitmaps[0].bitmap.as_ref().unwrap().name,
            r"ui\list_bkd"
        );
    }

    /// A 4 by 2 picture whose pixel x, y has R = 10 x + y + n and G = n.
    fn picture_bgra(n: u8) -> Vec<u8> {
        (0..2u8)
            .flat_map(|y| (0..4u8).flat_map(move |x| [0, n, 10 * x + y + n, 255]))
            .collect()
    }

    fn button_sequences() -> Vec<Sequence> {
        vec![
            Sequence {
                name: "a".into(),
                first_bitmap: 1,
                bitmap_count: 1,
                sprites: Vec::new(),
            },
            Sequence {
                name: "b".into(),
                first_bitmap: 0,
                bitmap_count: 1,
                sprites: vec![Sprite {
                    bitmap: 1,
                    left: 0.5,
                    right: 1.0,
                    top: 0.5,
                    bottom: 1.0,
                    registration: [0.0; 2],
                }],
            },
        ]
    }

    #[test]
    fn mcc_pictures_decode_by_name_with_frames() {
        let mut dat = TexturesDat::default();
        let entries = vec![
            dat.push_record(
                4,
                2,
                Format::A8R8G8B8,
                &picture_bgra(0),
                1,
                mcc::RecordLayout::Single,
            ),
            dat.push_record(
                4,
                2,
                Format::A8R8G8B8,
                &picture_bgra(100),
                3,
                mcc::RecordLayout::SizesFirst,
            ),
        ];
        let map = MapBuilder::new(64)
            .tag(
                "bitm",
                r"ui\shared\buttons",
                Meta::Bitmap {
                    entries,
                    sequences: button_sequences(),
                },
            )
            .build();
        let mut pictures = MccPictures {
            map: mcc::Map::from_reader(Cursor::new(map)).unwrap(),
            shared: None,
            textures: Some(mcc::Textures::from_reader(Cursor::new(dat.bytes)).unwrap()),
        };
        let info = pictures.bitmap(r"UI\Shared\Buttons").unwrap();
        assert_eq!(info.images.len(), 2);
        assert_eq!((info.images[1].width, info.images[1].height), (4, 2));
        assert_eq!(info.sequences, button_sequences());
        assert_eq!(
            info.frame(0),
            Some(Frame {
                image: 1,
                rect: [0, 0, 4, 2]
            })
        );
        assert_eq!(
            info.frame(1),
            Some(Frame {
                image: 1,
                rect: [2, 1, 2, 1]
            })
        );
        let p = pictures.picture(r"ui\shared\buttons").unwrap();
        assert_eq!(p.images[1].rgba[..4], [100, 100, 0, 255]);
        let crop = p.crop(p.frame(1).unwrap()).unwrap();
        assert_eq!((crop.width, crop.height), (2, 1));
        // Pixels (2, 1) and (3, 1) of image 1.
        assert_eq!(crop.rgba, [121, 100, 0, 255, 131, 100, 0, 255]);
        assert!(pictures.image(r"ui\shared\buttons", 2).is_err());
        assert!(pictures.bitmap(r"ui\nothing").is_err());
        // Without textures.dat, sizes still read but pixels don't.
        pictures.textures = None;
        assert!(pictures.bitmap(r"ui\shared\buttons").is_ok());
        assert!(pictures.image(r"ui\shared\buttons", 0).is_err());
    }

    /// A fresh folder for files a test writes.
    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("blam-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn menus_open_an_mcc_map_with_its_textures() {
        let dir = folder("mcc");
        let mut b = menus();
        let mut dat = TexturesDat::default();
        let logo = dat.push_record(
            4,
            2,
            Format::A8R8G8B8,
            &picture_bgra(7),
            1,
            mcc::RecordLayout::Single,
        );
        b.tags[1].2 = Meta::Bitmaps(vec![logo]);
        std::fs::write(dir.join("mainmenu.map"), b.build()).unwrap();
        std::fs::write(dir.join("textures.dat"), dat.bytes).unwrap();
        let mut menus = Menus::open(&dir.join("mainmenu.map")).unwrap();
        assert!(menus.mcc);
        let ui = menus.ui.as_ref().unwrap();
        assert_eq!(ui.screens.len(), 2);
        let main = ui.screen(r"ui\main_menu").unwrap();
        let name = main.panes[0].bitmaps[0].bitmap.clone().unwrap().name;
        let p = menus.pictures.picture(&name).unwrap();
        assert_eq!(p.images[0].rgba[..4], [7, 7, 0, 255]);
        // No shared.map beside it, which the notes say.
        assert_eq!(menus.notes.len(), 1, "{:?}", menus.notes);
        assert!(menus.notes[0].starts_with("no shared.map"));
        // Not a map at all.
        std::fs::write(dir.join("junk.map"), b"not a map").unwrap();
        assert!(Menus::open(&dir.join("junk.map")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Halo 2 Vista map holding one bitm tag: two 4 by 2 images and the
    /// button sequences, its pixels in the map (top bits of their pointers
    /// clear).
    fn vista_map() -> Vec<u8> {
        const MASK: u32 = 0x8000_0000;
        const META: usize = 0x1000;
        let put16 = |b: &mut [u8], o: usize, v: i16| b[o..o + 2].copy_from_slice(&v.to_le_bytes());
        let putf = |b: &mut [u8], o: usize, v: f32| put32(b, o, v.to_bits());
        let name = br"ui\shared\buttons";
        let mut b = vec![0u8; META + 0x400];
        put32(&mut b, 0, u32::from_be_bytes(*b"head"));
        put32(&mut b, 4, 8);
        put32(&mut b, 0x7FC, u32::from_be_bytes(*b"foot"));
        put32(&mut b, 0x14C, 2);
        // The tag name: index at 0x800, text at 0x810.
        put32(&mut b, 0x2CC, 1);
        put32(&mut b, 0x2D0, 0x810);
        put32(&mut b, 0x2D4, name.len() as u32 + 1);
        put32(&mut b, 0x2D8, 0x800);
        b[0x810..0x810 + name.len()].copy_from_slice(name);
        // The meta area: header, one group, one tag, its data and blocks.
        put32(&mut b, 0x10, META as u32);
        put32(&mut b, 0x14, 0x100);
        put32(&mut b, 0x1C, 0x400);
        put32(&mut b, 0x20, MASK);
        put32(&mut b, META, 0x20);
        put32(&mut b, META + 4, 1);
        put32(&mut b, META + 8, 0x2C);
        put32(&mut b, META + 0xC, u32::MAX);
        put32(&mut b, META + 0x10, u32::MAX);
        put32(&mut b, META + 0x18, 1);
        put32(&mut b, META + 0x1C, u32::from_be_bytes(*b"tags"));
        put32(&mut b, META + 0x20, u32::from_be_bytes(*b"bitm"));
        put32(&mut b, META + 0x2C, u32::from_be_bytes(*b"bitm"));
        put32(&mut b, META + 0x30, 0xE000_0000);
        put32(&mut b, META + 0x34, MASK + 0x100);
        put32(&mut b, META + 0x38, 0x50);
        let (data, sequences, sprites, entries) = (0x100, 0x180, 0x200, 0x280);
        put32(&mut b, META + data + 0x3C, 2);
        put32(&mut b, META + data + 0x40, MASK + sequences as u32);
        put32(&mut b, META + data + 0x44, 2);
        put32(&mut b, META + data + 0x48, MASK + entries as u32);
        for (k, s) in button_sequences().iter().enumerate() {
            let at = META + sequences + k * 0x3C;
            b[at..at + s.name.len()].copy_from_slice(s.name.as_bytes());
            put16(&mut b, at + 0x20, s.first_bitmap);
            put16(&mut b, at + 0x22, s.bitmap_count);
            put32(&mut b, at + 0x34, s.sprites.len() as u32);
            put32(&mut b, at + 0x38, MASK + sprites as u32);
            for (j, p) in s.sprites.iter().enumerate() {
                let at = META + sprites + j * 0x20;
                put16(&mut b, at, p.bitmap);
                putf(&mut b, at + 8, p.left);
                putf(&mut b, at + 0xC, p.right);
                putf(&mut b, at + 0x10, p.top);
                putf(&mut b, at + 0x14, p.bottom);
            }
        }
        let mut pixels = Vec::new();
        for n in 0..2 {
            let e = META + entries + n * 116;
            put16(&mut b, e + 4, 4);
            put16(&mut b, e + 6, 2);
            put16(&mut b, e + 12, 11);
            pixels.push((e, mcc::synthetic::zlib(&picture_bgra(n as u8 * 100))));
        }
        for (e, z) in pixels {
            let at = b.len() as u32;
            put32(&mut b, e + 28, at);
            put32(&mut b, e + 52, z.len() as u32);
            b.extend(z);
        }
        b
    }

    #[test]
    fn vista_pictures_decode_by_name_with_frames() {
        let dir = folder("vista");
        let path = dir.join("mainmenu.map");
        std::fs::write(&path, vista_map()).unwrap();
        let mut set = MapSet::open(&path).unwrap();
        let info = set.bitmap(r"ui\shared\buttons").unwrap();
        assert_eq!(info.images.len(), 2);
        assert_eq!(info.images[0].format, Format::A8R8G8B8);
        assert_eq!(info.sequences, button_sequences());
        let p = set.picture(r"UI\shared\buttons").unwrap();
        assert_eq!(p.images[1].rgba[..4], [100, 100, 0, 255]);
        let crop = p.crop(p.frame(1).unwrap()).unwrap();
        assert_eq!(crop.rgba, [121, 100, 0, 255, 131, 100, 0, 255]);
        assert!(set.bitmap(r"ui\nothing").is_err());
        // Menus::open takes it as Vista's, and finds no menus in it, but
        // still its pictures.
        let mut menus = Menus::open(&path).unwrap();
        assert!(!menus.mcc && menus.ui.is_err());
        assert!(menus.pictures.picture(r"ui\shared\buttons").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frames_fall_back_and_stay_inside_their_image() {
        let sizes = [(8, 4), (2, 2)];
        let none: Vec<Sequence> = Vec::new();
        assert_eq!(
            frame_of(&sizes, &none, 1),
            Some(Frame {
                image: 1,
                rect: [0, 0, 2, 2]
            })
        );
        assert_eq!(frame_of(&sizes, &none, 2), None);
        assert_eq!(
            frame_of(&sizes, &none, -3),
            Some(Frame {
                image: 0,
                rect: [0, 0, 8, 4]
            })
        );
        let mut seq = button_sequences();
        seq[1].sprites[0].bitmap = 5;
        assert_eq!(
            frame_of(&sizes, &seq, 1),
            None,
            "a sprite on a missing image"
        );
        seq[1].sprites[0] = Sprite {
            bitmap: 0,
            left: -1.0,
            right: 2.0,
            top: 0.75,
            bottom: 0.25,
            registration: [0.0; 2],
        };
        assert_eq!(
            frame_of(&sizes, &seq, 1),
            Some(Frame {
                image: 0,
                rect: [0, 1, 8, 2]
            })
        );
        let p = Picture {
            name: String::new(),
            images: vec![Image {
                width: 2,
                height: 1,
                rgba: vec![0; 8],
            }],
            sequences: Vec::new(),
        };
        assert!(p
            .crop(Frame {
                image: 0,
                rect: [1, 0, 2, 1]
            })
            .is_none());
        assert!(p
            .crop(Frame {
                image: 1,
                rect: [0, 0, 1, 1]
            })
            .is_none());
    }

    /// Checks the real menus: the logo, the main menu's list and skin, the
    /// shell's framing, a sound and a legend.
    fn check_real_menus(ui: &Ui) {
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

    /// The real menus, from the mainmenu.map in `H2_MAPS` (Halo 2 Vista).
    #[test]
    #[ignore]
    fn mainmenu_maps_menus_read() {
        let dir = std::env::var("H2_MAPS").expect("H2_MAPS");
        let mut set = MapSet::open(std::path::Path::new(&dir).join("mainmenu.map")).unwrap();
        check_real_menus(&read(&mut set).unwrap());
    }

    /// The real menus and their pictures, from MCC's mainmenu.map and
    /// textures.dat in `H2_MCC_MAPS`.
    #[test]
    #[ignore]
    fn mcc_mainmenu_menus_and_pictures_read() {
        let dir = std::env::var("H2_MCC_MAPS").expect("H2_MCC_MAPS");
        let mut menus = Menus::open(&std::path::Path::new(&dir).join("mainmenu.map")).unwrap();
        assert!(menus.mcc, "{:?}", menus.notes);
        let ui = menus.ui.as_ref().unwrap();
        check_real_menus(ui);
        for screen in ["start_screen", "main_menu", "game_shell_background"] {
            let s = ui
                .screens
                .iter()
                .find(|s| s.name.ends_with(&format!("\\{screen}")))
                .unwrap();
            let names: Vec<String> = s
                .panes
                .iter()
                .flat_map(|p| p.bitmaps.iter().filter_map(|b| b.bitmap.clone()))
                .map(|t| t.name)
                .collect();
            for name in names {
                let p = menus.pictures.picture(&name);
                assert!(p.is_ok(), "{name}: {:?}", p.err());
            }
        }
    }
}
