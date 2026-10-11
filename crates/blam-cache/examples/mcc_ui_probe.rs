//! A console probe of MCC's Halo 2 UI tags (cache format 13), for the
//! owner's PC: checks A to H of `docs/notes/launcher/menu.md`, section 9.2.
//! It prints only (pass or fail, sizes, formats, counts, names and a few
//! of Halo 2's menu labels) and never writes a file. Expected Halo 2 Vista
//! values are in brackets. A check that fails says why, and the next one
//! runs.
//!
//! ```text
//! cargo run --release -p blam-cache --example mcc_ui_probe -- ^
//!     "<MCC>\halo2\h2_maps_win64_dx11\mainmenu.map" [<Vista maps\mainmenu.map>]
//! ```
//!
//! The textures.dat and shared.map beside MCC's map are used when found.
//! With a Vista mainmenu.map as well, the two maps' screens are compared
//! (the diff that covers C to F) and each picture's pixels are compared by
//! hash.

use blam_cache::bitmap::{Format, Sequence};
use blam_cache::mcc::{self, BitmapEntry};
use blam_cache::ui::{self, MenuMap, Pictures, Reader};
use blam_cache::{DatumIndex, GroupTag, MapSet};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufReader;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

type Map = mcc::Map<BufReader<File>>;
type Textures = mcc::Textures<BufReader<File>>;

const MAIN_MENU: &str = r"ui\screens\game_shell\main_menu_screen\main_menu";
const MAIN_SKIN: &str = r"ui\list_skins\main_menu\main_menu";
const COLLECTION: &str = r"ui\main_menu";
/// The screens whose pictures check H decodes, by the end of their names.
const PICTURE_SCREENS: [&str; 3] = ["start_screen", "main_menu", "game_shell_background"];

fn group(s: &str) -> GroupTag {
    GroupTag::parse(s).expect("four letters")
}

/// A group as its four letters, or in hex when they aren't letters.
fn show(g: GroupTag) -> String {
    let printable =
        g.0.to_be_bytes()
            .iter()
            .all(|c| c.is_ascii_graphic() || *c == b' ');
    match printable || g.is_none() {
        true => g.to_string(),
        false => format!("{:#010x}", g.0),
    }
}

struct Probe {
    map: Map,
    shared: Option<Map>,
    textures: Option<Textures>,
    vista: Option<MapSet>,
    ui: Option<ui::Ui>,
    vista_ui: Option<ui::Ui>,
    passed: usize,
    failed: Vec<String>,
}

impl Probe {
    fn pass(&mut self, what: &str) {
        self.passed += 1;
        println!("  PASS {what}");
    }

    fn fail(&mut self, what: &str, why: &str) {
        self.failed.push(what.to_string());
        println!("  FAIL {what}: {why}");
    }

    /// Passes or fails `what`, saying `detail` either way.
    fn check(&mut self, ok: bool, what: &str, detail: &str) {
        match ok {
            true => self.pass(&format!("{what} ({detail})")),
            false => self.fail(what, detail),
        }
    }

    /// Runs one check; an error or a panic in it fails it and the probe
    /// goes on.
    fn run(&mut self, title: &str, check: fn(&mut Probe) -> Result<(), String>) {
        println!("\n== {title}");
        match catch_unwind(AssertUnwindSafe(|| check(self))) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => self.fail(title, &e),
            Err(_) => self.fail(
                title,
                "panicked (a bug in the reader; the message is above)",
            ),
        }
    }

    /// The tag of `group` named `name`, or the first whose name ends with
    /// `\<last part of name>`.
    fn find(&self, g: &str, name: &str) -> Option<mcc::Tag> {
        let g = group(g);
        let last = name.rsplit('\\').next().unwrap_or(name);
        self.map.find_tag(g, name).cloned().or_else(|| {
            self.map
                .tags
                .iter()
                .find(|t| t.group == g && t.name.ends_with(&format!("\\{last}")))
                .cloned()
        })
    }

    fn meta(&mut self, tag: &mcc::Tag) -> Result<Vec<u8>, String> {
        if !tag.has_data() {
            return Err(format!("{} has no meta in this map", tag.name));
        }
        self.map.tag_meta(tag).map_err(|e| e.to_string())
    }

    fn name_of(&self, datum: DatumIndex) -> String {
        self.map.tag(datum).map_or_else(
            || format!("{:08x} (no such tag)", datum.0),
            |t| t.name.clone(),
        )
    }

    fn ui(&self) -> Result<&ui::Ui, String> {
        self.ui
            .as_ref()
            .ok_or_else(|| "the menus didn't read (see 'Reading the menus')".to_string())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().map(PathBuf::from) else {
        eprintln!("usage: mcc_ui_probe <MCC mainmenu.map> [<Vista mainmenu.map>]");
        std::process::exit(2);
    };
    let map = match Map::open(&path) {
        Ok(m) => m,
        Err(e) => {
            println!("FAIL: {} doesn't open as MCC's map: {e}", path.display());
            std::process::exit(1);
        }
    };
    let h = &map.header;
    println!(
        "MCC map {}: name {:?}, scenario {:?}, image {:#x} bytes in {} chunks of {:#x}, {} tags",
        path.display(),
        h.map_name,
        h.scenario_path,
        h.image_size,
        h.chunk_count,
        h.chunk_size,
        map.tags.len()
    );
    let beside = |name: &str| path.with_file_name(name);
    let shared = match Map::open(&beside("shared.map")) {
        Ok(s) => {
            println!("shared.map: {} tags", s.tags.len());
            Some(s)
        }
        Err(e) => {
            println!("shared.map: not used ({e})");
            None
        }
    };
    let textures = match Textures::open(&beside("textures.dat")) {
        Ok(t) => {
            println!("textures.dat: {} bytes", t.len());
            Some(t)
        }
        Err(e) => {
            println!("textures.dat: missing ({e}); pictures won't decode");
            None
        }
    };
    let vista = args.get(1).and_then(|v| match MapSet::open(Path::new(v)) {
        Ok(set) => {
            println!("Vista map {v}: {} tags", set.map.tags.len());
            Some(set)
        }
        Err(e) => {
            println!("Vista map {v}: doesn't open ({e}); the comparisons are skipped");
            None
        }
    });
    let mut p = Probe {
        map,
        shared,
        textures,
        vista,
        ui: None,
        vista_ui: None,
        passed: 0,
        failed: Vec::new(),
    };
    p.run("A. The header and string ids", check_a);
    p.run("B. Tag counts", check_b);
    p.run("C. wgtz ui\\main_menu", check_c);
    p.run(
        "Reading the menus (ui.rs through the format-13 reader)",
        read_menus,
    );
    p.run("D. wigl", check_d);
    p.run("E. wgit main_menu", check_e);
    p.run("F. skin main_menu", check_f);
    p.run("G. Strings", check_g);
    p.run("H. Bitmaps", check_h);
    if p.vista.is_some() {
        p.run("C to F: MCC's screens against Vista's", diff_screens);
    }
    println!("\n{} passed, {} failed", p.passed, p.failed.len());
    for f in &p.failed {
        println!("  failed: {f}");
    }
}

fn check_a(p: &mut Probe) -> Result<(), String> {
    let words: Vec<String> = [
        0x18, 0x1C, 0x30, 0x34, 0x38, 0x3C, 0x2D0, 0x2E4, 0x2E8, 0x2EC, 0x2F0, 0x2F4,
    ]
    .iter()
    .map(|&o| format!("{o:#x}={:#x}", p.map.header_word(o).unwrap_or(0)))
    .collect();
    println!("  header words: {}", words.join(" "));
    let h = p.map.header.clone();
    println!(
        "  image {:#x}; index at {:#x}, {:#x} bytes; string ids: {} at {:#x} ({:#x} bytes), index at {:#x}",
        h.image_size,
        h.index_offset,
        h.index_size,
        h.string_count,
        h.strings_offset,
        h.strings_size,
        h.string_index_offset
    );
    let ids = match p.map.read_string_ids() {
        Ok(ids) => ids.clone(),
        Err(e) => {
            p.fail("the string-id table reads", &e.to_string());
            return Ok(());
        }
    };
    p.pass(&format!(
        "the string-id index and text lie inside the image ({} strings)",
        ids.names.len()
    ));
    let first: Vec<&str> = ids
        .names
        .iter()
        .skip(1)
        .take(5)
        .map(String::as_str)
        .collect();
    println!("  strings 1 to 5: {first:?}");
    let zero = format!("{:?}", ids.names[0]);
    p.check(ids.names[0].is_empty(), "string 0 is empty", &zero);
    let end = ids.end();
    p.check(
        end == ids.size as usize,
        "the last string ends at the text's size",
        &format!("ends at {end:#x}, size {:#x}", ids.size),
    );
    // main_menu's header id, or the first screen's that has one.
    let wgit = group("wgit");
    let mut screens: Vec<mcc::Tag> = p.find("wgit", MAIN_MENU).into_iter().collect();
    screens.extend(p.map.tags.iter().filter(|t| t.group == wgit).cloned());
    for tag in screens {
        let Ok(meta) = p.meta(&tag) else { continue };
        let Some(id) = meta
            .get(0x2C..0x30)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        else {
            continue;
        };
        if id == 0 {
            println!("  {} has no header id (0)", tag.name);
            continue;
        }
        let name = ids.get(id).unwrap_or("").to_string();
        p.check(
            !name.is_empty() && name.len() == (id >> 24) as usize,
            "a screen's header id resolves to a name of its length",
            &format!(
                "{}: id {id:#010x} -> {name:?}, length {}",
                tag.name,
                name.len()
            ),
        );
        return Ok(());
    }
    p.fail(
        "a screen's header id resolves",
        "no screen with a header id read",
    );
    Ok(())
}

fn check_b(p: &mut Probe) -> Result<(), String> {
    let groups = [
        ("wgtz", "1"),
        ("wigl", "1"),
        ("wgit", "131"),
        ("skin", "38; the wigl lists 26"),
        ("unic", "-"),
        ("bitm", "-"),
        ("snd!", "-"),
        ("lsnd", "-"),
        ("ugh!", "-"),
        ("matg", "-"),
        ("goof", "-"),
        ("scnr", "-"),
    ];
    let mut counts = Vec::new();
    for (g, want) in groups {
        let tags: Vec<&mcc::Tag> = p.map.tags.iter().filter(|t| t.group == group(g)).collect();
        let with = tags.iter().filter(|t| t.has_data()).count();
        println!("  {g}: {} ({with} with meta here) [{want}]", tags.len());
        counts.push(tags.len());
    }
    p.check(
        counts[0] == 1 && counts[1] == 1 && counts[2] > 0,
        "one wgtz, one wigl and some wgit",
        &format!("{} wgtz, {} wigl, {} wgit", counts[0], counts[1], counts[2]),
    );
    if let Some(v) = &p.vista {
        let names = |tags: Vec<String>| -> BTreeSet<String> {
            tags.into_iter().map(|n| n.to_lowercase()).collect()
        };
        let mine = names(
            p.map
                .tags
                .iter()
                .filter(|t| t.group == group("wgit"))
                .map(|t| t.name.clone())
                .collect(),
        );
        let theirs = names(
            v.map
                .tags
                .iter()
                .filter(|t| t.group == group("wgit"))
                .map(|t| t.name.clone())
                .collect(),
        );
        for (label, only) in [
            (
                "only in MCC's",
                mine.difference(&theirs).collect::<Vec<_>>(),
            ),
            (
                "only in Vista's",
                theirs.difference(&mine).collect::<Vec<_>>(),
            ),
        ] {
            println!("  wgit {label}: {}", only.len());
            for n in only.iter().take(60) {
                println!("    {n}");
            }
        }
    }
    Ok(())
}

fn check_c(p: &mut Probe) -> Result<(), String> {
    let tag = p
        .find("wgtz", COLLECTION)
        .or_else(|| {
            p.map
                .tags
                .iter()
                .find(|t| t.group == group("wgtz"))
                .cloned()
        })
        .ok_or("no wgtz tag")?;
    let meta = p.meta(&tag)?;
    println!(
        "  {} at {:#x}, {:#x} bytes of meta",
        tag.name, tag.address, tag.size
    );
    let r = |at: usize| mcc::tag_ref(&meta, at).unwrap_or((GroupTag::NONE, DatumIndex::NONE));
    let (g, d) = r(0);
    // The tag at the datum's index itself, not `Map::tag`, which would
    // also find a datum whose index bits point at the wrong row.
    let at_index = p.map.tags.get(usize::from(d.index()));
    p.check(
        g == group("wigl") && at_index.is_some_and(|t| t.datum == d && t.group == g),
        "+0x0 is a wigl reference whose datum matches the index",
        &format!("{} {:08x} -> {}", show(g), d.0, p.name_of(d)),
    );
    let (count, address) = mcc::block_header(&meta, 8).map_err(|e| e.to_string())?;
    let screens = p
        .map
        .block(&meta, 8, mcc::TAG_REF_SIZE)
        .map_err(|e| e.to_string())?;
    let mut bad = Vec::new();
    let mut good = 0;
    let (refs, _) = screens.as_chunks::<{ mcc::TAG_REF_SIZE }>();
    for s in refs {
        let (sg, sd) = mcc::tag_ref(s, 0).expect("8 bytes");
        match p
            .map
            .tag(sd)
            .filter(|t| t.group == sg && sg == group("wgit"))
        {
            Some(_) => good += 1,
            None => bad.push(format!("{} {:08x}", show(sg), sd.0)),
        }
    }
    p.check(
        count == 133,
        "+0x8 holds 133 screens",
        &format!("{count} [133]"),
    );
    p.check(
        bad.is_empty() && good > 0,
        "every screen is a wgit reference that resolves",
        &format!(
            "{good} resolve, {} don't {:?}",
            bad.len(),
            &bad[..bad.len().min(5)]
        ),
    );
    p.check(
        tag.holds(address, screens.len()),
        "the nested block lies inside the tag's own meta",
        &format!(
            "block at {address:#x}, {:#x} bytes; meta {:#x}..{:#x}",
            screens.len(),
            tag.address,
            u64::from(tag.address) + u64::from(tag.size)
        ),
    );
    for (at, want) in [(0x10, "goof"), (0x18, "unic")] {
        let (g, d) = r(at);
        p.check(
            g == group(want) && p.map.tag(d).is_some(),
            &format!("+{at:#x} is a {want} reference"),
            &format!("{} -> {}", show(g), p.name_of(d)),
        );
    }
    Ok(())
}

fn read_menus(p: &mut Probe) -> Result<(), String> {
    let (ui, problems) = {
        let mut m = ui::Mcc::new(&mut p.map, p.shared.as_mut());
        let ui = ui::read_from(&mut m);
        (ui, m.problems.clone())
    };
    println!("  {problems:?}");
    match ui {
        Ok(ui) => {
            p.pass(&format!(
                "the menus read: {} screens, {} skins",
                ui.screens.len(),
                ui.globals.skins.len()
            ));
            p.check(
                problems.bad_blocks == 0,
                "every nested block read",
                &format!(
                    "{} didn't: {:?}",
                    problems.bad_blocks, problems.first_bad_block
                ),
            );
            // Not a failure: MCC's mainmenu.map has 285 blocks outside
            // their tag's meta (2026-10-10), and every screen and skin still
            // matches Vista's, so format 13 seems to share blocks between
            // tags. Printed so a change in the count shows.
            println!(
                "  NOTE {} nested blocks lie outside their tag's meta [285]; first: {:?}",
                problems.outside, problems.first_outside
            );
            p.ui = Some(ui);
        }
        Err(e) => p.fail("the menus read", &e.to_string()),
    }
    if let Some(set) = p.vista.as_mut() {
        match ui::read(set) {
            Ok(ui) => p.vista_ui = Some(ui),
            Err(e) => println!("  Vista's menus don't read: {e}"),
        }
    }
    Ok(())
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn check_d(p: &mut Probe) -> Result<(), String> {
    let g = p.ui()?.globals.clone();
    p.check(
        close(g.overlay_alpha_mod, 0.10),
        "0x44, the overlay's alpha mod",
        &format!("{:.3} [0.10]", g.overlay_alpha_mod),
    );
    let [r, gr, b, a] = g.overlay_color;
    p.check(
        close(a, 0.85) && close(r, 0.0) && close(gr, 0.08) && close(b, 0.17),
        "0x6C, the overlay colour (a, r, g, b)",
        &format!("{a:.2}, {r:.2}, {gr:.2}, {b:.2} [0.85, 0, 0.08, 0.17]"),
    );
    let s = &g.sounds;
    let sounds = [&s.cursor, &s.select, &s.error, &s.advance, &s.retreat];
    let want = ["cursor1", "forward1", "flag_fail", "advance", "back1"];
    let names: Vec<&str> = sounds.iter().map(|n| n.as_deref().unwrap_or("-")).collect();
    let ok = names.iter().zip(want).all(|(n, w)| n.ends_with(w));
    p.check(
        ok,
        "the first five sound references at 0x90",
        &format!("{names:?} [{want:?}]"),
    );
    p.check(
        g.skins.len() == 26,
        "26 skins at 0x138",
        &format!("{} [26]", g.skins.len()),
    );
    // The header fonts as stored, since the parse turns a bad one into the
    // title font.
    let wigl = p
        .map
        .tags
        .iter()
        .find(|t| t.group == group("wigl"))
        .cloned();
    if let Some(meta) = wigl.and_then(|t| p.meta(&t).ok()) {
        let fonts: Vec<u16> = (0..4)
            .filter_map(|k| meta.get(0x160 + 2 * k..0x162 + 2 * k))
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        p.check(
            fonts.len() == 4 && fonts.iter().all(|&f| f < 12),
            "the header fonts at 0x160 are below 12",
            &format!("{fonts:?}"),
        );
    }
    let full = g.header_bounds[0];
    let rect = [full.top, full.left, full.bottom, full.right];
    p.check(
        rect == [567, -730, 520, 50],
        "the full header rectangle at 0x178",
        &format!("{rect:?} [567, -730, 520, 50]"),
    );
    let music = g.music.clone().unwrap_or_default();
    p.check(
        music.ends_with("main_menu_music"),
        "the music at 0x1B8",
        &format!("{music:?} [main_menu_music]"),
    );
    p.check(
        g.music_fade_ms == 5500,
        "the music's fade at 0x1C0",
        &format!("{} [5500]", g.music_fade_ms),
    );
    Ok(())
}

fn main_menu(ui: &ui::Ui) -> Option<&ui::Screen> {
    ui.screen(MAIN_MENU)
        .or_else(|| ui.screens.iter().find(|s| s.name.ends_with("\\main_menu")))
}

fn check_e(p: &mut Probe) -> Result<(), String> {
    let s = main_menu(p.ui()?).ok_or("no main_menu screen")?.clone();
    println!("  {}: {} panes", s.name, s.panes.len());
    p.check(
        s.flags & ui::NO_HEADER != 0 && s.screen_id == 6 && s.button_key == 0,
        "NO_HEADER, screen id 6, legend 0",
        &format!(
            "flags {:#x}, id {:#x}, legend {} [NO_HEADER, 6, 0]",
            s.flags, s.screen_id, s.button_key
        ),
    );
    let lists: Vec<&ui::List> = s.panes.iter().flat_map(|p| &p.lists).collect();
    let list = lists
        .iter()
        .find(|l| l.skin == 11 && l.visible == 6 && l.corner == [-178, -80]);
    p.check(
        list.is_some(),
        "the list in skin 11, 6 visible, at (-178, -80)",
        &format!(
            "{:?}",
            lists
                .iter()
                .map(|l| (l.skin, l.visible, l.corner))
                .collect::<Vec<_>>()
        ),
    );
    let bitmaps: Vec<&ui::Bitmap> = s.panes.iter().flat_map(|p| &p.bitmaps).collect();
    let name = |b: &ui::Bitmap| {
        b.bitmap
            .as_ref()
            .map_or("-", |t| t.name.as_str())
            .to_string()
    };
    let logo = bitmaps.iter().find(|b| b.corner == [-511, 90]);
    p.check(
        logo.is_some(),
        "the logo at (-511, 90)",
        &logo.map_or("no bitmap there".into(), |b| name(b)),
    );
    let tracks: Vec<&&ui::Bitmap> = bitmaps
        .iter()
        .filter(|b| {
            let n = name(b);
            n.contains("track") && !n.contains("brace")
        })
        .collect();
    for t in &tracks {
        println!(
            "    track y {:5} {} wraps/s {:.3}",
            t.corner[1],
            name(t).rsplit('\\').next().unwrap_or(""),
            t.wraps_per_second[0]
        );
    }
    p.check(
        tracks.len() == 11,
        "the 11 tracks of mainmenu-plan.md 1.3",
        &format!("{} [11]", tracks.len()),
    );
    Ok(())
}

fn check_f(p: &mut Probe) -> Result<(), String> {
    let ui = p.ui()?;
    let skin = ui
        .globals
        .skins
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(MAIN_SKIN))
        .or_else(|| ui.globals.skins.get(11))
        .ok_or("no main_menu skin")?
        .clone();
    println!("  {}", skin.name);
    let mut periods = Vec::new();
    let mut alphas = Vec::new();
    for (k, a) in skin.item_animations.iter().enumerate() {
        let keys: Vec<String> = a
            .keyframes
            .iter()
            .map(|f| format!("{:.2}", f.alpha))
            .collect();
        println!(
            "    item animation {k}: {} ms, alphas {}",
            a.period_ms,
            keys.join(" ")
        );
        periods.push(a.period_ms);
        alphas.extend(a.keyframes.iter().map(|f| f.alpha));
    }
    let has = |v: f32| alphas.iter().any(|&a| close(a, v));
    p.check(
        [120, 200, 90].iter().all(|ms| periods.contains(ms)),
        "item animations of 120, 200 and 90 ms",
        &format!("{periods:?}"),
    );
    p.check(
        has(0.5) && has(1.0) && has(0.7),
        "alphas 0.5, 1.0 and 0.7",
        &format!("{} keyframes", alphas.len()),
    );
    Ok(())
}

fn check_g(p: &mut Probe) -> Result<(), String> {
    match p.map.language_table() {
        Ok(l) => p.pass(&format!(
            "the English table resolves, each offset on a string's start ({} strings, {})",
            l.strings.len(),
            l.source
        )),
        Err(e) => p.fail("the English table resolves", &e.to_string()),
    }
    let live = p
        .map
        .tags
        .iter()
        .filter(|t| t.group == group("wgit"))
        .find(|t| t.name.contains("xbox_live") && t.name.contains("menu"))
        .or_else(|| {
            p.map.tags.iter().find(|t| {
                t.group == group("wgit") && t.name.contains("live") && t.name.contains("menu")
            })
        })
        .cloned();
    let screens: Vec<mcc::Tag> = p.find("wgit", MAIN_MENU).into_iter().chain(live).collect();
    for tag in screens {
        let meta = p.meta(&tag)?;
        let Some((_, unic)) = mcc::tag_ref(&meta, 0x18) else {
            continue;
        };
        let strings = {
            let mut m = ui::Mcc::new(&mut p.map, p.shared.as_mut());
            let s = m.unicode_strings(unic);
            let named: Vec<(String, String)> =
                s.into_iter().map(|(id, w)| (m.string_id(id), w)).collect();
            (named, m.problems.language.clone())
        };
        println!("  {} (strings {}):", tag.name, p.name_of(unic));
        if let Some(why) = strings.1 {
            println!("    no English strings: {why}");
        }
        for (id, words) in strings.0 {
            println!("    {id} = {words:?}");
        }
    }
    Ok(())
}

/// FNV-1a over a picture's pixels, for comparing MCC's and Vista's.
fn hash(rgba: &[u8]) -> u64 {
    rgba.iter().fold(0xCBF2_9CE4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn format_name(f: Format) -> String {
    match f {
        Format::Other(n) => format!("{n} (unknown here)"),
        f => format!("{} {f:?}", f.number()),
    }
}

/// The names of every picture the start screen, the main menu and the
/// shell background name, and the main menu list skin's.
fn picture_names(ui: &ui::Ui) -> Vec<String> {
    let mut names = Vec::new();
    let mut add = |t: &Option<ui::TagRef>| {
        if let Some(t) = t {
            if !names.contains(&t.name) {
                names.push(t.name.clone());
            }
        }
    };
    for s in ui.screens.iter().filter(|s| {
        PICTURE_SCREENS
            .iter()
            .any(|e| s.name.ends_with(&format!("\\{e}")))
    }) {
        for pane in &s.panes {
            pane.bitmaps.iter().for_each(|b| add(&b.bitmap));
            pane.buttons.iter().for_each(|b| add(&b.bitmap));
        }
    }
    if let Some(skin) = ui
        .globals
        .skins
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(MAIN_SKIN))
    {
        add(&skin.arrows);
        skin.bitmaps.iter().for_each(|b| add(&b.bitmap));
    }
    names
}

fn print_sequences(sequences: &[Sequence], size: (u32, u32)) {
    for s in sequences {
        let sprites: Vec<String> = s
            .sprites
            .iter()
            .take(4)
            .map(|p| format!("image {} {:?}", p.bitmap, p.pixels(size.0, size.1)))
            .collect();
        println!(
            "      sequence {:?}: images {}+{}, {} sprites {}",
            s.name,
            s.first_bitmap,
            s.bitmap_count,
            s.sprites.len(),
            sprites.join(", ")
        );
    }
}

fn check_h(p: &mut Probe) -> Result<(), String> {
    let mut names = picture_names(p.ui()?);
    // The button pictures, by name, whichever screens use them.
    let buttons: Vec<String> = p
        .map
        .tags
        .iter()
        .filter(|t| t.group == group("bitm") && t.name.contains("button"))
        .take(12)
        .map(|t| t.name.clone())
        .collect();
    for b in buttons {
        if !names.contains(&b) {
            names.push(b);
        }
    }
    println!("  {} pictures", names.len());
    let mut mcc_formats = BTreeSet::new();
    let mut vista_formats = BTreeSet::new();
    let (mut decoded, mut total) = (0, 0);
    for name in &names {
        let Some(tag) = p.map.find_tag(group("bitm"), name).cloned() else {
            println!("  {name}: no such bitmap tag");
            continue;
        };
        let file = match (tag.has_data(), p.shared.as_mut()) {
            (true, _) => &mut p.map,
            (false, Some(s)) => s,
            (false, None) => {
                println!("  {name}: no meta here and no shared.map");
                continue;
            }
        };
        let tag = file.find_tag(group("bitm"), name).cloned().unwrap_or(tag);
        let entries = match (file.bitmaps(&tag), file.bitmap_block(&tag)) {
            (Ok(e), Ok(raw)) => (e, raw),
            (Err(e), _) | (_, Err(e)) => {
                p.fail(&format!("{name}'s images read"), &e.to_string());
                continue;
            }
        };
        let sequences = file.sequences(&tag);
        println!("  {name}: {} images", entries.0.len());
        let vista_info = p.vista.as_mut().and_then(|v| v.bitmap(name).ok());
        for (i, e) in entries.0.iter().enumerate() {
            total += 1;
            mcc_formats.insert(e.format.number());
            if i < 4 {
                print_entry(
                    i,
                    e,
                    &entries.1[i * mcc::BITMAP_ENTRY_SIZE..(i + 1) * mcc::BITMAP_ENTRY_SIZE],
                );
            }
            let Some(textures) = p.textures.as_mut() else {
                continue;
            };
            match textures.record(e) {
                Ok(r) => {
                    // A raw stream's size shows negative, as it is stored.
                    let sizes: Vec<i64> = r
                        .streams
                        .iter()
                        .map(|s| i64::from(s.size) * if s.raw { -1 } else { 1 })
                        .collect();
                    if i < 4 {
                        println!(
                            "      record at {:#x}: {:?}, {} streams {:?}{}",
                            r.offset,
                            r.layout,
                            sizes.len(),
                            &sizes[..sizes.len().min(4)],
                            if sizes.len() > 4 { " ..." } else { "" }
                        );
                    }
                }
                Err(err) => println!("      image {i}: record: {err}"),
            }
            match textures.decode(e) {
                Ok(d) => {
                    decoded += 1;
                    let h = hash(&d.image.rgba);
                    let vista = match (&vista_info, p.vista.as_mut()) {
                        (Some(info), Some(set)) if i < info.images.len() => {
                            vista_formats.insert(info.images[i].format.number());
                            match set.image(name, i) {
                                Ok(v) if hash(&v.rgba) == h => "same pixels as Vista's".to_string(),
                                Ok(v) => format!(
                                    "differs from Vista's ({}x{} {})",
                                    v.width,
                                    v.height,
                                    format_name(info.images[i].format)
                                ),
                                Err(err) => format!("Vista's doesn't decode: {err}"),
                            }
                        }
                        (None, Some(_)) => "not in Vista's map".to_string(),
                        _ => String::new(),
                    };
                    if i < 4 || !vista.starts_with("same") {
                        println!(
                            "      image {i}: decoded {}x{}, inflated {} bytes, rows {} of {} bytes, hash {h:016x} {vista}",
                            d.image.width, d.image.height, d.inflated, d.row_pitch, d.row_bytes
                        );
                    }
                }
                Err(err) => println!("      image {i}: doesn't decode: {err}"),
            }
        }
        let size = entries
            .0
            .first()
            .map_or((1, 1), |e| (u32::from(e.width), u32::from(e.height)));
        match sequences {
            Ok(s) => print_sequences(&s, size),
            Err(e) => println!("      sequences don't read: {e}"),
        }
    }
    p.check(
        total > 0 && decoded == total,
        "every picture decodes",
        &format!("{decoded} of {total} images"),
    );
    println!("  formats in MCC's: {mcc_formats:?}");
    if p.vista.is_some() {
        let only: Vec<i16> = mcc_formats.difference(&vista_formats).copied().collect();
        println!("  formats in Vista's: {vista_formats:?}; in MCC's alone: {only:?}");
    }
    Ok(())
}

/// One image entry's fields, and the words of it no field is known for
/// (the native mip info and tile mode are thought to be among them).
fn print_entry(i: usize, e: &BitmapEntry, raw: &[u8]) {
    println!(
        "    image {i}: {}x{} depth {} kind {} format {} flags {:#06x} mips {} registration {:?}",
        e.width,
        e.height,
        e.depth,
        e.kind,
        format_name(e.format),
        e.flags,
        e.mip_count,
        e.registration
    );
    println!(
        "      pointer {:#x} (top bits {}), stored {}; other levels at {:x?} sizes {:?}",
        e.pointer,
        e.pointer >> 30,
        e.stored_size,
        e.lod_pointers,
        e.lod_sizes
    );
    let unknown: Vec<String> = (0x18..0x38)
        .chain(0x68..mcc::BITMAP_ENTRY_SIZE)
        .step_by(4)
        .filter_map(|o| {
            let w = u32::from_le_bytes(raw.get(o..o + 4)?.try_into().ok()?);
            (w != 0).then(|| format!("+{o:#x}={w:#x}"))
        })
        .take(16)
        .collect();
    match unknown.is_empty() {
        true => println!("      other words: all 0"),
        false => println!("      other words: {}", unknown.join(" ")),
    }
}

/// The menus with every tag reference's datum cleared, so two maps compare
/// by names.
fn by_name(ui: &ui::Ui) -> ui::Ui {
    let mut ui = ui.clone();
    let clear = |t: &mut Option<ui::TagRef>| {
        if let Some(t) = t {
            t.datum = DatumIndex::NONE;
        }
    };
    for s in &mut ui.globals.skins {
        clear(&mut s.arrows);
        s.bitmaps.iter_mut().for_each(|b| clear(&mut b.bitmap));
    }
    for s in &mut ui.screens {
        for p in &mut s.panes {
            p.bitmaps.iter_mut().for_each(|b| clear(&mut b.bitmap));
            p.buttons.iter_mut().for_each(|b| clear(&mut b.bitmap));
            p.players.iter_mut().for_each(|b| clear(&mut b.skin));
        }
    }
    ui
}

/// The first line where two texts differ, each cut to a readable length.
fn first_difference(a: &str, b: &str) -> String {
    let cut = |s: &str| s.trim().chars().take(140).collect::<String>();
    let mut lines = a.lines().zip(b.lines());
    match lines.find(|(x, y)| x != y) {
        Some((x, y)) => format!("MCC {} / Vista {}", cut(x), cut(y)),
        None => "one is longer".into(),
    }
}

fn diff_screens(p: &mut Probe) -> Result<(), String> {
    let mine = by_name(p.ui()?);
    let theirs = by_name(p.vista_ui.as_ref().ok_or("Vista's menus didn't read")?);
    let (g1, g2) = (
        format!("{:#?}", mine.globals),
        format!("{:#?}", theirs.globals),
    );
    p.check(
        g1 == g2,
        "the globals and skins match Vista's",
        &if g1 == g2 {
            "same".into()
        } else {
            first_difference(&g1, &g2)
        },
    );
    let (mut same, mut differ) = (0, Vec::new());
    for s in &mine.screens {
        match theirs
            .screens
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(&s.name))
        {
            Some(t) if t == s => same += 1,
            Some(t) => differ.push((
                s.name.clone(),
                first_difference(&format!("{s:#?}"), &format!("{t:#?}")),
            )),
            None => {}
        }
    }
    for (name, d) in differ.iter().take(30) {
        println!("    {name}: {d}");
    }
    p.check(
        differ.is_empty(),
        "every screen in both maps matches",
        &format!("{same} match, {} differ", differ.len()),
    );
    Ok(())
}
