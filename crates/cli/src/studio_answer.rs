//! The answer of a studio command: one object with `ok`, `code`, `error`,
//! `data`, `warnings` and `next`, and the exit codes (docs/uplift/README.md,
//! rules 3 and 4). Self-contained on purpose: the shared answer module the
//! rest of the CLI is getting has the same fields and codes, and this one
//! is replaced by it where `cmd_studio` and `cmd_img` (main.rs) emit.
//!
//! Exit codes: 0 done, 1 a gate failed, 2 the request was wrong, 3 something
//! needed is missing (a file, the ROM, a capture), 4 an internal error.
//!
//! The studio's commands print their reports with `println!` and fail with a
//! `String`. With `--json` the program runs itself again without the
//! option, and this module turns what that run did into the object: stdout
//! lines become `data.lines`, `warning: ` lines of stderr become
//! `warnings`, an `error: ` line (and what follows it) the `error` and a
//! `next: ` line the `next`; the child's exit code is the `code`. So no
//! command's printing changes, and a command's own structured data
//! (`remodel img`) goes through `Answer` directly.

use serde_json::{json, Value};
use std::process::{Command, ExitCode, Stdio};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    Done = 0,
    GateFailed = 1,
    BadRequest = 2,
    Missing = 3,
    Internal = 4,
}

pub struct Answer {
    pub code: Code,
    pub error: Option<String>,
    pub data: Value,
    pub warnings: Vec<String>,
    pub next: Option<String>,
}

impl Answer {
    pub fn done(data: Value) -> Answer {
        Answer { code: Code::Done, error: None, data, warnings: vec![], next: None }
    }

    pub fn fail(code: Code, error: impl Into<String>) -> Answer {
        Answer { code, error: Some(error.into()), data: Value::Null, warnings: vec![], next: None }
    }

    pub fn ok(&self) -> bool {
        self.code == Code::Done
    }

    /// The object: `ok`, `code`, `error`, `data`, `warnings`, `next`.
    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.ok(),
            "code": self.code as i32,
            "error": self.error,
            "data": self.data,
            "warnings": self.warnings,
            "next": self.next,
        })
    }

    pub fn print_json(&self) {
        println!("{}", serde_json::to_string(&self.to_json()).unwrap_or_else(|_| "{}".into()));
    }

    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code as u8)
    }
}

/// Which exit code a command's error message means. The studio's errors are
/// plain strings; this reads what they say.
pub fn classify(msg: &str) -> Code {
    // The first line says it; a usage text after it may say "needs the ROMs".
    let m = msg.lines().next().unwrap_or("").to_ascii_lowercase();
    if m.starts_with("internal:") {
        return Code::Internal;
    }
    const MISSING: &[&str] = &[
        "not found",
        "no such file",
        "not a studio project",
        "cannot find",
        "no texture map",
        "no original",
        "no frames in both",
        "is missing",
        "needs the rom",
        "cannot start",
        "does not exist",
    ];
    if MISSING.iter().any(|p| m.contains(p)) {
        return Code::Missing;
    }
    const IO: &[&str] = &["os error", "permission denied", "i/o error"];
    if IO.iter().any(|p| m.contains(p)) {
        return Code::Internal;
    }
    Code::BadRequest
}

/// The names in `candidates` near `word`: a small edit distance, or one
/// holding the other, best first, at most three.
pub fn near<'a>(word: &str, candidates: &[&'a str]) -> Vec<&'a str> {
    let w = word.trim_start_matches('-');
    let mut v: Vec<(usize, &str)> = candidates
        .iter()
        .filter_map(|c| {
            let cc = c.trim_start_matches('-');
            let d = edit_distance(w, cc);
            let contains = w.len() >= 3 && (cc.contains(w) || w.contains(cc));
            (d <= 2.max(w.len() / 4) || contains).then_some((if contains { d.min(1) } else { d }, *c))
        })
        .collect();
    v.sort_by_key(|(d, c)| (*d, *c));
    v.into_iter().map(|(_, c)| c).take(3).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + (a[i - 1] != b[j - 1]) as usize;
            cur.push(sub.min(prev[j] + 1).min(cur[j - 1] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// "unknown X `w`; did you mean A, B?" with the full list when nothing is near.
pub fn unknown(what: &str, word: &str, candidates: &[&str]) -> String {
    let n = near(word, candidates);
    if n.is_empty() {
        format!("unknown {what} `{word}`; the {what}s are: {}", candidates.join(", "))
    } else {
        format!("unknown {what} `{word}`; did you mean {}?", n.join(" or "))
    }
}

/// Run this program again with `args` (`--json` removed), capture what it
/// did and make the answer of it.
pub fn run_as_answer(args: &[String]) -> Answer {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return Answer::fail(Code::Internal, format!("internal: cannot find this program: {e}")),
    };
    let out = match Command::new(exe).args(args).stdin(Stdio::null()).output() {
        Ok(o) => o,
        Err(e) => return Answer::fail(Code::Internal, format!("internal: cannot run: {e}")),
    };
    let lines: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
    let err_text = String::from_utf8_lossy(&out.stderr).to_string();
    let mut a = parse_stderr(&err_text);
    a.code = match out.status.code() {
        Some(0) => Code::Done,
        Some(1) => Code::GateFailed,
        Some(2) => Code::BadRequest,
        Some(3) => Code::Missing,
        _ => Code::Internal,
    };
    if a.code != Code::Done && a.error.is_none() {
        a.error = Some(match a.code {
            Code::GateFailed => "a check did not pass (see data.lines)".to_string(),
            _ => format!("the command failed ({})", out.status),
        });
    }
    a.data = json!({ "lines": lines });
    a
}

/// `warning: ` lines, and `error: ` with the lines after it up to a
/// `next: ` line, from a command's stderr.
fn parse_stderr(text: &str) -> Answer {
    let mut a = Answer::done(Value::Null);
    let mut err: Option<Vec<String>> = None;
    for l in text.lines() {
        if let Some(w) = l.strip_prefix("warning: ") {
            a.warnings.push(w.to_string());
            if let Some(e) = err.take() {
                a.error = Some(e.join("\n"));
            }
        } else if let Some(n) = l.strip_prefix("next: ") {
            a.next = Some(n.to_string());
            if let Some(e) = err.take() {
                a.error = Some(e.join("\n"));
            }
        } else if let Some(e) = l.strip_prefix("error: ") {
            err = Some(vec![e.to_string()]);
        } else if let Some(e) = err.as_mut() {
            e.push(l.to_string());
        }
    }
    if let Some(e) = err {
        a.error = Some(e.join("\n"));
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_has_the_fields_of_the_spec() {
        let mut a = Answer::fail(Code::Missing, "scene 'x' has no local copy");
        a.next = Some("remodel scene make x".into());
        a.warnings.push("w".into());
        let j = a.to_json();
        for k in ["ok", "code", "error", "data", "warnings", "next"] {
            assert!(j.get(k).is_some(), "{k}");
        }
        assert_eq!(j["ok"], false);
        assert_eq!(j["code"], 3);
        let d = Answer::done(json!({"n": 1}));
        assert_eq!(d.to_json()["ok"], true);
        assert!(d.to_json()["error"].is_null());
    }

    #[test]
    fn codes_follow_the_message() {
        assert_eq!(classify("x: not a studio project (no project.txt)"), Code::Missing);
        assert_eq!(classify("reference image a.png not found"), Code::Missing);
        assert_eq!(classify("--frame: `x` is not a frame number"), Code::BadRequest);
        assert_eq!(classify("unknown option `--forse`"), Code::BadRequest);
        assert_eq!(classify("internal: boom"), Code::Internal);
        assert_eq!(classify("unknown subcommand `x`\nusage ... (needs the ROMs)"), Code::BadRequest);
        assert_eq!(classify("write failed (os error 28)"), Code::Internal);
    }

    #[test]
    fn near_matches() {
        let c = ["--replace", "--force", "--keep-size", "--dry-run"];
        assert_eq!(near("--replac", &c), vec!["--replace"]);
        assert_eq!(near("--forse", &c), vec!["--force"]);
        assert!(near("--zzzzzz", &c).is_empty());
        assert!(unknown("option", "--zzzzzz", &c).contains("the options are"));
        assert!(unknown("option", "--keepsize", &c).contains("did you mean --keep-size"));
    }

    #[test]
    fn stderr_lines_become_fields() {
        let a = parse_stderr("noise\nwarning: one\nerror: bad thing\nmore of it\nnext: do this\nwarning: two\n");
        assert_eq!(a.warnings, vec!["one", "two"]);
        assert_eq!(a.error.as_deref(), Some("bad thing\nmore of it"));
        assert_eq!(a.next.as_deref(), Some("do this"));
    }
}
