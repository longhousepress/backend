use crate::config::Config;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rocket::fs::NamedFile;
use rocket::http::Status;
use rocket::response::{Responder, Result as RespResult};
use rocket::{Request, State};
use sha2::Sha256;
use std::path::Path;
use subtle::ConstantTimeEq;

// HMAC-SHA256 produces 32-byte (256-bit) signatures
const HMAC_SHA256_OUTPUT_SIZE: usize = 32;

// Current token format version
const TOKEN_VERSION: u8 = 1;

#[get("/api/download/<tok>")]
pub async fn download(config: &State<Config>, tok: &str) -> Result<DownloadResponder, Status> {
    // Verify the token and extract the filepath from its payload
    let file_path = match verify(tok, &config.token_key) {
        Ok(p) => p,
        Err(e) => {
            rocket::warn!("Invalid download token: {}", e);
            return Err(Status::Gone);
        }
    };

    let named_file = NamedFile::open(&file_path).await.map_err(|e| {
        rocket::error!("Failed to open file for download: {:?}", e);
        Status::InternalServerError
    })?;

    // Extract filename from path
    let filename = Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .to_string();

    Ok(DownloadResponder {
        file: named_file,
        filename,
    })
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

#[derive(Debug)]
pub struct DownloadResponder {
    pub file: NamedFile,
    pub filename: String,
}

impl<'r> Responder<'r, 'static> for DownloadResponder {
    fn respond_to(self, req: &'r Request<'_>) -> RespResult<'static> {
        let mut response = self.file.respond_to(req)?;
        response.set_raw_header(
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", self.filename),
        );
        Ok(response)
    }
}
