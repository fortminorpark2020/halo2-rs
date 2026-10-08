//! HUD definitions (`nhdt`): the crosshair, ammo meter and scope overlays a
//! weapon draws, as bitmaps placed relative to screen anchors.

use crate::bitmap::Sequence;
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

/// Halo 2's three HUD layouts. Each widget has its own bitmap sequence,
/// offset and registration point for a view that fills the screen, one that
/// takes half of it (two players, or the first of three) and a quarter (the
/// other two of three, or four players). Index a widget's arrays with
/// `split as usize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSplit {
    Full,
    Half,
    Quarter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitmapWidget {
    pub name: String,
    pub anchor: Anchor,
    pub flags: u16,
    pub bitmap: DatumIndex,
    /// Sequence of the bitmap tag to draw in each layout; negative hides
    /// the widget in that layout.
    pub sequence: [i8; 3],
    /// Where the widget sits from its anchor in each layout. Full screen
    /// offsets count pixels of the HUD's bitmaps (Halo 2 draws them for a
    /// 1280x960 screen); half and quarter screen ones count two.
    pub offset: [[i16; 2]; 3],
    /// Which point of the image sits at the offset (0..1 of its size), in
    /// each layout.
    pub registration: [[f32; 2]; 3],
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
            sequence: [w[0x30] as i8, w[0x31] as i8, w[0x32] as i8],
            offset: [0x34, 0x38, 0x3C].map(|o| [i16_at(w, o), i16_at(w, o + 2)]),
            registration: [0x40, 0x48, 0x50].map(|o| [f32_at(w, o), f32_at(w, o + 4)]),
        })
        .collect())
}

/// The image of a widget's bitmap that it draws for `sequence`, picked as
/// Halo 2 does: the sequence's first image, or image 0 of a bitmap with no
/// sequences at all. None hides the widget (a negative sequence, one the
/// bitmap doesn't have, or one with no images).
pub fn widget_image(sequences: &[Sequence], sequence: i8) -> Option<usize> {
    let s = usize::try_from(sequence).ok()?;
    match sequences.get(s) {
        Some(seq) if seq.bitmap_count > 0 => usize::try_from(seq.first_bitmap).ok(),
        Some(_) => None,
        None => (s == 0 && sequences.is_empty()).then_some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(first_bitmap: i16, bitmap_count: i16) -> Sequence {
        Sequence {
            name: String::new(),
            first_bitmap,
            bitmap_count,
            sprites: Vec::new(),
        }
    }

    #[test]
    fn widget_images_follow_the_sequence() {
        // The motion sensor: full screen art, then the smaller splitscreen
        // art; the shield meter: a Spartan's and an Elite's image each.
        let seqs = [seq(0, 1), seq(1, 1), seq(2, 2)];
        assert_eq!(widget_image(&seqs, 1), Some(1));
        assert_eq!(widget_image(&seqs, 2), Some(2));
        assert_eq!(widget_image(&seqs, -1), None);
        assert_eq!(widget_image(&seqs, 3), None);
        assert_eq!(widget_image(&[seq(4, 0)], 0), None);
        // An ammo meter's bitmap has no sequences: its one image.
        assert_eq!(widget_image(&[], 0), Some(0));
        assert_eq!(widget_image(&[], 1), None);
    }
}
