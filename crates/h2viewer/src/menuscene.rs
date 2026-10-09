//! The scene behind the menus, as Halo 2 showed it: mainmenu.map's,
//! flown through by the map's own script. It's read in the background and
//! stays on the GPU beside the level being played; without it, the menus
//! show the level from a circling camera.

use crate::flythrough::Flythrough;
use crate::gpu::{DrawCall, Fx, Gpu};
use crate::scene::Backdrop;
use glam::{Mat4, Vec3};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// mainmenu.map's scene once it's on the GPU.
struct Shown {
    sky: Option<usize>,
    world: Vec<DrawCall>,
    flythrough: Flythrough,
}

#[derive(Default)]
pub struct MenuScene {
    /// mainmenu.map, while it's read.
    loading: Option<Receiver<Option<Backdrop>>>,
    shown: Option<Shown>,
    /// Seconds into the flythrough: it starts over after each game.
    time: f32,
    /// Where the camera holds instead (for screenshots).
    held: Option<f32>,
}

/// What the menus' view shows: the sky, what's in the world, and the
/// camera (position, forward, up).
pub struct MenuView<'a> {
    pub sky: Option<usize>,
    pub world: &'a [DrawCall],
    pub camera: (Vec3, Vec3, Vec3),
}

impl MenuScene {
    /// Start reading mainmenu.map, next to the other maps, if it's there.
    /// `held`: seconds into the flythrough the camera stays at.
    pub fn load(maps: &Path, held: Option<f32>) -> MenuScene {
        let path = maps.join("mainmenu.map");
        let loading = path.exists().then(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                let backdrop = Backdrop::load(&path);
                match &backdrop {
                    Ok(_) => println!(
                        "menus: {} read in {:.1} s",
                        path.display(),
                        started.elapsed().as_secs_f32()
                    ),
                    Err(e) => println!("warning: {}: {e}", path.display()),
                }
                let _ = tx.send(backdrop.ok());
            });
            rx
        });
        MenuScene {
            loading,
            held,
            ..MenuScene::default()
        }
    }

    /// Once read, put it on the GPU (if it has its flythrough; otherwise
    /// the menus go on showing the level).
    pub fn poll(&mut self, gpu: Option<&mut Gpu>) {
        let Some(rx) = &self.loading else {
            return;
        };
        let backdrop = match rx.try_recv() {
            Ok(b) => b,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        self.loading = None;
        let (Some(mut b), Some(gpu)) = (backdrop, gpu) else {
            return;
        };
        let Some(flythrough) = b.flythrough.take() else {
            println!("warning: mainmenu.map has no flythrough; the menus show the level");
            return;
        };
        gpu.load_backdrop(&b);
        let draw = |mesh, model, light| DrawCall {
            mesh,
            model,
            light,
            colors: None,
            emblem: None,
            fx: Fx::default(),
        };
        let mut world = vec![draw(0, Mat4::IDENTITY, None)];
        world.extend(b.objects.iter().map(|o| draw(o.mesh, o.transform, o.light)));
        self.shown = Some(Shown {
            sky: b.sky,
            world,
            flythrough,
        });
    }

    /// The flythrough's clock runs while the menus are up, and starts over
    /// when they're `hidden` (in a game, or behind the intro).
    pub fn advance(&mut self, dt: f32, hidden: bool) {
        self.time = if hidden { 0.0 } else { self.time + dt };
    }

    pub fn view(&self) -> Option<MenuView<'_>> {
        let s = self.shown.as_ref()?;
        Some(MenuView {
            sky: s.sky,
            world: &s.world,
            camera: s.flythrough.camera(self.held.unwrap_or(self.time)),
        })
    }
}
