//! The shared answer every uplift command gives (docs/uplift/README.md,
//! rules 3 to 5; docs/uplift/interface.md, "The envelope").
//!
//! One `Answer` per command run. With `--json` it prints one object,
//! written by `serde_json`:
//!
//! ```json
//! {"ok": false, "code": 3, "error": "scene 'x' has no local copy",
//!  "data": null, "warnings": [], "next": "remodel scene make x"}
//! ```
//!
//! `code` is the exit code: 0 done, 1 a gate failed, 2 the request was
//! wrong, 3 something needed is missing (ROM, scene, capture), 4 an internal
//! error. Without `--json` the same answer prints as text: the command's
//! own report on success, `error: ...` and `next: ...` on stderr on failure.
//!
//! Using it:
//!
//! ```ignore
//! let a = Answer::done(json!({"picture": path})).warn("pack refused").next("remodel scene list");
//! // or Answer::fail(Code::Missing, "scene 'x' has no local copy").next("remodel scene make x")
//! // or Answer::unknown("scene", "cours", &names)   (code 2, lists near matches)
//! return a.emit(json_flag, || report_text);
//! ```
//!
//! `emit` prints and returns the `ExitCode`. Commands never print their own
//! envelope and never choose their own exit codes.

use serde_json::{json, Value};
use std::process::ExitCode;

/// What a command leaves with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Done = 0,
    /// A gate (a check with a threshold) failed.
    GateFailed = 1,
    /// The request was wrong: an unknown name, a bad option or value.
    BadRequest = 2,
    /// Something needed is missing: a ROM, a scene, a capture.
    Missing = 3,
    /// A bug or an environment failure (no GPU, disk full).
    Internal = 4,
}

/// The fields of an answer (read them through `Answer`'s `Deref`).
#[derive(Clone, Debug)]
pub struct Parts {
    pub code: Code,
    pub error: Option<String>,
    pub data: Value,
    pub warnings: Vec<String>,
    pub next: Option<String>,
}

/// One command's answer. Boxed, so it is cheap to carry in an `Err`.
#[derive(Clone, Debug)]
pub struct Answer(Box<Parts>);

impl std::ops::Deref for Answer {
    type Target = Parts;
    fn deref(&self) -> &Parts {
        &self.0
    }
}

impl std::ops::DerefMut for Answer {
    fn deref_mut(&mut self) -> &mut Parts {
        &mut self.0
    }
}

impl Answer {
    /// A successful answer carrying `data`.
    pub fn done(data: Value) -> Answer {
        Answer(Box::new(Parts { code: Code::Done, error: None, data, warnings: vec![], next: None }))
    }

    /// A failed answer; `code` is not `Done`.
    pub fn fail(code: Code, error: impl Into<String>) -> Answer {
        debug_assert!(code != Code::Done);
        Answer(Box::new(Parts { code, error: Some(error.into()), data: Value::Null, warnings: vec![], next: None }))
    }

    /// A gate that failed, with the numbers that show it.
    pub fn gate(error: impl Into<String>, data: Value) -> Answer {
        Answer::fail(Code::GateFailed, error).with_data(data)
    }

    /// Set the answer's `data` (a failure may carry some too).
    pub fn with_data(mut self, data: Value) -> Answer {
        self.0.data = data;
        self
    }

    /// An unknown name (code 2), listing the near matches among `known`.
    pub fn unknown(kind: &str, name: &str, known: &[String]) -> Answer {
        let near = near_matches(name, known);
        let mut msg = format!("unknown {kind} `{name}`");
        if near.is_empty() {
            if !known.is_empty() {
                msg += &format!(" (known: {})", known.join(", "));
            }
        } else {
            msg += &format!(" (did you mean: {}?)", near.join(", "));
        }
        Answer::fail(Code::BadRequest, msg).with_data(json!({ "kind": kind, "name": name, "near": near }))
    }

    pub fn warn(mut self, w: impl Into<String>) -> Answer {
        self.0.warnings.push(w.into());
        self
    }

    /// A suggested next command.
    pub fn next(mut self, n: impl Into<String>) -> Answer {
        self.0.next = Some(n.into());
        self
    }

    /// The error text (empty on success).
    pub fn message(&self) -> String {
        self.error.clone().unwrap_or_default()
    }

    pub fn ok(&self) -> bool {
        self.code == Code::Done
    }

    /// The object on one line, on stdout.
    pub fn print_json(&self) {
        println!("{}", serde_json::to_string(&self.to_json()).unwrap_or_else(|_| "{}".into()));
    }

    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code as u8)
    }

    /// The envelope.
    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.ok(),
            "code": self.code as u8,
            "error": self.error,
            "data": self.data,
            "warnings": self.warnings,
            "next": self.next,
        })
    }

    /// Print the answer (the envelope on stdout with `json`; else `text()`
    /// on stdout, or on failure `error:` and `next:` on stderr) and give
    /// the exit code. Warnings go to stderr in text mode.
    pub fn emit(&self, json: bool, text: impl FnOnce() -> String) -> ExitCode {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&self.to_json())
                    .unwrap_or_else(|e| format!("{{\"ok\":false,\"code\":4,\"error\":\"{e}\"}}"))
            );
        } else {
            for w in &self.warnings {
                eprintln!("warning: {w}");
            }
            match &self.error {
                None => print!("{}", text()),
                Some(e) => {
                    eprintln!("error: {e}");
                    if !self.data.is_null() {
                        let t = text();
                        if !t.is_empty() {
                            print!("{t}");
                        }
                    }
                }
            }
            if let Some(n) = &self.next {
                eprintln!("next: {n}");
            }
        }
        self.exit_code()
    }
}

/// Names in `known` close to `name`: containing it or contained by it, or
/// within a small edit distance (at most 3, and under half the length).
pub fn near_matches(name: &str, known: &[String]) -> Vec<String> {
    let n = name.to_lowercase();
    let mut scored: Vec<(usize, &String)> = known
        .iter()
        .filter_map(|k| {
            let kl = k.to_lowercase();
            let d = edit_distance(&n, &kl);
            let sub = !n.is_empty() && (kl.contains(&n) || n.contains(&kl));
            (sub || d <= 3.min(kl.len().max(n.len()) / 2)).then_some((if sub { 0 } else { d }, k))
        })
        .collect();
    scored.sort();
    scored.into_iter().take(5).map(|(_, k)| k.clone()).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + (ca != cb) as usize).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_has_every_field() {
        let a = Answer::done(json!({"n": 1})).warn("w").next("remodel x");
        let v = a.to_json();
        assert_eq!(v["ok"], true);
        assert_eq!(v["code"], 0);
        assert_eq!(v["error"], Value::Null);
        assert_eq!(v["data"]["n"], 1);
        assert_eq!(v["warnings"][0], "w");
        assert_eq!(v["next"], "remodel x");
    }

    #[test]
    fn failures_carry_their_exit_code() {
        for (c, n) in [(Code::GateFailed, 1), (Code::BadRequest, 2), (Code::Missing, 3), (Code::Internal, 4)] {
            let a = Answer::fail(c, "no");
            assert_eq!(a.to_json()["code"], n);
            assert_eq!(a.to_json()["ok"], false);
            assert_eq!(a.exit_code(), ExitCode::from(n as u8));
        }
    }

    #[test]
    fn unknown_names_list_near_matches() {
        let known: Vec<String> = ["course-start", "title", "select-skater"].iter().map(|s| s.to_string()).collect();
        let a = Answer::unknown("scene", "cours-start", &known);
        assert_eq!(a.code, Code::BadRequest);
        assert!(a.error.as_ref().unwrap().contains("course-start"), "{:?}", a.error);
        assert_eq!(a.data["near"][0], "course-start");
        // Nothing near: the known names are listed instead.
        let b = Answer::unknown("scene", "zzzzzzzz", &known);
        assert!(b.error.as_ref().unwrap().contains("known: course-start"));
    }
}
