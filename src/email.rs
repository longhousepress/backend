use crate::config::Config;
use crate::models::Book;
use anyhow::{Context as AnyhowContext, Result};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use tera::{Context, Tera};


// Render the purchase email using Tera and send it with lettre.
// The template file should live at `templates/purchase_email.html.tera`
pub async fn send_purchase_email(
    config: &Config,
    recipient_email: &str,
    order_id: i64,
    books: &[Book],
) -> Result<()> {
    // Initialize Tera each call. Template parsing errors are considered fatal at runtime.
    let tera = Tera::new("templates/**/*.html.tera")
        .map_err(|e| anyhow::anyhow!("template initialization error: {}", e))?;

    // Build template context
    let mut ctx = Context::new();
    ctx.insert("order_id", &order_id);
    ctx.insert("books", books);
    ctx.insert("base_url", &config.base_url);

    // Render template to HTML string
    let body = tera
        .render("purchase_email.html.tera", &ctx)
        .map_err(|e| anyhow::anyhow!("template render error: {}", e))?;

    // Build the email message
    let email = Message::builder()
        .from(
            format!("{} <{}>", config.smtp_from_name, config.smtp_from_email)
                .parse()
                .context("invalid from address")?,
        )
        .to(recipient_email.parse().context("invalid recipient address")?)
        .subject("Your Dragon Books Order - Download Links Inside")
        .header(ContentType::TEXT_HTML)
        .body(body)
        .context("failed to build email message")?;

    // Resolve SMTP host to an IP address (prefer IPv4).
    let addr = (config.smtp_host.as_str(), config.smtp_port)
        .to_socket_addrs()
        .context("DNS lookup failed")?
        .find(|a| a.is_ipv4())
        .or_else(|| {
            (config.smtp_host.as_str(), config.smtp_port)
                .to_socket_addrs()
                .ok()?
                .find(|a| a.is_ipv6())
        })
        .ok_or_else(|| anyhow::anyhow!("No IP address found for {}", config.smtp_host))?;

    let host_ip = match addr {
        SocketAddr::V4(v4) => v4.ip().to_string(),
        SocketAddr::V6(v6) => v6.ip().to_string(),
    };

    let creds = Credentials::new(config.smtp_username.clone(), config.smtp_password.clone());

    // Build async SMTP transport with required TLS parameters.
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host_ip)
        .port(config.smtp_port)
        .tls(Tls::Required(
            TlsParameters::new(config.smtp_host.clone())
                .context("invalid TLS parameters for SMTP")?,
        ))
        .credentials(creds)
        .timeout(Some(Duration::from_secs(20)))
        .build();

    // Send the email
    match mailer.send(email).await {
        Ok(_) => {
            rocket::info!(
                "Purchase email sent successfully to {} for order {}",
                recipient_email, order_id
            );
            Ok(())
        }
        Err(e) => {
            rocket::error!(
                "Failed to send purchase email to {} for order {}: {:?}",
                recipient_email, order_id, e
            );
            Err(e.into())
        }
    }
}
