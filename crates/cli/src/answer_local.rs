//! The one shape every `--json` command answers with
//! (docs/uplift/interface.md, "The envelope"): `ok`, `code`, `error`,
//! `data`, `warnings`, `next`, all six always present, written with
//! `serde_json`, one object on stdout, and the exit code equal to `code`.
//!
//! This is a small stand-in with the shared module's shape
//! (`crates/cli/src/answer.rs`, built beside it): at merge the commands
//! below switch to that module and this file goes. Emitted from
//! `tour.rs` and `coverage.rs`, through [`Answer::finish`].

use serde_json::{json, Value};
use std::process::ExitCode;

/// Exit codes: the request was done.
pub const DONE: u8 = 0;
/// A gate at `error` failed.
#[allow(dead_code)]
pub const GATE_FAILED: u8 = 1;
/// The request was wrong: an unknown name, a bad option.
pub const BAD_REQUEST: u8 = 2;
/// Something the command needs is missing: a ROM, a map, a tour.
pub const MISSING: u8 = 3;
/// An internal error.
pub const INTERNAL: u8 = 4;

pub struct Answer {
    pub code: u8,
    pub error: Option<String>,
    pub data: Value,
    pub warnings: Vec<String>,
    pub next: Option<String>,
}

impl Answer {
    /// A request that was done, with its data.
    pub fn done(data: Value) -> Self {
        Answer { code: DONE, error: None, data, warnings: Vec::new(), next: None }
    }

    /// A request that was not: the exit `code`, what went wrong, and
    /// (where there is one) the command that would put it right.
    pub fn fail(code: u8, error: impl Into<String>, next: Option<&str>) -> Self {
        Answer {
            code,
            error: Some(error.into()),
            data: Value::Null,
            warnings: Vec::new(),
            next: next.map(str::to_string),
        }
    }

    pub fn warn(mut self, w: impl Into<String>) -> Self {
        self.warnings.push(w.into());
        self
    }

    pub fn next(mut self, n: impl Into<String>) -> Self {
        self.next = Some(n.into());
        self
    }

    /// The envelope as one JSON value.
    pub fn to_json(&self) -> Value {
        json!({
            "ok": self.code == DONE,
            "code": self.code,
            "error": self.error,
            "data": self.data,
            "warnings": self.warnings,
            "next": self.next,
        })
    }

    /// Print the answer (`json`: the envelope on stdout; otherwise
    /// `text` for people, an error on stderr) and give the exit code.
    pub fn finish(self, json: bool, text: impl FnOnce(&Value) -> String) -> ExitCode {
        if json {
            println!("{}", serde_json::to_string_pretty(&self.to_json()).unwrap_or_default());
        } else {
            match &self.error {
                None => print!("{}", text(&self.data)),
                Some(e) => eprintln!("error: {e}"),
            }
            for w in &self.warnings {
                eprintln!("warning: {w}");
            }
            if let Some(n) = &self.next {
                eprintln!("next: {n}");
            }
        }
        ExitCode::from(self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The envelope always has its six fields, `code` is the exit code and
    /// `error` is null on success.
    #[test]
    fn the_envelope_has_six_fields() {
        let ok = Answer::done(json!({"n": 1})).to_json();
        let bad = Answer::fail(MISSING, "no map", Some("remodel tour run --all")).warn("w").to_json();
        for v in [&ok, &bad] {
            let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
            assert_eq!(keys.len(), 6, "{keys:?}");
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
}
