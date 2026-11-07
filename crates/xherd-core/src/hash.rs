//! Content hashes for approvals (design §3.4).

use sha2::{Digest, Sha256};

/// SHA-256 (hex) of the bytes with `\r\n` normalised to `\n`, so an editor
/// changing line endings does not re-open an approved document (R-107).
pub fn content_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    let mut start = 0;
    for i in 0..bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            h.update(&bytes[start..i]);
            start = i + 1;
        }
    }
    h.update(&bytes[start..]);
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_are_normalised() {
        assert_eq!(content_sha256(b"a\r\nb\r\n"), content_sha256(b"a\nb\n"));
        assert_ne!(content_sha256(b"a\nb"), content_sha256(b"a\nb\n"));
        assert_eq!(content_sha256(b"a\rb"), content_sha256(b"a\rb"));
        assert_eq!(
            content_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
