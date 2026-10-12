//! The menu scene drawn offscreen as the lobby draws it, to PNGs, with
//! each frame's mean colour and times: for checking the look on a PC (or
//! tuning the veil) against a screenshot of Halo 2's own menu.
//!
//!   cargo run --release -p h2scene --example scene_shot -- <mainmenu.map or its folder> <out folder> [WxH] [seconds ...]
//!
//! `H2LOBBY_VEIL=r,g,b,k` changes the veil (`render::Veil::parse`), `off`
//! leaves it out. A software adapter (lavapipe, WARP) is allowed here.
//! The PNGs are pictures of the game's files: keep them on the PC, never
//! in the repository.

use h2scene::{Eye, Renderer, Veil};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(map), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!(
            "usage: scene_shot <mainmenu.map or its folder> <out folder> [WxH] [seconds ...]"
        );
        std::process::exit(2);
    };
    let mut path = PathBuf::from(map);
    if path.is_dir() {
        path = path.join("mainmenu.map");
    }
    let mut rest = &args[2..];
    let (mut w, mut h) = (1920u32, 1080u32);
    if let Some((a, b)) = rest.first().and_then(|s| s.split_once('x')) {
        if let (Ok(a), Ok(b)) = (a.parse(), b.parse()) {
            (w, h) = (a, b);
            rest = &rest[1..];
        }
    }
    let mut times: Vec<f64> = rest.iter().filter_map(|s| s.parse().ok()).collect();
    let veil = match std::env::var("H2LOBBY_VEIL") {
        Ok(v) if v == "off" => Veil::NONE,
        Ok(v) => Veil::parse(&v).unwrap_or_else(|| {
            eprintln!("H2LOBBY_VEIL: want r,g,b,k");
            std::process::exit(2);
        }),
        Err(_) => Veil::menu(),
    };
    let started = Instant::now();
    let scene = match h2scene::load(&path, &|l| println!("{l}")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!("read in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
    if times.is_empty() {
        // Every 5 seconds of the loop.
        let length = scene.flythrough.seconds();
        times = (0..)
            .map(|k| k as f64 * 5.0)
            .take_while(|&t| t < length)
            .collect();
    }
    let opened = Instant::now();
    let mut r = match Renderer::new(true, veil) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = r.upload(scene.parts()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    println!(
        "{} opened and the scene uploaded in {:.0} ms; veil {veil:?}",
        r.adapter(),
        opened.elapsed().as_secs_f64() * 1000.0
    );
    let out = Path::new(out);
    let _ = std::fs::create_dir_all(out);
    let mut total = [0.0f64; 3];
    for &t in &times {
        let eye = Eye::flythrough(&scene.flythrough, t as f32);
        let frame = match r.render(w, h, eye) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        };
        let mean = mean(&frame.px);
        for c in 0..3 {
            total[c] += mean[c];
        }
        let file = out.join(format!("scene-{w}x{h}-{t:05.1}s.png"));
        if let Err(e) = save_png(&file, w, h, &frame.px) {
            eprintln!("{}: {e}", file.display());
        }
        println!(
            "{t:6.1} s: mean ({:.0}, {:.0}, {:.0}), {:.1} ms to draw, {:.1} ms to read back: {}",
            mean[0],
            mean[1],
            mean[2],
            frame.render_ms,
            frame.readback_ms,
            file.display()
        );
        r.recycle(frame.px);
    }
    let n = times.len().max(1) as f64;
    println!(
        "mean over {} frames: ({:.0}, {:.0}, {:.0})",
        times.len(),
        total[0] / n,
        total[1] / n,
        total[2] / n
    );
}

/// The mean red, green and blue of 0x00RRGGBB pixels.
fn mean(px: &[u32]) -> [f64; 3] {
    let mut sum = [0u64; 3];
    for p in px {
        sum[0] += u64::from((p >> 16) & 0xFF);
        sum[1] += u64::from((p >> 8) & 0xFF);
        sum[2] += u64::from(p & 0xFF);
    }
    let n = px.len().max(1) as f64;
    sum.map(|s| s as f64 / n)
}

fn save_png(path: &Path, w: u32, h: u32, px: &[u32]) -> Result<(), String> {
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
