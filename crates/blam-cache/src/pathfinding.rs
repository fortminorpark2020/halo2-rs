//! The AI's map of where it can walk: each structure BSP's pathfinding
//! sectors (floor polygons) and the links (edges) between them.

use crate::{f32_at, i16_at, u32_at, CacheFile, Result, StructureBsp};
use std::io::{Read, Seek};

const SBSP_PATHFINDING: usize = 0xC4;
const PATHFINDING_SIZE: usize = 0x74;
const SECTOR_SIZE: usize = 0x8;
const LINK_SIZE: usize = 0x10;
const VERTEX_SIZE: usize = 0xC;
const OBJECT_SIZE: usize = 0x1C;

/// Sector flags.
pub const SECTOR_WALKABLE: u16 = 1 << 0;
pub const SECTOR_FLOOR: u16 = 1 << 4;
/// Link flags.
pub const LINK_BOTH_WALKABLE: u16 = 1 << 5;
pub const LINK_LEDGE: u16 = 1 << 10;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sector {
    pub flags: u16,
    pub first_link: u32,
}

/// An edge between two sectors: from `vertices.0` to `.1`, with `left`
/// on one side and `right` on the other. `forward` is the next edge
/// around the left sector, `reverse` the next around the right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    pub vertices: (u16, u16),
    pub flags: u16,
    pub forward: u16,
    pub reverse: u16,
    pub left: Option<u16>,
    pub right: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pathfinding {
    pub sectors: Vec<Sector>,
    pub links: Vec<Link>,
    pub vertices: Vec<[f32; 3]>,
    /// Sectors (first, last) on objects (in the object's own frame), not
    /// the level.
    pub object_sectors: Vec<(u32, u32)>,
}

impl Pathfinding {
    /// The corners of a sector, in order, by walking its edge ring.
    pub fn polygon(&self, sector: usize) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        let Some(s) = self.sectors.get(sector) else {
            return out;
        };
        let mut e = s.first_link as usize;
        for _ in 0..self.links.len().min(256) {
            let Some(l) = self.links.get(e) else {
                break;
            };
            let (v, next) = if l.left == Some(sector as u16) {
                (l.vertices.0, l.forward)
            } else {
                (l.vertices.1, l.reverse)
            };
            if let Some(&p) = self.vertices.get(v as usize) {
                out.push(p);
            }
            e = next as usize;
            if e == s.first_link as usize {
                break;
            }
        }
        out
    }
}

fn index(b: &[u8], o: usize) -> Option<u16> {
    u16::try_from(i16_at(b, o)).ok()
}

impl<R: Read + Seek> CacheFile<R> {
    /// A structure BSP's pathfinding data (empty where it has none).
    pub fn bsp_pathfinding(&mut self, bsp: &StructureBsp) -> Result<Pathfinding> {
        let region = bsp.region;
        let sbsp = self.read_in(region, bsp.bsp_address, 0x23C)?;
        let data = self.read_block(region, &sbsp, SBSP_PATHFINDING, PATHFINDING_SIZE)?;
        let mut out = Pathfinding::default();
        for d in data.as_chunks::<PATHFINDING_SIZE>().0 {
            let base = out.vertices.len() as u16;
            let first = out.links.len() as u32;
            let sector_base = out.sectors.len() as u16;
            let sectors = self.read_block(region, d, 0x0, SECTOR_SIZE)?;
            let links = self.read_block(region, d, 0x8, LINK_SIZE)?;
            let vertices = self.read_block(region, d, 0x28, VERTEX_SIZE)?;
            let objects = self.read_block(region, d, 0x30, OBJECT_SIZE)?;
            out.object_sectors
                .extend(objects.as_chunks::<OBJECT_SIZE>().0.iter().map(|o| {
                    let base = u32::from(sector_base);
                    (u32_at(o, 4) + base, u32_at(o, 8) + base)
                }));
            out.sectors
                .extend(sectors.as_chunks::<SECTOR_SIZE>().0.iter().map(|s| Sector {
                    flags: i16_at(s, 0) as u16,
                    first_link: u32_at(s, 4) + first,
                }));
            let link_base = first as u16;
            out.links
                .extend(links.as_chunks::<LINK_SIZE>().0.iter().map(|l| Link {
                    vertices: (i16_at(l, 0) as u16 + base, i16_at(l, 2) as u16 + base),
                    flags: i16_at(l, 4) as u16,
                    forward: i16_at(l, 8) as u16 + link_base,
                    reverse: i16_at(l, 0xA) as u16 + link_base,
                    left: index(l, 0xC).map(|s| s + sector_base),
                    right: index(l, 0xE).map(|s| s + sector_base),
                }));
            out.vertices.extend(
                vertices
                    .as_chunks::<VERTEX_SIZE>()
                    .0
                    .iter()
                    .map(|v| [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)]),
            );
        }
        Ok(out)
    }
}
