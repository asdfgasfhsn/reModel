//! Player user names: what a player is called in the lobby, in the join
//! list and in the status and end messages. A name is only ever shown; it
//! is never part of the machine's state or of any hash.

/// The longest name, in characters.
pub const MAX_NAME: usize = 16;

/// What an empty name becomes.
pub const FALLBACK_NAME: &str = "Player";

/// `s` as a name: control characters dropped, white space trimmed and
/// collapsed, at most [`MAX_NAME`] characters. May be empty.
pub fn sanitize_name(s: &str) -> String {
    let kept: String = s.chars().filter(|c| !c.is_control() || c.is_whitespace()).collect();
    let words: Vec<&str> = kept.split_whitespace().collect();
    let joined = words.join(" ");
    joined.chars().take(MAX_NAME).collect::<String>().trim_end().to_string()
}

/// The name a player gets until they choose one: the OS login name if there
/// is one, else "Player".
pub fn default_name() -> String {
    ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|v| sanitize_name(&v))
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| FALLBACK_NAME.to_string())
}

/// `s` sanitised, or [`default_name`] if nothing is left.
pub fn name_or_default(s: &str) -> String {
    let n = sanitize_name(s);
    if n.is_empty() {
        default_name()
    } else {
        n
    }
}

/// The names as shown when two players share one: the second "Sam" becomes
/// "Sam (2)", the third "Sam (3)".
pub fn display_names(names: &[String]) -> Vec<String> {
    let mut seen: Vec<(&str, usize)> = Vec::new();
    names
        .iter()
        .map(|n| match seen.iter_mut().find(|(s, _)| s == n) {
            Some((_, k)) => {
                *k += 1;
                format!("{n} ({k})")
            }
            None => {
                seen.push((n, 1));
                n.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(sanitize_name("  Sam  "), "Sam");
        assert_eq!(sanitize_name("Sa\u{7}m\n"), "Sam");
        assert_eq!(sanitize_name("a   b\tc"), "a b c");
        assert_eq!(sanitize_name("abcdefghijklmnopqrstuvwxyz"), "abcdefghijklmnop");
        assert_eq!(sanitize_name("abcdefghijklmno qrs"), "abcdefghijklmno");
        assert_eq!(sanitize_name("\u{0}\u{1}"), "");
        // Characters, not bytes.
        assert_eq!(sanitize_name(&"é".repeat(40)).chars().count(), MAX_NAME);
    }

    #[test]
    fn a_name_is_always_found() {
        assert!(!name_or_default("").is_empty());
        assert!(default_name().chars().count() <= MAX_NAME);
        assert_eq!(name_or_default("Kim"), "Kim");
    }

    #[test]
    fn duplicates_are_told_apart() {
        let n: Vec<String> = ["Sam", "Kim", "Sam", "Sam"].iter().map(|s| s.to_string()).collect();
        assert_eq!(display_names(&n), ["Sam", "Kim", "Sam (2)", "Sam (3)"]);
    }
}
