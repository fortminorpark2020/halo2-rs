//! The start screen and the main menu as the player's own mainmenu.map
//! gives them: MCC's (cache format 13, with the shared.map and
//! textures.dat beside it) or Halo 2 Vista's, read with
//! `blam_cache::ui`. The layouts come from its UI tags (`tags`) and the
//! pictures from its bitmaps (`art::read`); whatever can't be read keeps
//! the built-in numbers and flat shapes, so opening one never fails.
//! Where each piece came from is logged once, when it is opened, as the
//! lobby logs where its rank icons came from.

use crate::art::{self, last_part, ArtSource, FlatArt, MemoryArt};
use crate::layout::{MainMenuLayout, StartLayout};
use crate::tags;
use blam_cache::ui::{Menus, Pictures, Ui};
use std::path::{Path, PathBuf};

/// The start screen's and main menu's layouts and pictures.
#[derive(Debug, Default)]
pub struct Shell {
    pub start: StartLayout,
    pub main: MainMenuLayout,
    pub art: MemoryArt,
    /// Where each piece came from, a line each (what `open` logged).
    pub sources: Vec<String>,
}

/// The map a path names: itself, or a folder's mainmenu.map.
fn map_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("mainmenu.map")
    } else {
        path.to_path_buf()
    }
}

impl Shell {
    /// From the mainmenu.map at `path` (a folder means its mainmenu.map),
    /// logging where each piece came from. What can't be read keeps the
    /// built-in numbers and flat shapes.
    pub fn open(path: &Path, log: &dyn Fn(&str)) -> Shell {
        let path = map_path(path);
        let shell = match Menus::open(&path) {
            Ok(mut m) => {
                let kind = match m.mcc {
                    true => "MCC's, cache format 13",
                    false => "Halo 2 Vista's",
                };
                let mut notes = vec![format!("reading {} ({kind})", path.display())];
                notes.append(&mut m.notes);
                if let Err(e) = &m.ui {
                    notes.push(format!("its UI tags didn't read: {e}"));
                }
                let mut shell = Shell::from_tags(m.ui.as_ref().ok(), Some(m.pictures.as_mut()));
                let mut sources: Vec<String> =
                    notes.into_iter().map(|n| format!("menus: {n}")).collect();
                sources.append(&mut shell.sources);
                shell.sources = sources;
                shell
            }
            Err(e) => {
                let mut shell = Shell::from_tags(None, None);
                let why = format!("menus: {}: {e}", path.display());
                shell.sources.insert(0, why);
                shell
            }
        };
        for line in &shell.sources {
            log(line);
        }
        shell
    }

    /// From UI tags already read (`ui`) and the pictures of the same map,
    /// either of which may be missing. Nothing is logged; `sources` says
    /// where each piece came from.
    pub fn from_tags(ui: Option<&Ui>, pictures: Option<&mut dyn Pictures>) -> Shell {
        // The bitmaps the layouts draw, then their pictures, then the
        // layouts again with the pictures' sizes (a list's step).
        let mut scratch = Vec::new();
        let start = tags::start_layout(ui, &mut scratch);
        let mut main = tags::main_menu_layout(ui, &FlatArt, &mut scratch);
        main.framing = tags::framing(ui, &mut scratch);
        let wanted = tags::wanted(&start, &main);
        let mut found = Vec::new();
        let art = match pictures {
            Some(p) => {
                let loaded = art::read(p, &wanted);
                note_pictures(&loaded, &mut found);
                loaded.art
            }
            None => {
                found.push("menus: no pictures: flat shapes".to_string());
                MemoryArt::new()
            }
        };
        let mut sources = Vec::new();
        let mut start = tags::start_layout(ui, &mut sources);
        let mut main = tags::main_menu_layout(ui, &art, &mut sources);
        let framing = tags::framing(ui, &mut sources);
        start.framing = framing.clone();
        main.framing = framing;
        sources.append(&mut found);
        Shell {
            start,
            main,
            art,
            sources,
        }
    }

    /// The pictures, as an art source.
    pub fn art(&self) -> &dyn ArtSource {
        &self.art
    }
}

/// A line saying how many pictures were found, then one for each reason
/// some weren't, with the bitmaps it kept out (drawn flat).
fn note_pictures(loaded: &art::Loaded, sources: &mut Vec<String>) {
    sources.push(format!(
        "menus: pictures for {} of {} bitmaps",
        loaded.found, loaded.wanted
    ));
    let mut reasons: Vec<(String, Vec<&str>)> = Vec::new();
    for (name, why) in &loaded.missing {
        // A reason naming its bitmap is the same reason as for the next.
        let why = why.replace(&format!(" {name}"), "");
        let name = last_part(name);
        match reasons.iter_mut().find(|r| r.0 == why) {
            Some(r) if !r.1.contains(&name) => r.1.push(name),
            Some(_) => {}
            None => reasons.push((why, vec![name])),
        }
    }
    for (why, names) in reasons {
        sources.push(format!("menus: drawn flat ({why}): {}", names.join(", ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::Focus;
    use crate::layout::Space;
    use crate::paint::{Resources, Texture};
    use crate::screens::{self, MainMenu};
    use crate::text::Fonts;
    use blam_cache::bitmap::Format;
    use blam_cache::mcc;
    use blam_cache::mcc::synthetic::{MapBuilder, Meta, Struct, TexturesDat};
    use std::cell::RefCell;

    // Field places in the UI tags (Assembly's Halo 2 plugins, as
    // `blam_cache::ui` reads them).
    const TAG_REF: usize = 8;
    const WGTZ_SCREENS: usize = 0x8;
    const WIGL_SIZE: usize = 0x1C4;
    const WIGL_ANIMATIONS: usize = 0x120;
    const WIGL_SKINS: usize = 0x138;
    const ANIMATION_SIZE: usize = 0x2C;
    const KEYFRAME_SIZE: usize = 0x14;
    const WGIT_SIZE: usize = 0x70;
    const WGIT_PANES: usize = 0x20;
    const PANE_SIZE: usize = 0x4C;
    const PANE_LISTS: usize = 0xC;
    const PANE_TEXTS: usize = 0x1C;
    const PANE_BITMAPS: usize = 0x24;
    const LIST_SIZE: usize = 0x18;
    const TEXT_SIZE: usize = 0x2C;
    const BITMAP_SIZE: usize = 0x38;
    const SKIN_SIZE: usize = 0x3C;
    const SKIN_ITEM_ANIMATIONS: usize = 0x14;
    const ITEM_ANIMATION_SIZE: usize = 0x10;
    const SKIN_TEXTS: usize = 0x1C;
    /// The made-up logo's colour (red, green, blue).
    const LOGO_RGB: [u8; 3] = [240, 160, 32];

    fn keyframe(alpha: f32) -> Struct {
        Struct::new(KEYFRAME_SIZE).f32(4, alpha)
    }

    /// A text block in `font`, its box top, left, bottom, right.
    fn text(font: i16, [t, l, b, r]: [i16; 4]) -> Struct {
        Struct::new(TEXT_SIZE)
            .i16(0xA, font)
            .f32(0x10, 0.62)
            .f32(0x14, 0.74)
            .f32(0x18, 0.84)
            .i16(0x1C, t)
            .i16(0x1E, l)
            .i16(0x20, b)
            .i16(0x22, r)
    }

    /// A tiny format-13 mainmenu.map: a main menu whose logo (4 by 4, in
    /// textures.dat) is at (-300, 250) at twice its size and fades in
    /// with the globals' animation 1, whose list is in skin 0 at (-100,
    /// -50) with 4 rows showing, and whose skin fades its rows in over
    /// 300 ms. It has no start screen and no shell background.
    fn mainmenu(dat: &mut TexturesDat) -> MapBuilder {
        let bgra: Vec<u8> = [LOGO_RGB[2], LOGO_RGB[1], LOGO_RGB[0], 255].repeat(16);
        let logo = dat.push_record(4, 4, Format::A8R8G8B8, &bgra, 1, mcc::RecordLayout::Single);
        // Animations are numbered from 1 in the tags (0 is none).
        let logo_widget = Struct::new(BITMAP_SIZE)
            .i16(4, 2)
            .i16(0xC, -300)
            .i16(0xE, 250)
            .tag_ref(0x18, "bitm", crate::layout::LOGO)
            .f32(0x30, 2.0)
            .f32(0x34, 2.0);
        let list = Struct::new(LIST_SIZE)
            .u32(0, 1)
            .i16(4, 0)
            .i16(6, 4)
            .i16(8, -100)
            .i16(0xA, -50);
        let pane = Struct::new(PANE_SIZE)
            .block(PANE_LISTS, vec![list])
            .block(PANE_TEXTS, vec![text(5, [-560, 300, -600, 600])])
            .block(PANE_BITMAPS, vec![logo_widget]);
        let main_menu = Struct::new(WGIT_SIZE)
            .i16(4, 6)
            .tag_ref(0x18, "unic", "")
            .block(WGIT_PANES, vec![pane]);
        let fade = |ms: u32, from: f32, to: f32| {
            Struct::new(ITEM_ANIMATION_SIZE)
                .u32(4, ms)
                .block(8, vec![keyframe(from), keyframe(to)])
        };
        let skin = Struct::new(SKIN_SIZE)
            .tag_ref(4, "bitm", "")
            .block(
                SKIN_ITEM_ANIMATIONS,
                vec![fade(300, 0.25, 1.0), fade(100, 1.0, 0.25)],
            )
            .block(SKIN_TEXTS, vec![text(10, [10, -62, -40, 418])]);
        let screen_fade = Struct::new(ANIMATION_SIZE)
            .u32(4, 500)
            .block(8, vec![keyframe(0.0), keyframe(1.0)]);
        let mut globals = Struct::new(WIGL_SIZE)
            .block(
                WIGL_ANIMATIONS,
                vec![Struct::new(ANIMATION_SIZE), screen_fade],
            )
            .block(
                WIGL_SKINS,
                vec![Struct::new(TAG_REF).tag_ref(0, "skin", crate::layout::MAIN_MENU_SKIN)],
            );
        // Every other tag reference in the globals names nothing.
        for o in [0x90, 0x98, 0xA0, 0xA8, 0xB0, 0x140, 0x1B8] {
            globals = globals.tag_ref(o, "snd!", "");
        }
        let wgtz = Struct::new(0x20)
            .tag_ref(0, "wigl", "ui\\ui_shared_globals")
            .block(
                WGTZ_SCREENS,
                vec![Struct::new(TAG_REF).tag_ref(0, "wgit", tags::MAIN_MENU)],
            )
            .tag_ref(0x10, "goof", "")
            .tag_ref(0x18, "unic", "");
        MapBuilder::new(256)
            .tag("bitm", crate::layout::LOGO, Meta::Bitmaps(vec![logo]))
            .tag("wgit", tags::MAIN_MENU, Meta::Struct(main_menu))
            .tag("skin", crate::layout::MAIN_MENU_SKIN, Meta::Struct(skin))
            .tag("wigl", "ui\\ui_shared_globals", Meta::Struct(globals))
            .tag("wgtz", "ui\\main_menu", Meta::Struct(wgtz))
    }

    /// A fresh folder for files a test writes.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("h2ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Opens `dir`'s mainmenu.map, giving the shell and what it logged.
    fn open(dir: &Path) -> (Shell, Vec<String>) {
        let lines = RefCell::new(Vec::new());
        let shell = Shell::open(dir, &|l: &str| lines.borrow_mut().push(l.to_string()));
        (shell, lines.into_inner())
    }

    #[test]
    fn the_main_menu_draws_from_a_format_13_maps_tags_and_pictures() {
        let dir = folder("mcc");
        let mut dat = TexturesDat::default();
        let map = mainmenu(&mut dat).build();
        std::fs::write(dir.join("mainmenu.map"), map).unwrap();
        std::fs::write(dir.join("textures.dat"), &dat.bytes).unwrap();
        let (shell, log) = open(&dir);
        assert_eq!(log, shell.sources);
        assert!(log[0].contains("MCC's, cache format 13"), "{log:?}");
        assert!(log
            .iter()
            .any(|l| l.contains("main menu pictures and places from wgit")));
        assert!(log.iter().any(|l| l.contains("pictures for 1 of")));
        // No start screen in the tags: its pieces are the built-in ones.
        assert!(log
            .iter()
            .any(|l| l.contains("start screen pictures and places: built-in")));
        assert_eq!(shell.start.art, StartLayout::default().art);
        let list = &shell.main.list;
        assert_eq!((list.corner, list.visible), ([-100.0, -50.0], 4));
        assert_eq!(list.skin.items.gain.period_ms, 300.0);
        assert_eq!(list.step, 50.0);

        // Drawn at 1080p over the scene, settled: the logo's quad is where
        // its tag puts it, twice its 4 pixels a side.
        let fonts = Fonts::fallback();
        let rows = ["ONLINE", "SETTINGS"];
        let m = MainMenu {
            layout: &shell.main,
            opened: 0.0,
            rows: &rows,
            focus: Focus::on(0, 0.0),
            small_print: "",
        };
        let space = Space::new(1920, 1080);
        let list = screens::main_menu(5.0, &m, shell.art(), &fonts, space);
        let logo: Vec<_> = list
            .quads
            .iter()
            .filter(|q| matches!(q.texture, Texture::Art(_)))
            .collect();
        assert_eq!(logo.len(), 1, "{:?}", list.quads);
        let want = space.rect([-300.0, 250.0, -292.0, 242.0]);
        for (got, want) in logo[0].rect.iter().zip(want) {
            assert!((got - want).abs() < 1e-3, "{logo:?}");
        }
        assert_eq!(logo[0].color[0][3], 1.0);
        // Half way through its 500 ms fade.
        let early = screens::main_menu(0.25, &m, shell.art(), &fonts, space);
        let q = early
            .quads
            .iter()
            .find(|q| matches!(q.texture, Texture::Art(_)));
        assert!((q.unwrap().color[0][3] - 0.5).abs() < 1e-4);
        // On the CPU, its pixels are the picture's colour.
        let mut px = vec![0u32; 1920 * 1080];
        let res = Resources {
            art: shell.art(),
            fonts: &fonts,
        };
        crate::cpu::draw_u32(&list, &mut px, 1920, 1080, &res);
        let [x, y] = space.at([-296.0, 246.0]).map(|v| v as usize);
        let [r, g, b] = LOGO_RGB.map(u32::from);
        assert_eq!(px[y * 1920 + x], (r << 16) | (g << 8) | b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_textures_or_a_map_the_pieces_draw_flat() {
        let dir = folder("flat");
        let mut dat = TexturesDat::default();
        std::fs::write(dir.join("mainmenu.map"), mainmenu(&mut dat).build()).unwrap();
        // No textures.dat: the tags' places, but flat shapes.
        let (shell, log) = open(&dir.join("mainmenu.map"));
        assert!(log.iter().any(|l| l.contains("no textures.dat")), "{log:?}");
        assert!(log
            .iter()
            .any(|l| l.contains("drawn flat") && l.contains("start_screen")));
        assert!(shell.art.is_empty());
        assert_eq!(shell.main.list.corner, [-100.0, -50.0]);
        let logo = &shell.main.art[0];
        assert_eq!(logo.corner, [-300.0, 250.0]);
        assert!(matches!(logo.flat, crate::layout::Flat::Lettering { .. }));
        // Not a map: all built in.
        std::fs::write(dir.join("junk.map"), b"not a map at all").unwrap();
        let (shell, log) = open(&dir.join("junk.map"));
        assert_eq!(shell.main, MainMenuLayout::default());
        assert!(shell.art.is_empty());
        assert!(log[0].contains("junk.map"), "{log:?}");
        // No file at all.
        let (shell, _) = open(&dir.join("missing.map"));
        assert_eq!(shell.start, StartLayout::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Checks the real menus as menu.md section 10 expects them: the logo
    /// at (-511, 90), the list in skin 11 at (-178, -80), the framing at
    /// (-1070, 654) at 1.08, and every picture of both screens found.
    fn check_real(shell: &Shell) {
        let log = shell.sources.join("\n");
        assert!(!log.contains("built-in"), "{log}");
        assert!(!log.contains("drawn flat"), "{log}");
        for art in [&shell.start.art, &shell.main.art] {
            let logo = art.iter().find(|b| b.name == crate::layout::LOGO).unwrap();
            assert_eq!(logo.corner, [-511.0, 90.0]);
        }
        let list = &shell.main.list;
        assert_eq!(list.corner, [-178.0, -80.0]);
        assert!(log.contains("(number 11)"), "{log}");
        assert_eq!(list.skin.items.gain.period_ms, 120.0);
        let f = &shell.main.framing;
        assert_eq!(f.corner, [-1070.0, 654.0]);
        assert!((f.scale()[0] - 1.08).abs() < 0.005);
    }

    /// MCC's mainmenu.map in `H2_MCC_MAPS`.
    #[test]
    #[ignore]
    fn real_mcc_menus_come_from_their_tags() {
        let dir = std::env::var("H2_MCC_MAPS").expect("H2_MCC_MAPS");
        let (shell, log) = open(Path::new(&dir));
        assert!(log[0].contains("MCC's"), "{log:?}");
        check_real(&shell);
    }

    /// Halo 2 Vista's mainmenu.map in `H2_MAPS`.
    #[test]
    #[ignore]
    fn real_vista_menus_come_from_their_tags() {
        let dir = std::env::var("H2_MAPS").expect("H2_MAPS");
        let (shell, log) = open(Path::new(&dir));
        assert!(log[0].contains("Vista's"), "{log:?}");
        check_real(&shell);
    }
}
