//! HUD definitions (`nhdt`): the crosshair, ammo meter and scope overlays a
//! weapon draws, as bitmaps placed relative to screen anchors.

use crate::mapset::MapSet;
use crate::{f32_at, i16_at, u32_at, DatumIndex, Result};

const NHDT_BITMAP_WIDGETS: usize = 0x8;
const BITMAP_WIDGET_SIZE: usize = 0x64;

/// Where on screen a widget is positioned from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    HealthAndShield,
    WeaponHud,
    MotionSensor,
    Scoreboard,
    Crosshair,
    LockOnTarget,
    Other(i16),
}

impl From<i16> for Anchor {
    fn from(v: i16) -> Self {
        match v {
            0 => Anchor::HealthAndShield,
            1 => Anchor::WeaponHud,
            2 => Anchor::MotionSensor,
            3 => Anchor::Scoreboard,
            4 => Anchor::Crosshair,
            5 => Anchor::LockOnTarget,
            o => Anchor::Other(o),
        }
    }
}

pub const FLIP_HORIZONTALLY: u16 = 1;
pub const FLIP_VERTICALLY: u16 = 2;
pub const SCOPE_MIRROR_HORIZONTALLY: u16 = 4;
pub const SCOPE_MIRROR_VERTICALLY: u16 = 8;
pub const SCOPE_STRETCH: u16 = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct BitmapWidget {
    pub name: String,
    pub anchor: Anchor,
    pub flags: u16,
    pub bitmap: DatumIndex,
    /// Sequence of the bitmap tag to draw (full-screen layout).
    pub sequence: i8,
    /// Pixels from the anchor, in the 640x480 layout.
    pub offset: [i16; 2],
    /// Which point of the image sits at the offset (0..1 of its size).
    pub registration: [f32; 2],
}

pub fn read_bitmap_widgets(set: &mut MapSet, nhdt: DatumIndex) -> Result<Vec<BitmapWidget>> {
    let (src, _, d) = set.tag_data(nhdt)?;
    let file = set.get(src);
    let region = file.meta_region();
    let raw = file.read_block(region, &d, NHDT_BITMAP_WIDGETS, BITMAP_WIDGET_SIZE)?;
    Ok(raw
        .as_chunks::<BITMAP_WIDGET_SIZE>()
        .0
        .iter()
        .map(|w| BitmapWidget {
            name: file.string_id(u32_at(w, 0)).unwrap_or_default().to_string(),
            anchor: Anchor::from(i16_at(w, 0x1C)),
            flags: u16::from_le_bytes([w[0x1E], w[0x1F]]),
            bitmap: DatumIndex(u32_at(w, 0x24)),
            sequence: w[0x30] as i8,
            offset: [i16_at(w, 0x34), i16_at(w, 0x36)],
            registration: [f32_at(w, 0x40), f32_at(w, 0x44)],
        })
        .collect())
}
