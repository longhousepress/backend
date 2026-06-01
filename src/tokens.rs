use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::Path;
use subtle::ConstantTimeEq;

// HMAC-SHA256 produces 32-byte (256-bit) signatures
const HMAC_SHA256_OUTPUT_SIZE: usize = 32;

// Current token format version
const TOKEN_VERSION: u8 = 1;

// Mint a download token with version byte for future compatibility
pub fn mint(filepath: &str, token_key: &str) -> String {
    let mut payload = Vec::new();

    // 1) version byte (1 byte) - for future version checking
    payload.push(TOKEN_VERSION);

    // 2) filepath length (2 bytes) + filepath
    let path_bytes = filepath.as_bytes();
    payload.extend_from_slice(&(path_bytes.len() as u16).to_be_bytes());
    payload.extend_from_slice(path_bytes);

    // 3) sign with HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(token_key.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(&payload);
    let sig = mac.finalize().into_bytes();
    payload.extend_from_slice(&sig);

    URL_SAFE_NO_PAD.encode(&payload)
}

pub fn verify(tok: &str, token_key: &str) -> Result<String, String> {
    // Decode base64 token
    let buf = URL_SAFE_NO_PAD
        .decode(tok)
        .map_err(|_| "bad base64".to_string())?;

    // Expect at minimum: version (1 byte) + path_len (2 bytes) + signature (32 bytes)
    const MIN_PAYLOAD_AND_SIG: usize = 3 + HMAC_SHA256_OUTPUT_SIZE;
    if buf.len() < MIN_PAYLOAD_AND_SIG {
        return Err("token too short".to_string());
    }

    // Split payload and signature (last bytes are HMAC-SHA256 signature)
    let (payload, received_sig) = buf.split_at(buf.len() - HMAC_SHA256_OUTPUT_SIZE);

    // Compute expected signature using HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(token_key.as_bytes())
        .map_err(|_| "invalid key".to_string())?;
    mac.update(payload);
    let expected_sig = mac.finalize().into_bytes();

    // Constant-time comparison to avoid leaking timing info
    if received_sig.ct_eq(expected_sig.as_slice()).into() {
        // Signature is valid, proceed to parse the payload
    } else {
        return Err("signature mismatch".to_string());
    }

    // Check version byte
    let version = payload[0];
    if version != TOKEN_VERSION {
        return Err(format!("unsupported token version: {}", version));
    }

    // Ensure payload contains at least version (1 byte) + path_len (2 bytes)
    if payload.len() < 3 {
        return Err("payload too short".to_string());
    }

    // Read path length safely (bytes 1-2)
    let path_len = u16::from_be_bytes(
        payload[1..3]
            .try_into()
            .map_err(|_| "malformed payload".to_string())?,
    ) as usize;

    let start = 3usize;
    // Bounds-check the path slice to avoid panics on malformed tokens
    if start.checked_add(path_len).is_none() || start + path_len > payload.len() {
        return Err("path length out of bounds".to_string());
    }

    let filepath = std::str::from_utf8(&payload[start..start + path_len])
        .map_err(|_| "invalid UTF-8".to_string())?
        .to_string();

    // Basic sanitization to prevent directory traversal and other surprises:
    // - Reject absolute paths
    // - Reject any parent directory components (`..`)
    // - Reject backslashes (avoid Windows-style escape)
    // - Reject embedded NULs
    if filepath.contains('\0') {
        return Err("invalid filepath".to_string());
    }
    if filepath.contains('\\') {
        return Err("invalid filepath".to_string());
    }

    let path = Path::new(&filepath);
    if path.is_absolute() {
        return Err("absolute paths not allowed".to_string());
    }
    for comp in path.components() {
        use std::path::Component;
        if let Component::ParentDir = comp {
            return Err("parent directory traversal not allowed".to_string());
        }
    }

    // At this point the filepath is considered safe enough to hand to NamedFile::open.
    Ok(filepath)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "test-secret-key";

    #[test]
    fn roundtrip() {
        let token = mint("static/books/test.epub", TEST_KEY);
        let filepath = verify(&token, TEST_KEY).unwrap();
        assert_eq!(filepath, "static/books/test.epub");
    }

    #[test]
    fn roundtrip_no_static_prefix() {
        let token = mint("books/test.epub", TEST_KEY);
        let filepath = verify(&token, TEST_KEY).unwrap();
        assert_eq!(filepath, "books/test.epub");
    }

    #[test]
    fn roundtrip_nested_path() {
        let token = mint("static/books/author/title.epub", TEST_KEY);
        let filepath = verify(&token, TEST_KEY).unwrap();
        assert_eq!(filepath, "static/books/author/title.epub");
    }

    #[test]
    fn wrong_key_rejects() {
        let token = mint("static/books/test.epub", TEST_KEY);
        assert!(verify(&token, "different-key").is_err());
    }

    #[test]
    fn empty_token_rejects() {
        assert!(verify("", TEST_KEY).is_err());
    }

    #[test]
    fn garbage_base64_rejects() {
        assert!(verify("!!!not-base64!!!", TEST_KEY).is_err());
    }

    #[test]
    fn truncated_token_rejects() {
        // Valid base64 but too short to contain version + length + signature
        let short = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &[0x01, 0x00]);
        assert!(verify(&short, TEST_KEY).is_err());
    }

    #[test]
    fn tampered_signature_rejects() {
        let token = mint("static/books/test.epub", TEST_KEY);
        let mut buf = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &token,
        ).unwrap();
        // flip last byte of signature
        let last = buf.len() - 1;
        buf[last] ^= 0xff;
        let tampered = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &buf,
        );
        assert!(verify(&tampered, TEST_KEY).is_err());
    }

    #[test]
    fn tampered_filepath_rejects() {
        let token = mint("static/books/test.epub", TEST_KEY);
        let mut buf = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &token,
        ).unwrap();
        // flip a byte in the filepath region (after version + length)
        buf[4] ^= 0xff;
        let tampered = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &buf,
        );
        assert!(verify(&tampered, TEST_KEY).is_err());
    }

    #[test]
    fn path_traversal_rejects() {
        let token = mint("static/../../../etc/passwd", TEST_KEY);
        assert!(verify(&token, TEST_KEY).is_err());
    }

    #[test]
    fn dot_dot_in_middle_rejects() {
        let token = mint("static/books/../../../etc/passwd", TEST_KEY);
        assert!(verify(&token, TEST_KEY).is_err());
    }

    #[test]
    fn absolute_path_rejects() {
        let token = mint("/etc/passwd", TEST_KEY);
        assert!(verify(&token, TEST_KEY).is_err());
    }

    #[test]
    fn nul_byte_rejects() {
        let token = mint("static/foo\0bar", TEST_KEY);
        assert!(verify(&token, TEST_KEY).is_err());
    }

    #[test]
    fn backslash_rejects() {
        let token = mint("static\\..\\..\\windows\\system32", TEST_KEY);
        assert!(verify(&token, TEST_KEY).is_err());
    }

    #[test]
    fn different_keys_produce_different_tokens() {
        let t1 = mint("static/books/test.epub", "key-a");
        let t2 = mint("static/books/test.epub", "key-b");
        assert_ne!(t1, t2);
    }

    #[test]
    fn different_paths_produce_different_tokens() {
        let t1 = mint("static/books/a.epub", TEST_KEY);
        let t2 = mint("static/books/b.epub", TEST_KEY);
        assert_ne!(t1, t2);
    }
}
