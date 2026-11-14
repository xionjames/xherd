//! Herd view plugins: static files the daemon serves and the Herd page shows
//! in a sandboxed frame. Nothing of a view runs in the daemon or in the UI's
//! origin; these helpers decide which files may be served and how.

/// Largest view file served (bytes).
pub const MAX_FILE: u64 = 16 * 1024 * 1024;

/// The built-in view plugins' files (plugin, path inside its folder, bytes),
/// from `plugins/<plugin>/`; the manifest is not one of them. A test compares
/// the table with the folders.
pub const BUILTIN_FILES: &[(&str, &str, &[u8])] = &[
    (
        "herd-tiles",
        "index.html",
        include_bytes!("../../../plugins/herd-tiles/index.html"),
    ),
    (
        "herd-tiles",
        "tiles.css",
        include_bytes!("../../../plugins/herd-tiles/tiles.css"),
    ),
    (
        "herd-tiles",
        "tiles.js",
        include_bytes!("../../../plugins/herd-tiles/tiles.js"),
    ),
];

/// A built-in view plugin's file, by plugin name and path.
pub fn builtin_file(name: &str, path: &str) -> Option<&'static [u8]> {
    BUILTIN_FILES
        .iter()
        .find(|(n, p, _)| *n == name && *p == path)
        .map(|(_, _, bytes)| *bytes)
}

/// `path` when it is a plain relative path inside a plugin folder: segments
/// separated by `/`, none empty or starting with `.` (so no `..`, and hidden
/// files stay private), no `\` and no control characters.
pub fn clean_path(path: &str) -> Option<&str> {
    let plain = !path.is_empty()
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && !seg.starts_with('.'));
    plain.then_some(path)
}

/// The `Content-Type` of a view file, by extension (a fixed table).
pub fn content_type(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The `Content-Security-Policy` of every view file of plugin `name`.
///
/// `sandbox allow-scripts` gives the document an opaque origin wherever it is
/// shown, even opened in a tab of its own. Sources are the plugin's own files
/// and the view kit, path-restricted from the request's `Host` so a view
/// cannot load `/api/v1/…` JSON as a script or image; a `Host` with other
/// characters than `[A-Za-z0-9.:[]-]` falls back to `'self'`. No fetches,
/// forms, frames, workers or inline scripts.
pub fn csp(host: Option<&str>, name: &str) -> String {
    let plain = |h: &&str| {
        !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '[' | ']' | '-'))
    };
    let src = match host.filter(plain) {
        Some(h) => format!("http://{h}/api/v1/plugins/{name}/view/ http://{h}/api/v1/view-kit/"),
        None => "'self'".into(),
    };
    format!(
        "sandbox allow-scripts; default-src 'none'; script-src {src}; \
         style-src {src} 'unsafe-inline'; img-src {src} data:; font-src {src}; \
         connect-src 'none'; form-action 'none'; frame-src 'none'; child-src 'none'; \
         worker-src 'none'; base-uri 'none'; frame-ancestors 'self'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, MANIFEST_FILE, builtins};

    /// Every file of every built-in view plugin folder, manifest aside, is in
    /// the table, and nothing else is.
    #[test]
    fn builtin_files_match_the_plugin_folders() {
        let plugins = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let mut on_disk = Vec::new();
        for m in builtins().iter().filter(|m| m.kind == Kind::View) {
            let mut stack = vec![plugins.join(&m.name)];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    let rel = path.strip_prefix(plugins.join(&m.name)).unwrap();
                    let rel = rel.to_str().unwrap().replace('\\', "/");
                    if rel != MANIFEST_FILE {
                        on_disk.push((m.name.clone(), rel, std::fs::read(&path).unwrap()));
                    }
                }
            }
        }
        on_disk.sort();
        let mut table: Vec<_> = BUILTIN_FILES
            .iter()
            .map(|(n, p, b)| (n.to_string(), p.to_string(), b.to_vec()))
            .collect();
        table.sort();
        assert_eq!(table, on_disk);
        for (_, path, _) in &table {
            assert_eq!(clean_path(path), Some(path.as_str()));
        }
    }

    #[test]
    fn builtin_file_finds_herd_tiles() {
        let html = builtin_file("herd-tiles", "index.html").unwrap();
        assert!(std::str::from_utf8(html).unwrap().contains("tiles.js"));
        assert!(builtin_file("herd-tiles", "missing.js").is_none());
        assert!(builtin_file("other", "index.html").is_none());
        let tiles = builtins()
            .into_iter()
            .find(|m| m.name == "herd-tiles")
            .unwrap();
        assert_eq!(tiles.kind, Kind::View);
        assert_eq!(crate::manifest::view_entry(&tiles), Ok("index.html"));
    }

    #[test]
    fn clean_paths() {
        for ok in ["index.html", "a/b.js", "assets/x-1.woff2", "a b.css"] {
            assert_eq!(clean_path(ok), Some(ok));
        }
        for bad in [
            "",
            "/x",
            "../x",
            "a/../x",
            "./x",
            "a//b",
            "a/",
            "a\\b",
            ".git/config",
            "a/.env",
            "a\nb",
            "a\0b",
        ] {
            assert_eq!(clean_path(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("a/b.MJS"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("x.woff2"), "font/woff2");
        assert_eq!(content_type("x.exe"), "application/octet-stream");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }

    #[test]
    fn csp_restricts_sources_to_the_view_and_the_kit() {
        let p = csp(Some("127.0.0.1:4740"), "herd-tiles");
        assert!(p.starts_with("sandbox allow-scripts; default-src 'none';"));
        assert!(p.contains(
            "script-src http://127.0.0.1:4740/api/v1/plugins/herd-tiles/view/ \
             http://127.0.0.1:4740/api/v1/view-kit/;"
        ));
        assert!(p.contains("connect-src 'none'"));
        assert!(p.contains("frame-ancestors 'self'"));
        assert!(!p.contains("script-src 'self'"));
        assert!(csp(Some("[::1]:4740"), "v").contains("http://[::1]:4740/"));
        for odd in [Some("evil.com/ x"), Some("a;b"), Some(""), None] {
            let p = csp(odd, "v");
            assert!(p.contains("script-src 'self';"), "{odd:?}");
            assert!(!p.contains("http://"), "{odd:?}");
        }
    }
}
