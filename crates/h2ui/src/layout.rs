//! Halo 2's UI space and the layout numbers the menus use when no UI tags
//! are read (`docs/notes/mainmenu-plan.md` 1.3 and 1.4; menu-preview's
//! fallbacks).
//!
//! UI units put the origin in the middle of the screen, +x right and +y
//! up, 1,200 units from the bottom to the top. The Xbox's menus were laid
//! out in a 640 by 480 frame, 2.5 units to its pixel (±800 by ±600), which
//! is always shown whole; a wider window shows more at the sides (16:9 is
//! about ±1067), a narrower one more above and below. A screen's bitmaps
//! are placed by their top left corner and a list skin's by their bottom
//! left, at a unit to each of their pixels times their scale.
//!
//! Boxes in UI units are `[left, top, right, bottom]` (+y up); boxes in
//! window pixels are `[x0, y0, x1, y1]` (+y down).

use crate::anim::{Animation, ItemLook};
use crate::paint::Blend;
use crate::text::Justify;
use crate::{Font, Rgba};

/// UI units from the bottom of the screen to its top.
pub const HEIGHT: f32 = 1200.0;
/// The Xbox's 640 by 480 frame in UI units, shown whole in any window.
pub const FRAME: [f32; 4] = [-800.0, 600.0, 800.0, -600.0];
/// The 16:9 safe area (inferred from the tags, checked with a mock).
pub const SAFE_16_9: [f32; 4] = [-1067.0, 600.0, 1067.0, -600.0];

/// UI units on a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    /// Window pixels to a UI unit.
    pub k: f32,
    /// The window pixel at the UI origin.
    pub origin: [f32; 2],
    /// The window's size in pixels.
    pub size: [f32; 2],
}

impl Space {
    /// UI space on a `w` by `h` window: as tall as the window, unless the
    /// window is narrower than 4:3, when the frame's width fills it.
    pub fn new(w: u32, h: u32) -> Space {
        let (w, h) = (w as f32, h as f32);
        let frame_w = FRAME[2] - FRAME[0];
        Space {
            k: (h / HEIGHT).min(w / frame_w),
            origin: [w * 0.5, h * 0.5],
            size: [w, h],
        }
    }

    /// A point in UI units as a window pixel.
    pub fn at(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        [self.origin[0] + x * self.k, self.origin[1] - y * self.k]
    }

    /// A box in UI units as a box of window pixels.
    pub fn rect(&self, [l, t, r, b]: [f32; 4]) -> [f32; 4] {
        let [x0, y0] = self.at([l, t]);
        let [x1, y1] = self.at([r, b]);
        [x0, y0, x1, y1]
    }

    /// A window pixel in UI units (for the mouse).
    pub fn unit(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let k = self.k.max(f32::MIN_POSITIVE);
        [(x - self.origin[0]) / k, (self.origin[1] - y) / k]
    }

    /// The box of UI units the window shows.
    pub fn shown(&self) -> [f32; 4] {
        let [l, t] = self.unit([0.0, 0.0]);
        let [r, b] = self.unit(self.size);
        [l, t, r, b]
    }

    /// The whole window, in window pixels.
    pub fn window(&self) -> [f32; 4] {
        [0.0, 0.0, self.size[0], self.size[1]]
    }
}

/// A box moved by `[x, y]`.
pub fn offset([l, t, r, b]: [f32; 4], [x, y]: [f32; 2]) -> [f32; 4] {
    [l + x, t + y, r + x, b + y]
}

/// Whether a point is in a box (UI units).
pub fn contains([l, t, r, b]: [f32; 4], [x, y]: [f32; 2]) -> bool {
    x >= l && x < r && y <= t && y > b
}

/// Halo 2's colours (gamma space), from the tags unless marked.
pub mod colour {
    use crate::Rgba;

    /// The main menu's rows (skin 11's text) and their shadow.
    pub const MAIN_MENU: [f32; 3] = [0.62, 0.74, 0.84];
    pub const MAIN_MENU_SHADOW: [f32; 3] = [0.29, 0.33, 0.37];
    /// Headers (the globals' text colour).
    pub const HEADER: [f32; 3] = [0.68, 0.76, 0.85];
    /// The button legends' words: not the headers' colour. An estimate
    /// from the owner's Xbox screenshot, where they are near white and
    /// less blue than the header; Vista (Cartographer) draws them a
    /// neutral grey, about 0.53. Their A, B, X and Y keep the font's own
    /// colours.
    pub const LEGEND: [f32; 3] = [1.0, 1.0, 1.0];
    /// "PRESS START", and the small print at the bottom right.
    pub const PRESS_START: [f32; 3] = [0.64, 0.72, 0.87];
    pub const SMALL_PRINT: [f32; 3] = [0.64, 0.72, 0.87];
    /// The default list skin's text.
    pub const LIST_TEXT: [f32; 3] = [0.56, 0.69, 0.81];
    /// The small drifting callouts on sub-screens.
    pub const CALLOUT: [f32; 3] = [0.25, 0.39, 0.54];
    /// The globals' overlay: the navy that dims what is behind a dialog.
    pub const OVERLAY: Rgba = [0.0, 0.08, 0.17, 0.85];

    // The flat look, without the art. Estimates: Halo 2's colours where
    // the art's own are known roughly (the glow bars are blue with alpha
    // up to 86/255, the tracks reach 24/255), our own choice otherwise.

    /// The glow behind a main menu row, at its brightest.
    pub const GLOW: Rgba = [0.2, 0.45, 0.85, 86.0 / 255.0];
    /// The sheen strips' darkening (multiplied).
    pub const SHEEN: Rgba = [0.72, 0.76, 0.84, 1.0];
    /// The track lines, a little stronger than the art's so they show.
    pub const TRACK: Rgba = [0.64, 0.72, 0.87, 0.14];
    pub const BRACE: Rgba = [0.64, 0.72, 0.87, 0.35];
    /// The navy behind the start screen and main menu without the scene,
    /// top to bottom (the overlay's navy, darkened downwards).
    pub const NAVY_TOP: Rgba = [0.03, 0.11, 0.21, 1.0];
    pub const NAVY_BOTTOM: Rgba = [0.0, 0.02, 0.06, 1.0];
    /// One flat grey-navy behind the shell's framing picture (screens
    /// after the main menu), (33, 40, 67). An estimate: Cartographer's
    /// lobby shows a flat (23, 32, 56) above the header band and below
    /// the legend band (the owner's Xbox screenshot about (21, 28, 50)),
    /// where the framing's flat (9, 21, 42) is at alpha 109/255, and
    /// (10, 22, 43) inside it, at 242/255; this is the colour under it
    /// that gives both, drawn straight over (our CPU drawing gives
    /// (23, 32, 56) and (11, 22, 43) with Vista's framing).
    pub const SHELL_BACKDROP: Rgba = [33.0 / 255.0, 40.0 / 255.0, 67.0 / 255.0, 1.0];
    /// The logo's lettering without its picture: a chrome blue, light at
    /// the top.
    pub const CHROME_TOP: Rgba = [0.88, 0.93, 0.98, 1.0];
    pub const CHROME_BOTTOM: Rgba = [0.4, 0.55, 0.75, 1.0];
}

/// A dialog's size: its header and button legend go in its size's box.
pub use blam_cache::ui::DialogSize;

/// A dialog's header box (the globals' bounds by its size).
pub fn header_box(size: DialogSize) -> [f32; 4] {
    HEADER_BOUNDS[size as usize]
}

/// A dialog's button legend box.
pub fn legend_box(size: DialogSize) -> [f32; 4] {
    LEGEND_BOUNDS[size as usize]
}

/// Where a dialog whose pane is at the Xbox's scale has its header, large,
/// half and quarter. The globals' boxes are for Vista's own dialogs, whose
/// frames are drawn 1:1; on the Xbox's frames, drawn at 2.5, they land on
/// the dialog's list or text (menu.md 9.2). These sit in the frame's top
/// band, placed from its visible edges as Vista's dialog_ok and
/// large_error place theirs (about 16 to 18 units under the top, 28 to
/// 39 in from the left), drawn on Cartographer's map. Estimates, for P6's
/// screenshots of the Xbox's dialogs to confirm. Each keeps clear of what
/// is under it: the quarter dialogs' text from 178 down, Player Options'
/// rows from 110 down and its help from x -2 right, the playlists' first
/// row's bracket from 406 down.
pub const XBOX_DIALOG_HEADER_BOUNDS: [[f32; 4]; 3] = [
    [-640.0, 482.0, 55.0, 410.0],
    [-611.0, 244.0, -20.0, 204.0],
    [-319.0, 242.0, 350.0, 202.0],
];

/// The header box of a screen of `size` whose pane is at `scale`, when it
/// isn't the globals' own: a dialog at the Xbox's scale
/// (`XBOX_DIALOG_HEADER_BOUNDS`). A full screen keeps the globals' box at
/// either scale, and so does every legend.
pub fn xbox_dialog_header(size: DialogSize, scale: Scale) -> Option<[f32; 4]> {
    match (size, scale) {
        (DialogSize::Full, _) | (_, Scale::Vista) => None,
        (size, Scale::Xbox) => Some(XBOX_DIALOG_HEADER_BOUNDS[size as usize - 1]),
    }
}

/// The globals' header and button legend boxes, full, large, half and
/// quarter.
pub const HEADER_BOUNDS: [[f32; 4]; 4] = [
    [-730.0, 567.0, 50.0, 520.0],
    [-500.0, 390.0, 55.0, 310.0],
    [-500.0, 158.0, 255.0, 73.0],
    [-252.0, 182.0, 350.0, 142.0],
];
pub const LEGEND_BOUNDS: [[f32; 4]; 4] = [
    [100.0, -500.0, 730.0, -540.0],
    [0.0, -354.0, 535.0, -394.0],
    [-400.0, -225.0, 480.0, -270.0],
    [-175.0, -225.0, 235.0, -265.0],
];
/// The button legends the menus use (`\u{e100}` is A, `\u{e101}` B), by
/// the globals' names for them.
pub const LEGENDS: [(&str, &str); 3] = [
    ("a_select", "\u{e100} SELECT"),
    ("a_select_b_back", "\u{e100} SELECT \u{e101} BACK"),
    ("a_select_b_cancel", "\u{e100} SELECT \u{e101} CANCEL"),
];

/// A controller button in a button legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    A,
    B,
    X,
    Y,
    Lb,
    Rb,
    Start,
    Back,
    White,
    Black,
}

impl Face {
    /// The six with a glyph in Halo 2's fonts, by code point.
    const GLYPHS: [(Face, char); 6] = [
        (Face::A, '\u{e100}'),
        (Face::B, '\u{e101}'),
        (Face::X, '\u{e102}'),
        (Face::Y, '\u{e103}'),
        (Face::Black, '\u{e104}'),
        (Face::White, '\u{e105}'),
    ];

    /// The face a glyph of Halo 2's fonts shows.
    pub fn of_glyph(c: char) -> Option<Face> {
        Face::GLYPHS.iter().find(|g| g.1 == c).map(|g| g.0)
    }

    /// What a legend shows for it after the pad was used: its glyph in
    /// Halo 2's fonts (U+E100 A, U+E101 B, U+E102 X, U+E103 Y, U+E104
    /// Black, U+E105 White; the built-in font draws them as coloured
    /// discs), or its name for the four that have none.
    pub fn glyph(self) -> String {
        match Face::GLYPHS.iter().find(|g| g.0 == self) {
            Some(g) => g.1.to_string(),
            None => match self {
                Face::Lb => "LB",
                Face::Rb => "RB",
                Face::Start => "START",
                _ => "BACK",
            }
            .to_string(),
        }
    }
}

/// The key that does what `face` does in the lobby, as a legend names it
/// after the keyboard was used (Vista's keyboard legends are mangled in
/// the tags, so these are built here): Enter is A and Start, Esc B and
/// Back, X and Y the letters, Q and E the bumpers. White and Black have
/// no key: none.
pub fn key_name(face: Face) -> &'static str {
    match face {
        Face::A | Face::Start => "ENTER",
        Face::B | Face::Back => "ESC",
        Face::X => "X",
        Face::Y => "Y",
        Face::Lb => "Q",
        Face::Rb => "E",
        Face::White | Face::Black => "",
    }
}

/// One button of a legend and what it does ("SELECT"). A `keyless` one
/// shows what it does alone, with no glyph or key name: where its button
/// or key does nothing (a click on it can still press it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegendItem {
    pub face: Face,
    pub label: String,
    pub keyless: bool,
}

impl LegendItem {
    pub fn new(face: Face, label: &str) -> LegendItem {
        LegendItem {
            face,
            label: label.to_string(),
            keyless: false,
        }
    }

    /// `face`'s item shown by its label alone (`keyless`).
    pub fn label_only(face: Face, label: &str) -> LegendItem {
        LegendItem {
            keyless: true,
            ..LegendItem::new(face, label)
        }
    }
}

/// A legend by name, with the keys that do what its buttons do when the
/// keyboard was used last (`key_name`).
pub fn legend(name: &str, keyboard: bool) -> Option<String> {
    let line = LEGENDS.iter().find(|l| l.0 == name)?.1;
    if !keyboard {
        return Some(line.to_string());
    }
    let mut keys = String::new();
    for word in line.split(' ') {
        let mut glyph = word.chars();
        let key = match (glyph.next().and_then(Face::of_glyph), glyph.next()) {
            (Some(face), None) => key_name(face),
            _ => word,
        };
        if !keys.is_empty() {
            keys.push_str(if key != word { "   " } else { " " });
        }
        keys.push_str(key);
    }
    Some(keys)
}

/// The dialogs' boxes without the art: behind their question and
/// answers (our own placement from menu-preview, an estimate), and the
/// question's own (error_dialog_ok_cancel's text).
pub const DIALOG_BOX: [f32; 4] = [-290.0, 200.0, 300.0, -260.0];
pub const QUESTION_BOX: [f32; 4] = [-250.0, 140.0, 240.0, -80.0];
/// The large dialog's box (the pause menu's in menu-preview).
pub const LARGE_DIALOG_BOX: [f32; 4] = [-540.0, 180.0, 560.0, -260.0];
/// Where notes go: on most screens above the legend, on the main menu
/// under its list (our own placements from menu-preview; estimates).
pub const NOTE: [f32; 4] = [-730.0, -380.0, 300.0, -490.0];
pub const MAIN_NOTE: [f32; 4] = [-600.0, -370.0, 600.0, -480.0];

/// Where a screen's list goes: its first item's corner, the step down to
/// each next one, and an item's box from its corner (all UI units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub corner: [f32; 2],
    pub step: f32,
    pub item: [f32; 4],
}

impl Place {
    /// Item `k`'s corner.
    pub fn corner(&self, k: usize) -> [f32; 2] {
        [self.corner[0], self.corner[1] - self.step * k as f32]
    }

    /// Item `k`'s box.
    pub fn item(&self, k: usize) -> [f32; 4] {
        offset(self.item, self.corner(k))
    }
}

/// The lists' places as mainmenu.map's tags have them: the main menu's
/// (its items its skin's text), the game options' (top_level_settings,
/// each item a settings_list_bkd), System Link's (network_squad_browser,
/// game_browser_list's item_background) and the dialogs'
/// (error_dialog_ok_cancel and error_dialog_large, the default skin's
/// item_background). The main menu's step of 50 is inferred.
pub const MAIN_PLACE: Place = Place {
    corner: [-178.0, -80.0],
    step: 50.0,
    item: [-62.0, 10.0, 418.0, -40.0],
};
pub const OPTIONS_PLACE: Place = Place {
    corner: [-500.0, 300.0],
    step: 52.0,
    item: [15.0, 62.0, 1031.0, 10.0],
};
pub const BROWSER_PLACE: Place = Place {
    corner: [-670.0, 412.0],
    step: 32.0,
    item: [8.0, 32.0, 1358.0, 0.0],
};
pub const DIALOG_PLACE: Place = Place {
    corner: [-255.0, -140.0],
    step: 46.0,
    item: [40.0, 38.0, 440.0, -4.0],
};
pub const LARGE_DIALOG_PLACE: Place = Place {
    corner: [-500.0, 40.0],
    step: 46.0,
    item: [40.0, 38.0, 440.0, -4.0],
};
/// menu-preview's own place for the online lists: the wide default skin
/// (default_wide) under the header.
pub const ONLINE_PLACE: Place = Place {
    corner: [-640.0, 380.0],
    step: 46.0,
    item: [40.0, 38.0, 960.0, -4.0],
};

/// The pregame lobby (pregame_lobby) without the art.
pub mod pregame {
    /// "Quick Options:" (gametype_options_format), and how far right the
    /// lines below it reach.
    pub const QUICK_OPTIONS: [f32; 4] = [-675.0, -195.0, -300.0, -235.0];
    pub const LINE_RIGHT: f32 = -50.0;
    /// The game type's and the map's lines (gametype_format,
    /// mapname_format).
    pub const GAME_TYPE_LINE: [f32; 4] = [-580.0, -95.0, -150.0, -135.0];
    pub const MAP_LINE: [f32; 4] = [-580.0, -130.0, -150.0, -170.0];
    /// The first two buttons' text (START GAME, GAME SETUP), their
    /// pictures' offset from it and size.
    pub const BUTTONS: [[f32; 4]; 2] =
        [[-690.0, 480.0, -360.0, 440.0], [-320.0, 480.0, 8.0, 440.0]];
    pub const BUTTON_OFFSET: [f32; 2] = [-15.0, -12.0];
    pub const BUTTON_SIZE: [f32; 2] = [358.0, 64.0];
    /// The map's picture (unknown_map's place and size).
    pub const MAP_PICTURE_AT: [f32; 2] = [-448.0, -30.0];
    pub const MAP_PICTURE_SIZE: [f32; 2] = [440.0, 414.0];
    /// The lobby display's status line (game_cant_start_no_session's
    /// box), and the display's lower edge.
    pub const STATUS: [f32; 4] = [-650.0, 320.0, -30.0, 280.0];
    pub const ABOUT_BOTTOM: f32 = 60.0;
    /// Panels behind the left side and the players, without the art.
    pub const PANEL: [f32; 4] = [-705.0, 350.0, -8.0, -480.0];
    pub const PLAYERS_PANEL: [f32; 4] = [80.0, 560.0, 700.0, 60.0];
    /// The players down the right: from the player count (540) across the
    /// speakers (100) to the download bars (680), a row as far down as the
    /// speakers (39), or up to `ROSTER_STEP` when there are few; the gap
    /// between two columns of them.
    pub const ROSTER: [f32; 4] = [100.0, 540.0, 680.0, 100.0];
    pub const ROSTER_HEAD: f32 = 47.0;
    pub const PLAYER_STEP: f32 = 39.0;
    pub const ROSTER_STEP: f32 = 56.0;
    pub const ROSTER_GAP: f32 = 20.0;
}

/// The System Link browser (network_squad_browser) without the art.
pub mod browser {
    /// The game list's columns (game_browser_list's texts: host, map,
    /// players, status) from an item's corner.
    pub const COLUMNS: [(usize, [f32; 4]); 4] = [
        (0, [66.0, 30.0, 418.0, 0.0]),
        (1, [422.0, 30.0, 608.0, 0.0]),
        (4, [1002.0, 30.0, 1102.0, 0.0]),
        (5, [1107.0, 30.0, 1355.0, 0.0]),
    ];
    /// The column heads: the tag's text name, our label, its box.
    pub const HEADS: [(&str, &str, [f32; 4]); 4] = [
        ("host_gamename", "HOST", [-604.0, 495.0, -250.0, 460.0]),
        ("map_head", "MAP", [-250.0, 495.0, -60.0, 460.0]),
        ("player_head", "PLAYERS", [330.0, 495.0, 435.0, 460.0]),
        ("status_head", "STATUS", [435.0, 495.0, 680.0, 460.0]),
    ];
    /// Where it says there are no games (no_games), its help
    /// (help_create_game), the chosen game's name and about box, and its
    /// map's picture (unknown_map's place, at 0.68).
    pub const NO_GAMES: [f32; 4] = [-300.0, 410.0, 300.0, 370.0];
    pub const HELP: [f32; 4] = [-610.0, -235.0, -200.0, -380.0];
    pub const GAME: [f32; 4] = [-50.0, -180.0, 550.0, -210.0];
    pub const ABOUT: [f32; 4] = [-70.0, -235.0, 390.0, -440.0];
    pub const PICTURE_AT: [f32; 2] = [410.0, -175.0];
    pub const PICTURE_SIZE: [f32; 2] = [299.0, 282.0];
}

/// How a bitmap draws when the art source hasn't its picture: a flat
/// shape in Halo 2's colours, or nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flat {
    None,
    /// A bar this size (UI units), brightest down its middle (a glow bar).
    Glow {
        size: [f32; 2],
        color: Rgba,
    },
    /// A line this size in dashes that scroll as the bitmap would.
    Track {
        size: [f32; 2],
        color: Rgba,
    },
    /// A box this size, multiplied, with a darker band moving across it
    /// as the bitmap would scroll (a sheen strip).
    Sheen {
        size: [f32; 2],
        color: Rgba,
    },
    /// A thin bracket this size (the track brace).
    Brace {
        size: [f32; 2],
        color: Rgba,
    },
    /// A plain panel this size: `color` with an `edge` round it (a
    /// dialog's frame pieces, a button, a list item's backing).
    Panel {
        size: [f32; 2],
        color: Rgba,
        edge: Rgba,
    },
    /// Lettering in a box this size (the logo): `text` in `font`, this
    /// many times its size, shaded from `color` at the top to `bottom`.
    Lettering {
        size: [f32; 2],
        text: &'static str,
        font: Font,
        scale: f32,
        color: Rgba,
        bottom: Rgba,
    },
}

/// A bitmap on a screen or in a list skin, as its tag places it.
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapWidget {
    /// The bitmap tag's name (its whole path, or the last part of it).
    pub name: String,
    /// Which of its sequences (or images, without sequences).
    pub frame: usize,
    /// Its corner: top left on a screen, bottom left in a list skin, from
    /// the screen's origin or the item's corner (UI units).
    pub corner: [f32; 2],
    /// Its scale across and down (0 is 1).
    pub scale: [f32; 2],
    pub blend: Blend,
    /// How fast it scrolls, in widths and heights a second.
    pub wraps_per_second: [f32; 2],
    /// Lower depths are drawn first.
    pub depth: i16,
    /// It doesn't count towards a list item's height.
    pub ignore_for_list_size: bool,
    /// How it comes in when its screen opens, after its delay.
    pub intro: Option<Animation>,
    pub delay_ms: f32,
    /// How it draws without its picture.
    pub flat: Flat,
}

impl BitmapWidget {
    /// A plain bitmap at `corner`, with nothing else set.
    pub fn new(name: &str, corner: [f32; 2]) -> BitmapWidget {
        BitmapWidget {
            name: name.to_string(),
            frame: 0,
            corner,
            scale: [0.0, 0.0],
            blend: Blend::Plain,
            wraps_per_second: [0.0, 0.0],
            depth: 0,
            ignore_for_list_size: false,
            intro: None,
            delay_ms: 0.0,
            flat: Flat::None,
        }
    }

    /// Its scale across and down (0 taken as 1).
    pub fn scale(&self) -> [f32; 2] {
        self.scale.map(|s| if s > 0.0 { s } else { 1.0 })
    }

    /// Its size without its picture (UI units), from its flat shape.
    pub fn flat_size(&self) -> Option<[f32; 2]> {
        match self.flat {
            Flat::None => None,
            Flat::Glow { size, .. }
            | Flat::Track { size, .. }
            | Flat::Sheen { size, .. }
            | Flat::Brace { size, .. }
            | Flat::Panel { size, .. }
            | Flat::Lettering { size, .. } => Some(size),
        }
    }
}

/// A text on a screen or in a list skin.
#[derive(Clone, Debug, PartialEq)]
pub struct TextWidget {
    pub font: Font,
    pub color: [f32; 3],
    /// Its box, from the screen's origin or the item's corner.
    pub bounds: [f32; 4],
    pub justify: Justify,
    pub pulsating: bool,
    pub depth: i16,
    pub intro: Option<Animation>,
    pub delay_ms: f32,
    /// Typed out behind a cursor (an editable text, the tag's text flag
    /// 8): the Xbox draws no drop shadow under it (`paint::TEXT_SHADOW`).
    pub cursor: bool,
}

impl TextWidget {
    pub fn new(font: Font, color: [f32; 3], bounds: [f32; 4], justify: Justify) -> TextWidget {
        TextWidget {
            font,
            color,
            bounds,
            justify,
            pulsating: false,
            depth: 0,
            intro: None,
            delay_ms: 0.0,
            cursor: false,
        }
    }
}

/// A list skin: what each item draws, from its corner, and how its items
/// light up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinLayout {
    pub bitmaps: Vec<BitmapWidget>,
    /// Its texts: the row's label in each (a shadow is a second text,
    /// listed after the label and drawn under it).
    pub texts: Vec<TextWidget>,
    pub items: ItemLook,
    /// Its scroll arrows, when it has them.
    pub arrows: Option<Arrows>,
}

/// A list skin's scroll arrows: one bitmap, its first frame pointing up
/// and its second down (the default skin's are so), each with its bottom
/// left this far from the list's corner (UI units, as the skin's own
/// bitmaps).
#[derive(Clone, Debug, PartialEq)]
pub struct Arrows {
    pub bitmap: String,
    pub up: [f32; 2],
    pub down: [f32; 2],
}

/// A list on a screen.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLayout {
    pub skin: SkinLayout,
    /// The first item's corner, the step down to the next, and how many
    /// items show at once.
    pub corner: [f32; 2],
    pub step: f32,
    pub visible: usize,
    /// Going down past the last item comes back to the first.
    pub wraps: bool,
    pub intro: Option<Animation>,
    pub delay_ms: f32,
}

impl ListLayout {
    /// Item `k`'s corner.
    pub fn corner(&self, k: usize) -> [f32; 2] {
        [self.corner[0], self.corner[1] - self.step * k as f32]
    }
}

/// Which units a pane's numbers are in. Vista redid most of mainmenu.map
/// in its own UI units (1:1); the screens it never showed (most of the
/// online ones, a few dialogs) kept the Xbox's numbers, which are its
/// 640 by 480 frame's pixels from the middle, drawn with pictures made
/// for that frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scale {
    #[default]
    Vista,
    Xbox,
}

/// UI units to an Xbox pixel (`FRAME` is 640 by 480 of them), for an Xbox
/// pane's places and sizes; and how much bigger its pictures are drawn
/// than their own scale says (the pictures Vista and MCC have are twice
/// the Xbox's size: 2.5 / 2). Both are checked against the owner's
/// screenshot of the Xbox's pregame lobby and three bitmaps only. An Xbox
/// pane's text is drawn `XBOX_BITMAP` times bigger too (Vista's fonts are
/// about twice the Xbox's, as its pictures are): the same body font
/// measured on the Xbox screenshot (a 415-pixel JPEG) and on
/// Cartographer's lobby gives about 1.15 to 1.27, so 1.25 is an estimate
/// within that.
pub const XBOX_POSITION: f32 = 2.5;
pub const XBOX_BITMAP: f32 = 1.25;

impl Scale {
    /// How much a pane's places and boxes grow.
    pub fn position(self) -> f32 {
        match self {
            Scale::Vista => 1.0,
            Scale::Xbox => XBOX_POSITION,
        }
    }

    /// How much a pane's pictures (and its texts' glyphs) grow beyond
    /// their own scale.
    pub fn bitmap(self) -> f32 {
        match self {
            Scale::Vista => 1.0,
            Scale::Xbox => XBOX_BITMAP,
        }
    }

    /// Its name in `H2LOBBY_SCALE` and menu_png's arguments.
    pub fn name(self) -> &'static str {
        match self {
            Scale::Vista => "vista",
            Scale::Xbox => "xbox",
        }
    }

    pub fn parse(s: &str) -> Option<Scale> {
        match s.trim().to_ascii_lowercase().as_str() {
            "vista" | "1" | "1:1" => Some(Scale::Vista),
            "xbox" | "2.5" => Some(Scale::Xbox),
            _ => None,
        }
    }

    /// A point at this scale in UI units.
    pub fn point(self, [x, y]: [f32; 2]) -> [f32; 2] {
        let k = self.position();
        [x * k, y * k]
    }

    /// A box at this scale in UI units.
    pub fn rect(self, r: [f32; 4]) -> [f32; 4] {
        let k = self.position();
        r.map(|v| v * k)
    }
}

/// A text slot of a screen: the string id its tag names (its `key`;
/// `#<n>` for the `n`th text of its pane when it names none), the words
/// the screen's string list has for it, and where and how it is drawn.
/// Which words it shows is the screen's binding's choice
/// (`screens::Words`). Its glyphs are drawn `glyphs` times its font's
/// size, and wrap at that size: its pane's `Scale::bitmap` (1.25 on an
/// Xbox pane).
#[derive(Clone, Debug, PartialEq)]
pub struct SlotText {
    pub key: String,
    pub words: Option<String>,
    pub widget: TextWidget,
    pub glyphs: f32,
}

/// A button of a screen (the pregame lobby's START GAME): its text slot,
/// and the picture behind it, whose corner is the text box's top left
/// moved by the tag's offset (its y counted downwards).
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonLayout {
    pub text: SlotText,
    pub bitmap: Option<BitmapWidget>,
}

impl ButtonLayout {
    /// Its text's string id (`SlotText::key`).
    pub fn key(&self) -> &str {
        &self.text.key
    }
}

/// A block of players (the lobby's roster): the first row's corner, how
/// many it holds in how many rows and columns, the step down to the next
/// row and across to the next column (UI units), and the player skin each
/// row is drawn with (its tag's name; player skins are drawn in 2.10).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayersLayout {
    pub corner: [f32; 2],
    pub max: usize,
    pub rows: usize,
    pub columns: usize,
    /// Down from one row to the next (the tags give it negative, as +y is
    /// up; this is the distance).
    pub row_height: f32,
    pub column_width: f32,
    pub skin: Option<String>,
}

impl PlayersLayout {
    /// Player `k`'s corner, down each column then across.
    pub fn corner(&self, k: usize) -> [f32; 2] {
        let rows = self.rows.max(1);
        let (column, row) = (k / rows, k % rows);
        [
            self.corner[0] + self.column_width * column as f32,
            self.corner[1] - self.row_height * row as f32,
        ]
    }
}

/// One pane of a screen (a screen shows one at a time; a pane-per-item
/// screen shows the one for its list's focused item), in UI units at its
/// `scale`'s size.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneLayout {
    pub scale: Scale,
    pub bitmaps: Vec<BitmapWidget>,
    pub texts: Vec<SlotText>,
    pub lists: Vec<ListLayout>,
    pub buttons: Vec<ButtonLayout>,
    pub players: Vec<PlayersLayout>,
}

impl PaneLayout {
    /// The same pane at another scale: every place and box moved out or
    /// in, its pictures and texts' glyphs grown or shrunk (its lists'
    /// skins stay as they are, as at either scale).
    pub fn at_scale(&self, to: Scale) -> PaneLayout {
        let from = self.scale;
        let k = to.position() / from.position();
        let pk = to.bitmap() / from.bitmap();
        let point = |[x, y]: [f32; 2]| [x * k, y * k];
        let picture = |w: &BitmapWidget| {
            let mut w = w.clone();
            w.corner = point(w.corner);
            if pk != 1.0 {
                w.scale = w.scale().map(|s| s * pk);
                if let Flat::Panel { size, color, edge } = w.flat {
                    w.flat = Flat::Panel {
                        size: size.map(|v| v * pk),
                        color,
                        edge,
                    };
                }
            }
            w
        };
        let text = |t: &SlotText| {
            let mut t = t.clone();
            t.widget.bounds = t.widget.bounds.map(|v| v * k);
            t.glyphs *= pk;
            t
        };
        PaneLayout {
            scale: to,
            bitmaps: self.bitmaps.iter().map(picture).collect(),
            texts: self.texts.iter().map(text).collect(),
            lists: self
                .lists
                .iter()
                .map(|l| ListLayout {
                    corner: point(l.corner),
                    ..l.clone()
                })
                .collect(),
            buttons: self
                .buttons
                .iter()
                .map(|b| ButtonLayout {
                    text: text(&b.text),
                    bitmap: b.bitmap.as_ref().map(picture),
                })
                .collect(),
            players: self
                .players
                .iter()
                .map(|e| PlayersLayout {
                    corner: point(e.corner),
                    row_height: e.row_height * k,
                    column_width: e.column_width * k,
                    ..e.clone()
                })
                .collect(),
        }
    }
}

/// Any screen of mainmenu.map from its tag (`tags::screen_layout`): its
/// size (where its header and legend go), its header's string id and
/// words, its legend (the globals' `button_key` number; 0 none), whether
/// it dims what is under it, whether its panes go with its list's items,
/// its panes, and its string list.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenLayout {
    pub id: Option<crate::catalog::ScreenId>,
    pub path: String,
    pub size: DialogSize,
    pub has_header: bool,
    pub header_id: String,
    pub header_words: Option<String>,
    pub button_key: u16,
    /// Shown over the screen under it, dimmed by the overlay (flag 0x20
    /// clear).
    pub overlay: bool,
    /// Its panes follow its list's focus (flag 2).
    pub pane_per_item: bool,
    pub panes: Vec<PaneLayout>,
    /// Its string list: string id name, English words.
    pub strings: Vec<(String, String)>,
}

impl ScreenLayout {
    /// The pane at `k`, or the last one (none when it has none).
    pub fn pane(&self, k: usize) -> Option<&PaneLayout> {
        self.panes.get(k).or(self.panes.last())
    }

    /// The words of a string id in its string list.
    pub fn string(&self, key: &str) -> Option<&str> {
        self.strings
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, w)| w.as_str())
    }
}

/// The start screen's logo (1,024 by 128), top left at (-511, 90), which
/// fades in over 250 ms (the globals' animation 29).
pub const LOGO: &str = "ui\\screens\\game_shell\\start_screen\\start_screen";
pub const LOGO_CORNER: [f32; 2] = [-511.0, 90.0];
pub const LOGO_SIZE: [f32; 2] = [1024.0, 128.0];
/// The navy framing behind sub-screens (2048 by 1303), and its place
/// (game_shell_background's first bitmap).
pub const FRAMING: &str = "ui\\global_bitmaps\\framing_center";
pub const FRAMING_CORNER: [f32; 2] = [-1070.0, 654.0];
pub const FRAMING_SCALE: f32 = 1.08;
/// The main menu's list skin (11).
pub const MAIN_MENU_SKIN: &str = "ui\\list_skins\\main_menu\\main_menu";

/// The track lines of the start screen and main menu: their bitmap, top
/// edge and how fast they scroll (wraps a second). Each starts at x -1,200
/// at scale 1.2.
pub const TRACKS: [(&str, f32, f32); 11] = [
    ("track2b", 120.0, 0.010),
    ("track4", 128.0, 0.030),
    ("track6", -40.0, 0.050),
    ("track7c", -44.0, 0.080),
    ("track7d", 124.0, 0.020),
    ("track5", -370.0, 0.100),
    ("track7c", -376.0, 0.050),
    ("track_timeline2a", -510.0, 0.010),
    ("track_timeline2b", -500.0, 0.060),
    ("track7c", 500.0, 0.035),
    ("track5", 550.0, 0.080),
];
pub const TRACK_X: f32 = -1200.0;
pub const TRACK_SCALE: f32 = 1.2;
/// A track's size without its picture: its bitmap is 2,048 wide (2,458
/// at its scale); how tall its line is, is an estimate.
pub const TRACK_SIZE: [f32; 2] = [2458.0, 3.0];
pub const TIMELINE_SIZE: [f32; 2] = [2458.0, 8.0];
/// The track brace (its frame 1) and its corner; its size without its
/// picture is an estimate.
pub const BRACE: &str = "track_brace";
pub const BRACE_CORNER: [f32; 2] = [300.0, -483.0];
pub const BRACE_SIZE: [f32; 2] = [200.0, 24.0];
/// "PRESS START", centred in its box and pulsating, in the title font.
pub const PRESS_START_BOX: [f32; 4] = [-400.0, -65.0, 400.0, -105.0];
/// The small print at the bottom right of both screens (left justified).
pub const SMALL_PRINT_BOX: [f32; 4] = [376.0, -562.0, 610.0, -600.0];
/// How far below its label a main menu row's shadow is.
pub const SHADOW_DROP: f32 = 5.0;

/// The pieces both the start screen and the main menu have: the logo, the
/// tracks and the brace.
pub fn shell_art() -> Vec<BitmapWidget> {
    let mut art: Vec<BitmapWidget> = TRACKS
        .iter()
        .map(|&(name, y, wraps)| {
            let size = if name.contains("timeline") {
                TIMELINE_SIZE
            } else {
                TRACK_SIZE
            };
            BitmapWidget {
                scale: [TRACK_SCALE; 2],
                wraps_per_second: [wraps, 0.0],
                depth: 1,
                flat: Flat::Track {
                    size,
                    color: colour::TRACK,
                },
                ..BitmapWidget::new(name, [TRACK_X, y])
            }
        })
        .collect();
    art.push(BitmapWidget {
        frame: 1,
        depth: 1,
        flat: Flat::Brace {
            size: BRACE_SIZE,
            color: colour::BRACE,
        },
        ..BitmapWidget::new(BRACE, BRACE_CORNER)
    });
    // Drawn over the tracks (their depth is the tag's; the logo's is an
    // estimate).
    art.push(BitmapWidget {
        depth: 3,
        intro: Some(Animation::screen_fade()),
        flat: Flat::Lettering {
            size: LOGO_SIZE,
            text: "HALO 2",
            font: Font::SuperLarge,
            // Our own choice, to fill the logo's box.
            scale: 2.6,
            color: colour::CHROME_TOP,
            bottom: colour::CHROME_BOTTOM,
        },
        ..BitmapWidget::new(LOGO, LOGO_CORNER)
    });
    art
}

/// The navy framing, for a still background (option C).
pub fn framing() -> BitmapWidget {
    BitmapWidget {
        scale: [FRAMING_SCALE; 2],
        ..BitmapWidget::new(FRAMING, FRAMING_CORNER)
    }
}

/// The start screen's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct StartLayout {
    pub art: Vec<BitmapWidget>,
    pub press_start: TextWidget,
    pub small_print: TextWidget,
    /// The framing over a still background (`screens::Backdrop::Still`).
    pub framing: BitmapWidget,
}

impl Default for StartLayout {
    /// The numbers of mainmenu.map's start_screen.
    fn default() -> StartLayout {
        StartLayout {
            art: shell_art(),
            press_start: TextWidget {
                pulsating: true,
                depth: 12,
                intro: Some(Animation::screen_fade()),
                ..TextWidget::new(
                    Font::Title,
                    colour::PRESS_START,
                    PRESS_START_BOX,
                    Justify::Center,
                )
            },
            small_print: small_print(),
            framing: framing(),
        }
    }
}

/// The small print at the bottom right.
fn small_print() -> TextWidget {
    TextWidget {
        depth: 12,
        ..TextWidget::new(
            Font::SplitHudMessage,
            colour::SMALL_PRINT,
            SMALL_PRINT_BOX,
            Justify::Left,
        )
    }
}

/// The main menu's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct MainMenuLayout {
    pub art: Vec<BitmapWidget>,
    pub list: ListLayout,
    pub small_print: TextWidget,
    /// The framing over a still background (`screens::Backdrop::Still`).
    pub framing: BitmapWidget,
}

/// Skin 11's bitmaps: the glow bar (480 by 62), the rings at its right,
/// and two sheen strips multiplied over it, scrolling opposite ways. The
/// rings' and strips' sizes without their pictures are estimates (the
/// strips split the bar at the right strip's corner).
pub fn main_menu_skin() -> SkinLayout {
    let glow = BitmapWidget {
        flat: Flat::Glow {
            size: [480.0, 62.0],
            color: colour::GLOW,
        },
        ..BitmapWidget::new("list_bkd", [-60.0, -40.0])
    };
    let rings = BitmapWidget {
        ignore_for_list_size: true,
        ..BitmapWidget::new("list_bkd_rings_right", [-54.0, -40.0])
    };
    let sheen = |name: &str, corner: [f32; 2], wraps: f32, width: f32| BitmapWidget {
        blend: Blend::Multiply,
        wraps_per_second: [wraps, 0.0],
        depth: 2,
        ignore_for_list_size: true,
        flat: Flat::Sheen {
            size: [width, 62.0],
            color: colour::SHEEN,
        },
        ..BitmapWidget::new(name, corner)
    };
    let label = TextWidget {
        depth: 12,
        ..TextWidget::new(
            Font::MainMenu,
            colour::MAIN_MENU,
            MAIN_PLACE.item,
            Justify::Center,
        )
    };
    let shadow = TextWidget {
        color: colour::MAIN_MENU_SHADOW,
        bounds: offset(MAIN_PLACE.item, [0.0, -SHADOW_DROP]),
        ..label.clone()
    };
    SkinLayout {
        bitmaps: vec![
            glow,
            rings,
            sheen("list_bkd_multiply", [150.0, -40.0], 0.2, 270.0),
            sheen("list_bkd_multiply_left", [-60.0, -40.0], -0.2, 210.0),
        ],
        texts: vec![label, shadow],
        items: ItemLook::default(),
        arrows: None,
    }
}

impl Default for MainMenuLayout {
    /// The numbers of mainmenu.map's main_menu. How its list comes in
    /// (animation 34) isn't read yet: a 250 ms fade is an estimate.
    fn default() -> MainMenuLayout {
        MainMenuLayout {
            art: shell_art(),
            list: ListLayout {
                skin: main_menu_skin(),
                corner: MAIN_PLACE.corner,
                step: MAIN_PLACE.step,
                visible: 6,
                wraps: true,
                intro: Some(Animation::screen_fade()),
                delay_ms: 0.0,
            },
            small_print: small_print(),
            framing: framing(),
        }
    }
}

/// What every screen after the main menu sits in (menu.md 1.3): the
/// framing and track strips behind it (`game_shell_background`), the
/// header at its top left and the button legend at its bottom right, by
/// the screen's size (`DialogSize`), and the overlay that dims what is
/// behind a dialog. Drawn by `chrome`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChromeLayout {
    /// The pictures that don't move, kept and copied in under each frame
    /// (`chrome::still`), and the ones that scroll, drawn every frame
    /// (`chrome::moving`).
    pub still: Vec<BitmapWidget>,
    pub moving: Vec<BitmapWidget>,
    /// By `DialogSize`: the header's box, font and colour, and the
    /// legend's.
    pub headers: [TextWidget; 4],
    pub legends: [TextWidget; 4],
    pub overlay: Rgba,
}

impl ChromeLayout {
    /// The header of a screen of `size`.
    pub fn header(&self, size: DialogSize) -> &TextWidget {
        &self.headers[size as usize]
    }

    /// The button legend of a screen of `size`.
    pub fn legend(&self, size: DialogSize) -> &TextWidget {
        &self.legends[size as usize]
    }
}

impl Default for ChromeLayout {
    /// Without the tags: the framing alone (it draws nothing without its
    /// picture, leaving the navy), the globals' boxes, the header in the
    /// title font and the globals' text colour, the legend in the body
    /// font and its own colour (`colour::LEGEND`), and the globals'
    /// overlay.
    fn default() -> ChromeLayout {
        let text = |bounds: [f32; 4], font: Font, color, justify: Justify| {
            TextWidget::new(font, color, bounds, justify)
        };
        let (header, legend) = (colour::HEADER, colour::LEGEND);
        ChromeLayout {
            still: vec![framing()],
            moving: Vec::new(),
            headers: HEADER_BOUNDS.map(|b| text(b, Font::Title, header, Justify::Left)),
            legends: LEGEND_BOUNDS.map(|b| text(b, Font::Body, legend, Justify::Right)),
            overlay: colour::OVERLAY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn ui_space_fills_a_16_9_window_top_to_bottom() {
        let s = Space::new(1920, 1080);
        assert_eq!(s.k, 0.9);
        assert_eq!(s.at([0.0, 0.0]), [960.0, 540.0]);
        assert_eq!(s.at([0.0, 600.0]), [960.0, 0.0]);
        assert_eq!(s.at([0.0, -600.0]), [960.0, 1080.0]);
        // The 16:9 safe area reaches the window's sides.
        let [x0, _, x1, _] = s.rect(SAFE_16_9);
        assert!(x0.abs() < 0.5 && (x1 - 1920.0).abs() < 0.5);
        assert_eq!(
            s.rect([-100.0, 50.0, 100.0, -50.0]),
            [870.0, 495.0, 1050.0, 585.0]
        );
        // At 1280x720, 0.6 pixels a unit.
        let s = Space::new(1280, 720);
        assert_eq!(s.k, 0.6);
        assert!(near(s.at([-511.0, 90.0]), [333.4, 306.0]));
    }

    #[test]
    fn a_4_3_window_shows_the_xbox_frame_exactly() {
        let s = Space::new(1024, 768);
        assert_eq!(s.k, 0.64);
        assert_eq!(s.rect(FRAME), [0.0, 0.0, 1024.0, 768.0]);
        assert!(near(s.unit([0.0, 0.0]), [-800.0, 600.0]));
        // Narrower than 4:3: the frame's width fills it, with room above
        // and below.
        let s = Space::new(800, 1000);
        assert_eq!(s.k, 0.5);
        let [x0, y0, x1, y1] = s.rect(FRAME);
        assert_eq!([x0, x1], [0.0, 800.0]);
        assert_eq!([y0, y1], [200.0, 800.0]);
    }

    #[test]
    fn a_21_9_window_shows_more_at_the_sides() {
        let s = Space::new(2560, 1080);
        assert_eq!(s.k, 0.9);
        assert_eq!(s.rect(FRAME), [560.0, 0.0, 2000.0, 1080.0]);
        let [l, t, r, b] = s.shown();
        assert!((l + 1422.22).abs() < 0.01 && (r - 1422.22).abs() < 0.01);
        assert!((t - 600.0).abs() < 1e-3 && (b + 600.0).abs() < 1e-3);
        // Pixels and units go both ways.
        assert!(near(s.unit(s.at([123.0, -45.0])), [123.0, -45.0]));
        // A window with no size maps everything to its corner.
        let s = Space::new(0, 0);
        assert_eq!(s.at([500.0, 500.0]), [0.0, 0.0]);
    }

    #[test]
    fn boxes_and_places() {
        assert_eq!(
            offset([1.0, 2.0, 3.0, 0.0], [10.0, -1.0]),
            [11.0, 1.0, 13.0, -1.0]
        );
        assert!(contains([0.0, 10.0, 10.0, 0.0], [5.0, 5.0]));
        assert!(!contains([0.0, 10.0, 10.0, 0.0], [5.0, -1.0]));
        assert_eq!(MAIN_PLACE.corner(2), [-178.0, -180.0]);
        assert_eq!(MAIN_PLACE.item(1), [-240.0, -120.0, 240.0, -170.0]);
        // The main menu's rows are centred on the screen.
        let [l, _, r, _] = MAIN_PLACE.item(0);
        assert_eq!(l + r, 0.0);
        assert_eq!(
            header_box(DialogSize::Quarter),
            [-252.0, 182.0, 350.0, 142.0]
        );
        assert_eq!(legend_box(DialogSize::Full), [100.0, -500.0, 730.0, -540.0]);
    }

    #[test]
    fn legends_name_the_keys_for_the_keyboard() {
        assert_eq!(
            legend("a_select_b_back", false).unwrap(),
            "\u{e100} SELECT \u{e101} BACK"
        );
        assert_eq!(
            legend("a_select_b_back", true).unwrap(),
            "ENTER SELECT   ESC BACK"
        );
        assert_eq!(legend("a_select", true).unwrap(), "ENTER SELECT");
        assert!(legend("nope", false).is_none());
        // Every face's key, and its glyph or name for the pad.
        let keys = [
            (Face::A, "ENTER", "\u{e100}"),
            (Face::B, "ESC", "\u{e101}"),
            (Face::X, "X", "\u{e102}"),
            (Face::Y, "Y", "\u{e103}"),
            (Face::Lb, "Q", "LB"),
            (Face::Rb, "E", "RB"),
            (Face::Start, "ENTER", "START"),
            (Face::Back, "ESC", "BACK"),
            (Face::White, "", "\u{e105}"),
            (Face::Black, "", "\u{e104}"),
        ];
        for (face, key, glyph) in keys {
            assert_eq!((key_name(face), face.glyph().as_str()), (key, glyph));
        }
        assert_eq!(Face::of_glyph('\u{e103}'), Some(Face::Y));
        assert_eq!(Face::of_glyph('Y'), None);
    }

    #[test]
    fn the_chromes_default_is_the_globals_boxes() {
        let c = ChromeLayout::default();
        assert_eq!(c.still, [framing()]);
        assert!(c.moving.is_empty());
        for size in [
            DialogSize::Full,
            DialogSize::Large,
            DialogSize::Half,
            DialogSize::Quarter,
        ] {
            let (h, l) = (c.header(size), c.legend(size));
            assert_eq!(h.bounds, header_box(size));
            assert_eq!((h.font, h.justify), (Font::Title, Justify::Left));
            assert_eq!(l.bounds, legend_box(size));
            assert_eq!((l.font, l.justify), (Font::Body, Justify::Right));
            assert_eq!((h.color, l.color), (colour::HEADER, colour::LEGEND));
        }
        assert_eq!(c.overlay, colour::OVERLAY);
    }

    #[test]
    fn the_fallback_layouts_hold_halo_2s_numbers() {
        let start = StartLayout::default();
        let logo = start.art.iter().find(|b| b.name == LOGO).unwrap();
        assert_eq!(logo.corner, [-511.0, 90.0]);
        // Centred across.
        assert_eq!(logo.corner[0] * 2.0 + LOGO_SIZE[0], 2.0);
        assert_eq!(logo.intro.as_ref().unwrap().period_ms, 250.0);
        assert_eq!(
            start
                .art
                .iter()
                .filter(|b| b.name.starts_with("track"))
                .count(),
            12
        );
        let brace = start.art.iter().find(|b| b.name == BRACE).unwrap();
        assert_eq!((brace.frame, brace.corner), (1, [300.0, -483.0]));
        assert!(start.press_start.pulsating);
        let main = MainMenuLayout::default();
        assert_eq!(main.list.corner, [-178.0, -80.0]);
        assert_eq!(main.list.corner(1), [-178.0, -130.0]);
        assert_eq!(main.list.skin.texts[0].color, [0.62, 0.74, 0.84]);
        assert_eq!(main.list.skin.texts[1].bounds[1], 5.0);
        let multiplied = main
            .list
            .skin
            .bitmaps
            .iter()
            .filter(|b| b.blend == Blend::Multiply)
            .map(|b| b.wraps_per_second[0])
            .collect::<Vec<_>>();
        assert_eq!(multiplied, [0.2, -0.2]);
        assert_eq!(framing().scale(), [1.08, 1.08]);
        assert_eq!(BitmapWidget::new("x", [0.0, 0.0]).scale(), [1.0, 1.0]);
    }
}
