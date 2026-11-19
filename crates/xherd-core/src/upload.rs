//! Names of uploaded files (R2, issue #192). Pure: the client's file name is
//! never trusted, so it is reduced to a plain basename before it reaches the
//! disk, and a name that is taken gets a number instead of being overwritten.

/// Longest cleaned name, in bytes (the cleaned name is ASCII).
pub const MAX_NAME: usize = 100;

/// Longest extension kept when a name is shortened.
const MAX_EXT: usize = 20;

/// The base name of what a client sent: what follows the last `/` or `\`,
/// every character outside `A-Za-z0-9._-` replaced by `_`, leading dots
/// stripped (no hidden files, no `..`), `file` when nothing is left, and at
/// most [`MAX_NAME`] bytes with the extension kept.
pub fn clean_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let plain: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let plain = plain.trim_start_matches('.');
    if plain.is_empty() {
        return "file".to_string();
    }
    if plain.len() <= MAX_NAME {
        return plain.to_string();
    }
    let (stem, ext) = split_ext(plain);
    let ext = if ext.len() > MAX_EXT { "" } else { ext };
    let keep = MAX_NAME - ext.len();
    if ext.is_empty() {
        return plain[..MAX_NAME].to_string();
    }
    format!("{}{ext}", &stem[..keep])
}

/// The name for the `n`-th try: `n` 1 is the name itself, then `shot-2.png`,
/// `shot-3.png`; without an extension `shot-2`.
pub fn numbered(name: &str, n: u32) -> String {
    if n <= 1 {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    format!("{stem}-{n}{ext}")
}

/// Split at the last dot, the dot going with the extension; a name with no
/// dot, or only a leading one, has no extension.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn hostile_names_become_plain() {
        assert_eq!(clean_name("../x"), "x");
        assert_eq!(clean_name("/etc/passwd"), "passwd");
        assert_eq!(clean_name("..\\..\\win.ini"), "win.ini");
        assert_eq!(clean_name(".env"), "env");
        assert_eq!(clean_name("..."), "file");
        assert_eq!(clean_name(".."), "file");
    }

    #[test]
    fn unusual_characters_are_replaced() {
        assert_eq!(clean_name("a b?.png"), "a_b_.png");
        assert_eq!(
            clean_name("Captura de pantalla.png"),
            "Captura_de_pantalla.png"
        );
        assert_eq!(clean_name("a;rm -rf.sh"), "a_rm_-rf.sh");
    }

    #[test]
    fn empty_gives_file() {
        assert_eq!(clean_name(""), "file");
        assert_eq!(clean_name("dir/"), "file");
        assert_eq!(clean_name("\\"), "file");
    }

    #[test]
    fn long_names_keep_the_extension() {
        let raw = format!("{}.png", "a".repeat(300));
        let n = clean_name(&raw);
        assert_eq!(n.len(), MAX_NAME);
        assert!(n.ends_with(".png"));
        assert!(n.starts_with("aaaa"));
    }

    #[test]
    fn long_names_without_extension_are_cut() {
        let n = clean_name(&"b".repeat(300));
        assert_eq!(n.len(), MAX_NAME);
        // An absurd extension is not kept.
        let n = clean_name(&format!("x.{}", "c".repeat(300)));
        assert_eq!(n.len(), MAX_NAME);
    }

    #[test]
    fn a_short_name_is_unchanged() {
        assert_eq!(clean_name("shot.png"), "shot.png");
        assert_eq!(clean_name("a-b_c.tar.gz"), "a-b_c.tar.gz");
    }

    #[test]
    fn numbering() {
        assert_eq!(numbered("shot.png", 1), "shot.png");
        assert_eq!(numbered("shot.png", 2), "shot-2.png");
        assert_eq!(numbered("shot.png", 3), "shot-3.png");
        assert_eq!(numbered("shot", 2), "shot-2");
        assert_eq!(numbered("a.tar.gz", 2), "a.tar-2.gz");
    }

    proptest! {
        #[test]
        fn cleaned_names_are_always_safe(raw in "\\PC{0,300}") {
            let n = clean_name(&raw);
            prop_assert!(!n.is_empty());
            prop_assert!(n.len() <= MAX_NAME);
            prop_assert!(!n.contains('/') && !n.contains('\\'));
            prop_assert!(!n.starts_with('.'));
            prop_assert!(n.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)));
            // Cleaning twice changes nothing.
            prop_assert_eq!(clean_name(&n), n);
        }
    }
}
