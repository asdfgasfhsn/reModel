//! `remodel run ... --texture-pack DIR --pack-log FILE`: draw every frame of
//! the run on the GPU with the pack, as the player does (its background
//! decoding, uploads and eviction; `model2::dirpack`), at the arcade's pace,
//! and write the pack diagnostic (`model2::packlog`) of what each frame's
//! lookups gave. Nothing is shown or saved but the log; `remodel
//! pack-flicker FILE` reads it.

use gpu::Renderer;
use model2::capture::Frame;
use model2::packlog::{Fixes, PackLog};
use model2::raster;
use model2::video::{self, View};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Run {
    gpu: Renderer,
    view: View,
    start: Instant,
    pace: bool,
    drawn: u64,
}

/// The arcade's frame time.
const FRAME: f64 = 1.0 / 57.524;

impl Run {
    pub fn new(
        pack: &Path,
        log: &Path,
        fixes: Fixes,
        view: View,
        pace: bool,
        filter: model2::enhance::TexFilter,
    ) -> Result<Run, String> {
        let mut gpu = Renderer::new()?;
        let dir = model2::dirpack::DirPack::open(pack)?;
        println!("pack log      {} images in {}, log to {}", dir.len(), pack.display(), log.display());
        gpu.set_pack(Some(Box::new(dir)));
        gpu.set_pack_fixes(fixes);
        gpu.set_pack_log(Some(PackLog::create(log).map_err(|e| format!("{}: {e}", log.display()))?));
        let mut q = gpu.quality();
        q.tex_filter = filter;
        gpu.set_quality(q);
        println!("pack fixes    {fixes:?}");
        Ok(Run { gpu, view, start: Instant::now(), pace, drawn: 0 })
    }

    /// Draw the machine's frame as the player would, then wait for the
    /// frame's time if pacing.
    pub fn frame(&mut self, m: &mut model2::Machine) {
        let f = Frame::grab(&mut m.bus);
        let src = f.sources(&m.bus.texture_rom);
        let mut polys = raster::build_shaded_with(&f.stream, &src, None, None);
        video::HudLayout::new(&f.tiles, &f.palette, &self.view).anchor(&mut polys, &self.view);
        let placed = raster::place(&mut polys, &self.view);
        self.gpu.set_log_frame(f.frame);
        let layers = self.gpu.layers_2d(&f.tiles, &f.palette, &self.view, !placed.is_empty());
        let _ = self.gpu.frame(&polys, &placed, &src, &self.view, &layers);
        self.drawn += 1;
        if self.pace {
            let due = self.start + Duration::from_secs_f64(self.drawn as f64 * FRAME);
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
        }
    }

    pub fn summary(&mut self) -> String {
        let s = self.gpu.pack_stats();
        format!(
            "{} frames drawn in {:.1} s (the arcade's pace: {:.1} s), {} images held ({} MB), {} preloaded, {} tiles held",
            self.drawn,
            self.start.elapsed().as_secs_f64(),
            self.drawn as f64 * FRAME,
            s.held,
            s.bytes >> 20,
            self.gpu.preloaded(),
            s.tiles_held
        )
    }
}
