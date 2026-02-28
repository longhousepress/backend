use crate::config::Config;
use crate::models::Book;
use anyhow::Result;
use resend_rs::types::CreateEmailBaseOptions;
use resend_rs::Resend;
use tera::{Context, Tera};

// Render the purchase email using Tera and send it with Resend.
// The template file should live at `templates/purchase_email.html.tera`
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
        .render("purchase_email.html.tera", &ctx)
        .map_err(|e| anyhow::anyhow!("template render error: {}", e))?;

    // Initialize Resend client
    let resend = Resend::new(&config.resend_api_key);

    // Create email
    let email = CreateEmailBaseOptions::new(
        &config.resend_from_email,
        [recipient_email],
        "Your Dragon Books Order - Download Links Inside",
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