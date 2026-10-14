//! What h2scene reads from a mainmenu.map, and how long it takes:
//!
//!   cargo run --release -p h2scene --example scene_info -- <mainmenu.map or its maps folder>
//!
//! For checking the scene on a PC before the lobby draws it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let Some(arg) = std::env::args_os().nth(1) else {
        eprintln!("usage: scene_info <mainmenu.map or the folder holding it>");
        std::process::exit(2);
    };
    let mut path = PathBuf::from(arg);
    if path.is_dir() {
        path = path.join("mainmenu.map");
    }
    let started = Instant::now();
    let scene = match h2scene::load(&path, &|line| println!("{line}")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let ms = started.elapsed().as_secs_f64() * 1000.0;

    println!("{}", path.display());
    println!("BSPs: {}", scene.bsps);
    let (level, objects, sky) = scene.triangles();
    println!(
        "triangles: {} ({level} level, {objects} scenery as placed, {sky} sky)",
        level + objects + sky
    );
    println!(
        "textures: {} (one the fallback; {:.1} MB with mips), {} not decoded",
        scene.textures.len(),
        scene.texture_bytes() as f64 / 1e6,
        scene.failed_textures
    );
    let passes: Vec<String> = scene
        .materials_by_pass()
        .iter()
        .map(|(p, n)| format!("{n} {p:?}"))
        .collect();
    println!(
        "materials: {} ({})",
        scene.materials.len(),
        passes.join(", ")
    );
    let mut by_name: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for o in &scene.objects {
        let tris = scene.meshes.get(o.mesh).map_or(0, |m| m.triangle_count());
        let e = by_name.entry(o.name.as_str()).or_default();
        e.0 += 1;
        e.1 = tris;
    }
    println!("objects: {}", scene.objects.len());
    for (name, (count, tris)) in by_name {
        println!("  {count} x {name} ({tris} triangles each)");
    }
    let lit = scene.objects.iter().filter(|o| o.light.is_some()).count();
    println!("  {lit} lit by the floor under them");
    match scene.sky {
        Some(m) => println!(
            "sky: mesh {m}, {} batches",
            scene.meshes.get(m).map_or(0, |m| m.batches.len())
        ),
        None => println!("sky: none"),
    }
    let a = scene.atmosphere;
    match a.fog {
        Some(f) => println!(
            "fog: colour {:.3?}, density up to {:.2}, from {:.0} to {:.0} units",
            f.color, f.max_density, f.start, f.opaque
        ),
        None => println!("fog: none"),
    }
    if let Some(c) = a.clear_color {
        println!("clear colour: {c:.3?}");
    }
    let fly = &scene.flythrough;
    println!(
        "camera points: {} ({} flown through), moves in a loop: {}",
        fly.points(),
        fly.points_used(),
        fly.moves()
    );
    println!(
        "loop: {:.0} ticks, {:.1} seconds",
        fly.ticks(),
        fly.seconds()
    );
    let (at, forward, _) = fly.camera(0.0);
    println!("camera at the start: {at:.1} facing {forward:.2}");
    println!("load: {ms:.0} ms");
}
