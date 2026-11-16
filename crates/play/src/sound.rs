//! The sound boards on their own thread, playing through the system's audio
//! API (cpal). The game sends commands in order over one channel, so a save
//! request sees the boards exactly as the machine is when it's made.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// Audio the queue should hold (seconds) while the game's frames come
/// steadily; output starts once it does.
pub const AUDIO_TARGET: f64 = 0.05;
/// The most it grows to when frames come unevenly (seconds), and how much
/// longer than the longest recent gap between redraws it holds, so a
/// frame that takes two or three refreshes drains the queue without
/// emptying it, and without the pitch bending to match.
const AUDIO_TARGET_MAX: f64 = 0.15;
const GAP_HEADROOM: f64 = 1.25;
/// Seconds the longest recent gap is remembered for.
const GAP_MEMORY: f64 = 8.0;
/// A gap between redraws longer than this (seconds) is a hitch, not a
/// slow game.
const STALL: f64 = 0.1;
/// Seconds the target takes to follow it.
const TARGET_TAU: f64 = 0.5;
/// Most audio queued before the oldest is dropped (seconds).
const AUDIO_MAX: f64 = 0.25;

/// One frame's sound link bytes (as `Model2Bus::uart_log`) and the cycle
/// to render up to.
type Link = (Vec<(u64, bool, u8)>, u64);

enum Command {
    Link(Link),
    /// Reply with the boards' state.
    Save(mpsc::Sender<Vec<u8>>),
    /// Restore the boards' state (and drop queued audio); reply Ok or why not.
    Load(Vec<u8>, mpsc::Sender<Result<(), String>>),
    /// Mix in dB: sound board, music board.
    Mix(f32, f32),
}

/// Audio queue shared with the output callback: interleaved stereo at the
/// device's rate.
#[derive(Default)]
pub struct Queue {
    samples: std::collections::VecDeque<f32>,
    pub underruns: u64,
    /// Playing (else filling up to `AUDIO_TARGET` first).
    primed: bool,
    /// Samples per second (per channel) at the device.
    rate: f64,
    /// Seconds queued before playing (again) after running dry.
    target: f64,
}

pub struct Sound {
    _stream: cpal::Stream,
    pub queue: Arc<Mutex<Queue>>,
    commands: mpsc::Sender<Command>,
    /// Output samples (44.1 kHz, since boot) the game asked for, and the
    /// boards have made.
    asked: u64,
    made: Arc<AtomicU64>,
    pub rate: u32,
    /// Game samples per real-time sample (f64 bits): the game's speed
    /// relative to real time, times the drift correction.
    ratio: Arc<AtomicU64>,
    /// The rate control: follows the game's actual pace, holds the queue
    /// at `AUDIO_TARGET`.
    pace: model2::pacing::Pace,
    /// The longest recent gap between redraws (seconds, decaying), and the
    /// queue target it sets.
    gap: f64,
    target: f64,
}

impl Sound {
    pub fn new(roms: &romset::RomSet, mix: (f32, f32)) -> Result<Sound, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| format!("audio: {e}"))?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let config = supported.config();
        // For the log: the device, once (and again if it changes).
        {
            static LAST: Mutex<String> = Mutex::new(String::new());
            let name = device.description().map_or_else(|_| "an unnamed device".to_string(), |d| d.name().to_string());
            let line = format!(
                "{name}, {rate} Hz, {channels} channels, {:?}, buffer {:?}",
                supported.sample_format(),
                supported.buffer_size()
            );
            let mut last = LAST.lock().unwrap();
            if *last != line {
                println!("audio         {line}");
                *last = line;
            }
        }
        let queue = Arc::new(Mutex::new(Queue { rate: rate as f64, target: AUDIO_TARGET, ..Queue::default() }));
        let q = queue.clone();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| fill(&q, out, channels, |v| v),
                |e| eprintln!("audio: {e}"),
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                config,
                move |out: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    fill(&q, out, channels, |v| (v * 32767.0).clamp(-32768.0, 32767.0) as i16)
                },
                |e| eprintln!("audio: {e}"),
                None,
            ),
            f => return Err(format!("audio: unsupported sample format {f:?}")),
        }
        .map_err(|e| format!("audio: {e}"))?;
        stream.play().map_err(|e| format!("audio: {e}"))?;

        let (commands, rx) = mpsc::channel::<Command>();
        let made = Arc::new(AtomicU64::new(0));
        let mut live = model2::audio::Live::new(roms);
        live.set_mix(mix.0, mix.1);
        let ratio = Arc::new(AtomicU64::new(1f64.to_bits()));
        let (q, m, r) = (queue.clone(), made.clone(), ratio.clone());
        std::thread::Builder::new()
            .name("sound".into())
            .spawn(move || {
                let mut pcm = Vec::new();
                // Game time at 44.1 kHz to the device rate.
                let base = model2::audio::RATE as f64 / rate as f64;
                let mut resampler = model2::audio::Resampler::default();
                let max = (AUDIO_MAX * rate as f64) as usize * 2;
                while let Ok(command) = rx.recv() {
                    match command {
                        Command::Link((bytes, until)) => {
                            pcm.clear();
                            live.feed(&bytes, until, &mut pcm);
                            m.store(live.samples(), Ordering::Relaxed);
                            let step = base * f64::from_bits(r.load(Ordering::Relaxed));
                            let mut out = Vec::with_capacity(pcm.len() * 2);
                            resampler.run(&pcm, step, &mut out);
                            let mut q = q.lock().unwrap();
                            q.samples.extend(out);
                            let excess = q.samples.len().saturating_sub(max);
                            q.samples.drain(..excess);
                        }
                        Command::Save(reply) => {
                            let _ = reply.send(state::to_bytes(&live));
                        }
                        Command::Load(bytes, reply) => {
                            let result = model2::audio::load_state(&mut live, &bytes);
                            m.store(live.samples(), Ordering::Relaxed);
                            q.lock().unwrap().samples.clear();
                            let _ = reply.send(result);
                        }
                        Command::Mix(sfx, music) => live.set_mix(sfx, music),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Sound {
            _stream: stream,
            queue,
            commands,
            asked: 0,
            made,
            rate,
            ratio,
            pace: Default::default(),
            gap: 0.0,
            target: AUDIO_TARGET,
        })
    }

    /// Set the game's speed relative to real time (1 = exact), after `dt`
    /// seconds in which it ran `ran` frames of the `due` it should have:
    /// the audio plays at that speed, held at the queue target (`AUDIO_TARGET`,
    /// deeper while redraws come unevenly) and
    /// slowed with the game once it persistently can't keep it
    /// (`model2::pacing::Pace`).
    pub fn set_speed(&mut self, speed: f64, dt: f64, ran: f64, due: f64) {
        // Redraws that come further apart than usual (a slow draw, a frame
        // that takes several refreshes) need a deeper queue; the rate control's
        // patience scales with it.
        if dt > 0.0 && dt <= model2::pacing::Pace::GAP {
            self.gap = (self.gap * (-dt / GAP_MEMORY).exp()).max(dt);
            let goal = (self.gap * GAP_HEADROOM).clamp(AUDIO_TARGET, AUDIO_TARGET_MAX);
            self.target += (goal - self.target) * (dt / TARGET_TAU).min(1.0);
            self.queue.lock().unwrap().target = self.target;
        }
        // A hitch (a driver or window-system stall) is a pause in the game,
        // not the game running slow: it would show to the rate control as a
        // few frames run of many due and drag the pitch down for seconds
        // after, so it is left to the queue.
        if dt > STALL {
            return;
        }
        let step = self.pace.update(dt, speed, ran, due, Some(self.queued()), self.target);
        self.ratio.store(step.to_bits(), Ordering::Relaxed);
    }

    /// Seconds of audio queued or still being made.
    pub fn queued(&self) -> f64 {
        let in_queue = self.queue.lock().unwrap().samples.len() as f64 / 2.0 / self.rate as f64;
        let in_flight = self.asked.saturating_sub(self.made.load(Ordering::Relaxed));
        in_queue + in_flight as f64 / model2::audio::RATE as f64
    }

    /// Send the sound link bytes written since the last call.
    pub fn send(&mut self, bus: &mut model2::Model2Bus) {
        self.asked = (bus.now as u128 * model2::audio::RATE as u128 / model2::bus::CPU_HZ as u128) as u64;
        let _ = self.commands.send(Command::Link((std::mem::take(&mut bus.uart_log), bus.now)));
    }

    /// The boards' state, once they've caught up with everything sent.
    pub fn save(&self) -> Vec<u8> {
        let (tx, rx) = mpsc::channel();
        let _ = self.commands.send(Command::Save(tx));
        rx.recv().unwrap_or_default()
    }

    /// Restore the boards (for a machine just loaded at `bus`).
    pub fn load(&mut self, bytes: Vec<u8>, bus: &model2::Model2Bus) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let _ = self.commands.send(Command::Load(bytes, tx));
        self.asked = (bus.now as u128 * model2::audio::RATE as u128 / model2::bus::CPU_HZ as u128) as u64;
        rx.recv().map_err(|_| "sound thread stopped".to_string())?
    }

    pub fn set_mix(&self, sfx: f32, music: f32) {
        let _ = self.commands.send(Command::Mix(sfx, music));
    }
}

/// Copy queued audio into a device buffer of `channels` channels.
fn fill<T: Copy>(q: &Mutex<Queue>, out: &mut [T], channels: usize, conv: impl Fn(f32) -> T) {
    let mut q = q.lock().unwrap();
    if !q.primed && q.samples.len() as f64 >= q.target * q.rate * 2.0 {
        q.primed = true;
    }
    if !q.primed {
        out.iter_mut().for_each(|s| *s = conv(0.0));
        return;
    }
    let mut dry = false;
    for frame in out.chunks_mut(channels) {
        let (l, r) = match (q.samples.pop_front(), q.samples.pop_front()) {
            (Some(l), Some(r)) => (l, r),
            _ => {
                dry = true;
                (0.0, 0.0)
            }
        };
        for (c, s) in frame.iter_mut().enumerate() {
            *s = conv(if c % 2 == 0 { l } else { r });
        }
    }
    if dry {
        q.underruns += 1;
        q.primed = false;
    }
}
