use crate::config::Config;
use crate::tokens::verify;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rocket::fs::NamedFile;
use rocket::http::Status;
use rocket::response::{Responder, Result as RespResult};
use rocket::{Request, State};
use std::path::Path;

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

#[derive(Debug)]
pub struct DownloadResponder {
    pub file: NamedFile,
    pub filename: String,
}

impl<'r> Responder<'r, 'static> for DownloadResponder {
    fn respond_to(self, req: &'r Request<'_>) -> RespResult<'static> {
        let mut response = self.file.respond_to(req)?;

        // Use RFC 6266 format with percent-encoding for the filename* parameter
        // This handles special characters, unicode, quotes, and other edge cases correctly
        let encoded_filename = utf8_percent_encode(&self.filename, NON_ALPHANUMERIC).to_string();

        // Use both filename (ASCII fallback) and filename* (RFC 5987) for maximum compatibility
        // The ASCII fallback replaces non-ASCII chars with underscores
        let ascii_filename: String = self
            .filename
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        response.set_raw_header(
            "Content-Disposition",
            format!(
                "attachment; filename=\"{}\"; filename*=UTF-8''{}",
                ascii_filename, encoded_filename
            ),
        );
        Ok(response)
    }
}
