//! Halo 2's menu art, from the player's own mainmenu.map, and its fonts,
//! from the `fonts` folder beside it: the screens' bitmaps and text, the
//! list skins' item backgrounds and the button legends, drawn where the
//! screens' tags put them (`blam_cache::ui`). Without mainmenu.map the
//! menus draw plain panels instead, and without the fonts (or a glyph)
//! the remake's own 5x7 font.
//!
//! Menu units put the origin in the middle of the window, +x right and +y
//! up, 1200 units to the height of the menus' 640x480 frame (2.5 units to
//! each of its pixels).

use crate::gpu::{hud_mode, UI_TEXTURES};
use crate::hud::HudBuilder;
use blam_cache::bitmap::{self, Image};
use blam_cache::font::{self as fonts, Font, FontFile};
use blam_cache::ui::{self, Animation, ListSkin, Screen, Ui};
use blam_cache::MapSet;
use std::collections::HashMap;
use std::path::Path;

/// The screens the remake's menus are drawn from.
pub const START_SCREEN: &str = "ui\\screens\\game_shell\\start_screen\\start_screen";
pub const MAIN_MENU: &str = "ui\\screens\\game_shell\\main_menu_screen\\main_menu";
/// The framing and moving tracks behind every screen past the main menu.
pub const BACKGROUND: &str = "ui\\screens\\game_shell\\main_menu_screen\\game_shell_background";
pub const LOBBY: &str = "ui\\screens\\game_shell\\pregame_lobby\\pregame_lobby";
pub const BROWSER: &str =
    "ui\\screens\\game_shell\\network_squad_browser_screen\\network_squad_browser";
pub const PROFILE: &str =
    "ui\\screens\\game_shell\\settings_screen\\player_profile\\edit_profile_menu";
pub const OPTIONS: &str = "ui\\screens\\game_shell\\settings_screen\\variant_settings\\editing_format_screens\\top_level_settings";
/// The small dialog (a question and two answers) and the larger one.
pub const DIALOG: &str = "ui\\screens\\misc\\error_dialog_ok_cancel";
pub const LARGE_DIALOG: &str = "ui\\screens\\misc\\error_dialog_large";
const SCREENS: [&str; 9] = [
    START_SCREEN,
    MAIN_MENU,
    BACKGROUND,
    LOBBY,
    BROWSER,
    PROFILE,
    OPTIONS,
    DIALOG,
    LARGE_DIALOG,
];
/// The default list skin stretched wide, for the online lists.
pub const WIDE_SKIN: &str = "ui\\list_skins\\default\\default_wide";

/// Text of a font's slot without its font file: the 5x7 font, this many
/// menu units tall (about the line height of Halo 2 Vista's font in the
/// slot, whose pixels are a menu unit each).
fn fallback_height(font: Font) -> f32 {
    match font {
        Font::SuperLarge => 48.0,
        Font::Title => 36.0,
        Font::MainMenu => 30.0,
        Font::LargeBody => 26.0,
        Font::FullHudMessage => 24.0,
        Font::Body | Font::EnglishBody | Font::HudNumber => 22.0,
        Font::SplitHudMessage | Font::Terminal | Font::TextChat => 20.0,
        Font::Subtitle => 18.0,
    }
}

/// The smallest text the menus draw with the 5x7 font, in window pixels:
/// a pixel to each of its pixels (in its 6x8 cell).
pub const MIN_TEXT: f32 = 8.0;
/// And with Halo 2's fonts: a font's ascent is never under this many
/// window pixels (capitals about 7 tall).
const MIN_ASCENT: f32 = 10.0;
/// Glyphs packed into a font's atlas this far apart, and this wide.
const ATLAS_PAD: usize = 2;
const ATLAS_W: usize = 1024;

/// The controller's buttons in the fonts' private code points (the
/// button legends' `\u{e100}`...), and the 5x7 font's stand-in colours.
const BUTTONS: [(char, char, [f32; 3]); 4] = [
    ('\u{e100}', 'A', [0.25, 0.75, 0.2]),
    ('\u{e101}', 'B', [0.85, 0.15, 0.1]),
    ('\u{e102}', 'X', [0.15, 0.35, 0.9]),
    ('\u{e103}', 'Y', [0.9, 0.75, 0.1]),
];

/// The characters the menus draw.
fn wanted(c: char) -> bool {
    (' '..='~').contains(&c) || BUTTONS.iter().any(|b| b.0 == c)
}

/// One of the art's pictures: its texture and size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub texture: usize,
    pub size: [f32; 2],
}

/// A glyph's place in its font's atlas.
#[derive(Clone, Copy, Debug)]
struct AtlasGlyph {
    code: char,
    uv: [f32; 4],
    size: [f32; 2],
    /// How far right of the pen its picture starts, and how far down its
    /// picture the baseline is.
    origin: [f32; 2],
    advance: f32,
}

/// A font's glyphs packed in one texture.
#[derive(Clone, Debug)]
struct Atlas {
    texture: usize,
    ascent: f32,
    descent: f32,
    glyphs: Vec<AtlasGlyph>,
    kerning: Vec<(u8, u8, i16)>,
}

impl Atlas {
    fn glyph(&self, c: char) -> Option<&AtlasGlyph> {
        self.glyphs
            .binary_search_by_key(&c, |g| g.code)
            .ok()
            .map(|k| &self.glyphs[k])
    }

    fn kern(&self, a: char, b: char) -> f32 {
        let (Ok(a), Ok(b)) = (u8::try_from(a), u8::try_from(b)) else {
            return 0.0;
        };
        self.kerning
            .iter()
            .find(|k| k.0 == a && k.1 == b)
            .map_or(0.0, |k| k.2 as f32)
    }
}

/// The menus' art and fonts.
#[derive(Default)]
pub struct MenuArt {
    pub ui: Ui,
    /// The pictures and font atlases, in texture order from `UI_TEXTURES`,
    /// until they're uploaded.
    pub images: Vec<Image>,
    /// By bitmap tag and frame; the all clear ones are left out.
    pictures: HashMap<(String, i16), Picture>,
    /// Each font slot's atlas, by `Font::index`.
    fonts: Vec<Option<Atlas>>,
}

impl MenuArt {
    /// The art of `dir`'s mainmenu.map and the fonts of its `fonts`
    /// folder. What won't read is left out (and drawn plainly).
    pub fn load(dir: &Path) -> MenuArt {
        let mut art = MenuArt::default();
        let table = fonts::read_table(&dir.join("fonts"), wanted);
        art.fonts = table
            .iter()
            .map(|f| f.as_ref().map(|f| art.add_atlas(f)))
            .collect();
        let found = art.fonts.iter().filter(|f| f.is_some()).count();
        if found < art.fonts.len() {
            println!("menus: {found} of {} fonts found", art.fonts.len());
        }
        let read = MapSet::open(dir.join("mainmenu.map"))
            .and_then(|mut set| ui::read(&mut set).map(|ui| (set, ui)));
        match read {
            Ok((mut set, ui)) => {
                art.ui = ui;
                art.add_pictures(&mut set);
            }
            Err(e) => println!("warning: no menu art: {e}"),
        }
        art
    }

    /// The bitmaps of the screens drawn, and of their lists' skins.
    fn add_pictures(&mut self, set: &mut MapSet) {
        let mut wanted: Vec<(ui::TagRef, i16)> = Vec::new();
        let mut want = |b: &ui::Bitmap| {
            if let Some(t) = &b.bitmap {
                wanted.push((t.clone(), b.frame));
            }
        };
        let mut skins = vec![self.wide_skin()];
        for screen in SCREENS.iter().filter_map(|&n| self.ui.screen(n)) {
            for pane in &screen.panes {
                pane.bitmaps.iter().for_each(&mut want);
                for b in &pane.buttons {
                    if let Some(t) = &b.bitmap {
                        want(&ui::Bitmap {
                            bitmap: Some(t.clone()),
                            frame: 0,
                            ..blank_bitmap()
                        });
                    }
                }
                skins.extend(pane.lists.iter().map(|l| l.skin.max(0) as usize));
            }
        }
        for skin in skins.iter().filter_map(|&k| self.ui.globals.skins.get(k)) {
            skin.bitmaps.iter().for_each(&mut want);
        }
        for (tag, frame) in wanted {
            let key = (tag.name.clone(), frame);
            if self.pictures.contains_key(&key) {
                continue;
            }
            // A frame is an image, or a sequence's first image.
            let sequences = bitmap::read_sequences(set, tag.datum).unwrap_or_default();
            let index = sequences
                .get(frame.max(0) as usize)
                .map_or(frame.max(0) as usize, |s| s.first_bitmap.max(0) as usize);
            let Ok(image) = bitmap::read_bitmap_at(set, tag.datum, index) else {
                continue;
            };
            if image.rgba.chunks(4).all(|p| p[3] == 0) {
                continue;
            }
            let picture = Picture {
                texture: UI_TEXTURES + self.images.len(),
                size: [image.width as f32, image.height as f32],
            };
            self.pictures.insert(key, picture);
            self.images.push(image);
        }
    }

    /// Pack a font's glyphs into an atlas, keeping it to upload.
    fn add_atlas(&mut self, font: &FontFile) -> Atlas {
        let (mut x, mut y, mut row) = (ATLAS_PAD, ATLAS_PAD, 0);
        let mut places = Vec::new();
        for g in &font.glyphs {
            let (w, h) = (g.width as usize, g.height as usize);
            if x + w + ATLAS_PAD > ATLAS_W {
                (x, y, row) = (ATLAS_PAD, y + row + ATLAS_PAD, 0);
            }
            places.push((x, y));
            x += w + ATLAS_PAD;
            row = row.max(h);
        }
        let height = y + row + ATLAS_PAD;
        let mut rgba = vec![0u8; ATLAS_W * height * 4];
        let mut glyphs = Vec::new();
        for (g, &(x, y)) in font.glyphs.iter().zip(&places) {
            let w = g.width as usize;
            for (row, line) in g.rgba.chunks(w * 4).enumerate() {
                let at = ((y + row) * ATLAS_W + x) * 4;
                rgba[at..at + line.len()].copy_from_slice(line);
            }
            let (aw, ah) = (ATLAS_W as f32, height as f32);
            let size = [g.width as f32, g.height as f32];
            glyphs.push(AtlasGlyph {
                code: g.code,
                uv: [
                    x as f32 / aw,
                    y as f32 / ah,
                    (x as f32 + size[0]) / aw,
                    (y as f32 + size[1]) / ah,
                ],
                size,
                origin: [g.origin[0] as f32, g.origin[1] as f32],
                advance: g.advance as f32,
            });
        }
        let atlas = Atlas {
            texture: UI_TEXTURES + self.images.len(),
            ascent: font.ascent.max(1) as f32,
            descent: font.descent.max(0) as f32,
            glyphs,
            kerning: font.kerning.clone(),
        };
        self.images.push(Image {
            width: ATLAS_W as u32,
            height: height as u32,
            rgba,
        });
        atlas
    }

    /// Whether mainmenu.map's art was read.
    pub fn loaded(&self) -> bool {
        !self.ui.screens.is_empty()
    }

    pub fn screen(&self, name: &str) -> Option<&Screen> {
        self.ui.screen(name)
    }

    /// The skin of a screen's first list.
    pub fn list_skin(&self, screen: &str) -> Option<&ListSkin> {
        let list = self.screen(screen)?.panes.first()?.lists.first()?;
        self.ui.globals.skins.get(list.skin.max(0) as usize)
    }

    fn wide_skin(&self) -> usize {
        let skins = &self.ui.globals.skins;
        skins.iter().position(|s| s.name == WIDE_SKIN).unwrap_or(0)
    }

    /// The skin the online lists use.
    pub fn online_skin(&self) -> Option<&ListSkin> {
        self.ui.globals.skins.get(self.wide_skin())
    }

    pub fn picture(&self, b: &ui::Bitmap) -> Option<Picture> {
        let name = &b.bitmap.as_ref()?.name;
        self.pictures.get(&(name.clone(), b.frame)).copied()
    }

    /// A bitmap's size in menu units: its pixels times its scale.
    pub fn size(&self, b: &ui::Bitmap) -> Option<[f32; 2]> {
        let p = self.picture(b)?;
        Some([0, 1].map(|k| p.size[k] * scale(b.scale[k])))
    }

    /// How far apart a list skin's items are (menu units).
    pub fn item_height(&self, skin: &ListSkin) -> f32 {
        skin.item_height(|b| self.size(b).map_or(0.0, |s| s[1]))
    }

    /// A list skin's item's box from its corner (left, top, right,
    /// bottom, menu units): its backgrounds' (those that size the list),
    /// or else its first text's.
    pub fn item_box(&self, skin: &ListSkin) -> Option<[f32; 4]> {
        let backgrounds = skin
            .bitmaps
            .iter()
            .filter(|b| b.flags & ui::IGNORE_FOR_LIST_SIZE == 0)
            .filter_map(|b| {
                let [w, h] = self.size(b)?;
                let [x, y] = [b.corner[0] as f32, b.corner[1] as f32];
                Some([x, y + h, x + w, y])
            });
        let union = |a: [f32; 4], b: [f32; 4]| {
            [a[0].min(b[0]), a[1].max(b[1]), a[2].max(b[2]), a[3].min(b[3])]
        };
        backgrounds.reduce(union).or_else(|| {
            let r = skin.texts.first()?.bounds;
            Some([r.left, r.top, r.right, r.bottom].map(f32::from))
        })
    }

    fn atlas(&self, font: Font) -> Option<&Atlas> {
        let slot = |f: Font| self.fonts.get(f.index()).and_then(Option::as_ref);
        slot(font).or_else(|| slot(Font::Body))
    }

    /// A screen's intro animation (by its index in the globals).
    pub fn intro(&self, animation: Option<usize>) -> Option<&Animation> {
        let a = self.ui.globals.animations.get(animation?)?;
        Some(&a.intro)
    }
}

/// A bitmap widget with nothing set, to fill in.
fn blank_bitmap() -> ui::Bitmap {
    ui::Bitmap {
        flags: 0,
        animation: None,
        delay_ms: 0,
        multiply: false,
        frame: 0,
        corner: [0, 0],
        wraps_per_second: [0.0, 0.0],
        bitmap: None,
        depth: 0,
        scale: [0.0, 0.0],
    }
}

/// A bitmap's scale: 0 is none (1).
fn scale(s: f32) -> f32 {
    if s > 0.0 {
        s
    } else {
        1.0
    }
}

/// An animation `t` seconds in: its alpha and how far it has moved (menu
/// units), between its keyframes, which are spread evenly over its period;
/// the last keyframe once it's over (and none for no animation).
pub fn animate(anim: Option<&Animation>, t: f32) -> (f32, [f32; 2]) {
    let Some(a) = anim.filter(|a| !a.keyframes.is_empty()) else {
        return (1.0, [0.0, 0.0]);
    };
    let keys = &a.keyframes;
    let last = keys.len() - 1;
    let along = if a.period_ms <= 0 || last == 0 {
        last as f32
    } else {
        (t * 1000.0 / a.period_ms as f32).clamp(0.0, 1.0) * last as f32
    };
    let k = (along.floor() as usize).min(last);
    let (from, to) = (&keys[k], &keys[(k + 1).min(last)]);
    let f = along - k as f32;
    let mix = |a: f32, b: f32| a + (b - a) * f;
    (
        mix(from.alpha, to.alpha),
        [
            mix(from.position[0], to.position[0]),
            mix(from.position[1], to.position[1]),
        ],
    )
}

/// The highest alpha an animation reaches (1 without one).
pub fn peak(anim: Option<&Animation>) -> f32 {
    anim.and_then(|a| a.keyframes.iter().map(|k| k.alpha).reduce(f32::max))
        .filter(|&p| p > 0.0)
        .unwrap_or(1.0)
}

/// A colour of the tags (gamma space) as a HUD colour (linear).
pub fn tag_color(c: [f32; 3], alpha: f32) -> [f32; 4] {
    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), alpha]
}

/// Menu units on the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    /// Window pixels per unit.
    pub k: f32,
    /// The window pixel at the origin.
    pub origin: [f32; 2],
}

impl Space {
    pub fn at(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        [self.origin[0] + x * self.k, self.origin[1] - y * self.k]
    }

    /// A box (left, top, right, bottom) as window pixels (x0, y0, x1, y1).
    pub fn rect(&self, [l, t, r, b]: [f32; 4]) -> [f32; 4] {
        let [x0, y0] = self.at([l, t]);
        let [x1, y1] = self.at([r, b]);
        [x0, y0, x1, y1]
    }
}

/// Where text goes in its box: across (`ui::LEFT_JUSTIFY`,
/// `ui::RIGHT_JUSTIFY`, otherwise centred) and in what.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub font: Font,
    pub flags: u32,
    pub color: [f32; 4],
}

impl Style {
    pub fn new(font: Font, flags: u32, color: [f32; 4]) -> Style {
        Style { font, flags, color }
    }

    /// A text widget's look, faded to `alpha`.
    pub fn of(t: &ui::Text, alpha: f32) -> Style {
        Style::new(t.font, t.flags, tag_color(t.color, alpha))
    }

    pub fn alpha(self, alpha: f32) -> Style {
        let mut c = self.color;
        c[3] *= alpha;
        Style { color: c, ..self }
    }
}

/// Draws the menus' art and text on a HUD.
pub struct Painter<'a> {
    pub hb: &'a mut HudBuilder,
    pub art: &'a MenuArt,
    pub sp: Space,
    /// The 5x7 font's texture, and a white one.
    pub font: usize,
    pub white: usize,
    /// Seconds the menus have been up: the art scrolls by it.
    pub clock: f32,
    /// Text is drawn this much smaller than its font (1, but in
    /// `line_to_fit`).
    shrink: f32,
}

impl<'a> Painter<'a> {
    /// Drawing on `hb` in menu units `sp`, with the 5x7 font's and a white
    /// texture, `clock` seconds after the menus came up.
    pub fn new(
        hb: &'a mut HudBuilder,
        art: &'a MenuArt,
        sp: Space,
        [font, white]: [usize; 2],
        clock: f32,
    ) -> Painter<'a> {
        Painter {
            hb,
            art,
            sp,
            font,
            white,
            clock,
            shrink: 1.0,
        }
    }

    /// A bitmap with its top left at `at` (menu units) and its own size,
    /// faded to `alpha` (and stretched across by `stretch`); multiplied
    /// art darkens what's behind.
    pub fn bitmap(&mut self, b: &ui::Bitmap, at: [f32; 2], alpha: f32, stretch: f32) {
        self.bitmap_but(b, at, alpha, stretch, None);
    }

    /// A bitmap as `bitmap` draws it, but for where it covers `hole` (menu
    /// units), which it's drawn around.
    fn bitmap_but(
        &mut self,
        b: &ui::Bitmap,
        at: [f32; 2],
        alpha: f32,
        stretch: f32,
        hole: Option<[f32; 4]>,
    ) {
        let (Some(p), Some([w, h])) = (self.art.picture(b), self.art.size(b)) else {
            return;
        };
        let [l, t, r, bottom] = [at[0], at[1], at[0] + w * stretch, at[1] - h];
        let parts = match hole {
            Some([hl, ht, hr, hb]) if hl < r && hr > l && hb < t && ht > bottom => vec![
                // Left, right, above and below the hole.
                [l, t, hl, bottom],
                [hr, t, r, bottom],
                [hl, t, hr, ht],
                [hl, hb, hr, bottom],
            ],
            _ => vec![[l, t, r, bottom]],
        };
        let [su, sv] = b.wraps_per_second.map(|v| v * self.clock);
        let u = |x: f32| su + (x - l) / (r - l);
        let v = |y: f32| sv + (t - y) / (t - bottom);
        let mode = if b.multiply {
            hud_mode::MULTIPLY
        } else {
            hud_mode::PLAIN
        };
        for [x0, y0, x1, y1] in parts {
            let [x0, y0, x1, y1] = [x0.max(l), y0.min(t), x1.min(r), y1.max(bottom)];
            if x1 <= x0 || y0 <= y1 {
                continue;
            }
            let rect = self.sp.rect([x0, y0, x1, y1]);
            let uv = [u(x0), v(y0), u(x1), v(y1)];
            self.hb.quad(p.texture, rect, uv, [1.0, 1.0, 1.0, alpha], mode, 0.0);
        }
    }

    /// A thin light rim around a box (menu units), two pixels wide.
    pub fn rim(&mut self, [l, t, r, b]: [f32; 4], alpha: f32) {
        let rim = [0.35, 0.5, 0.7, 0.5 * alpha];
        let e = 2.0 / self.sp.k;
        for edge in [
            [l - e, t + e, r + e, t],
            [l - e, b, r + e, b - e],
            [l - e, t, l, b],
            [r, t, r + e, b],
        ] {
            self.quad(edge, rim);
        }
    }

    /// A screen's bitmaps (its first pane's, of those `keep` keeps) in
    /// their depth order, `age` seconds after it came up, as its intro
    /// animations bring them in; around `hole`, if there's one.
    pub fn screen(
        &mut self,
        name: &str,
        age: f32,
        keep: impl Fn(&ui::Bitmap) -> bool,
        hole: Option<[f32; 4]>,
    ) {
        let Some(pane) = self.art.screen(name).and_then(|s| s.panes.first()) else {
            return;
        };
        let mut bitmaps: Vec<&ui::Bitmap> = pane.bitmaps.iter().filter(|b| keep(b)).collect();
        bitmaps.sort_by_key(|b| b.depth);
        for b in bitmaps {
            let t = age - b.delay_ms as f32 / 1000.0;
            let (alpha, [dx, dy]) = animate(self.art.intro(b.animation), t);
            let at = [b.corner[0] as f32 + dx, b.corner[1] as f32 + dy];
            self.bitmap_but(b, at, alpha, 1.0, hole);
        }
    }

    /// A list item's skin bitmaps (those `keep` keeps), placed by their
    /// bottom left from the item's corner, faded to `alpha`, the item
    /// backgrounds stretched across by `stretch`.
    pub fn item(
        &mut self,
        skin: &ListSkin,
        corner: [f32; 2],
        alpha: f32,
        stretch: f32,
        keep: impl Fn(&ui::Bitmap) -> bool,
    ) {
        let mut bitmaps: Vec<&ui::Bitmap> = skin.bitmaps.iter().filter(|b| keep(b)).collect();
        bitmaps.sort_by_key(|b| b.depth);
        for b in bitmaps {
            let Some([_, h]) = self.art.size(b) else {
                continue;
            };
            let x = corner[0] + b.corner[0] as f32;
            let bottom = corner[1] + b.corner[1] as f32;
            // Only the item's background stretches; caps and marks don't.
            let s = if b.flags & ui::IGNORE_FOR_LIST_SIZE == 0 {
                stretch
            } else {
                1.0
            };
            self.bitmap(b, [x, bottom + h], alpha, s);
        }
    }

    /// A plain rectangle (menu units).
    pub fn quad(&mut self, rect: [f32; 4], color: [f32; 4]) {
        let r = self.sp.rect(rect);
        self.hb
            .quad(self.white, r, [0.0; 4], color, hud_mode::PLAIN, 0.0);
    }

    /// Window pixels per font pixel of text in `font`, and its ascent and
    /// descent in window pixels; None for the 5x7 font.
    fn metrics(&self, font: Font) -> Option<(&'a Atlas, f32)> {
        let art: &'a MenuArt = self.art;
        let a = art.atlas(font)?;
        let s = (self.sp.k * self.shrink).max(MIN_ASCENT / a.ascent);
        Some((a, s))
    }

    /// The 5x7 font's height for `font`, in window pixels.
    fn cell(&self, font: Font) -> f32 {
        (fallback_height(font) * self.sp.k * self.shrink).max(MIN_TEXT)
    }

    /// How tall a line of `font` is, in menu units.
    pub fn line_height(&self, font: Font) -> f32 {
        match self.metrics(font) {
            Some((a, s)) => (a.ascent + a.descent) * s / self.sp.k,
            None => self.cell(font) / self.sp.k,
        }
    }

    /// How wide `text` is in `font`, in menu units.
    pub fn width(&self, font: Font, text: &str) -> f32 {
        let px = match self.metrics(font) {
            Some((a, s)) => {
                let mut w = 0.0;
                let mut last = None;
                for c in text.chars() {
                    w += self.advance(a, c, last) * s;
                    last = Some(c);
                }
                w
            }
            None => text.chars().count() as f32 * self.cell(font) * crate::font::ASPECT,
        };
        px / self.sp.k
    }

    /// How far the pen moves past `c` (font pixels), kerned after `last`.
    fn advance(&self, a: &Atlas, c: char, last: Option<char>) -> f32 {
        let kern = last.map_or(0.0, |l| a.kern(l, c));
        match a.glyph(c) {
            Some(g) => g.advance + kern,
            // Drawn with the 5x7 font: its cell, at the font's height.
            None => (a.ascent + a.descent) * crate::font::ASPECT,
        }
    }

    /// `text` cut to fit `room` menu units, marked ".." where it was cut.
    pub fn fit(&self, font: Font, text: &str, room: f32) -> String {
        if self.width(font, text) <= room {
            return text.to_string();
        }
        let mut kept: Vec<char> = text.chars().collect();
        while !kept.is_empty() {
            kept.pop();
            let cut = format!("{}..", kept.iter().collect::<String>().trim_end());
            if self.width(font, &cut) <= room {
                return cut;
            }
        }
        String::new()
    }

    /// `text` broken into lines at most `room` menu units wide, at spaces
    /// (and at line breaks).
    pub fn wrap(&self, font: Font, text: &str, room: f32) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for paragraph in text.split(['\n', '\r']).filter(|p| !p.trim().is_empty()) {
            let mut line = String::new();
            for word in paragraph.split_whitespace() {
                let longer = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if line.is_empty() || self.width(font, &longer) <= room {
                    line = longer;
                } else {
                    lines.push(std::mem::replace(&mut line, word.to_string()));
                }
            }
            lines.push(line);
        }
        lines
    }

    /// A line of text in a box (menu units: left, top, right, bottom):
    /// across as its style says, centred down.
    pub fn line(&mut self, bounds: [f32; 4], style: Style, text: &str) {
        let [l, t, r, b] = bounds;
        let width = self.width(style.font, text);
        let x = if style.flags & ui::LEFT_JUSTIFY != 0 {
            l
        } else if style.flags & ui::RIGHT_JUSTIFY != 0 {
            r - width
        } else {
            (l + r - width) * 0.5
        };
        let top = (t + b + self.line_height(style.font)) * 0.5;
        let mut color = style.color;
        if style.flags & ui::PULSATING != 0 {
            // Halo 2's "press any key" breathes about every second and a half.
            let phase = (self.clock * std::f32::consts::TAU / 1.5).cos();
            color[3] *= 0.6 + 0.4 * phase;
        }
        self.text_at([x, top], style.font, color, text);
    }

    /// A line of text as `line` draws it, but made smaller (down to `least`
    /// of its size) to fit its box's width, and cut to fit if it must.
    pub fn line_to_fit(&mut self, bounds: [f32; 4], style: Style, text: &str, least: f32) {
        let room = bounds[2] - bounds[0];
        let wide = self.width(style.font, text);
        self.shrink = (room / wide.max(1e-3)).clamp(least, 1.0);
        let text = self.fit(style.font, text, room);
        self.line(bounds, style, &text);
        self.shrink = 1.0;
    }

    /// Lines of text from the top of a box, wrapped to its width; as many
    /// as fit. Returns how many were drawn.
    pub fn paragraph(&mut self, bounds: [f32; 4], style: Style, text: &str) -> usize {
        let [l, t, r, b] = bounds;
        let step = self.line_height(style.font);
        let lines = self.wrap(style.font, text, r - l);
        let fits = (((t - b) / step).floor() as usize).max(1);
        for (k, line) in lines.iter().take(fits).enumerate() {
            let top = t - step * k as f32;
            self.line([l, top, r, top - step], style, line);
        }
        lines.len().min(fits)
    }

    /// Text with the top left of its line at `at` (menu units).
    pub fn text_at(&mut self, at: [f32; 2], font: Font, color: [f32; 4], text: &str) {
        let [x, y] = self.sp.at(at);
        let Some((a, s)) = self.metrics(font) else {
            let h = self.cell(font);
            self.glyphs_5x7([x, y], h, text, color);
            return;
        };
        let baseline = y + a.ascent * s;
        let mut pen = x;
        let mut last = None;
        for c in text.chars() {
            pen += last.map_or(0.0, |l| a.kern(l, c)) * s;
            match a.glyph(c) {
                Some(g) => {
                    let x0 = pen + g.origin[0] * s;
                    let y0 = baseline - g.origin[1] * s;
                    let rect = [x0, y0, x0 + g.size[0] * s, y0 + g.size[1] * s];
                    self.hb
                        .quad(a.texture, rect, g.uv, color, hud_mode::PLAIN, 0.0);
                }
                None => {
                    let h = (a.ascent + a.descent) * s;
                    self.glyphs_5x7([pen, y], h, &c.to_string(), color);
                }
            }
            pen += self.advance(a, c, None) * s;
            last = Some(c);
        }
    }

    /// Text in the 5x7 font, its cells `h` window pixels tall, from `at`;
    /// the controller's buttons as coloured squares with their letters.
    fn glyphs_5x7(&mut self, [x, y]: [f32; 2], h: f32, text: &str, color: [f32; 4]) {
        let cw = h * crate::font::ASPECT;
        for (k, c) in text.chars().enumerate() {
            let x0 = x + cw * k as f32;
            match BUTTONS.iter().find(|b| b.0 == c) {
                Some(&(_, letter, rgb)) => {
                    let fill = [rgb[0], rgb[1], rgb[2], color[3]];
                    let rect = [x0, y, x0 + cw, y + h];
                    self.hb
                        .quad(self.white, rect, [0.0; 4], fill, hud_mode::PLAIN, 0.0);
                    let ink = [1.0, 1.0, 1.0, color[3]];
                    self.hb
                        .text_left(self.font, [x0, y], h, &letter.to_string(), ink);
                }
                None => self
                    .hb
                    .text_left(self.font, [x0, y], h, &c.to_string(), color),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_cache::ui::Keyframe;

    fn fade(period_ms: i32, alphas: &[f32]) -> Animation {
        Animation {
            period_ms,
            keyframes: alphas
                .iter()
                .map(|&alpha| Keyframe {
                    alpha,
                    position: [0.0; 3],
                })
                .collect(),
        }
    }

    #[test]
    fn animations_run_through_their_keyframes() {
        // The main menu's list: from clear to half, over 250 ms.
        let a = fade(250, &[0.0, 0.5]);
        assert_eq!(animate(Some(&a), 0.0).0, 0.0);
        assert_eq!(animate(Some(&a), 0.125).0, 0.25);
        assert_eq!(animate(Some(&a), 9.0).0, 0.5);
        assert_eq!(peak(Some(&a)), 0.5);
        // Four keyframes: the last third of the time is the fade.
        let a = fade(300, &[0.0, 0.0, 0.0, 1.0]);
        assert_eq!(animate(Some(&a), 0.15).0, 0.0);
        assert!((animate(Some(&a), 0.25).0 - 0.5).abs() < 1e-5);
        assert_eq!(animate(None, 0.0), (1.0, [0.0, 0.0]));
    }

    #[test]
    fn menu_units_fill_the_frame() {
        // 1280x720: 1.5 pixels to the frame's pixel, so 0.6 to a unit.
        let sp = Space {
            k: 0.6,
            origin: [640.0, 360.0],
        };
        assert_eq!(sp.at([0.0, 600.0]), [640.0, 0.0]);
        assert_eq!(sp.rect([-100.0, 50.0, 100.0, -50.0]), [580.0, 330.0, 700.0, 390.0]);
    }

    #[test]
    fn without_fonts_text_is_the_5x7_font_never_too_small() {
        let art = MenuArt::default();
        let mut hb = HudBuilder::new(320.0, 180.0);
        let sp = Space {
            k: 0.15,
            origin: [160.0, 90.0],
        };
        let mut p = Painter::new(&mut hb, &art, sp, [0, 1], 0.0);
        let style = Style::new(Font::Body, ui::LEFT_JUSTIFY, [1.0; 4]);
        assert_eq!(p.line_height(Font::Body), MIN_TEXT / 0.15);
        p.line([-500.0, 100.0, 500.0, 0.0], style, "A\u{e100}");
        // A's cell, then the button's square and its letter.
        let b = hb.finish();
        let heights: Vec<f32> = b
            .iter()
            .flat_map(|b| b.vertices.chunks(6))
            .map(|q| q[2].position[1] - q[0].position[1])
            .collect();
        assert_eq!(heights, [MIN_TEXT; 3]);
        assert!(b.iter().any(|b| b.texture == 1));
    }

    #[test]
    fn fonts_pack_into_an_atlas() {
        let mut art = MenuArt::default();
        let glyph = |code, w: u16| fonts::Glyph {
            code,
            advance: w + 1,
            width: w,
            height: 3,
            origin: [0, 2],
            rgba: vec![255; w as usize * 3 * 4],
        };
        let font = FontFile {
            ascent: 2,
            descent: 1,
            glyphs: vec![glyph('A', 2), glyph('B', 3)],
            kerning: vec![(b'A', b'B', -1)],
        };
        let atlas = art.add_atlas(&font);
        assert_eq!(art.images.len(), 1);
        assert_eq!(atlas.texture, UI_TEXTURES);
        let b = atlas.glyph('B').unwrap();
        // After A and the padding either side of it.
        assert_eq!(b.uv[0], (ATLAS_PAD * 2 + 2) as f32 / ATLAS_W as f32);
        assert_eq!(atlas.kern('A', 'B'), -1.0);
        let img = &art.images[0];
        let at = (ATLAS_PAD * img.width as usize + ATLAS_PAD) * 4;
        assert_eq!(img.rgba[at + 3], 255);
    }
}
