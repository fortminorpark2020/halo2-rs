//! Draws the start screen, the main menu and the shell of the screens
//! after it (`shell-*.png`: the chrome with the header PREGAME LOBBY and
//! the legend A SELECT, B BACK) on the CPU and writes them as PNGs, at
//! 1920x1080 and 1280x720, printing how long a frame takes: the still
//! background is drawn once (its time printed apart) and copied in under
//! each frame, as a caller keeps it (`cpu::Kept`).
//!
//! ```text
//! cargo run --release -p h2ui --example menu_png -- <out dir> [<fonts dir>]
//!     [--mcc <MCC mainmenu.map>] [--vista <Vista mainmenu.map>]
//! ```
//!
//! Each screen is drawn in the flat look (the built-in numbers, no art),
//! and with made-up stand-in pictures of the real art's sizes (gradients
//! drawn here, no game data), so the time of the textured path can be
//! measured too. With `--mcc` or `--vista`, each is also drawn as that
//! mainmenu.map's UI tags and pictures give it (`h2ui::shell`), and where
//! each piece came from is printed first.
//!
//! Text is in Halo 2's fonts from the fonts folder given, else from the
//! one beside the map (MCC's `halo2\h2_fonts`, Vista's `maps\fonts`),
//! else in the built-in font. Pictures drawn with the real art or fonts
//! hold the game's pictures and glyphs: they stay on your PC.
//!
//! Any other screen (`h2ui::catalog`, or any wgit by its whole tag name)
//! is drawn instead with `--screen` or `--all`, from `--mcc` or `--vista`:
//!
//! ```text
//! menu_png <out dir> --vista <mainmenu.map> --screen <id or path> [--pane <n>]
//!     [--words tag|ids|hide] [--scale xbox|vista] [--focus <n>] [--size <w>x<h>]
//! menu_png <out dir> --mcc <mainmenu.map> --all [--words ...]
//! ```
//!
//! Each pane is drawn in the map's look and the flat look, settled, over
//! the shell (and the overlay, for a dialog), to
//! `<id>-<pane>-<look>-<w>x<h>.png`. `--words tag` shows the tags' words
//! through `screens::wording` (format tokens hide their text), `ids` each
//! slot's string id instead (what a lobby binding names), `hide` none.
//! For each it prints the quads, how much of the window they cover, the
//! CPU time of a frame over 30 draws, and for a dialog which scale puts
//! its header box in the top quarter of its frame pieces, and the box its
//! header is drawn in.
//!
//! A bitmap's frames are written instead with `--frames` (several may be
//! given; give one map, as both write the same file names):
//!
//! ```text
//! menu_png <out dir> --vista <mainmenu.map> --frames <bitmap>
//! ```
//!
//! It prints the bitmap's images and sequences and writes each frame, over
//! a mid grey, to `frame-<last name>-<k>.png`.

use h2ui::anim::Focus;
use h2ui::art::{self, ArtImage, ArtSource, Bitmap, FlatArt, MemoryArt, Sequence};
use h2ui::catalog::{Override, ScreenId};
use h2ui::chrome;
use h2ui::cpu::Kept;
use h2ui::layout::{
    self, ChromeLayout, DialogSize, Face, LegendItem, MainMenuLayout, PaneLayout, Scale,
    ScreenLayout, Space, StartLayout,
};
use h2ui::paint::{DrawList, Painter, Resources};
use h2ui::screens::{self, Backdrop, ListState, MainMenu, Row, ScreenState, Show, Start, Words};
use h2ui::shell::Shell;
use h2ui::tags;
use h2ui::text::Fonts;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Frames timed per screen.
const FRAMES: u32 = 30;
/// When the screens are drawn (seconds on the menus' clock): settled, and
/// a while in, so the art has scrolled.
const CLOCK: f64 = 7.3;
const USAGE: &str = "usage: menu_png <out dir> [<fonts dir>] [--mcc <MCC mainmenu.map>] \
                     [--vista <Vista mainmenu.map>] \
                     [--screen <id or path> | --all | --frames <bitmap>] \
                     [--pane <n>] [--words tag|ids|hide] [--scale xbox|vista] [--focus <n>] \
                     [--size <w>x<h>]";

/// What a screen's text slots show (`--words`).
#[derive(Clone, Copy, PartialEq)]
enum WordsArg {
    Tag,
    Ids,
    Hide,
}

/// What to draw.
struct Args {
    out: PathBuf,
    fonts: Option<PathBuf>,
    mcc: Option<PathBuf>,
    vista: Option<PathBuf>,
    /// `--screen` (several may be given), or every catalog screen.
    screens: Vec<String>,
    /// `--frames <bitmap>`: print a bitmap's images and sequences, and
    /// write each of its frames (several may be given).
    frames: Vec<String>,
    all: bool,
    pane: Option<usize>,
    words: WordsArg,
    scale: Option<Scale>,
    focus: usize,
    sizes: Vec<(u32, u32)>,
}

fn args() -> Result<Args, String> {
    let mut list = std::env::args_os().skip(1);
    let mut plain = Vec::new();
    let (mut mcc, mut vista) = (None, None);
    let mut screens = Vec::new();
    let mut frames = Vec::new();
    let (mut all, mut pane, mut words, mut scale, mut focus) =
        (false, None, WordsArg::Tag, None, 0);
    let mut sizes = Vec::new();
    while let Some(a) = list.next() {
        let flag = a.to_str().unwrap_or("");
        if !flag.starts_with("--") {
            plain.push(PathBuf::from(a));
            continue;
        }
        if flag == "--all" {
            all = true;
            continue;
        }
        let value = list
            .next()
            .and_then(|v| v.into_string().ok())
            .ok_or_else(|| format!("{flag} needs a value"))?;
        let number = |v: &str| v.parse::<usize>().map_err(|_| format!("{flag} {v}?"));
        match flag {
            "--mcc" => mcc = Some(PathBuf::from(value)),
            "--vista" => vista = Some(PathBuf::from(value)),
            "--screen" => screens.push(value),
            "--frames" => frames.push(value),
            "--pane" => pane = Some(number(&value)?),
            "--focus" => focus = number(&value)?,
            "--words" => {
                words = match value.as_str() {
                    "tag" => WordsArg::Tag,
                    "ids" => WordsArg::Ids,
                    "hide" => WordsArg::Hide,
                    _ => return Err(format!("--words {value}?")),
                }
            }
            "--scale" => {
                scale = Some(Scale::parse(&value).ok_or_else(|| format!("--scale {value}?"))?)
            }
            "--size" => {
                let (w, h) = value.split_once('x').ok_or("--size <w>x<h>")?;
                let (w, h) = (number(w)?, number(h)?);
                sizes.push((w as u32, h as u32));
            }
            _ => return Err(format!("unknown {flag}")),
        }
    }
    let mut plain = plain.into_iter();
    let out = plain.next().ok_or("no out dir")?;
    let fonts = plain.next();
    if plain.next().is_some() {
        return Err("too many folders".into());
    }
    if (all || !screens.is_empty() || !frames.is_empty()) && mcc.is_none() && vista.is_none() {
        return Err("--screen, --all and --frames need --mcc or --vista".into());
    }
    if sizes.is_empty() {
        sizes = vec![(1920, 1080), (1280, 720)];
    }
    Ok(Args {
        out,
        fonts,
        mcc,
        vista,
        screens,
        frames,
        all,
        pane,
        words,
        scale,
        focus,
        sizes,
    })
}

/// The fonts folder beside a mainmenu.map: MCC's `halo2\h2_fonts` (the
/// maps are in `halo2\h2_maps_win64_dx11`) or Vista's `maps\fonts`.
fn fonts_beside(map: &Path) -> Option<PathBuf> {
    let dir = map.parent()?;
    [
        dir.parent().map(|d| d.join("h2_fonts")),
        Some(dir.join("fonts")),
    ]
    .into_iter()
    .flatten()
    .find(|d| d.is_dir())
}

/// One way to draw the screens: its name, layouts and pictures.
struct Look<'a> {
    name: &'static str,
    start: &'a StartLayout,
    main: &'a MainMenuLayout,
    chrome: &'a ChromeLayout,
    art: &'a dyn ArtSource,
}

fn main() {
    let args = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let out = &args.out;
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("{}: {e}", out.display());
        std::process::exit(1);
    }
    let fonts_dir = args.fonts.clone().or_else(|| {
        [&args.mcc, &args.vista]
            .into_iter()
            .flatten()
            .find_map(|m| fonts_beside(m))
    });
    let fonts = match fonts_dir {
        Some(dir) => {
            let f = Fonts::read(&dir);
            println!("Halo 2 fonts from {}: {:?}", dir.display(), f.loaded());
            f
        }
        None => Fonts::fallback(),
    };
    let log = |line: &str| eprintln!("{line}");
    if !args.frames.is_empty() {
        for map in [&args.mcc, &args.vista].into_iter().flatten() {
            for name in &args.frames {
                if let Err(e) = write_frames(map, name, out) {
                    eprintln!("{name}: {e}");
                }
            }
        }
        return;
    }
    if args.all || !args.screens.is_empty() {
        draw_screens(&args, &fonts);
        return;
    }
    let mcc = args.mcc.as_deref().map(|m| Shell::open(m, &log));
    let vista = args.vista.as_deref().map(|m| Shell::open(m, &log));
    let standin = standin_art();
    let start_layout = StartLayout::default();
    let main_layout = MainMenuLayout::default();
    let chrome_layout = ChromeLayout::default();
    let mut looks = vec![
        Look {
            name: "flat",
            start: &start_layout,
            main: &main_layout,
            chrome: &chrome_layout,
            art: &FlatArt,
        },
        Look {
            name: "standin",
            start: &start_layout,
            main: &main_layout,
            chrome: &chrome_layout,
            art: &standin,
        },
    ];
    for (name, shell) in [("mcc", &mcc), ("vista", &vista)] {
        if let Some(s) = shell {
            looks.push(Look {
                name,
                start: &s.start,
                main: &s.main,
                chrome: &s.chrome,
                art: s.art(),
            });
        }
    }
    let legend = [
        LegendItem::new(Face::A, "SELECT"),
        LegendItem::new(Face::B, "BACK"),
    ];
    let rows = ["ONLINE", "SPLIT SCREEN", "SETTINGS"];
    for (w, h) in [(1920u32, 1080u32), (1280, 720)] {
        let space = Space::new(w, h);
        for look in &looks {
            let art = look.art;
            let start = |clock: f64| {
                let s = Start {
                    layout: look.start,
                    opened: 0.0,
                    text: screens::PRESS_START,
                    small_print: "",
                };
                screens::start(clock, &s, art, &fonts, space)
            };
            let main = |clock: f64| {
                let m = MainMenu {
                    layout: look.main,
                    opened: 0.0,
                    rows: &rows,
                    focus: Focus::on(0, 0.0),
                    small_print: "PLAYER ONE",
                };
                screens::main_menu(clock, &m, art, &fonts, space)
            };
            let c = look.chrome;
            let shell = |clock: f64| {
                let mut list = chrome::moving(clock, c, art, &fonts, space);
                let mut p = Painter::new(space, art, &fonts, clock);
                chrome::header(&mut p, c, DialogSize::Full, "PREGAME LOBBY");
                chrome::legend(&mut p, c, DialogSize::Full, &legend, false);
                let over = p.finish();
                list.quads.extend(over.quads);
                list.texts.extend(over.texts);
                list
            };
            let res = Resources { art, fonts: &fonts };
            let framing = &look.main.framing;
            let background = |clock: f64| {
                screens::background(clock, Backdrop::Still, framing, art, &fonts, space)
            };
            let behind_shell = |clock: f64| chrome::still(clock, c, art, &fonts, space);
            let look = look.name;
            for (name, build, behind) in [
                (
                    "start",
                    &start as &dyn Fn(f64) -> DrawList,
                    &background as &dyn Fn(f64) -> DrawList,
                ),
                ("main", &main, &background),
                ("shell", &shell, &behind_shell),
            ] {
                let file = out.join(format!("{name}-{look}-{w}x{h}.png"));
                let t = time(behind, build, w, h, &res);
                if let Err(e) = save(&file, &t.px, w, h) {
                    eprintln!("{}: {e}", file.display());
                    std::process::exit(1);
                }
                println!(
                    "{name:5} {look:7} {w}x{h}: {:3} quads, build {:.3} ms, draw {:.2} ms a \
                     frame (background once: {:.2} ms) -> {}",
                    t.quads,
                    t.build_ms,
                    t.draw_ms,
                    t.background_ms,
                    file.display()
                );
            }
        }
    }
}

/// A bitmap's images and sequences, printed, and each frame a widget can
/// show (`frame-<last name>-<k>.png`), as `blam_cache::ui::Picture::frame`
/// cuts it.
fn write_frames(map: &Path, name: &str, out: &Path) -> Result<(), String> {
    let map = if map.is_dir() {
        map.join("mainmenu.map")
    } else {
        map.to_path_buf()
    };
    let mut m = blam_cache::ui::Menus::open(&map).map_err(|e| e.to_string())?;
    let picture = m.pictures.picture(name).map_err(|e| e.to_string())?;
    for (k, i) in picture.images.iter().enumerate() {
        println!("{name}: image {k}: {}x{}", i.width, i.height);
    }
    for (k, s) in picture.sequences.iter().enumerate() {
        let sprites: Vec<String> = s
            .sprites
            .iter()
            .map(|p| {
                format!(
                    "{}:[{:.3},{:.3},{:.3},{:.3}]",
                    p.bitmap, p.left, p.top, p.right, p.bottom
                )
            })
            .collect();
        println!(
            "{name}: sequence {k}: first image {}, {} images, {} sprites {}",
            s.first_bitmap,
            s.bitmap_count,
            s.sprites.len(),
            sprites.join(" ")
        );
    }
    let frames = picture.sequences.len().max(picture.images.len());
    for k in 0..frames {
        let Some(frame) = picture.frame(k as i16) else {
            continue;
        };
        let Some(img) = picture.crop(frame) else {
            continue;
        };
        let px: Vec<u32> = img
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                // Over a mid grey, so clear parts show.
                let a = u32::from(c[3]);
                let mix = |v: u8| (u32::from(v) * a + 96 * (255 - a)) / 255;
                (mix(c[0]) << 16) | (mix(c[1]) << 8) | mix(c[2])
            })
            .collect();
        let file = out.join(format!("frame-{}-{k}.png", tags_last(name)));
        save(&file, &px, img.width, img.height)?;
        println!(
            "{name}: frame {k}: {}x{} -> {}",
            img.width,
            img.height,
            file.display()
        );
    }
    Ok(())
}

/// The frame pieces of Halo 2's dialogs, by their tags' last names.
const FRAME_PIECES: [&str; 4] = ["ul_07", "br_07", "dialog1", "dialog2"];

/// A screen to draw: its name for the files, its layout, and its own
/// pictures when it isn't a catalog screen (the shell's art has those).
struct Target {
    name: String,
    layout: ScreenLayout,
    art: Option<MemoryArt>,
}

/// `--screen`'s and `--all`'s screens, from each map given.
fn draw_screens(args: &Args, fonts: &Fonts) {
    let log = |line: &str| eprintln!("{line}");
    let overrides: Vec<Override> = match args.scale {
        Some(scale) => ScreenId::ALL
            .into_iter()
            .map(|id| Override {
                id,
                pane: None,
                scale,
            })
            .collect(),
        None => Vec::new(),
    };
    for (look, map) in [("mcc", &args.mcc), ("vista", &args.vista)] {
        let Some(map) = map else { continue };
        let opened = Instant::now();
        let shell = Shell::open_with(map, &log, &overrides);
        println!(
            "{look}: opened in {} ms, {:.1} MB of pictures decoded, {} catalog screens",
            opened.elapsed().as_millis(),
            shell.art.bytes() as f64 / (1024.0 * 1024.0),
            shell.screens.len()
        );
        let mut targets: Vec<Target> = Vec::new();
        if args.all {
            for id in ScreenId::ALL {
                match shell.screen(id) {
                    Some(l) => targets.push(Target {
                        name: id.name().to_string(),
                        layout: l.clone(),
                        art: None,
                    }),
                    None => println!("{look}: {} not found", id.name()),
                }
            }
        }
        for want in &args.screens {
            if let Some(id) = ScreenId::find(want) {
                match shell.screen(id) {
                    Some(l) => targets.push(Target {
                        name: id.name().to_string(),
                        layout: l.clone(),
                        art: None,
                    }),
                    None => println!("{look}: {} not found", id.name()),
                }
                continue;
            }
            // Any other screen by its whole tag name, with its pictures.
            match screen_by_path(map, want, args.scale.unwrap_or_default()) {
                Ok((layout, art)) => targets.push(Target {
                    name: tags_last(want).to_string(),
                    layout,
                    art: Some(art),
                }),
                Err(e) => println!("{look}: {want}: {e}"),
            }
        }
        for t in &targets {
            let art: &dyn ArtSource = match &t.art {
                Some(own) => own,
                None => shell.art(),
            };
            draw_target(args, fonts, &shell.chrome, t, look, art);
        }
    }
}

fn tags_last(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// A screen that isn't in the catalog, laid out from the map's tags with
/// its own pictures.
fn screen_by_path(
    map: &Path,
    path: &str,
    scale: Scale,
) -> Result<(ScreenLayout, MemoryArt), String> {
    let map = if map.is_dir() {
        map.join("mainmenu.map")
    } else {
        map.to_path_buf()
    };
    let mut m = blam_cache::ui::Menus::open(&map).map_err(|e| e.to_string())?;
    let ui = m.ui.as_ref().map_err(|e| e.to_string())?;
    let mut notes = Vec::new();
    let first = tags::screen_layout_at(ui, path, &[scale], &FlatArt, &mut notes)?;
    let wants = tags::screen_wanted(&first);
    let loaded = art::read_some(
        m.pictures.as_mut(),
        &wants.needed,
        &wants.extra,
        &wants.every,
    );
    let layout = tags::screen_layout_at(ui, path, &[scale], &loaded.art, &mut notes)?;
    for n in notes {
        eprintln!("{n}");
    }
    Ok((layout, loaded.art))
}

/// Made-up rows for a list: enough to scroll, a cell for each of its
/// skin's texts.
fn rows(list: usize, texts: usize, visible: usize, ids: bool) -> Vec<Row<'static>> {
    (0..visible + 3)
        .map(|r| Row {
            cells: (0..texts.max(1))
                .map(|c| {
                    Cow::Owned(match (ids, c) {
                        (true, _) => format!("list{list} row{r} cell{c}"),
                        (false, 0) => format!("ROW {}", r + 1),
                        (false, _) => format!("{}", (r + 1) * 10 + c),
                    })
                })
                .collect(),
            frames: Vec::new(),
            enabled: r != 2,
        })
        .collect()
}

/// The box covered by a pane's dialog frame pieces at their pictures'
/// sizes (or flat shapes'), UI units.
fn frame_box(pane: &PaneLayout, art: &dyn ArtSource) -> Option<[f32; 4]> {
    pane.bitmaps
        .iter()
        .filter(|w| {
            FRAME_PIECES
                .iter()
                .any(|f| tags_last(&w.name).eq_ignore_ascii_case(f))
        })
        .filter_map(|w| {
            let [sx, sy] = w.scale();
            let size = art
                .picture(&w.name, w.frame)
                .map(|p| [p.size[0] * sx, p.size[1] * sy])
                .or_else(|| w.flat_size())?;
            let [x, y] = w.corner;
            Some([x, y, x + size[0], y - size[1]])
        })
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].max(b[1]),
                a[2].max(b[2]),
                a[3].min(b[3]),
            ]
        })
}

/// Whether a header box sits in the top quarter of a frame's box: its
/// middle across inside the frame, and its middle down in the frame's top
/// quarter.
fn header_fits(header: [f32; 4], frame: [f32; 4]) -> bool {
    let [l, t, r, b] = frame;
    let (x, y) = ((header[0] + header[2]) * 0.5, (header[1] + header[3]) * 0.5);
    x > l && x < r && y <= t && y >= t - (t - b) * 0.25
}

/// Every pane of a screen (or `--pane`'s), in the map's look and the flat
/// look, at each size, timed.
fn draw_target(
    args: &Args,
    fonts: &Fonts,
    chrome_layout: &ChromeLayout,
    t: &Target,
    look: &str,
    art: &dyn ArtSource,
) {
    let layout = &t.layout;
    let panes: Vec<usize> = match args.pane {
        Some(p) => vec![p],
        None => (0..layout.panes.len().max(1)).collect(),
    };
    let legend = [
        LegendItem::new(Face::A, "SELECT"),
        LegendItem::new(Face::B, "BACK"),
    ];
    let ids = args.words == WordsArg::Ids;
    for pane_k in panes {
        let Some(pane) = layout.pane(pane_k) else {
            println!("{look} {}: no panes", t.name);
            continue;
        };
        let scales: Vec<&str> = layout.panes.iter().map(|p| p.scale.name()).collect();
        if layout.size != DialogSize::Full {
            // The header-fit check, at both scales: the globals' header
            // box (Vista's units) against the frame pieces; then the box
            // the header is drawn in (`screens::header_widget`: ours on a
            // dialog at the Xbox's scale).
            for scale in [Scale::Vista, Scale::Xbox] {
                let at = pane.at_scale(scale);
                let Some(frame) = frame_box(&at, art) else {
                    continue;
                };
                let plain = chrome_layout.header(layout.size).bounds;
                let scaled = scale.rect(plain);
                println!(
                    "{look} {}#{pane_k} {:?} at {}: frame {:?}, header {:?} {}, scaled \
                     with it {:?} {}",
                    t.name,
                    layout.size,
                    scale.name(),
                    frame.map(f32::round),
                    plain,
                    if header_fits(plain, frame) {
                        "fits"
                    } else {
                        "outside"
                    },
                    scaled,
                    if header_fits(scaled, frame) {
                        "fits"
                    } else {
                        "outside"
                    },
                );
            }
            let drawn = screens::header_widget(layout, pane_k, chrome_layout).bounds;
            println!(
                "{look} {}#{pane_k} header drawn in {drawn:?} ({} pane)",
                t.name,
                pane.scale.name()
            );
        }
        let made: Vec<Vec<Row>> = pane
            .lists
            .iter()
            .enumerate()
            .map(|(k, l)| rows(k, l.skin.texts.len(), l.visible, ids))
            .collect();
        let has_list = !pane.lists.is_empty();
        let lists: Vec<ListState> = made
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let focus = (k == 0).then(|| Focus::on(args.focus.min(r.len() - 1), 0.0));
                let visible = pane.lists[k].visible;
                ListState {
                    rows: r,
                    focus,
                    first: screens::first_shown(r.len(), visible, args.focus),
                }
            })
            .collect();
        let words = |s: &h2ui::layout::SlotText| match args.words {
            WordsArg::Tag => Words::Tag,
            WordsArg::Ids => Words::Ours(Cow::Owned(s.key.clone())),
            WordsArg::Hide => Words::Hide,
        };
        let tokens = |_: char| None;
        let pictures = |_: &h2ui::layout::BitmapWidget| Show::Keep;
        let header = match args.words {
            WordsArg::Tag => Words::Tag,
            WordsArg::Ids => Words::Ours(Cow::Owned(layout.header_id.clone())),
            WordsArg::Hide => Words::Hide,
        };
        let state = ScreenState {
            pane: pane_k,
            opened: 0.0,
            chrome: chrome_layout,
            words: &words,
            tokens: &tokens,
            lists: &lists,
            button_focus: (!has_list).then_some(args.focus),
            disabled_buttons: &[],
            hidden_buttons: &[],
            header,
            legend: &legend,
            keyboard: false,
            pictures: &pictures,
        };
        let settled = screens::intro_end(layout, pane_k) + 0.5;
        let dialog = layout.overlay && layout.size != DialogSize::Full;
        for (name, art) in [(look, art), ("flat", &FlatArt as &dyn ArtSource)] {
            for &(w, h) in &args.sizes {
                let space = Space::new(w, h);
                let build = |clock: f64| {
                    let mut list = chrome::moving(clock, chrome_layout, art, fonts, space);
                    if dialog {
                        let over = screens::overlay(chrome_layout, space);
                        list.quads.extend(over.quads);
                    }
                    let (screen, _) = screens::screen(clock, layout, &state, art, fonts, space);
                    list.quads.extend(screen.quads);
                    list.texts.extend(screen.texts);
                    list
                };
                let behind = |clock: f64| chrome::still(clock, chrome_layout, art, fonts, space);
                let res = Resources { art, fonts };
                let timing = time_at(settled, &behind, &build, w, h, &res);
                let (screen, hits) = screens::screen(settled, layout, &state, art, fonts, space);
                let share = screen.fill_area() / (f64::from(w) * f64::from(h));
                let file = args
                    .out
                    .join(format!("{}-{pane_k}-{name}-{w}x{h}.png", t.name));
                if let Err(e) = save(&file, &timing.px, w, h) {
                    eprintln!("{}: {e}", file.display());
                    std::process::exit(1);
                }
                println!(
                    "{:16} pane {pane_k} ({}) {name:5} {w}x{h}: {:4} quads, covers {:.2} of \
                     the window, {} hit areas, build {:.3} ms, draw {:.2} ms a frame \
                     (shell once: {:.2} ms) -> {}",
                    t.name,
                    scales.join("/"),
                    screen.quads.len(),
                    share,
                    hits.len(),
                    timing.build_ms,
                    timing.draw_ms,
                    timing.background_ms,
                    file.display()
                );
            }
        }
    }
}

/// What `time` measured.
struct Timing {
    /// The last frame.
    px: Vec<u32>,
    /// A frame's average time to build its lists, and to draw them (the
    /// kept background copied in, then the screen), in ms.
    build_ms: f64,
    draw_ms: f64,
    /// Drawing the background the once (ms).
    background_ms: f64,
    /// The screen's quads.
    quads: usize,
}

/// Draws `build`'s screen `FRAMES` times over `background`, kept (their
/// clock moving on as at 60 frames a second).
fn time(
    background: &dyn Fn(f64) -> DrawList,
    build: &dyn Fn(f64) -> DrawList,
    w: u32,
    h: u32,
    res: &Resources,
) -> Timing {
    time_at(CLOCK, background, build, w, h, res)
}

/// As `time`, from `start` on the menus' clock.
fn time_at(
    start: f64,
    background: &dyn Fn(f64) -> DrawList,
    build: &dyn Fn(f64) -> DrawList,
    w: u32,
    h: u32,
    res: &Resources,
) -> Timing {
    let (w, h) = (w as usize, h as usize);
    let mut px = vec![0u32; w * h];
    let mut kept = Kept::default();
    let t = Instant::now();
    kept.draw(&background(start), &mut px, w, h, res);
    let background_ms = t.elapsed().as_secs_f64() * 1000.0;
    let (mut building, mut drawing, mut quads) = (0.0, 0.0, 0);
    for k in 0..FRAMES {
        let clock = start + f64::from(k) / 60.0;
        let t = Instant::now();
        let behind = background(clock);
        let list = build(clock);
        building += t.elapsed().as_secs_f64();
        let t = Instant::now();
        kept.draw(&behind, &mut px, w, h, res);
        h2ui::cpu::draw_u32(&list, &mut px, w, h, res);
        drawing += t.elapsed().as_secs_f64();
        quads = list.quads.len();
    }
    let n = f64::from(FRAMES);
    Timing {
        px,
        build_ms: building * 1000.0 / n,
        draw_ms: drawing * 1000.0 / n,
        background_ms,
        quads,
    }
}

fn save(path: &Path, px: &[u32], w: u32, h: u32) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let rgb: Vec<u8> = px
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    writer.write_image_data(&rgb).map_err(|e| e.to_string())
}

/// A `w` by `h` image whose pixel at (x, y), as shares of its width and
/// height, is `f`'s colour.
fn image(w: usize, h: usize, f: impl Fn(f32, f32) -> [f32; 4]) -> ArtImage {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let c = f((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            rgba.extend(c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    ArtImage {
        width: w,
        height: h,
        rgba,
    }
}

/// Stand-ins for the start screen's and main menu's pictures, at the real
/// ones' sizes where they're known (an estimate otherwise), drawn here:
/// soft bars, dashes and gradients, nothing of the game's.
fn standin_art() -> MemoryArt {
    let mut art = MemoryArt::new();
    // Soft edges: 1 inside, fading over the last `edge` of each side.
    let soft = |t: f32, edge: f32| (t.min(1.0 - t) / edge).clamp(0.0, 1.0);
    art.add_image(
        layout::LOGO,
        image(1024, 128, |x, y| {
            let a = soft(x * 4.0 % 1.0, 0.08) * soft(y, 0.2) * soft(x, 0.02);
            let shade = 1.0 - 0.5 * y;
            [0.8 * shade, 0.88 * shade, 1.0 * shade, a]
        }),
    );
    for (name, _, _) in layout::TRACKS {
        let tall = if name.contains("timeline") { 8 } else { 4 };
        art.add_image(
            name,
            image(2048, tall, |x, _| {
                let dash = if (x * 37.0).fract() < 0.6 {
                    24.0 / 255.0
                } else {
                    0.0
                };
                [0.7, 0.8, 0.95, dash]
            }),
        );
    }
    let brace = |w: usize, h: usize| {
        image(w, h, |x, y| {
            let on = y < 0.15 || x < 0.05 || x > 0.95;
            [0.7, 0.8, 0.95, if on { 0.5 } else { 0.0 }]
        })
    };
    art.add(
        layout::BRACE,
        Bitmap {
            images: vec![brace(160, 24), brace(200, 24)],
            sequences: vec![
                Sequence {
                    first_image: 0,
                    sprites: Vec::new(),
                },
                Sequence {
                    first_image: 1,
                    sprites: Vec::new(),
                },
            ],
        },
    );
    art.add_image(
        "list_bkd",
        image(480, 62, |x, y| {
            [0.25, 0.5, 0.9, 86.0 / 255.0 * soft(x, 0.15) * soft(y, 0.4)]
        }),
    );
    // Its corner is near the bar's, so it is as wide as the bar, with a
    // ring towards its right end.
    art.add_image(
        "list_bkd_rings_right",
        image(480, 62, |x, y| {
            let r = (((x - 0.86) * 480.0 / 62.0).powi(2) + (y - 0.5).powi(2)).sqrt();
            let ring = (1.0 - ((r - 0.3).abs() / 0.04)).clamp(0.0, 1.0);
            [0.6, 0.75, 1.0, 0.5 * ring]
        }),
    );
    for (name, w) in [("list_bkd_multiply", 270), ("list_bkd_multiply_left", 210)] {
        art.add_image(
            name,
            image(w, 62, |x, _| {
                let v = 0.75 + 0.25 * (x * std::f32::consts::TAU).cos();
                [v, v, v, 1.0]
            }),
        );
    }
    art.add_image(
        layout::FRAMING,
        image(2048, 1303, |x, y| {
            let hole = 1.0 - soft(x, 0.3) * soft(y, 0.3);
            [0.0, 0.07, 0.16, 0.4 + 0.6 * hole]
        }),
    );
    art
}
