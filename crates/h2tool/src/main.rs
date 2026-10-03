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
//!   h2tool scan  <maps folder>           summary line for every .map in a folder

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
        Some("level") if args.len() >= 2 => level(&args[1], args.get(2).map(String::as_str)),
        Some("obj") if args.len() >= 3 => obj(&args[1], &args[2]),
        Some("render") if args.len() >= 3 => render_png(&args[1], &args[2]),
        _ => {
            eprintln!(
                "usage: h2tool <info|tags|groups|check|obj|render|level|scan> <path> [group]"
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
