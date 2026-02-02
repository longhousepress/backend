use hmac_sha256::HMAC;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rocket::http::Status;
use rocket::fs::NamedFile;
use rocket::response::{Responder, Result as RespResult};
use rocket::Request;
use std::path::Path;

#[get("/api/download/<tok>")]
pub async fn download(tok: &str) -> Result<DownloadResponder, Status> {
    // Verify the token and extract the filepath from its payload
    let file_path = match verify(tok) {
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

pub fn verify(tok: &str) -> Result<String, String> {
    let buf = URL_SAFE_NO_PAD.decode(tok)
        .map_err(|_| "bad base64".to_string())?;

    if buf.len() < 50 { return Err("token too short".to_string()); }

    let secret = std::env::var("TOKEN_KEY")
        .map_err(|_| "missing TOKEN_KEY".to_string())?;

    let (payload, received_sig) = buf.split_at(buf.len() - 32);
    let expected_sig = HMAC::mac(payload, secret.as_bytes());

    if expected_sig.as_slice() != received_sig {
        return Err("signature mismatch".to_string());
    }

    let path_len = u16::from_be_bytes(payload[16..18].try_into().unwrap()) as usize;
    let filepath = std::str::from_utf8(&payload[18..18 + path_len])
        .map_err(|_| "invalid UTF-8".to_string())?;

    Ok(filepath.to_string())
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
