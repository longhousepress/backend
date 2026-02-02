use hmac_sha256::HMAC;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rocket::http::Status;
use rocket::fs::NamedFile;
use rocket::response::{Responder, Result as RespResult};
use rocket::{Request, State};
use std::path::Path;
use crate::config::Config;

#[get("/api/download/<tok>")]
pub async fn download(config: &State<Config>, tok: &str) -> Result<DownloadResponder, Status> {
    // Verify the token and extract the filepath from its payload
    let file_path = match verify(tok, &config.token_key) {
        Ok(p) => p,
        Err(_) => return Err(Status::Gone),
    };

    let named_file = NamedFile::open(&file_path).await.map_err(|e| {
        eprintln!("file open error: {:?}", e);
        Status::InternalServerError
    })?;

    // Extract filename from path
    let filename = Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .to_string();

    Ok(DownloadResponder { file: named_file, filename })
}

pub fn verify(tok: &str, token_key: &str) -> Result<String, String> {
    // Decode base64 token
    let buf = URL_SAFE_NO_PAD.decode(tok)
        .map_err(|_| "bad base64".to_string())?;

    // Expect at minimum: header (18 bytes) + signature (32 bytes)
    const MIN_PAYLOAD_AND_SIG: usize = 18 + 32;
    if buf.len() < MIN_PAYLOAD_AND_SIG {
        return Err("token too short".to_string());
    }

    // Split payload and signature (last 32 bytes are signature)
    let sig_len: usize = 32;
    let (payload, received_sig) = buf.split_at(buf.len() - sig_len);

    // Compute expected signature
    let expected_sig = HMAC::mac(payload, token_key.as_bytes());

    // Constant-time comparison to avoid leaking timing info
    if expected_sig.len() != received_sig.len() {
        return Err("signature mismatch".to_string());
    }
    let mut diff: u8 = 0;
    for (a, b) in expected_sig.iter().zip(received_sig.iter()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return Err("signature mismatch".to_string());
    }

    // Ensure payload contains at least the two bytes we use for path length
    if payload.len() < 18 {
        return Err("payload too short".to_string());
    }

    // Read path length safely
    let path_len = u16::from_be_bytes(
        payload[16..18]
            .try_into()
            .map_err(|_| "malformed payload".to_string())?,
    ) as usize;

    let start = 18usize;
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
        response.set_raw_header("Content-Disposition", format!("attachment; filename=\"{}\"", self.filename));
        Ok(response)
    }
}
