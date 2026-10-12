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

/// A legend by name, with the keys that do what its buttons do when the
/// keyboard was used last (Vista's keyboard legends are mangled in the
/// tags, so these are built here).
pub fn legend(name: &str, keyboard: bool) -> Option<String> {
    let line = LEGENDS.iter().find(|l| l.0 == name)?.1;
    if !keyboard {
        return Some(line.to_string());
    }
    let mut keys = String::new();
    for word in line.split(' ') {
        let key = match word {
            "\u{e100}" => "ENTER",
            "\u{e101}" => "ESC",
            w => w,
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
