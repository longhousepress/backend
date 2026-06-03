use crate::email::send_submission_email;
use crate::state::AppState;
use axum::extract::{Multipart, State};
use axum::response::Redirect;

pub async fn submit(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Redirect, (axum::http::StatusCode, String)> {
    let mut submitter = String::new();
    let mut submission_type = String::new();
    let mut message = String::new();
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();
    let mut file_content_type = String::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "submitter" => {
                submitter = field.text().await.unwrap_or_default();
            }
            "submission_type" => {
                submission_type = field.text().await.unwrap_or_default();
            }
            "message" => {
                message = field.text().await.unwrap_or_default();
            }
            "file" => {
                file_name = field
                    .file_name()
                    .unwrap_or("submission")
                    .to_string();
                file_content_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                file_bytes = Some(field.bytes().await.unwrap_or_default().to_vec());
            }
            _ => {}
        }
    }

    // Validate submitter is not empty
    let submitter = submitter.trim();
    if submitter.is_empty() {
        return Ok(Redirect::to(&format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode(
                "Submitter name is required",
                percent_encoding::NON_ALPHANUMERIC
            )
        )));
    }

    // Validate submission_type is one of the allowed values
    let submission_type = submission_type.trim();
    if submission_type != "Poetry" && submission_type != "Short Fiction" {
        return Ok(Redirect::to(&format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode(
                "Invalid submission type",
                percent_encoding::NON_ALPHANUMERIC
            )
        )));
    }

    // Validate file content type
    let allowed_types = [
        "application/pdf",
        "application/msword",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "text/plain",
        "text/markdown",
    ];

    let content_type_base = file_content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    if !allowed_types.contains(&content_type_base.as_str()) {
        return Ok(Redirect::to(&format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode(
                "Invalid file type",
                percent_encoding::NON_ALPHANUMERIC
            )
        )));
    }

    let bytes = match file_bytes {
        Some(b) => b,
        None => {
            return Ok(Redirect::to(&format!(
                "/submissions/error?reason={}",
                percent_encoding::utf8_percent_encode(
                    "No file uploaded",
                    percent_encoding::NON_ALPHANUMERIC
                )
            )));
        }
    };

    // Validate file size (10MB max)
    const MAX_SIZE: usize = 10 * 1024 * 1024; // 10MB in bytes
    if bytes.len() > MAX_SIZE {
        return Ok(Redirect::to(&format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode(
                "File too large (10MB max)",
                percent_encoding::NON_ALPHANUMERIC
            )
        )));
    }

    // Sanitize filename
    let filename: String = file_name
        .chars()
        .map(|c| if c == ' ' { '_' } else { c })
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(200)
        .collect::<String>();
    let filename = if filename.is_empty() {
        "submission".to_string()
    } else {
        filename
    };

    // Send the email
    if let Err(e) = send_submission_email(
        &state.config,
        &state.tera,
        submitter,
        submission_type,
        if message.is_empty() { None } else { Some(message.as_str()) },
        &filename,
        bytes,
        &content_type_base,
    )
    .await
    {
        tracing::error!("send_submission_email failed: {:?}", e);
        return Ok(Redirect::to(&format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode(
                "Failed to send submission — please try again",
                percent_encoding::NON_ALPHANUMERIC
            )
        )));
    }

    Ok(Redirect::to("/submissions/success"))
}
