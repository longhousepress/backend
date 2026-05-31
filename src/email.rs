use crate::config::Config;
use crate::models::Book;
use crate::tera::{PURCHASE_EMAIL, SUBMISSION_EMAIL};
use anyhow::Result;
use resend_rs::types::{CreateAttachment, CreateEmailBaseOptions};
use resend_rs::Resend;
use tera::{Context, Tera};

// Render the purchase email using Tera and send it with Resend.
pub async fn send_purchase_email(
    config: &Config,
    tera: &Tera,
    recipient_email: &str,
    order_id: i64,
    books: &[Book],
) -> Result<()> {
    // Build template context
    let mut ctx = Context::new();
    ctx.insert("order_id", &order_id);
    ctx.insert("books", books);
    ctx.insert("base_url", &config.base_url);

    // Render template to HTML string
    let body = tera
        .render(PURCHASE_EMAIL, &ctx)
        .map_err(|e| anyhow::anyhow!("template render error: {}", e))?;

    // Initialize Resend client
    let resend = Resend::new(&config.resend_api_key);

    // Create email
    let email = CreateEmailBaseOptions::new(
        &config.resend_from_email,
        [recipient_email],
        "Your Longhouse Press Order - Download Links Inside",
    )
    .with_html(&body);

    // Send the email
    match resend.emails.send(email).await {
        Ok(response) => {
            rocket::info!(
                "Purchase email sent successfully for order {} (Resend ID: {:?})",
                order_id,
                response.id
            );
            Ok(())
        }
        Err(e) => {
            rocket::error!(
                "Failed to send purchase email for order {}: {:?}",
                order_id,
                e
            );
            Err(e.into())
        }
    }
}

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
    let mut ctx = Context::new();
    ctx.insert("submitter", submitter);
    ctx.insert("submission_type", submission_type);
    ctx.insert("message", &message);
    ctx.insert("filename", filename);

    let body = tera
        .render(SUBMISSION_EMAIL, &ctx)
        .map_err(|e| anyhow::anyhow!("template render error: {}", e))?;

    let resend = Resend::new(&config.resend_api_key);

    let attachment = CreateAttachment::from_content(file_bytes)
        .with_filename(filename)
        .with_content_type(content_type);

    let subject = format!("New Submission: {} from {}", submission_type, submitter);
    let email = CreateEmailBaseOptions::new(
        &config.submissions_from_email,
        [config.submissions_to_email.as_str()],
        &subject,
    )
    .with_html(&body)
    .with_attachment(attachment);

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
