//! Draws the start screen and the main menu on the CPU and writes them as
//! PNGs, at 1920x1080 and 1280x720, printing how long a frame takes: the
//! still background is drawn once (its time printed apart) and copied in
//! under each frame, as a caller keeps it (`cpu::Kept`).
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

use h2ui::anim::Focus;
use h2ui::art::{ArtImage, ArtSource, Bitmap, FlatArt, MemoryArt, Sequence};
use h2ui::cpu::Kept;
use h2ui::layout::{self, MainMenuLayout, Space, StartLayout};
use h2ui::paint::{DrawList, Resources};
use h2ui::screens::{self, Backdrop, MainMenu, Start};
use h2ui::shell::Shell;
use h2ui::text::Fonts;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Frames timed per screen.
const FRAMES: u32 = 30;
/// When the screens are drawn (seconds on the menus' clock): settled, and
/// a while in, so the art has scrolled.
const CLOCK: f64 = 7.3;
const USAGE: &str = "usage: menu_png <out dir> [<fonts dir>] [--mcc <MCC mainmenu.map>] \
                     [--vista <Vista mainmenu.map>]";

/// What to draw.
struct Args {
    out: PathBuf,
    fonts: Option<PathBuf>,
    mcc: Option<PathBuf>,
    vista: Option<PathBuf>,
}

fn args() -> Result<Args, String> {
    let mut list = std::env::args_os().skip(1);
    let mut plain = Vec::new();
    let (mut mcc, mut vista) = (None, None);
    while let Some(a) = list.next() {
        let slot = match a.to_str() {
            Some("--mcc") => &mut mcc,
            Some("--vista") => &mut vista,
            Some(flag) if flag.starts_with("--") => return Err(format!("unknown {flag}")),
            _ => {
                plain.push(PathBuf::from(a));
                continue;
            }
        };
        let path = list.next().ok_or_else(|| format!("{a:?} needs a map"))?;
        *slot = Some(PathBuf::from(path));
    }
    let mut plain = plain.into_iter();
    let out = plain.next().ok_or("no out dir")?;
    let fonts = plain.next();
    if plain.next().is_some() {
        return Err("too many folders".into());
    }
    Ok(Args {
        out,
        fonts,
        mcc,
        vista,
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

/// One way to draw both screens: its name, layouts and pictures.
struct Look<'a> {
    name: &'static str,
    start: &'a StartLayout,
    main: &'a MainMenuLayout,
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
    let mcc = args.mcc.as_deref().map(|m| Shell::open(m, &log));
    let vista = args.vista.as_deref().map(|m| Shell::open(m, &log));
    let standin = standin_art();
    let start_layout = StartLayout::default();
    let main_layout = MainMenuLayout::default();
    let mut looks = vec![
        Look {
            name: "flat",
            start: &start_layout,
            main: &main_layout,
            art: &FlatArt,
        },
        Look {
            name: "standin",
            start: &start_layout,
            main: &main_layout,
            art: &standin,
        },
    ];
    for (name, shell) in [("mcc", &mcc), ("vista", &vista)] {
        if let Some(s) = shell {
            looks.push(Look {
                name,
                start: &s.start,
                main: &s.main,
                art: s.art(),
            });
        }
    }
    let rows = ["XBOX LIVE", "SPLIT SCREEN", "SETTINGS"];
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
            let res = Resources { art, fonts: &fonts };
            let framing = &look.main.framing;
            let background = |clock: f64| {
                screens::background(clock, Backdrop::Still, framing, art, &fonts, space)
            };
            let look = look.name;
            for (name, build) in [
                ("start", &start as &dyn Fn(f64) -> DrawList),
                ("main", &main),
            ] {
                let file = out.join(format!("{name}-{look}-{w}x{h}.png"));
                let t = time(&background, build, w, h, &res);
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
    let (w, h) = (w as usize, h as usize);
    let mut px = vec![0u32; w * h];
    let mut kept = Kept::default();
    let t = Instant::now();
    kept.draw(&background(CLOCK), &mut px, w, h, res);
    let background_ms = t.elapsed().as_secs_f64() * 1000.0;
    let (mut building, mut drawing, mut quads) = (0.0, 0.0, 0);
    for k in 0..FRAMES {
        let clock = CLOCK + f64::from(k) / 60.0;
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
