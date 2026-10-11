//! Where the menus' pictures come from. A source looks a picture up by its
//! bitmap tag's name and frame and hands over its pixels; it is fed from
//! the player's own `mainmenu.map` (MCC's or Vista's) at run time, never
//! from files in the repository. Without one (`FlatArt`), every screen
//! draws flat shapes in Halo 2's colours instead.
//!
//! `read` fills a `MemoryArt` from either kind of map through
//! `blam_cache::ui::Pictures` (MCC's map with its textures.dat,
//! `MccPictures`, or Halo 2 Vista's `MapSet`), decoding only the images
//! the wanted frames show.

use crate::paint::Texture;
use blam_cache::ui::Pictures;

/// An image's pixels: RGBA8, straight alpha, the top row first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageRef<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
}

impl ImageRef<'_> {
    /// Whether it holds as many pixels as it says.
    pub fn is_whole(&self) -> bool {
        self.width > 0 && self.height > 0 && self.rgba.len() >= self.width * self.height * 4
    }
}

/// A picture found: its texture, how big it is (pixels, which are UI
/// units at scale 1), and which part of the texture it is (left, top,
/// right, bottom, 0 to 1: a sprite, or all of it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub texture: Texture,
    pub size: [f32; 2],
    pub uv: [f32; 4],
}

/// A source of pictures.
pub trait ArtSource {
    /// The picture of a bitmap tag (by its whole name, or by the last part
    /// of it) at `frame`: its sequence `frame`'s first sprite, or without
    /// sequences its image `frame`.
    fn picture(&self, name: &str, frame: usize) -> Option<Picture>;

    /// The pixels behind a `Texture::Art` id this source gave.
    fn image(&self, id: u32) -> Option<ImageRef<'_>>;
}

/// No pictures: the flat look.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlatArt;

impl ArtSource for FlatArt {
    fn picture(&self, _name: &str, _frame: usize) -> Option<Picture> {
        None
    }

    fn image(&self, _id: u32) -> Option<ImageRef<'_>> {
        None
    }
}

/// An image of a bitmap tag, decoded to RGBA8 (straight alpha).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// A part of one of a bitmap's images.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub image: usize,
    /// Left, top, right, bottom, 0 to 1 across and down the image.
    pub uv: [f32; 4],
}

/// A bitmap tag's sequence: its first image, and its sprites (often none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sequence {
    pub first_image: usize,
    pub sprites: Vec<Sprite>,
}

/// A bitmap tag: its images and sequences, as the integration reads them
/// from the tags.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bitmap {
    pub images: Vec<ArtImage>,
    pub sequences: Vec<Sequence>,
}

/// Pictures held in memory, by bitmap tag name: what the launcher fills
/// from `mainmenu.map`, and what tests make up.
#[derive(Clone, Debug, Default)]
pub struct MemoryArt {
    bitmaps: Vec<(String, Bitmap)>,
    /// Each bitmap's first image's texture id (images are numbered on
    /// across bitmaps).
    first_id: Vec<u32>,
    count: u32,
}

/// The last part of a tag name (after its last backslash).
pub(crate) fn last_part(name: &str) -> &str {
    name.rsplit(['\\', '/']).next().unwrap_or(name)
}

impl MemoryArt {
    pub fn new() -> MemoryArt {
        MemoryArt::default()
    }

    /// Adds a bitmap tag under its name (a second one by the same name
    /// replaces nothing: the first is found first).
    pub fn add(&mut self, name: &str, bitmap: Bitmap) {
        self.first_id.push(self.count);
        self.count += bitmap.images.len() as u32;
        self.bitmaps.push((name.to_string(), bitmap));
    }

    /// Adds a bitmap of one image (no sequences).
    pub fn add_image(&mut self, name: &str, image: ArtImage) {
        self.add(
            name,
            Bitmap {
                images: vec![image],
                sequences: Vec::new(),
            },
        );
    }

    /// How many bitmaps it holds.
    pub fn len(&self) -> usize {
        self.bitmaps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bitmaps.is_empty()
    }

    /// A bitmap's place, by its whole name, else by the last part of it.
    fn find(&self, name: &str) -> Option<usize> {
        self.bitmaps
            .iter()
            .position(|(n, _)| n == name)
            .or_else(|| {
                let short = last_part(name);
                self.bitmaps.iter().position(|(n, _)| last_part(n) == short)
            })
    }
}

impl ArtSource for MemoryArt {
    fn picture(&self, name: &str, frame: usize) -> Option<Picture> {
        let k = self.find(name)?;
        let bitmap = &self.bitmaps[k].1;
        let (image, uv) = match bitmap.sequences.get(frame) {
            Some(s) => match s.sprites.first() {
                Some(sprite) => (sprite.image, sprite.uv),
                None => (s.first_image, [0.0, 0.0, 1.0, 1.0]),
            },
            None if bitmap.sequences.is_empty() => (frame, [0.0, 0.0, 1.0, 1.0]),
            None => return None,
        };
        // An image that didn't decode is missing.
        let img = bitmap.images.get(image).filter(|i| {
            let whole = ImageRef {
                width: i.width,
                height: i.height,
                rgba: &i.rgba,
            };
            whole.is_whole()
        })?;
        let size = [
            img.width as f32 * (uv[2] - uv[0]).abs(),
            img.height as f32 * (uv[3] - uv[1]).abs(),
        ];
        Some(Picture {
            texture: Texture::Art(self.first_id[k] + image as u32),
            size,
            uv,
        })
    }

    fn image(&self, id: u32) -> Option<ImageRef<'_>> {
        // The last bitmap whose first id is at or before `id`.
        let k = self
            .first_id
            .partition_point(|&first| first <= id)
            .checked_sub(1)?;
        let img = self.bitmaps[k]
            .1
            .images
            .get((id - self.first_id[k]) as usize)?;
        Some(ImageRef {
            width: img.width,
            height: img.height,
            rgba: &img.rgba,
        })
    }
}

/// What `read` loaded: the pictures, and each bitmap that couldn't be had
/// with why (frames of a bitmap that did read but not wholly count too).
#[derive(Debug, Default)]
pub struct Loaded {
    pub art: MemoryArt,
    /// How many of the wanted bitmaps were found with every frame wanted.
    pub found: usize,
    pub wanted: usize,
    /// A bitmap's name, and why it (or one of its frames) is missing.
    pub missing: Vec<(String, String)>,
}

/// Each bitmap tag named in `wanted` (by its whole name, with the frame
/// used; a name may come more than once) from a mainmenu.map's
/// `pictures`, with the images its frames show decoded and the rest left
/// empty, and its sequences, a sprite's part of its image as
/// `blam_cache::ui::BitmapInfo::frame` cuts it.
pub fn read(pictures: &mut dyn Pictures, wanted: &[(String, usize)]) -> Loaded {
    let mut names: Vec<(&str, Vec<usize>)> = Vec::new();
    for (name, frame) in wanted {
        match names.iter_mut().find(|n| n.0.eq_ignore_ascii_case(name)) {
            Some(n) if !n.1.contains(frame) => n.1.push(*frame),
            Some(_) => {}
            None => names.push((name, vec![*frame])),
        }
    }
    let mut out = Loaded {
        wanted: names.len(),
        ..Loaded::default()
    };
    for (name, frames) in names {
        let info = match pictures.bitmap(name) {
            Ok(info) => info,
            Err(e) => {
                out.missing.push((name.to_string(), e.to_string()));
                continue;
            }
        };
        let mut images: Vec<ArtImage> = vec![ArtImage::default(); info.images.len()];
        let mut whole = true;
        for frame in frames {
            let shown = i16::try_from(frame).ok().and_then(|f| info.frame(f));
            let Some(shown) = shown else {
                out.missing
                    .push((name.to_string(), format!("it has no frame {frame}")));
                whole = false;
                continue;
            };
            if images[shown.image].width > 0 {
                continue;
            }
            match pictures.image(name, shown.image) {
                Ok(im) => {
                    images[shown.image] = ArtImage {
                        width: im.width as usize,
                        height: im.height as usize,
                        rgba: im.rgba,
                    };
                }
                Err(e) => {
                    let why = format!("image {}: {e}", shown.image);
                    out.missing.push((name.to_string(), why));
                    whole = false;
                }
            }
        }
        if images.iter().all(|i| i.width == 0) {
            continue;
        }
        out.found += usize::from(whole);
        let sizes: Vec<(u32, u32)> = info.images.iter().map(|i| (i.width, i.height)).collect();
        let sequences = info
            .sequences
            .iter()
            .map(|s| Sequence {
                first_image: usize::try_from(s.first_bitmap).unwrap_or(0),
                sprites: s
                    .sprites
                    .iter()
                    .map(|p| {
                        // A sprite on an image the bitmap hasn't stays
                        // so, and shows nothing.
                        let image = usize::try_from(p.bitmap).unwrap_or(usize::MAX);
                        let uv = match sizes.get(image) {
                            Some(&(w, h)) => {
                                let [x, y, sw, sh] = p.pixels(w, h).map(|v| v as f32);
                                let (w, h) = (w.max(1) as f32, h.max(1) as f32);
                                [x / w, y / h, (x + sw) / w, (y + sh) / h]
                            }
                            None => [0.0, 0.0, 1.0, 1.0],
                        };
                        Sprite { image, uv }
                    })
                    .collect(),
            })
            .collect();
        out.art.add(&info.name, Bitmap { images, sequences });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: usize, h: usize, value: u8) -> ArtImage {
        ArtImage {
            width: w,
            height: h,
            rgba: vec![value; w * h * 4],
        }
    }

    /// Pictures as a map would give them: a bitmap of two images, the
    /// second's sequence a sprite of its right half; a bitmap whose image
    /// won't decode; and the images decoded so far.
    struct Fake {
        decoded: Vec<(String, usize)>,
    }

    impl Pictures for Fake {
        fn bitmap(&mut self, name: &str) -> blam_cache::Result<blam_cache::ui::BitmapInfo> {
            use blam_cache::bitmap::{Format, ImageInfo, Sequence, Sprite};
            let info = |width, height| ImageInfo {
                width,
                height,
                format: Format::A8R8G8B8,
            };
            let sprite = Sprite {
                bitmap: 1,
                left: 0.5,
                right: 1.0,
                top: 0.0,
                bottom: 1.0,
                registration: [0.0; 2],
            };
            let sequence = |first_bitmap, sprites| Sequence {
                name: String::new(),
                first_bitmap,
                bitmap_count: 1,
                sprites,
            };
            match name {
                "ui\\brace" => Ok(blam_cache::ui::BitmapInfo {
                    name: "ui\\brace".into(),
                    images: vec![info(2, 2), info(8, 2)],
                    sequences: vec![sequence(0, Vec::new()), sequence(1, vec![sprite])],
                }),
                "ui\\broken" => Ok(blam_cache::ui::BitmapInfo {
                    name: "ui\\broken".into(),
                    images: vec![info(2, 2)],
                    sequences: Vec::new(),
                }),
                _ => Err(blam_cache::Error::Corrupt(format!("no bitmap {name}"))),
            }
        }

        fn image(
            &mut self,
            name: &str,
            index: usize,
        ) -> blam_cache::Result<blam_cache::bitmap::Image> {
            if name == "ui\\broken" {
                return Err(blam_cache::Error::Corrupt("a format of its own".into()));
            }
            self.decoded.push((name.to_string(), index));
            let (width, height) = [(2, 2), (8, 2)][index];
            Ok(blam_cache::bitmap::Image {
                width,
                height,
                rgba: vec![index as u8 + 1; (width * height * 4) as usize],
            })
        }
    }

    #[test]
    fn pictures_read_from_a_map_decode_only_the_frames_wanted() {
        let mut fake = Fake {
            decoded: Vec::new(),
        };
        let wanted = [
            ("ui\\brace".to_string(), 1),
            ("UI\\Brace".to_string(), 1),
            ("ui\\broken".to_string(), 0),
            ("ui\\missing".to_string(), 0),
            ("ui\\brace".to_string(), 5),
        ];
        let loaded = read(&mut fake, &wanted);
        // Frame 1 is image 1's sprite, decoded once; image 0 isn't wanted.
        assert_eq!(fake.decoded, [("ui\\brace".to_string(), 1)]);
        assert_eq!((loaded.found, loaded.wanted), (0, 3));
        let why: Vec<&str> = loaded.missing.iter().map(|m| m.1.as_str()).collect();
        assert_eq!(why.len(), 3, "{why:?}");
        assert!(why[0].contains("no frame 5"));
        assert!(why[1].contains("a format of its own"));
        assert!(why[2].contains("no bitmap"));
        let art = &loaded.art;
        assert_eq!(art.len(), 1, "the broken bitmap has no pictures");
        let p = art.picture("brace", 1).unwrap();
        assert_eq!(p.uv, [0.5, 0.0, 1.0, 1.0]);
        assert_eq!(p.size, [4.0, 2.0]);
        assert_eq!(art.image(1).unwrap().rgba[0], 2);
        // Frame 0's image wasn't decoded, so it is missing (drawn flat).
        assert!(art.picture("brace", 0).is_none());
        // Every frame found: a bitmap counts.
        let loaded = read(&mut fake, &[("ui\\brace".to_string(), 0)]);
        assert_eq!(loaded.found, 1);
        assert!(loaded.missing.is_empty());
        assert_eq!(loaded.art.picture("ui\\brace", 0).unwrap().size, [2.0, 2.0]);
    }

    #[test]
    fn the_flat_source_has_nothing() {
        assert!(FlatArt.picture("anything", 0).is_none());
        assert!(FlatArt.image(0).is_none());
    }

    #[test]
    fn pictures_are_found_by_name_frame_and_sprite() {
        let mut art = MemoryArt::new();
        art.add_image(
            "ui\\screens\\game_shell\\start_screen\\start_screen",
            image(8, 4, 1),
        );
        art.add(
            "ui\\screens\\game_shell\\track_brace",
            Bitmap {
                images: vec![image(4, 4, 2), image(16, 8, 3)],
                sequences: vec![
                    Sequence {
                        first_image: 0,
                        sprites: Vec::new(),
                    },
                    Sequence {
                        first_image: 1,
                        sprites: vec![Sprite {
                            image: 1,
                            uv: [0.5, 0.0, 1.0, 0.5],
                        }],
                    },
                ],
            },
        );
        assert_eq!(art.len(), 2);
        let logo = art
            .picture("ui\\screens\\game_shell\\start_screen\\start_screen", 0)
            .unwrap();
        assert_eq!(logo.texture, Texture::Art(0));
        assert_eq!(logo.size, [8.0, 4.0]);
        // By the last part of the name; frame 1 is the second sequence's
        // sprite: the top right quarter of image 1.
        let brace = art.picture("track_brace", 1).unwrap();
        assert_eq!(brace.texture, Texture::Art(2));
        assert_eq!(brace.size, [8.0, 4.0]);
        assert_eq!(brace.uv, [0.5, 0.0, 1.0, 0.5]);
        let first = art.picture("track_brace", 0).unwrap();
        assert_eq!((first.texture, first.size), (Texture::Art(1), [4.0, 4.0]));
        // A frame past the sequences, or an image past the images.
        assert!(art.picture("track_brace", 2).is_none());
        assert!(art.picture("start_screen", 1).is_none());
        assert!(art.picture("missing", 0).is_none());
        // The pixels behind each id.
        assert_eq!(art.image(0).unwrap().rgba[0], 1);
        assert_eq!(art.image(1).unwrap().width, 4);
        assert_eq!(art.image(2).unwrap().rgba[0], 3);
        assert!(art.image(2).unwrap().is_whole());
        assert!(art.image(3).is_none());
    }
}
