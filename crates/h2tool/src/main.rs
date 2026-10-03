//! h2tool: inspect Halo 2 PC map files.
//!
//! Usage:
//!   h2tool info  <file.map>              header summary
//!   h2tool tags  <file.map> [group]      list tags, optionally only one group (e.g. sbsp)
//!   h2tool groups <file.map>             tag group counts
//!   h2tool check <file.map>              read every tag stored in the map to verify it
//!   h2tool obj   <file.map> <out.obj>    export level collision geometry
//!   h2tool render <file.map> <out.png>   software-rendered preview of the level
//!   h2tool level <file.map> [png dir]    render geometry + shader/texture summary (optionally dump textures)
//!   h2tool sim   <file.map>              drop a Spartan at every spawn and walk; reports collision sanity
//!   h2tool scan  <maps folder>           summary line for every .map in a folder
//!   h2tool model <file.map> <tag> [png]  render model summary for an object or `mode` tag
//!   h2tool weapon <file.map> <tag>       a weapon's firing stats (magazine, barrel, projectile)
//!   h2tool hud   <file.map> <nhdt> [dir] a HUD's bitmap widgets (optionally dump their images)
//!   h2tool jmad  <file.map> <tag>        an animation graph's skeleton and animations
//!   h2tool jmadscan <file.map>           decode every animation the map can see
//!   h2tool objects <file.map>            scenery and multiplayer item spawns
//!   h2tool sound <file.map> <name> [out.wav]  a sound's format (optionally written as WAV)
//!   h2tool soundscan <file.map>          decode every sound the map can see
//!   h2tool hex   <file.map> <datum|group:name> [path]
//!                                        hex dump of a tag, or of a block inside it;
//!                                        path = `offset:size[@index]/...` (hex), e.g. 8:c/0:10

mod render;

use blam_cache::{CacheFile, GroupTag};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") if args.len() >= 2 => info(&args[1]),
        Some("tags") if args.len() >= 2 => tags(&args[1], args.get(2).map(String::as_str)),
        Some("groups") if args.len() >= 2 => groups(&args[1]),
        Some("check") if args.len() >= 2 => check(&args[1]),
        Some("scan") if args.len() >= 2 => scan(&args[1]),
        Some("sim") if args.len() >= 2 => sim(&args[1]),
        Some("level") if args.len() >= 2 => level(&args[1], args.get(2).map(String::as_str)),
        Some("obj") if args.len() >= 3 => obj(&args[1], &args[2]),
        Some("render") if args.len() >= 3 => render_png(&args[1], &args[2]),
        Some("weapon") if args.len() >= 3 => weapon(&args[1], &args[2]),
        Some("hud") if args.len() >= 3 => hud(&args[1], &args[2], args.get(3).map(String::as_str)),
        Some("jmad") if args.len() >= 3 => jmad(&args[1], &args[2]),
        Some("jmadscan") if args.len() >= 2 => jmadscan(&args[1]),
        Some("shader") if args.len() >= 3 => shader_dump(&args[1], &args[2]),
        Some("lightmap") if args.len() >= 2 => lightmap_dump(&args[1]),
        Some("objects") if args.len() >= 2 => objects(&args[1]),
        Some("sound") if args.len() >= 3 => {
            sound(&args[1], &args[2], args.get(3).map(String::as_str))
        }
        Some("soundscan") if args.len() >= 2 => soundscan(&args[1]),
        Some("bitmap") if args.len() >= 4 => bitmap_png(&args[1], &args[2], &args[3]),
        Some("refs") if args.len() >= 3 => refs(&args[1], &args[2]),
        Some("events") if args.len() >= 2 => events(&args[1]),
        Some("netgame") if args.len() >= 2 => netgame(&args[1]),
        Some("sid") if args.len() >= 3 => {
            let map = blam_cache::CacheFile::open(&args[1]);
            map.map_err(Into::into).and_then(|m| {
                let id = u32::from_str_radix(args[2].trim_start_matches("0x"), 16)?;
                println!("{:?}", m.string_id(id));
                Ok(())
            })
        }
        Some("hex") if args.len() >= 3 => hex(&args[1], &args[2], args.get(3).map(String::as_str)),
        Some("model") if args.len() >= 3 => {
            model(&args[1], &args[2], args.get(3).map(String::as_str))
        }
        _ => {
            eprintln!(
                "usage: h2tool <info|tags|groups|check|obj|render|level|sim|scan|model> <path> [..]"
            );
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        // Output piped into something like `head` that closed early.
        Err(e)
            if e.downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) =>
        {
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type Res = Result<(), Box<dyn std::error::Error>>;

fn info(path: &str) -> Res {
    let mut map = CacheFile::open(path)?;
    let h = &map.header;
    println!("name:      {}", h.internal_name);
    println!("scenario:  {}", h.scenario_name);
    println!("type:      {:?}", h.map_type);
    println!("build:     {}", h.build);
    println!("size:      {} bytes", h.file_size);
    println!("tags:      {}", map.tags.len());
    println!("groups:    {}", map.groups.len());
    println!("strings:   {}", map.strings().len());
    if let Some(t) = map.tag(map.scenario) {
        println!("scnr tag:  {}", t.name);
    }
    let spawns = map.player_spawns()?;
    println!("spawns:    {}", spawns.len());
    if let Some(s) = spawns.first() {
        println!(
            "spawn 0:   {:?} facing {:.0} deg",
            s.position,
            s.facing.to_degrees()
        );
    }
    Ok(())
}

fn tags(path: &str, group: Option<&str>) -> Res {
    let map = CacheFile::open(path)?;
    let filter = group.and_then(GroupTag::parse);
    let mut out = io::stdout().lock();
    for t in &map.tags {
        if filter.is_some_and(|g| g != t.group) || t.group.is_none() {
            continue;
        }
        writeln!(
            out,
            "{:08x}  {}  {:>8}  {}",
            t.datum.0, t.group, t.size, t.name
        )?;
    }
    Ok(())
}

fn groups(path: &str) -> Res {
    let map = CacheFile::open(path)?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in map.tags.iter().filter(|t| !t.group.is_none()) {
        *counts.entry(t.group.to_string()).or_default() += 1;
    }
    let mut out = io::stdout().lock();
    for (g, n) in counts {
        writeln!(out, "{g}  {n}")?;
    }
    Ok(())
}

fn check(path: &str) -> Res {
    let mut map = CacheFile::open(path)?;
    let stored: Vec<_> = map.tags.iter().filter(|t| t.has_data()).cloned().collect();
    let mut bad = 0;
    for t in &stored {
        if let Err(e) = map.read_tag_data(t) {
            bad += 1;
            eprintln!("  {} {}: {e}", t.group, t.name);
        }
    }
    println!(
        "{} tags total, {} stored in this map, {} readable, {} failed",
        map.tags.len(),
        stored.len(),
        stored.len() - bad,
        bad
    );
    Ok(())
}

fn scan(dir: &str) -> Res {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("map")))
        .collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy();
        match CacheFile::open(&p) {
            Ok(m) => println!(
                "OK    {name:<28} {:<16?} {:>5} tags  {}",
                m.header.map_type,
                m.tags.len(),
                m.header.scenario_name
            ),
            Err(e) => println!("FAIL  {name:<28} {e}"),
        }
    }
    Ok(())
}

fn level_mesh(path: &str) -> Result<blam_cache::geometry::Mesh, Box<dyn std::error::Error>> {
    let mut map = CacheFile::open(path)?;
    let mut mesh = blam_cache::geometry::Mesh::default();
    for bsp in map.structure_bsps()? {
        let m = map.bsp_collision_mesh(&bsp)?;
        println!(
            "bsp {:08x}: {} vertices, {} triangles",
            bsp.sbsp.0,
            m.positions.len(),
            m.triangle_count()
        );
        mesh.append(&m);
    }
    if let Some((lo, hi)) = mesh.bounds() {
        println!("bounds: {lo:?} .. {hi:?}");
    }
    Ok(mesh)
}

fn obj(path: &str, out: &str) -> Res {
    std::fs::write(out, level_mesh(path)?.to_obj())?;
    Ok(())
}

/// Write a bitmap's first image as PNGs: colour, then each channel alone.
fn bitmap_png(path: &str, name: &str, out: &str) -> Res {
    use blam_cache::{bitmap, MapSet};
    let mut set = MapSet::open(path)?;
    let tag = find_tag(&set, "bitm", name).ok_or("no bitmap with that name")?;
    let img = bitmap::read_bitmap(&mut set, tag.datum)?;
    let (w, h) = (img.width as usize, img.height as usize);
    let rgb: Vec<u8> = img
        .rgba
        .chunks(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    render::write_png(&format!("{out}.png"), &rgb, w, h)?;
    for (c, label) in ["r", "g", "b", "a"].iter().enumerate() {
        let ch: Vec<u8> = img.rgba.chunks(4).flat_map(|p| [p[c]; 3]).collect();
        render::write_png(&format!("{out}_{label}.png"), &ch, w, h)?;
    }
    println!("{} {w}x{h} -> {out}.png and {out}_[rgba].png", tag.name);
    Ok(())
}

fn render_png(path: &str, out: &str) -> Res {
    let mesh = level_mesh(path)?;
    let (w, h) = (1280, 720);
    // Kill floors / soft ceilings sit well outside the playable space; skip the lowest slab.
    let clip = mesh.bounds().map(|(lo, _)| lo[2] + 1.0);
    render::write_png(out, &render::render(&mesh, w, h, clip), w, h)?;
    println!("wrote {out}");
    Ok(())
}

fn hud(path: &str, name: &str, dump: Option<&str>) -> Res {
    use blam_cache::{bitmap, hud, MapSet};
    let mut set = MapSet::open(path)?;
    let nhdt = GroupTag::parse("nhdt").unwrap();
    let tag = set
        .map
        .tags
        .iter()
        .find(|t| t.group == nhdt && (t.name == name || t.name.ends_with(name)))
        .ok_or("no HUD tag with that name")?
        .clone();
    let widgets = hud::read_bitmap_widgets(&mut set, tag.datum)?;
    for w in &widgets {
        let bname = set
            .map
            .tag(w.bitmap)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        println!(
            "{:<26} {:?} flags {:#x} seq {:>2} offset {:?} reg {:?} {bname}",
            w.name, w.anchor, w.flags, w.sequence, w.offset, w.registration
        );
    }
    if let Some(dir) = dump {
        std::fs::create_dir_all(dir)?;
        for w in widgets {
            if w.bitmap == blam_cache::DatumIndex::NONE {
                continue;
            }
            let seqs = bitmap::read_sequences(&mut set, w.bitmap).unwrap_or_default();
            let index = usize::try_from(w.sequence)
                .ok()
                .and_then(|s| seqs.get(s))
                .map(|s| s.first_bitmap.max(0) as usize)
                .unwrap_or(0);
            match bitmap::read_bitmap_at(&mut set, w.bitmap, index) {
                Ok(img) => {
                    let out = format!("{dir}/{}.png", w.name);
                    render::write_rgba_png(&out, &img.rgba, img.width, img.height)?;
                    println!("wrote {out} ({}x{})", img.width, img.height);
                }
                Err(e) => println!("{}: {e}", w.name),
            }
        }
    }
    Ok(())
}

fn weapon(path: &str, name: &str) -> Res {
    use blam_cache::{weapon, MapSet};
    let mut set = MapSet::open(path)?;
    let weap = GroupTag::parse("weap").unwrap();
    let tags: Vec<_> = set
        .map
        .tags
        .iter()
        .filter(|t| t.group == weap && (t.name == name || t.name.ends_with(name)))
        .map(|t| t.datum)
        .collect();
    for datum in tags {
        let w = weapon::read_weapon(&mut set, datum)?;
        println!("{w:#?}");
        for b in &w.barrels {
            if b.projectile == blam_cache::DatumIndex::NONE {
                continue;
            }
            let p = weapon::read_projectile(&mut set, b.projectile)?;
            println!("{p:#?}");
            if p.impact_damage != blam_cache::DatumIndex::NONE {
                println!("{:#?}", weapon::read_damage(&mut set, p.impact_damage)?);
            }
        }
        let fx = weapon::read_weapon_effects(&mut set, datum)?;
        let name_of = |set: &blam_cache::MapSet, d: blam_cache::DatumIndex| {
            set.locate(d)
                .map_or("?".to_string(), |(_, t)| format!("{} {}", t.group, t.name))
        };
        for (what, e) in [
            ("fire", fx.fire),
            ("empty", fx.empty),
            ("reload", fx.reload),
            ("ready", fx.ready),
            ("pickup", fx.pickup),
            ("zoom in", fx.zoom_in),
            ("zoom out", fx.zoom_out),
        ] {
            let Some(e) = e else { continue };
            let sounds = blam_cache::sound::effect_sounds(&mut set, e)?;
            let names: Vec<String> = sounds.iter().map(|&d| name_of(&set, d)).collect();
            println!("{what}: {} -> {names:?}", name_of(&set, e));
        }
    }
    Ok(())
}

fn model(path: &str, name: &str, png: Option<&str>) -> Res {
    use blam_cache::{model, MapSet};
    let mut set = MapSet::open(path)?;
    // `group:name` picks between tags sharing a name.
    let tag = match name.split_once(':').and_then(|(g, n)| find_tag(&set, g, n)) {
        Some(t) => t,
        None => set
            .map
            .tags
            .iter()
            .find(|t| t.name == name)
            .or_else(|| set.map.tags.iter().find(|t| t.name.ends_with(name)))
            .ok_or("no tag with that name")?
            .clone(),
    };
    println!("{} {}", tag.group, tag.name);
    let mode = if tag.group == GroupTag::parse("mode").unwrap() {
        tag.datum
    } else {
        model::object_render_model(&mut set, tag.datum)?
    };
    let m = model::read_render_model(&mut set, mode)?;
    println!(
        "render model {:08x}: {} sections, {} triangles, {} shaders, {} nodes",
        mode.0,
        m.sections.len(),
        m.triangle_count(),
        m.shaders.len(),
        m.nodes.len()
    );
    for s in &m.sections {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &s.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let mut per_node = BTreeMap::new();
        for b in &s.bones {
            *per_node.entry(b[0]).or_insert(0) += 1;
        }
        println!(
            "  section: {} vertices, {} triangles, bounds {lo:.3?} .. {hi:.3?}, vertices per main node {per_node:?}",
            s.positions.len(),
            s.triangle_count()
        );
    }
    for (i, n) in m.nodes.iter().enumerate() {
        println!(
            "  node {i:>2} {:<24} parent {:>2} t {:.3?} q {:.3?}",
            n.name, n.parent, n.translation, n.rotation
        );
    }
    for g in &m.markers {
        if let Some(mk) = g.markers.first() {
            println!(
                "  marker {:<24} node {} at {:.3?} q {:.3?}",
                g.name, mk.node, mk.translation, mk.rotation
            );
        }
    }
    if let Some(out) = png {
        let mut mesh = blam_cache::geometry::Mesh::default();
        for s in &m.sections {
            let base = mesh.positions.len() as u32;
            mesh.positions.extend_from_slice(&s.positions);
            for p in &s.parts {
                mesh.indices.extend(p.indices.iter().map(|i| i + base));
            }
        }
        let (w, h) = (800, 600);
        render::write_png(out, &render::render(&mesh, w, h, None), w, h)?;
        println!("wrote {out}");
    }
    Ok(())
}

fn level(path: &str, dump: Option<&str>) -> Res {
    use blam_cache::{bitmap, shader, MapSet};
    let mut set = MapSet::open(path)?;
    println!("shared.map loaded: {}", set.shared.is_some());
    for bsp in set.map.structure_bsps()? {
        let geo = blam_cache::render::bsp_render_geometry(&mut set, &bsp)?;
        let tris: usize = geo.sections.iter().map(|s| s.triangle_count()).sum();
        println!(
            "bsp {:08x}: {} sections, {tris} triangles, {} materials",
            bsp.sbsp.0,
            geo.sections.len(),
            geo.shaders.len()
        );
        for (i, &sh) in geo.shaders.iter().enumerate() {
            let name = set.map.tag(sh).map(|t| t.name.clone()).unwrap_or_default();
            let info = shader::read_shader(&mut set, sh);
            let tex = match &info {
                Ok(shader::ShaderInfo {
                    diffuse: Some(b),
                    template,
                    blend,
                    illum,
                    ..
                }) => {
                    let bname = set.locate(*b).map(|(_, t)| t.name).unwrap_or_default();
                    let extra = format!(
                        "[{template} {blend:?}{}]",
                        if illum.is_some() { " glow" } else { "" }
                    );
                    match bitmap::read_bitmap(&mut set, *b) {
                        Ok(img) => {
                            if let Some(dir) = dump {
                                let file = format!("{dir}/{i:02}.png");
                                render::write_rgba_png(&file, &img.rgba, img.width, img.height)?;
                            }
                            format!("{bname} {}x{} {extra}", img.width, img.height)
                        }
                        Err(e) => format!("{bname} ERROR {e} {extra}"),
                    }
                }
                Ok(_) => "no diffuse".into(),
                Err(e) => format!("shader ERROR {e}"),
            };
            println!("  {i:2} {name}\n     -> {tex}");
        }
    }
    Ok(())
}

fn sim(path: &str) -> Res {
    use blam_cache::{physics, MapSet};
    use h2sim::{Input, Player, World};
    let mut set = MapSet::open(path)?;
    let movement = physics::player_movement(&mut set)?;
    let biped = physics::player_biped(&mut set)?;
    println!("{movement:?}\n{biped:?}");
    let mesh = level_mesh(path)?;
    let world = World::new(&mesh.positions, &mesh.indices);
    let spawns = set.map.player_spawns()?;
    let (mut landed, mut fell, mut walked) = (0, 0, 0.0f32);
    for s in &spawns {
        let start = glam::Vec3::from(s.position) + glam::Vec3::Z * 0.05;
        let mut p = Player::new(start, movement, biped);
        for _ in 0..120 {
            p.update(&world, Input::default(), 1.0 / 60.0);
        }
        if p.grounded && (p.position.z - start.z).abs() < 0.3 {
            landed += 1;
        }
        let before = p.position;
        let input = Input {
            movement: glam::Vec2::new(0.0, 1.0),
            yaw: s.facing,
            ..Default::default()
        };
        for _ in 0..180 {
            p.update(&world, input, 1.0 / 60.0);
        }
        if p.position.z < world.min.z + 1.0 {
            fell += 1;
        }
        walked += (p.position - before).truncate().length();
    }
    println!(
        "{} spawns: {landed} landed on ground, {fell} fell out of the world, avg 3s walk {:.2} units",
        spawns.len(),
        walked / spawns.len().max(1) as f32
    );
    Ok(())
}

fn objects(path: &str) -> Res {
    use blam_cache::{scenario, MapSet};
    let mut set = MapSet::open(path)?;
    let name = |set: &MapSet, d: blam_cache::DatumIndex| {
        set.locate(d)
            .map(|(_, t)| format!("{} {}", t.group, t.name))
            .unwrap_or_else(|| format!("{:08x}", d.0))
    };
    for p in scenario::scenery(&mut set)? {
        println!(
            "scenery {:<70} at {:7.2?} rot {:5.2?} scale {}",
            name(&set, p.object),
            p.position,
            p.rotation,
            p.scale
        );
    }
    for i in scenario::netgame_equipment(&mut set)? {
        let items = scenario::item_collection(&mut set, i.collection).unwrap_or_default();
        let first = items
            .first()
            .map(|(_, d)| name(&set, *d))
            .unwrap_or_default();
        println!(
            "item {:<50} -> {:<60} at {:7.2?} respawn {}s",
            name(&set, i.collection),
            first,
            i.position,
            i.respawn_seconds
        );
    }
    Ok(())
}

fn hex_dump(bytes: &[u8], limit: usize) {
    for (row, chunk) in bytes.chunks(32).enumerate().take(limit.div_ceil(32)) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        println!("  {:04x}: {}", row * 32, hex.join(" "));
    }
}

fn hex(path: &str, tag: &str, block_path: Option<&str>) -> Res {
    use blam_cache::{DatumIndex, MapSet};
    let mut set = MapSet::open(path)?;
    let datum = match tag.split_once(':') {
        Some((group, name)) => find_tag(&set, group, name).ok_or("no such tag")?.datum,
        None => DatumIndex(u32::from_str_radix(tag, 16)?),
    };
    let (src, t, mut data) = set.tag_data(datum)?;
    println!("{} {} ({} bytes)", t.group, t.name, data.len());
    let region = set.get(src).meta_region();
    let mut size = data.len();
    for step in block_path
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
    {
        let (offset, rest) = step.split_once(':').ok_or("path step needs offset:size")?;
        let (elem, index) = match rest.split_once('@') {
            Some((e, i)) => (e, Some(usize::from_str_radix(i, 16)?)),
            None => (rest, None),
        };
        let offset = usize::from_str_radix(offset, 16)?;
        size = usize::from_str_radix(elem, 16)?;
        let block = set.get(src).read_block(region, &data, offset, size)?;
        println!(
            "block @{offset:#x}: {} x {size:#x}",
            block.len() / size.max(1)
        );
        data = match index {
            Some(i) => block
                .get(i * size..(i + 1) * size)
                .ok_or("index past the block")?
                .to_vec(),
            None => block,
        };
    }
    let limit = std::env::var("H2_LIMIT")
        .ok()
        .and_then(|l| l.parse().ok())
        .unwrap_or(0x400);
    for (i, e) in data.chunks(size.max(1)).enumerate() {
        if i * size >= limit {
            break;
        }
        println!("[{i}]");
        hex_dump(e, size);
    }
    Ok(())
}

/// The multiplayer announcer events (globals' event blocks): type, event,
/// fields, display string and English sound.
/// Multiplayer game type points and the spawns' teams and game types.
fn netgame(path: &str) -> Res {
    use blam_cache::{scenario, MapSet};
    let mut set = MapSet::open(path)?;
    for f in scenario::netgame_flags(&mut set)? {
        let [x, y, z] = f.position;
        println!(
            "{:?} team {} id {} at ({x:.2}, {y:.2}, {z:.2}) facing {:.2}",
            f.kind, f.team, f.identifier, f.facing
        );
    }
    for s in set.map.player_spawns()? {
        let [x, y, z] = s.position;
        println!(
            "spawn team {} game types {:?} at ({x:.2}, {y:.2}, {z:.2})",
            s.team, s.game_types
        );
    }
    Ok(())
}

fn events(path: &str) -> Res {
    use blam_cache::{i16_at, u32_at, DatumIndex, MapSet};
    let mut set = MapSet::open(path)?;
    let tag = find_tag(&set, "mulg", "multiplayer\\multiplayer_globals").ok_or("no mulg")?;
    let (src, _, data) = set.tag_data(tag.datum)?;
    let names = [
        "general",
        "flavor",
        "slayer",
        "ctf",
        "oddball",
        "unused",
        "king",
        "race",
        "juggernaut",
        "territories",
        "assault",
    ];
    for (i, kind) in names.iter().enumerate() {
        let map = set.get(src);
        let region = map.meta_region();
        let block = map.read_block(region, &data, 0x134 + i * 8, 0xA8)?;
        println!("{kind} events: {}", block.len() / 0xA8);
        for e in block.as_chunks::<0xA8>().0 {
            let sid = |o: usize| {
                let id = u32_at(e, o);
                set.map.string_id(id).unwrap_or("").to_string()
            };
            let sound = DatumIndex(u32_at(e, 0x48));
            let sound = set.locate(sound).map(|(_, t)| t.name).unwrap_or_default();
            println!(
                "  type {} event {:2} | {:04x} {:04x} {:04x} | {:?} | {:08x} {:08x} | {}",
                i16_at(e, 2),
                i16_at(e, 4),
                i16_at(e, 6),
                i16_at(e, 8),
                i16_at(e, 0xA),
                sid(0xC),
                u32_at(e, 0x10),
                u32_at(e, 0x40),
                sound.trim_start_matches("sound\\dialog\\multiplayer\\")
            );
        }
    }
    Ok(())
}

/// Which tags refer to a tag: every place its datum index appears in the
/// tag data of the map and the shared maps.
fn refs(path: &str, tag: &str) -> Res {
    use blam_cache::mapset::Source;
    use blam_cache::MapSet;
    let mut set = MapSet::open(path)?;
    let (group, name) = tag.split_once(':').ok_or("tag as group:name")?;
    let target = find_tag(&set, group, name).ok_or("no such tag")?;
    println!("{} {} {:08x}", target.group, target.name, target.datum.0);
    let needle = target.datum.0.to_le_bytes();
    for src in [Source::Map, Source::Shared, Source::SpShared] {
        let present = match src {
            Source::Map => true,
            Source::Shared => set.shared.is_some(),
            Source::SpShared => set.sp_shared.is_some(),
        };
        if !present {
            continue;
        }
        let map = set.get(src);
        let region = map.meta_region();
        let meta = map.read_in(region, region.base, region.size as usize)?;
        let mut tags: Vec<_> = map.tags.iter().filter(|t| t.has_data()).cloned().collect();
        tags.sort_by_key(|t| t.address);
        for (i, w) in meta.windows(4).enumerate() {
            if w != needle {
                continue;
            }
            let address = region.base + i as u32;
            let owner = tags.iter().rev().find(|t| t.address <= address);
            match owner {
                Some(t) => println!(
                    "  {src:?} {:#x}: {} {} +{:#x}{}",
                    address,
                    t.group,
                    t.name,
                    address - t.address,
                    if address - t.address >= t.size {
                        " (past its size)"
                    } else {
                        ""
                    }
                ),
                None => println!("  {src:?} {address:#x}: before any tag"),
            }
        }
    }
    Ok(())
}

fn find_tag(set: &blam_cache::MapSet, group: &str, name: &str) -> Option<blam_cache::Tag> {
    let g = GroupTag::parse(group)?;
    let tags = &set.map.tags;
    tags.iter()
        .find(|t| t.group == g && t.name == name)
        .or_else(|| tags.iter().find(|t| t.group == g && t.name.ends_with(name)))
        .cloned()
}

fn jmad(path: &str, name: &str) -> Res {
    use blam_cache::{animation, MapSet};
    let mut set = MapSet::open(path)?;
    let tag = find_tag(&set, "jmad", name).ok_or("no jmad with that name")?;
    println!("{}", tag.name);
    let g = animation::read_animation_graph(&mut set, tag.datum)?;
    for (i, n) in g.nodes.iter().enumerate() {
        println!("  node {i:>2} {:<20} parent {:>3}", n.name, n.parent);
    }
    for (i, snd) in g.sounds.iter().enumerate() {
        let name = snd
            .and_then(|d| set.locate(d))
            .map_or("-".to_string(), |(_, t)| t.name);
        println!("  sound {i:>2} {name}");
    }
    if std::env::var("H2_EVENTS").is_ok() {
        for a in &g.animations {
            if !a.sound_events.is_empty() || !a.frame_events.is_empty() {
                println!(
                    "  {}: sounds {:?} frames {:?}",
                    a.name, a.sound_events, a.frame_events
                );
            }
        }
        return Ok(());
    }
    // H2_TRACK=<anim index> prints node 0's translation and rotation per frame.
    if let Some(k) = std::env::var("H2_TRACK")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        if let Some(a) = g.animations.get(k) {
            println!("{} ({} frames)", a.name, a.frame_count);
            for f in 0..a.frame_count {
                let t = a.translations[0].as_ref().map(|t| t.sample(f as f32));
                let r = a.rotations[0].as_ref().map(|t| t.sample(f as f32));
                println!("  frame {f:>3} pelvis t {t:.3?} r {r:.3?}");
            }
        }
        return Ok(());
    }
    for (i, a) in g.animations.iter().enumerate() {
        if std::env::var("H2_NODES").is_ok() {
            let list = |v: Vec<bool>| -> String {
                v.iter()
                    .enumerate()
                    .filter(|(_, &b)| b)
                    .map(|(i, _)| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            println!(
                "      rot [{}] trans [{}]",
                list(a.rotations.iter().map(Option::is_some).collect()),
                list(a.translations.iter().map(Option::is_some).collect())
            );
        }
        println!(
            "  anim {i:>3} {:<40} {:?} {} frames{} rot {} trans {} scale {}",
            a.name,
            a.kind,
            a.frame_count,
            if a.decoded { "" } else { " (not decoded)" },
            a.rotations.iter().flatten().count(),
            a.translations.iter().flatten().count(),
            a.scales.iter().flatten().count(),
        );
    }
    Ok(())
}

/// Decode every animation graph the map can see; report failures.
fn sound(path: &str, name: &str, out: Option<&str>) -> Res {
    use blam_cache::{sound::SoundReader, MapSet};
    let mut set = MapSet::open(path)?;
    let g = GroupTag::parse("snd!").unwrap();
    let tag = set
        .map
        .tags
        .iter()
        .find(|t| t.group == g && (t.name == name || t.name.ends_with(name)))
        .cloned()
        .ok_or_else(|| format!("no sound named {name}"))?;
    let s = SoundReader::new().read(&mut set, tag.datum)?;
    println!(
        "{}: class {}, {} Hz, {} channel(s), {:?}, distance {:?}, gain {} dB, {} permutation(s), {:.2} s",
        s.name,
        s.class,
        s.sample_rate,
        s.channels,
        s.codec,
        s.distance,
        s.gain_db,
        s.permutations.len(),
        s.duration()
    );
    for (i, p) in s.permutations.iter().enumerate() {
        let peak = p.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        let rms =
            (p.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / p.len().max(1) as f64).sqrt();
        // Sample-to-sample change, relative to loudness: high for noise.
        let diff = (p
            .windows(2)
            .map(|w| (w[1] as f64 - w[0] as f64).powi(2))
            .sum::<f64>()
            / p.len().max(1) as f64)
            .sqrt();
        println!(
            "  permutation {i}: {} samples, peak {peak}, rms {rms:.0}, roughness {:.2}",
            p.len(),
            diff / rms.max(1.0)
        );
    }
    for (i, e) in s.encoded.iter().enumerate() {
        let head: Vec<String> = e.iter().take(16).map(|b| format!("{b:02x}")).collect();
        println!(
            "  permutation {i}: {} bytes encoded, starts {}",
            e.len(),
            head.join(" ")
        );
    }
    if let (Some(out), Some(e)) = (out, s.encoded.first()) {
        std::fs::write(out, e)?;
        println!("wrote {out} (raw {:?} data)", s.codec);
    }
    if let (Some(out), Some(p)) = (out, s.permutations.first()) {
        let mut wav = Vec::new();
        let data_len = (p.len() * 2) as u32;
        let ch = s.channels as u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&(ch as u16).to_le_bytes());
        wav.extend_from_slice(&s.sample_rate.to_le_bytes());
        wav.extend_from_slice(&(s.sample_rate * ch * 2).to_le_bytes());
        wav.extend_from_slice(&((ch * 2) as u16).to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for v in p {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(out, wav)?;
        println!("wrote {out}");
    }
    Ok(())
}

fn soundscan(path: &str) -> Res {
    use blam_cache::{sound::SoundReader, MapSet};
    let mut set = MapSet::open(path)?;
    let g = GroupTag::parse("snd!").unwrap();
    let tags: Vec<_> = set
        .map
        .tags
        .iter()
        .filter(|t| t.group == g)
        .cloned()
        .collect();
    let mut reader = SoundReader::new();
    let mut codecs: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed = 0;
    for tag in &tags {
        match reader.read(&mut set, tag.datum) {
            Ok(s) => {
                if std::env::var("H2_LIST").is_ok() {
                    println!("{:?} class {} {}", s.codec, s.class, s.name);
                }
                // H2_DUMP=dir writes undecoded sounds' raw data there.
                if let Ok(dir) = std::env::var("H2_DUMP") {
                    for (i, e) in s.encoded.iter().enumerate() {
                        let name = s.name.replace('\\', "_");
                        std::fs::write(format!("{dir}/{name}_{i}.raw"), e)?;
                    }
                }
                *codecs.entry(format!("{:?}", s.codec)).or_default() += 1
            }
            Err(e) => {
                failed += 1;
                if failed <= 10 {
                    println!("{}: {e}", tag.name);
                }
            }
        }
    }
    println!("{} sounds, {failed} failed", tags.len());
    for (c, n) in codecs {
        println!("  {c}: {n}");
    }
    Ok(())
}

fn jmadscan(path: &str) -> Res {
    use blam_cache::{animation, MapSet};
    let mut set = MapSet::open(path)?;
    let g = GroupTag::parse("jmad").unwrap();
    let tags: Vec<_> = set
        .map
        .tags
        .iter()
        .filter(|t| t.group == g)
        .cloned()
        .collect();
    let (mut total, mut ok) = (0, 0);
    let mut failed = BTreeMap::new();
    for tag in tags {
        let graph = match animation::read_animation_graph(&mut set, tag.datum) {
            Ok(g) => g,
            Err(e) => {
                println!("{}: {e}", tag.name);
                continue;
            }
        };
        for a in &graph.animations {
            total += 1;
            if a.decoded {
                ok += 1;
            } else {
                *failed.entry(tag.name.clone()).or_insert(0) += 1;
            }
        }
    }
    println!("{ok} of {total} animations decoded");
    for (name, n) in failed {
        println!("  {n:>4} not decoded in {name}");
    }
    Ok(())
}

/// Raw view of a shader's template, parameters and postprocess data.
fn shader_dump(path: &str, name: &str) -> Res {
    use blam_cache::{f32_at, i16_at, u32_at, DatumIndex, MapSet};
    let mut set = MapSet::open(path)?;
    let tag = find_tag(&set, "shad", name).ok_or("no shader with that name")?;
    println!("{}", tag.name);
    let (src, _, data) = set.tag_data(tag.datum)?;
    let tag_name = |set: &MapSet, d: u32| {
        set.locate(DatumIndex(d))
            .map(|(_, t)| t.name)
            .unwrap_or_else(|| format!("{d:08x}"))
    };
    println!("template {}", tag_name(&set, u32_at(&data, 4)));
    let file = set.get(src);
    let region = file.meta_region();
    let params = file.read_block(region, &data, 0x18, 0x28)?;
    let mut lines = Vec::new();
    for p in params.chunks(0x28) {
        lines.push((
            file.string_id(u32_at(p, 0)).unwrap_or("?").to_string(),
            i16_at(p, 4),
            u32_at(p, 0xC),
            f32_at(p, 0x10),
            [f32_at(p, 0x14), f32_at(p, 0x18), f32_at(p, 0x1C)],
        ));
    }
    let pp = file.read_block(region, &data, 0x20, 0x7C)?;
    let mut pp_lines = Vec::new();
    let mut pp_bitmaps = Vec::new();
    if let Some(p) = pp.get(..0x7C) {
        let bitmaps = file.read_block(region, p, 0x4, 0xC)?;
        for b in bitmaps.chunks(0xC) {
            pp_bitmaps.push(u32_at(b, 0));
        }
        let consts = file.read_block(region, p, 0xC, 0x4)?;
        for c in consts.chunks(4) {
            pp_lines.push(format!("  pp pixel constant argb {:02x?}", c));
        }
    }
    for (n, ty, bitmap, value, color) in lines {
        println!(
            "  param {n:<28} type {ty} bitmap {} value {value} color {color:.3?}",
            tag_name(&set, bitmap)
        );
    }
    for b in pp_bitmaps {
        println!("  pp bitmap {}", tag_name(&set, b));
    }
    for l in pp_lines {
        println!("{l}");
    }
    Ok(())
}

/// Raw view of each BSP's lightmap groups.
fn lightmap_dump(path: &str) -> Res {
    use blam_cache::{i16_at, i32_at, u32_at, DatumIndex, MapSet};
    let mut set = MapSet::open(path)?;
    let bsps = set.map.structure_bsps()?;
    for bsp in bsps {
        let (src, tag, data) = set.tag_data(bsp.lightmap)?;
        println!("{} ({} bytes)", tag.name, data.len());
        let file = set.get(src);
        let region = file.meta_region();
        let groups = file.read_block(region, &data, 0x80, 0x68)?;
        let mut bitmaps = Vec::new();
        for (gi, g) in groups.chunks(0x68).enumerate() {
            let palettes = i32_at(g, 0x8);
            let writable = i32_at(g, 0x10);
            let bitmap = DatumIndex(u32_at(g, 0x1C));
            let clusters = i32_at(g, 0x20);
            let info = file.read_block(region, g, 0x28, 4)?;
            let buckets = i32_at(g, 0x40);
            println!(
                "  group {gi}: palettes {palettes} writable {writable} bitmap {:08x} clusters {clusters} render info {} buckets {buckets} instances {} ",
                bitmap.0,
                info.len() / 4,
                i32_at(g, 0x48),
            );
            for (i, r) in info.chunks(4).take(12).enumerate() {
                println!(
                    "    cluster {i}: bitmap {} palette {}",
                    i16_at(r, 0),
                    r[2] as i8
                );
            }
            let inst = file.read_block(region, g, 0x48, 4)?;
            let list: Vec<String> = inst
                .chunks(4)
                .map(|r| format!("{}/{}", i16_at(r, 0), r[2] as i8))
                .collect();
            println!("    instances (bitmap/palette): {}", list.join(" "));
            let refs = file.read_block(region, g, 0x50, 0xC)?;
            for (i, r) in refs.chunks(0xC).enumerate() {
                let offsets = file.read_block(region, r, 0x4, 2)?;
                let o: Vec<i16> = offsets.chunks(2).map(|c| i16_at(c, 0)).collect();
                println!(
                    "    bucket ref {i}: flags {:04x} bucket {} offsets {o:?}",
                    i16_at(r, 0),
                    i16_at(r, 2)
                );
            }
            let buckets = file.read_block(region, g, 0x40, 0x38)?;
            for (i, b) in buckets.chunks(0x38).enumerate() {
                let res = file.read_block(region, b, 0x1C, 0x10)?;
                let r: Vec<(i16, i16, i32)> = res
                    .chunks(0x10)
                    .map(|c| (i16_at(c, 4), i16_at(c, 6), i32_at(c, 8)))
                    .collect();
                println!(
                    "    bucket {i}: flags {:04x} block {:08x} size {} section {} res {} {r:?}",
                    i16_at(b, 0),
                    u32_at(b, 0xC),
                    i32_at(b, 0x10),
                    i32_at(b, 0x14),
                    i32_at(b, 0x18)
                );
            }
            bitmaps.push(bitmap);
        }
        for b in bitmaps {
            let (bsrc, btag, bdata) = set.tag_data(b)?;
            println!("  bitmap {}", btag.name);
            let file = set.get(bsrc);
            let region = file.meta_region();
            let entries = file.read_block(region, &bdata, 68, 116)?;
            for (i, e) in entries.chunks(116).enumerate().take(8) {
                println!(
                    "    image {i}: {}x{} format {} flags {:04x} mips {} data {:08x} size {}",
                    i16_at(e, 4),
                    i16_at(e, 6),
                    i16_at(e, 12),
                    i16_at(e, 14),
                    i16_at(e, 20),
                    u32_at(e, 28),
                    i32_at(e, 52)
                );
            }
            println!("    {} images", entries.len() / 116);
            if let Ok(dir) = std::env::var("H2_DUMP") {
                for i in 0..4 {
                    let img = blam_cache::bitmap::read_bitmap_at(&mut set, b, i)?;
                    let rgb: Vec<u8> = img
                        .rgba
                        .chunks(4)
                        .flat_map(|p| [p[0], p[1], p[2]])
                        .collect();
                    render::write_png(
                        &format!("{dir}/lm_{i}.png"),
                        &rgb,
                        img.width as usize,
                        img.height as usize,
                    )?;
                    let a: Vec<u8> = img
                        .rgba
                        .chunks(4)
                        .flat_map(|p| [p[3], p[3], p[3]])
                        .collect();
                    render::write_png(
                        &format!("{dir}/lm_{i}_alpha.png"),
                        &a,
                        img.width as usize,
                        img.height as usize,
                    )?;
                }
            }
        }
    }
    Ok(())
}
