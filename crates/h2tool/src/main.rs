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
    }
    Ok(())
}

fn model(path: &str, name: &str, png: Option<&str>) -> Res {
    use blam_cache::{model, MapSet};
    let mut set = MapSet::open(path)?;
    let tag = set
        .map
        .tags
        .iter()
        .find(|t| t.name == name)
        .or_else(|| set.map.tags.iter().find(|t| t.name.ends_with(name)))
        .ok_or("no tag with that name")?
        .clone();
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
        println!(
            "  section: {} vertices, {} triangles, bounds {lo:.3?} .. {hi:.3?}",
            s.positions.len(),
            s.triangle_count()
        );
    }
    for (i, n) in m.nodes.iter().enumerate() {
        println!(
            "  node {i:>2} {:<24} parent {:>2} t {:.3?} q {:.3?}",
            set.map.string_id(n.name).unwrap_or("?"),
            n.parent,
            n.translation,
            n.rotation
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
                Ok(shader::ShaderInfo { diffuse: Some(b) }) => {
                    let bname = set.map.tag(*b).map(|t| t.name.clone()).unwrap_or_default();
                    match bitmap::read_bitmap(&mut set, *b) {
                        Ok(img) => {
                            if let Some(dir) = dump {
                                let file = format!("{dir}/{i:02}.png");
                                render::write_rgba_png(&file, &img.rgba, img.width, img.height)?;
                            }
                            format!("{bname} {}x{}", img.width, img.height)
                        }
                        Err(e) => format!("{bname} ERROR {e}"),
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
