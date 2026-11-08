//! The launcher's live attract mode previews: getting a game ready to show.
//!
//! A preview starts from a save state at the game's `attract_frame` (from
//! its manifest: a frame of the attract mode that shows it well), so it's
//! live almost at once. The state is made the first time a game is
//! previewed, by running it headless from power-on to that frame (with its
//! sound boards, so the preview has its sound), and kept in
//! `<data dir>/<set>/preview-<geometrizer>.save` (the save state format of
//! F5 and the slots), made again if it no longer loads. A fresh EEPROM is
//! used, never the player's.
//!
//! Each job runs on a thread of its own: loading the chips (and the state,
//! if there is one), or making the state. The board model can't move
//! between threads, so the main thread builds the preview's machine from
//! what a job hands back (`Ready`). One state is made at a time: making
//! another stops it. A job that only loads is dropped once its card isn't
//! selected any more; one making a state finishes it, so the next visit is
//! quick.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// A game ready to preview: its chips and the state to start from.
pub struct Ready {
    pub game: &'static romset::Game,
    pub roms: romset::RomSet,
    pub state: Vec<u8>,
    /// The state was made just now (else read from the cache).
    pub made: bool,
    /// Since the job started.
    pub took: Duration,
}

pub enum Event {
    Ready(Box<Ready>),
    /// The game can't be previewed (missing chips, say).
    Failed(String, String),
}

struct Job {
    set: String,
    cancel: Arc<AtomicBool>,
    /// Making the state: frames run so far, of `total`.
    making: bool,
    frames: Arc<AtomicU64>,
    total: u64,
}

/// The preview's cached state for `set` with the geometrizer `geo`.
pub fn cache_path(set: &str, geo: model2::geo::GeoMode) -> PathBuf {
    let geo = if geo == model2::geo::GeoMode::Chip { "chip" } else { "native" };
    crate::settings::data_dir().join(set).join(format!("preview-{geo}.save"))
}

pub struct Previews {
    jobs: Vec<Job>,
    tx: mpsc::Sender<(String, Result<Ready, String>)>,
    rx: mpsc::Receiver<(String, Result<Ready, String>)>,
}

impl Default for Previews {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Previews { jobs: Vec::new(), tx, rx }
    }
}

impl Previews {
    /// Get `game` ready to preview (from ROM folder `rom`, the geometrizer
    /// `geo`, the i960 interpreted if `interp`), unless it's on the way.
    /// Jobs only loading for other games are dropped; one making another
    /// game's state is too if this one needs making.
    pub fn request(&mut self, game: &'static romset::Game, rom: &Path, geo: model2::geo::GeoMode, interp: bool) {
        if self.jobs.iter().any(|j| j.set == game.id) {
            return;
        }
        let path = cache_path(game.id, geo);
        let make = !path.is_file();
        for j in &self.jobs {
            if !j.making || make {
                j.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.jobs.retain(|j| !j.cancel.load(Ordering::Relaxed));
        let job = Job {
            set: game.id.to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            making: make,
            frames: Arc::new(AtomicU64::new(0)),
            total: game.attract_frame,
        };
        let (cancel, frames, tx, rom) = (job.cancel.clone(), job.frames.clone(), self.tx.clone(), rom.to_path_buf());
        let spawned =
            std::thread::Builder::new().name(format!("preview {}", game.id)).spawn(move || {
                let start = Instant::now();
                let result = prepare(game, &rom, geo, interp, &path, &cancel, &frames)
                    .map(|(roms, state, made)| Ready { game, roms, state, made, took: start.elapsed() });
                if !cancel.load(Ordering::Relaxed) || result.as_ref().is_ok_and(|r| r.made) {
                    let _ = tx.send((game.id.to_string(), result));
                }
            });
        if spawned.is_ok() {
            self.jobs.push(job);
        }
    }

    /// Drop the jobs that only load (nothing is selected that needs them).
    pub fn cancel_loads(&mut self) {
        for j in self.jobs.iter().filter(|j| !j.making) {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.jobs.retain(|j| j.making);
    }

    /// Stop everything (the launcher closed).
    pub fn cancel_all(&mut self) {
        for j in &self.jobs {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.jobs.clear();
    }

    /// How far making `set`'s state has got (0..1), if it's being made.
    pub fn progress(&self, set: &str) -> Option<f32> {
        let j = self.jobs.iter().find(|j| j.set == set && j.making)?;
        Some(j.frames.load(Ordering::Relaxed) as f32 / j.total.max(1) as f32)
    }

    /// Whether anything is on the way for `set`.
    pub fn pending(&self, set: &str) -> bool {
        self.jobs.iter().any(|j| j.set == set)
    }

    /// Jobs finished since the last call. A finished state-making job
    /// whose game isn't wanted any more has still cached it; it's reported
    /// all the same and the caller drops what it doesn't want.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok((set, result)) = self.rx.try_recv() {
            self.jobs.retain(|j| j.set != set);
            out.push(match result {
                Ok(r) => Event::Ready(Box::new(r)),
                Err(e) => Event::Failed(set, e),
            });
        }
        out
    }
}

/// Load the chips, and the cached state or make it. Returns them and
/// whether the state was made.
fn prepare(
    game: &'static romset::Game,
    rom: &Path,
    geo: model2::geo::GeoMode,
    interp: bool,
    path: &Path,
    cancel: &AtomicBool,
    frames: &AtomicU64,
) -> Result<(romset::RomSet, Vec<u8>, bool), String> {
    let roms = romset::RomSet::load(game, rom, false).map_err(|e| e.to_string())?;
    if let Ok(state) = std::fs::read(path) {
        return Ok((roms, state, false));
    }
    let state = make_state(&roms, geo, interp, cancel, frames)?;
    // Written whole, then renamed: a half-written cache never loads.
    let tmp = path.with_extension("save.part");
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&tmp, &state))
        .and_then(|_| std::fs::rename(&tmp, path));
    if let Err(e) = written {
        eprintln!("preview: can't keep {}: {e}", path.display());
    }
    Ok((roms, state, true))
}

/// Run the game from power-on to its attract frame, with its sound boards
/// (on a thread of their own, as in play), and save it.
fn make_state(
    roms: &romset::RomSet,
    geo: model2::geo::GeoMode,
    interp: bool,
    cancel: &AtomicBool,
    frames: &AtomicU64,
) -> Result<Vec<u8>, String> {
    let mut m = model2::Machine::new(roms);
    if interp {
        m.disable_native();
    }
    m.bus.geo.set_mode(geo);
    let (link, boards) = mpsc::channel::<(Vec<(u64, bool, u8)>, u64)>();
    let mut live = model2::audio::Live::new(roms);
    let sound = std::thread::Builder::new()
        .name("preview sound".into())
        .spawn(move || {
            let mut pcm = Vec::new();
            while let Ok((bytes, until)) = boards.recv() {
                pcm.clear();
                live.feed(&bytes, until, &mut pcm);
            }
            state::to_bytes(&live)
        })
        .map_err(|e| e.to_string())?;
    let target = roms.game.attract_frame;
    while m.frames < target {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        m.run_slice(u64::MAX).map_err(|e| format!("stopped at frame {}: {e:?}", m.frames))?;
        let _ = link.send((std::mem::take(&mut m.bus.uart_log), m.bus.now));
        frames.store(m.frames, Ordering::Relaxed);
    }
    drop(link);
    let live = sound.join().map_err(|_| "the sound boards stopped".to_string())?;
    Ok(model2::save_session_parts(&mut m, Some(live)))
}
