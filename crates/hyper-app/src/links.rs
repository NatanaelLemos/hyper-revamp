use std::sync::OnceLock;

use regex::Regex;

use crate::grid_paint::GridSnapshot;

/// URL pattern for plain-text link detection on the hovered line.
///
/// Only http/https, matching the `xterm-addon-web-links` strict regex the
/// original loaded. Terminal output is untrusted — anything that prints to a
/// pane can offer a link — so `file://` must not become clickable here.
fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The trailing character class excludes punctuation that normally ends
        // a sentence rather than a URL, so "see https://example.com/docs!"
        // links to the URL without the "!".
        Regex::new(r#"https?://[^\s'"<>\(\)\[\]\{\}]*[^\s'"<>\(\)\[\]\{\}:,.!?~*_]"#)
            .expect("static regex")
    })
}

/// Only these schemes may be opened externally (`url-safety.ts` parity:
/// `isSafeExternalUrl` allowed http/https and nothing else).
pub fn is_safe_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// A link found under the pointer: URI plus the column range on that row.
pub struct HoveredLink {
    pub uri: String,
    pub row: usize,
    pub col_start: usize,
    pub col_end: usize,
}

/// Scan the snapshot row under the pointer for a URL covering `col`.
pub fn link_at(snap: &GridSnapshot, row: usize, col: usize) -> Option<HoveredLink> {
    if row >= snap.lines {
        return None;
    }
    // Build the row text with a char-index → column map (wide chars).
    let mut text = String::with_capacity(snap.cols);
    let mut byte_to_col: Vec<usize> = Vec::with_capacity(snap.cols + 1);
    let mut c = 0;
    while c < snap.cols {
        let cell = snap.cell(row, c);
        if cell.spacer {
            c += 1;
            continue;
        }
        for _ in 0..cell.ch.len_utf8() {
            byte_to_col.push(c);
        }
        text.push(cell.ch);
        c += if cell.wide { 2 } else { 1 };
    }
    byte_to_col.push(snap.cols);

    for m in url_regex().find_iter(&text) {
        let start_col = *byte_to_col.get(m.start())?;
        let end_col = *byte_to_col
            .get(m.end().saturating_sub(1))
            .unwrap_or(&snap.cols);
        if col >= start_col && col <= end_col {
            return Some(HoveredLink {
                uri: m.as_str().to_string(),
                row,
                col_start: start_col,
                col_end: end_col,
            });
        }
    }
    None
}

/// Does the configured activation key match current modifiers?
/// Empty string ⇒ no modifier required (Hyper default).
pub fn activation_matches(activation: &str, mods: egui::Modifiers) -> bool {
    match activation {
        "" => true,
        "ctrl" => mods.ctrl,
        "alt" => mods.alt,
        "shift" => mods.shift,
        "meta" => mods.mac_cmd || mods.command,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_url_allowlist() {
        assert!(is_safe_url("https://example.com"));
        assert!(is_safe_url("HTTP://EXAMPLE.COM"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("ssh://host"));
        // Terminal output is untrusted: a printed file:// link must not be
        // openable, matching `isSafeExternalUrl`.
        assert!(!is_safe_url("file:///Users/x/Downloads/payload.app"));
        assert!(!is_safe_url("mailto:a@b.c"));
    }

    fn find(text: &str) -> Option<&str> {
        url_regex().find(text).map(|m| m.as_str())
    }

    #[test]
    fn url_detection_matches_only_http_schemes() {
        assert_eq!(find("see https://example.com/docs"), Some("https://example.com/docs"));
        assert_eq!(find("file:///etc/passwd"), None);
        assert_eq!(find("mailto:a@b.c"), None);
    }

    #[test]
    fn trailing_sentence_punctuation_is_not_part_of_the_link() {
        assert_eq!(find("see https://example.com/docs!"), Some("https://example.com/docs"));
        assert_eq!(find("at https://example.com/a/b."), Some("https://example.com/a/b"));
        assert_eq!(find("(https://example.com/x)"), Some("https://example.com/x"));
        assert_eq!(find("<https://example.com/y>"), Some("https://example.com/y"));
        assert_eq!(find("https://example.com/p?q=1&r=2"), Some("https://example.com/p?q=1&r=2"));
        // A path that legitimately ends in a slash keeps it.
        assert_eq!(find("https://example.com/dir/"), Some("https://example.com/dir/"));
    }
}
