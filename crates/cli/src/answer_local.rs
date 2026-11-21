//! The one shape every `--json` command answers with
//! (docs/uplift/interface.md, "The envelope"): `ok`, `code`, `error`,
//! `data`, `warnings`, `next`, all six always present, written with
//! `serde_json`, one object on stdout, and the exit code equal to `code`.
//!
//! A stand-in with the names of the shared module (`crates/cli/src/answer.rs`,
//! built beside it), so that merging is a rename: `Code`, `Parts`, `Answer`
//! (derefs to `Parts`), `done`, `fail`, `gate`, `unknown`, `warn`, `next`,
//! `with_data`, `exit_code`, `to_json`, `emit`, `near_matches`. Emitted from
//! `tour.rs` and `coverage.rs` (through `tour::Finish`).

use serde_json::{json, Value};
use std::ops::{Deref, DerefMut};
use std::process::ExitCode;

/// Exit codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Code {
    Done = 0,
    GateFailed = 1,
    BadRequest = 2,
    Missing = 3,
    Internal = 4,
}

pub struct Parts {
    pub code: Code,
    pub error: Option<String>,
    pub data: Value,
    pub warnings: Vec<String>,
    pub next: Option<String>,
}

pub struct Answer(Box<Parts>);

impl Deref for Answer {
    type Target = Parts;
    fn deref(&self) -> &Parts {
        &self.0
    }
}

impl DerefMut for Answer {
    fn deref_mut(&mut self) -> &mut Parts {
        &mut self.0
    }
}

#[allow(dead_code)]
impl Answer {
    fn new(code: Code, error: Option<String>, data: Value) -> Self {
        Answer(Box::new(Parts { code, error, data, warnings: Vec::new(), next: None }))
    }

    /// A request that was done, with its data.
    pub fn done(data: Value) -> Self {
        Self::new(Code::Done, None, data)
    }

    /// A request that was not.
    pub fn fail(code: Code, msg: impl Into<String>) -> Self {
        Self::new(code, Some(msg.into()), Value::Null)
    }

    /// A gate at `error` failed, with what was measured.
    pub fn gate(msg: impl Into<String>, data: Value) -> Self {
        Self::new(Code::GateFailed, Some(msg.into()), data)
    }

    /// An unknown name: code 2, with the near matches.
    pub fn unknown(kind: &str, name: &str, known: &[&str]) -> Self {
        let near = near_matches(name, known);
        let hint = if near.is_empty() {
            format!(" (known: {})", known.join(", "))
        } else {
            format!(" (did you mean {}?)", near.join(", "))
        };
        Self::fail(Code::BadRequest, format!("no {kind} {name}{hint}"))
    }

    pub fn warn(mut self, w: impl Into<String>) -> Self {
        self.0.warnings.push(w.into());
        self
    }

    pub fn next(mut self, n: impl Into<String>) -> Self {
        self.0.next = Some(n.into());
        self
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.0.data = data;
        self
    }

    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code as u8)
    }

    /// The envelope as one JSON value.
    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.code == Code::Done,
            "code": self.code as u8,
            "error": self.error,
            "data": self.data,
            "warnings": self.warnings,
            "next": self.next,
        })
    }

    /// Print the answer (`json`: the envelope on stdout; otherwise `text`
    /// for people, an error on stderr) and give the exit code.
    pub fn emit(self, json: bool, text: impl FnOnce() -> String) -> ExitCode {
        if json {
            println!("{}", serde_json::to_string_pretty(&self.to_json()).unwrap_or_default());
        } else {
            match &self.error {
                None => print!("{}", text()),
                Some(e) => eprintln!("error: {e}"),
            }
            for w in &self.warnings {
                eprintln!("warning: {w}");
            }
            if let Some(n) = &self.next {
                eprintln!("next: {n}");
            }
        }
        self.exit_code()
    }
}

/// The names in `known` close to `name`: a small edit distance, or one
/// contains the other.
pub fn near_matches(name: &str, known: &[&str]) -> Vec<String> {
    known
        .iter()
        .copied()
        .filter(|c| distance(name, c) <= 2 || c.contains(name) || name.contains(c))
        .map(str::to_string)
        .collect()
}

fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut prev = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let cur = row[j];
            row[j] = (prev + (a[i - 1] != b[j - 1]) as usize).min(row[j] + 1).min(row[j - 1] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The envelope always has its six fields, `code` is the exit code and
    /// `error` is null on success.
    #[test]
    fn the_envelope_has_six_fields() {
        let ok = Answer::done(json!({"n": 1})).to_json();
        let bad = Answer::fail(Code::Missing, "no map").next("remodel tour run --all").warn("w").to_json();
        for v in [&ok, &bad] {
            assert_eq!(v.as_object().unwrap().len(), 6);
            for k in ["ok", "code", "error", "data", "warnings", "next"] {
                assert!(v.get(k).is_some(), "{k}");
            }
        }
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["code"], 0);
        assert!(ok["error"].is_null());
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["code"], 3);
        assert_eq!(bad["warnings"][0], "w");
        assert!(bad["data"].is_null());
    }

    #[test]
    fn an_unknown_name_lists_near_matches() {
        let a = Answer::unknown("tour", "coin-strat", &["bench", "coin-start"]);
        assert_eq!(a.code, Code::BadRequest);
        assert!(a.error.as_deref().unwrap().contains("did you mean coin-start"));
    }
}
