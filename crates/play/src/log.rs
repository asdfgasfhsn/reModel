//! The session log (docs/play.md, "Files"): everything the player prints,
//! on the terminal as before and also into `<data>/play.log` (the session
//! before kept as `play.prev.log`), so a run started from a desktop
//! launcher, without a terminal, still leaves something to report. A panic
//! is written there with its backtrace before anything else happens.
//!
//! On Unix the process's stdout and stderr are pipes read by a thread each,
//! which writes what comes to the terminal and to the file: every line,
//! the libraries' too. Elsewhere only the panic hook and [`line`] write the
//! file.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Mutex<File>> = OnceLock::new();

/// Start the log in `dir` (the player's data folder): keep the last one as
/// `play.prev.log`, open `play.log`, copy stdout and stderr into it, and
/// write panics to it. Returns the log's path, or None if it can't be made
/// (the player carries on without one).
pub fn start(dir: &Path) -> Option<PathBuf> {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("play.log");
    let _ = std::fs::rename(&path, dir.join("play.prev.log"));
    let file = File::create(&path).ok()?;
    let _ = FILE.set(Mutex::new(file));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let bt = std::backtrace::Backtrace::force_capture();
        write_file(&format!("panic         thread '{}': {info}\n{bt}\n", thread.name().unwrap_or("unnamed")));
        default(info);
    }));
    #[cfg(unix)]
    tee::start();
    Some(path)
}

/// Write `text` to the file only (not the terminal).
fn write_file(text: &str) {
    if let Some(f) = FILE.get() {
        if let Ok(mut f) = f.lock() {
            let _ = f.write_all(text.as_bytes());
            let _ = f.flush();
        }
    }
}

/// Before the process exits: let what was printed reach the file.
pub fn finish() {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    #[cfg(unix)]
    tee::finish();
}

#[cfg(unix)]
mod tee {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    use std::sync::Mutex;
    use std::thread::JoinHandle;

    /// Each copied stream: its original descriptor and the reader thread.
    static STREAMS: Mutex<Vec<(i32, i32, JoinHandle<()>)>> = Mutex::new(Vec::new());

    pub fn start() {
        for fd in [1, 2] {
            // SAFETY: plain descriptor calls; on any failure the stream is
            // left as it was.
            unsafe {
                let orig = libc::dup(fd);
                if orig < 0 {
                    continue;
                }
                let mut p = [0; 2];
                if libc::pipe(p.as_mut_ptr()) != 0 {
                    libc::close(orig);
                    continue;
                }
                let (r, w) = (p[0], p[1]);
                if libc::dup2(w, fd) < 0 {
                    libc::close(r);
                    libc::close(w);
                    libc::close(orig);
                    continue;
                }
                libc::close(w);
                let mut reader = std::fs::File::from_raw_fd(r);
                let out_fd = libc::dup(orig);
                let mut out = std::fs::File::from_raw_fd(out_fd);
                let h = std::thread::Builder::new()
                    .name(if fd == 1 { "log stdout" } else { "log stderr" }.into())
                    .spawn(move || {
                        let mut buf = [0u8; 8192];
                        while let Ok(n) = reader.read(&mut buf) {
                            if n == 0 {
                                break;
                            }
                            let _ = out.write_all(&buf[..n]);
                            super::write_file(&String::from_utf8_lossy(&buf[..n]));
                        }
                    });
                match h {
                    Ok(h) => STREAMS.lock().unwrap().push((fd, orig, h)),
                    Err(_) => {
                        libc::dup2(orig, fd);
                        libc::close(orig);
                    }
                }
            }
        }
    }

    /// Put the original descriptors back, which closes the pipes, and wait
    /// for the readers to write what was left in them.
    pub fn finish() {
        let streams = std::mem::take(&mut *STREAMS.lock().unwrap());
        for (fd, orig, h) in streams {
            // SAFETY: as in `start`.
            unsafe {
                libc::dup2(orig, fd);
                libc::close(orig);
            }
            let _ = h.join();
        }
    }
}

/// The host, for the start of the log: OS and architecture, the CPU's
/// name, its threads.
pub fn host() -> String {
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    format!("{} {}, {}, {threads} threads", std::env::consts::OS, std::env::consts::ARCH, cpu_name())
}

fn cpu_name() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(t) = std::fs::read_to_string("/proc/cpuinfo") {
        if let Some(l) = t.lines().find(|l| l.starts_with("model name")) {
            return l.split_once(':').map_or("", |(_, v)| v).trim().to_string();
        }
    }
    #[cfg(target_os = "macos")]
    // SAFETY: sysctlbyname with a buffer and its length.
    unsafe {
        let mut buf = [0u8; 128];
        let mut len = buf.len();
        let name = c"machdep.cpu.brand_string";
        if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) == 0 {
            let s = String::from_utf8_lossy(&buf[..len]);
            return s.trim_end_matches('\0').trim().to_string();
        }
    }
    #[cfg(windows)]
    if let Ok(s) = std::env::var("PROCESSOR_IDENTIFIER") {
        return s;
    }
    "an unknown CPU".into()
}
