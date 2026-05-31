use crate::config::Config;
use crate::tera::SUBMISSION_EMAIL;
use anyhow::Result;
use resend_rs::types::{CreateAttachment, CreateEmailBaseOptions};
use resend_rs::Resend;
use rocket::form::Form;
use rocket::fs::TempFile;
use rocket::response::Redirect;
use rocket::State;
use tera::{Context, Tera};

#[derive(FromForm)]
pub struct SubmissionForm<'r> {
    pub submitter: &'r str,
    pub submission_type: &'r str,
    pub message: Option<&'r str>,
    pub file: TempFile<'r>,
}

// Send submission email with attachment via Resend
pub async fn send_submission_email(
    config: &Config,
    tera: &Tera,
    submitter: &str,
    submission_type: &str,
    message: Option<&str>,
    filename: &str,
    file_bytes: Vec<u8>,
    content_type: &str,
) -> Result<()> {
    // Build template context
    let mut ctx = Context::new();
    ctx.insert("submitter", submitter);
    ctx.insert("submission_type", submission_type);
    ctx.insert("message", &message);
    ctx.insert("filename", filename);

    // Render template to HTML string
    let body = tera
        .render(SUBMISSION_EMAIL, &ctx)
        .map_err(|e| anyhow::anyhow!("template render error: {}", e))?;

    // Initialize Resend client
    let resend = Resend::new(&config.resend_api_key);

    // Create attachment
    let attachment = CreateAttachment::from_content(file_bytes)
        .with_filename(filename)
        .with_content_type(content_type);

    // Create email with attachment
    let subject = format!("New Submission: {} from {}", submission_type, submitter);
    let email = CreateEmailBaseOptions::new(
        &config.submissions_from_email,
        [config.submissions_to_email.as_str()],
        &subject,
    )
    .with_html(&body)
    .with_attachment(attachment);

    // Send the email
    match resend.emails.send(email).await {
        Ok(response) => {
            rocket::info!(
                "Submission email sent successfully from {} (Resend ID: {:?})",
                submitter,
                response.id
            );
            Ok(())
        }
        Err(e) => {
            rocket::error!(
                "Failed to send submission email from {}: {:?}",
                submitter,
                e
            );
            Err(anyhow::anyhow!("Resend API error: {:?}", e))
        }
    }
}

#[post("/submit", data = "<form>")]
pub async fn submit(
    config: &State<Config>,
    tera: &State<Tera>,
    mut form: Form<SubmissionForm<'_>>,
) -> Redirect {
    // Validate submitter is not empty
    let submitter = form.submitter.trim();
    if submitter.is_empty() {
        return Redirect::to(format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode("Submitter name is required", percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    // Validate submission_type is one of the allowed values
    let submission_type = form.submission_type.trim();
    if submission_type != "Poetry" && submission_type != "Short Fiction" {
        return Redirect::to(format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode("Invalid submission type", percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    // Validate file content type
    let content_type = form
        .file
        .content_type()
        .map(|ct| ct.to_string())
        .unwrap_or_default();

    let allowed_types = [
        "application/pdf",
        "application/msword",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "text/plain",
        "text/markdown",
    ];

    let content_type_base = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    if !allowed_types.contains(&content_type_base.as_str()) {
        return Redirect::to(format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode("Invalid file type", percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    // Validate file size (10MB max)
    const MAX_SIZE: u64 = 10 * 1024 * 1024; // 10MB in bytes
    if form.file.len() > MAX_SIZE {
        return Redirect::to(format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode("File too large (10MB max)", percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    // Get filename
    let filename = form
        .file
        .raw_name()
        .map(|f| {
            f.dangerous_unsafe_unsanitized_raw()
                .as_str()
                .chars()
                .map(|c| if c == ' ' { '_' } else { c })
                .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
                .take(200)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "submission".to_string());

    // Read file bytes using a temporary file
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(_) => {
            return Redirect::to(format!(
                "/submissions/error?reason={}",
                percent_encoding::utf8_percent_encode("Failed to send submission — please try again", percent_encoding::NON_ALPHANUMERIC)
            ));
        }
    };

    if let Err(_) = form.file.persist_to(tmp.path()).await {
        return Redirect::to(format!(
            "/submissions/error?reason={}",
            percent_encoding::utf8_percent_encode("Failed to send submission — please try again", percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    let bytes = match tokio::fs::read(tmp.path()).await {
        Ok(b) => b,
        Err(_) => {
            return Redirect::to(format!(
                "/submissions/error?reason={}",
                percent_encoding::utf8_percent_encode("Failed to send submission — please try again", percent_encoding::NON_ALPHANUMERIC)
            ));
        }
    };

    // tmp drops here and the file is deleted automatically

    // Send the email
    if let Err(e) = send_submission_email(
        config,
        tera,
        submitter,
        submission_type,
        form.message,
        &filename,
        bytes,
        &content_type_base,
    )
    .await
    {
        rocket::error!("send_submission_email failed: {:?}", e);
        return Redirect::to(format!(
            "{}/submissions/error?reason={}",
            config.base_url,
            percent_encoding::utf8_percent_encode("Failed to send submission — please try again", percent_encoding::NON_ALPHANUMERIC)
        ));
    }

    Redirect::to(format!("{}/submissions/success", config.base_url))
}
