//! Drawing a redraw: the game's frame and the overlay (egui) into the
//! window's surface, and presenting it.
//!
//! What the renderer reads from the machine is copied out first
//! ([`gpu::BoardFrame`], in [`Core::frame`]), so the drawing can run on
//! the draw thread while the main thread emulates the next frame and its
//! run-ahead ([`Drawer`]; docs/play.md, "The frame loop"). With
//! `--no-overlap` the same job runs on the main thread, as it is
//! submitted.

use crate::backdrop;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// The GPU side of the window: what a draw uses, on the draw thread while
/// one is in flight.
pub struct Core {
    pub surface: wgpu::Surface<'static>,
    pub renderer: gpu::Renderer,
    /// egui's renderer (the overlay's context and input stay on the main
    /// thread).
    pub egui: egui_wgpu::Renderer,
    pub backdrop: backdrop::Backdrop,
    /// The frame being drawn, copied from the machine shown.
    pub frame: gpu::BoardFrame,
    /// Photo mode: the geometry of its last picture, kept while the
    /// camera stays put (`photo::draw`).
    pub photo_shots: Option<(crate::photo::ShotKey, crate::photo::Shots)>,
}

impl Core {
    /// Forget what the renderer kept of the last frame (its frame copy, RAM
    /// versions and temporal history): the next one is drawn from scratch.
    pub fn forget_frame(&mut self) {
        self.frame.forget();
        self.renderer.forget_ram();
        self.renderer.reset_history();
    }
}

/// How to draw the game's frame in [`Core::frame`]: decided on the main
/// thread when it was captured.
pub struct GameJob {
    /// The frame is another machine's (run-ahead's, or the game's after
    /// it), or a new game's: upload video RAM whole.
    pub forget_ram: bool,
    /// Start temporal anti-aliasing's accumulation again.
    pub reset_history: bool,
    pub quality: gpu::Quality,
    /// Smooth shading's rules and HUD windows (`set_light_rules`).
    pub rules: (&'static romset::Shading, u64),
    /// Smooth motion's in-betweens, when they're drawn: when the newest
    /// game frame was due, and the game's frame period in seconds (the
    /// in-between's place is the time since, over it).
    pub smooth: Option<(Instant, f64)>,
    /// In-betweens get their own lighting side channel (smooth shading or
    /// material maps, with the geometrizer recording it).
    pub lit: bool,
}

/// What drawing the game's frame did.
#[derive(Clone, Copy, Default)]
pub struct GameReport {
    /// An in-between was drawn, in this host time (its geometrizer replay).
    pub smooth: Option<Duration>,
    /// Smooth shading's counts and the level drawn.
    pub shading: Option<(model2::light::Stats, gpu::Shading)>,
    pub pack: gpu::PackStats,
}

/// Draw the game's frame in `core.frame` into `target` as `view` at
/// `origin`: its own polygons, or with smooth motion an in-between; the
/// geometrizer's previous frame's if it was captured late. With temporal
/// anti-aliasing each image is tagged (`gpu::FrameTag`): the game's frame
/// N as (N, 1) by `present_board` itself (drawn again: a repeat), an
/// in-between at t as (N, t), a late frame as the frame its polygons are.
pub fn draw_game(
    core: &mut Core,
    job: &GameJob,
    target: &wgpu::TextureView,
    view: &model2::video::View,
    origin: (u32, u32),
    format: wgpu::TextureFormat,
) -> GameReport {
    let r = &mut core.renderer;
    let f = &core.frame;
    let mut report = GameReport::default();
    if job.forget_ram {
        r.forget_ram();
    }
    if job.reset_history {
        r.reset_history();
    }
    r.set_light_rules(job.rules.0, job.rules.1);
    let temporal = r.set_quality(job.quality).temporal > 0;
    if f.is_empty() {
        // Nothing captured yet: the black frame present_board would draw
        // of empty memory.
    } else if f.late() {
        r.present_board(f, None, None, view, target, format, origin);
    } else {
        // Smooth motion: the polygons part way from the frame before to
        // this one, by the time since this one was due.
        let t_smooth = Instant::now();
        let between = job.smooth.and_then(|(at, period)| {
            let sf = f.smooth()?;
            let t = (t_smooth.saturating_duration_since(at).as_secs_f64() / period) as f32;
            if job.lit {
                sf.render_lit(t).map(|(stream, side)| (stream, Some(std::sync::Arc::new(side)), sf, t))
            } else {
                sf.render(t).map(|stream| (stream, None, sf, t))
            }
        });
        if let Some((_, side, _, _)) = &between {
            r.set_light_frame(side.clone());
        }
        match &between {
            Some((stream, _, sf, t)) if temporal => {
                // Tagged (N, t): the renderer works out the motion since
                // the image before. t = 0 is the frame before's pose, which
                // follows that frame's own image (N-1, 1) as a step
                // forward, not a repeat.
                let tag = gpu::FrameTag { frame: sf.frame, t: t.max(1e-4) };
                let m = gpu::BusMotion::of(sf, tag, None);
                r.present_board(f, Some((stream, &sf.points)), Some(&m), view, target, format, origin);
                report.smooth = Some(t_smooth.elapsed());
            }
            Some((stream, _, sf, _)) => {
                report.smooth = Some(t_smooth.elapsed());
                r.present_board(f, Some((stream, &sf.points)), None, view, target, format, origin);
            }
            None => r.present_board(f, None, None, view, target, format, origin),
        }
    }
    // Smooth shading: what the frame was drawn as, and its cost (a repeat
    // of a frame costs nothing: the renderer keeps its shading).
    if let (Some(st), Some(eff)) = (r.light_stats(), r.light_effective()) {
        report.shading = Some((st, eff));
    }
    report.pack = r.pack_stats();
    report
}

/// The overlay's egui output for one redraw, laid out and tessellated on
/// the main thread, painted by [`paint_overlay`].
pub struct OverlayPaint {
    pub jobs: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    pub screen: egui_wgpu::ScreenDescriptor,
    /// Clear `target` first (nothing drawn under it).
    pub clear: bool,
}

/// Paint `o` over `target` and submit.
pub fn paint_overlay(core: &mut Core, o: &OverlayPaint, target: &wgpu::TextureView) {
    let (device, queue) = (core.renderer.device(), core.renderer.queue());
    for (id, deltas) in &o.textures.set {
        for delta in deltas {
            core.egui.update_texture(device, queue, *id, delta);
        }
    }
    let mut enc = device.create_command_encoder(&Default::default());
    let extra = core.egui.update_buffers(device, queue, &mut enc, &o.jobs, &o.screen);
    {
        let load = if o.clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load };
        let mut pass = enc
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("menu"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        core.egui.render(&mut pass, &o.jobs, &o.screen);
    }
    queue.submit(extra.into_iter().chain([enc.finish()]));
    for id in &o.textures.free {
        core.egui.free_texture(id);
    }
}

/// One redraw of the window: the game's frame (None: no game) and the
/// overlay (None: not shown), into the surface, presented.
pub struct Job {
    pub game: Option<GameJob>,
    /// Photo mode: the frame drawn as a photo instead (`photo::draw`).
    pub photo: Option<crate::photo::Job>,
    pub view: model2::video::View,
    pub origin: (u32, u32),
    /// The surface's configuration (its format; to configure it again if
    /// it was lost).
    pub config: wgpu::SurfaceConfiguration,
    pub overlay: Option<OverlayPaint>,
    /// `--frame-log`: hash the picture presented (read back; slow).
    pub hash: bool,
}

/// What a [`Job`] did.
#[derive(Clone, Copy)]
pub struct Report {
    /// The surface gave no texture (lost, outdated, hidden): nothing was
    /// drawn, the surface was configured again.
    pub missed: bool,
    /// Host time drawing: after the surface gave its texture (which can
    /// wait for vsync), until `present` (likewise).
    pub draw: Duration,
    pub game: Option<GameReport>,
    /// The picture's hash (`Job::hash`).
    pub hash: Option<u64>,
}

/// Run `job` on `core`; `started` once the surface has given its texture
/// (after any wait for vsync), as drawing starts.
pub fn run(core: &mut Core, job: Job, started: &dyn Fn()) -> Report {
    let frame = match core.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
        // Lost, outdated, hidden or timed out: configure again, skip a draw.
        _ => {
            core.surface.configure(core.renderer.device(), &job.config);
            return Report { missed: true, draw: Duration::ZERO, game: None, hash: None };
        }
    };
    let t = Instant::now();
    started();
    let target = frame.texture.create_view(&Default::default());
    let game = job.game.as_ref().map(|g| draw_game(core, g, &target, &job.view, job.origin, job.config.format));
    if let Some(p) = &job.photo {
        crate::photo::draw(core, p, &target, &job.view, job.origin, job.config.format);
    }
    if let Some(o) = &job.overlay {
        paint_overlay(core, o, &target);
    }
    let draw = t.elapsed();
    let hash = job.hash.then(|| {
        let (w, h) = (frame.texture.width(), frame.texture.height());
        fnv(&core.renderer.read_texture(&frame.texture, w, h))
    });
    core.renderer.queue().present(frame);
    Report { missed: false, draw, game, hash }
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The draw thread's side of [`Drawer`].
struct Thread {
    jobs: mpsc::Sender<(Box<Core>, Job)>,
    done: mpsc::Receiver<(Box<Core>, Report)>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Runs [`Job`]s: on the draw thread (overlap), one at a time, while the
/// main thread goes on; or on the main thread as they're submitted. The
/// [`Core`] goes with the job and comes back with its report.
pub struct Drawer {
    core: Option<Box<Core>>,
    thread: Option<Thread>,
    /// A job is on the draw thread, submitted then; and since when it has
    /// been drawing (set by the draw thread).
    busy: Option<Instant>,
    drawing: Arc<Mutex<Option<Instant>>>,
    /// The last job's report, until taken.
    report: Option<Report>,
}

impl Drawer {
    /// A drawer for `core`: with `wake`, on a draw thread that calls it
    /// whenever a job starts drawing and when it's done (to wake the event
    /// loop); without, inline.
    pub fn new(core: Core, wake: Option<Box<dyn Fn() + Send>>) -> Self {
        let drawing = Arc::new(Mutex::new(None));
        let thread = wake.map(|wake| {
            let since = drawing.clone();
            let (jobs, rx) = mpsc::channel::<(Box<Core>, Job)>();
            let (tx, done) = mpsc::channel();
            let handle = std::thread::Builder::new()
                .name("draw".into())
                .spawn(move || {
                    for (mut core, job) in rx {
                        let started = || {
                            *since.lock().unwrap() = Some(Instant::now());
                            wake();
                        };
                        let report = run(&mut core, job, &started);
                        if tx.send((core, report)).is_err() {
                            break;
                        }
                        wake();
                    }
                })
                .expect("start the draw thread");
            Thread { jobs, done, handle: Some(handle) }
        });
        Drawer { core: Some(Box::new(core)), thread, busy: None, drawing, report: None }
    }

    /// Whether jobs run on the draw thread.
    pub fn overlaps(&self) -> bool {
        self.thread.is_some()
    }

    /// When the job on the draw thread was submitted, while it runs.
    pub fn busy_since(&self) -> Option<Instant> {
        self.busy
    }

    /// When the job on the draw thread started drawing (got the surface's
    /// texture), once it has.
    pub fn drawing_since(&self) -> Option<Instant> {
        self.busy.and(*self.drawing.lock().unwrap())
    }

    /// Whether the job on the draw thread is done (without waiting); true
    /// when there is none.
    pub fn done(&mut self) -> bool {
        if self.busy.is_none() {
            return true;
        }
        let Some(t) = &self.thread else { return true };
        match t.done.try_recv() {
            Ok((core, report)) => {
                self.back(core, report);
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => panic!("the draw thread stopped"),
        }
    }

    /// Wait for the job on the draw thread, if any.
    pub fn finish(&mut self) {
        if self.busy.is_none() {
            return;
        }
        let (core, report) = self.thread.as_ref().unwrap().done.recv().expect("the draw thread stopped");
        self.back(core, report);
    }

    fn back(&mut self, core: Box<Core>, report: Report) {
        self.core = Some(core);
        self.report = Some(report);
        self.busy = None;
    }

    /// The core, once the job in flight (if any) is done.
    pub fn core(&mut self) -> &mut Core {
        self.finish();
        self.core.as_mut().unwrap()
    }

    /// Run `job`: on the draw thread (returns at once), or now.
    pub fn submit(&mut self, job: Job) {
        self.finish();
        let mut core = self.core.take().unwrap();
        match &self.thread {
            Some(t) => {
                *self.drawing.lock().unwrap() = None;
                t.jobs.send((core, job)).expect("the draw thread stopped");
                self.busy = Some(Instant::now());
            }
            None => {
                let report = run(&mut core, job, &|| {});
                self.back(core, report);
            }
        }
    }

    /// The report of the last job done, once.
    pub fn take_report(&mut self) -> Option<Report> {
        self.report.take()
    }
}

impl Drop for Drawer {
    fn drop(&mut self) {
        self.finish();
        if let Some(Thread { jobs, done, handle }) = self.thread.take() {
            drop((jobs, done));
            if let Some(h) = handle {
                let _ = h.join();
            }
        }
    }
}
