//! Display-name sanitization for untrusted attachment filenames.
//!
//! CONTRACT: the output is a *suggestion only* — safe to display in UI and to pre-fill an OS
//! save dialog's filename field. It must NEVER be joined into a filesystem path or used to
//! decide where bytes are written; the user-chosen dialog path is the only path we write to
//! (docs/architecture/attachments.md §6 "Filename handling"). Residual risks accepted under
//! that contract: Windows reserved device names (CON, NUL, …) and trailing dots/spaces are
//! not rewritten — the OS dialog handles those.
// TODO(attachments Phase 4): remove once save_attachment/UI call this.
#![allow(dead_code)]

/// Sanitize an untrusted filename into a safe display/dialog suggestion (never a path):
/// strips path separators and `..` traversal, control characters, and invisible Unicode
/// format characters (bidi overrides, zero-width chars — extension-spoofing vectors),
/// trims leading dots, caps length at 128 chars preserving the extension where possible,
/// and falls back to `"file"` rather than returning an empty string.
pub fn sanitize_filename(name: &str) -> String {
    let mut cleaned: String = name.chars().filter(|ch| !is_disallowed(*ch)).collect();

    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", "");
    }

    let cleaned = cleaned.trim_start_matches('.').to_string();
    let cleaned = if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    };

    cap_filename(cleaned, 128)
}

/// Path separators, control chars (Cc), and the invisible/format (Cf) characters usable for
/// name spoofing: a bidi override like U+202E can visually reverse "gpj.exe" into ".jpg" in
/// the save dialog — exactly where the user judges the file type.
fn is_disallowed(ch: char) -> bool {
    ch.is_control()
        || ch == '/'
        || ch == '\\'
        || matches!(
            ch,
            '\u{00AD}'                  // soft hyphen
            | '\u{061C}'                // Arabic letter mark
            | '\u{200B}'..='\u{200F}'   // zero-widths + LRM/RLM
            | '\u{202A}'..='\u{202E}'   // bidi embedding/overrides
            | '\u{2060}'..='\u{2064}'   // word joiner + invisible operators
            | '\u{2066}'..='\u{2069}'   // bidi isolates
            | '\u{FEFF}' // zero-width no-break space / BOM
        )
}

fn cap_filename(name: String, max_chars: usize) -> String {
    let char_count = name.chars().count();
    if char_count <= max_chars {
        return name;
    }

    let chars: Vec<char> = name.chars().collect();
    let extension_start = chars.iter().enumerate().rev().find_map(|(idx, ch)| {
        if *ch == '.' && idx > 0 && idx < chars.len() - 1 {
            Some(idx)
        } else {
            None
        }
    });

    if let Some(extension_start) = extension_start {
        let extension_len = chars.len() - extension_start;
        if extension_len < max_chars {
            let stem_len = max_chars - extension_len;
            let mut out: String = chars[..stem_len].iter().collect();
            out.extend(chars[extension_start..].iter());
            return out;
        }
    }

    chars[..max_chars].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_handles_paths_controls_lengths_and_unicode() {
        let long = format!("{}.png", "a".repeat(300));
        let sanitized_long = sanitize_filename(&long);

        let cases = [
            ("../../etc/passwd", "etcpasswd"),
            ("a/b\\c.txt", "abc.txt"),
            ("a\u{0000}b\n.txt", "ab.txt"),
            ("", "file"),
            ("東京.txt", "東京.txt"),
            ("..", "file"),
            (".hidden", "hidden"),
            // Bidi-override extension spoof: U+202E would render "gpj.exe" as "exe.jpg".
            ("ann\u{202E}gpj.exe", "anngpj.exe"),
            // Zero-width chars can hide a real extension boundary.
            ("doc\u{200B}ument.pdf\u{FEFF}", "document.pdf"),
        ];

        for (input, expected) in cases {
            assert_eq!(sanitize_filename(input), expected);
        }

        assert_eq!(sanitized_long.chars().count(), 128);
        assert!(sanitized_long.ends_with(".png"));
        assert_eq!(sanitized_long.chars().filter(|ch| *ch == 'a').count(), 124);
    }
}
