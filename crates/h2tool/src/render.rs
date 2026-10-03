//! Tiny software rasterizer for previewing level geometry without a GPU.

use blam_cache::geometry::Mesh;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Render `mesh` (Halo axes: z up) from an elevated three-quarter view.
/// Returns RGB pixels. `clip_z` drops geometry below a height (kill floors).
pub fn render(mesh: &Mesh, w: usize, h: usize, clip_z: Option<f32>) -> Vec<u8> {
    let mut rgb = vec![0u8; w * h * 3];
    for px in rgb.chunks_exact_mut(3) {
        px.copy_from_slice(&[24, 28, 36]);
    }
    let keep =
        |t: &[u32]| clip_z.is_none_or(|z| t.iter().all(|&i| mesh.positions[i as usize][2] >= z));
    let tris: Vec<&[u32]> = mesh.indices.chunks_exact(3).filter(|t| keep(t)).collect();
    if tris.is_empty() {
        return rgb;
    }
    // Frame the dense middle of the level (10th..90th percentile per axis) so the
    // big outer cliffs/skybox shell don't shrink the playable area to a speck.
    let mut lo = [0f32; 3];
    let mut hi = [0f32; 3];
    for k in 0..3 {
        let mut v: Vec<f32> = tris
            .iter()
            .flat_map(|t| t.iter().map(|&i| mesh.positions[i as usize][k]))
            .collect();
        v.sort_by(f32::total_cmp);
        lo[k] = v[v.len() / 10];
        hi[k] = v[v.len() * 9 / 10];
    }
    let center = [
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    ];
    let radius = dot(sub(hi, lo), sub(hi, lo)).sqrt() / 2.0;
    let eye = [
        center[0] + radius * 1.1,
        center[1] - radius * 1.1,
        center[2] + radius * 0.9,
    ];
    let fwd = norm(sub(center, eye));
    let right = norm(cross(fwd, [0.0, 0.0, 1.0]));
    let up = cross(right, fwd);
    let focal = 1.0 / (35f32.to_radians()).tan();
    let aspect = w as f32 / h as f32;
    let light = norm([0.4, -0.3, 0.85]);

    let project = |p: V3| -> Option<V3> {
        let d = sub(p, eye);
        let z = dot(d, fwd);
        if z < 0.1 {
            return None;
        }
        let x = dot(d, right) * focal / z / aspect;
        let y = dot(d, up) * focal / z;
        Some([(x + 1.0) * 0.5 * w as f32, (1.0 - y) * 0.5 * h as f32, z])
    };

    let mut depth = vec![f32::MAX; w * h];
    for t in tris {
        let p: Vec<V3> = t.iter().map(|&i| mesh.positions[i as usize]).collect();
        let n = norm(cross(sub(p[1], p[0]), sub(p[2], p[0])));
        // Back-face cull so the level's outer shell doesn't hide the playable space.
        if dot(n, sub(p[0], eye)) >= 0.0 {
            continue;
        }
        let shade = 0.25 + 0.75 * dot(n, light).abs();
        let height = ((p[0][2] - lo[2]) / (hi[2] - lo[2]).max(1e-3)).clamp(0.0, 1.0);
        let base = [
            150.0 + 60.0 * height,
            160.0 + 40.0 * height,
            175.0 - 30.0 * height,
        ];
        let color = base.map(|c| (c * shade).min(255.0) as u8);
        let (Some(a), Some(b), Some(c)) = (project(p[0]), project(p[1]), project(p[2])) else {
            continue;
        };
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-6 {
            continue;
        }
        let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
        let x1 = (a[0].max(b[0]).max(c[0]).ceil() as usize).min(w - 1);
        let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
        let y1 = (a[1].max(b[1]).max(c[1]).ceil() as usize).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b[0] - fx) * (c[1] - fy) - (b[1] - fy) * (c[0] - fx)) / area;
                let w1 = ((c[0] - fx) * (a[1] - fy) - (c[1] - fy) * (a[0] - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * a[2] + w1 * b[2] + w2 * c[2];
                let i = y * w + x;
                if z < depth[i] {
                    depth[i] = z;
                    rgb[i * 3..i * 3 + 3].copy_from_slice(&color);
                }
            }
        }
    }
    rgb
}

pub fn write_png(path: &str, rgb: &[u8], w: usize, h: usize) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgb)?;
    Ok(())
}
