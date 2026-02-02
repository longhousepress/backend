use lettre::{Message, AsyncTransport, AsyncSmtpTransport, Tokio1Executor};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use anyhow::Result;
use tracing::{info, error};
use std::net::{ToSocketAddrs, SocketAddr};
use std::time::Duration;
use crate::config::Config;
use crate::models::Book;

// Send a purchase confirmation email with download links
pub async fn send_purchase_email(
    config: &Config,
    recipient_email: &str,
    order_id: i64,
    books: &[Book],
) -> Result<()> {
    let subject = "Your Dragon Books Order - Download Links Inside";
    let body = build_email_body(config, order_id, books);

    let email = Message::builder()
        .from(format!("{} <{}>", config.smtp_from_name, config.smtp_from_email).parse()?)
        .to(recipient_email.parse()?)
        .subject(subject)
        .header(ContentType::TEXT_HTML)
        .body(body)?;

    // Resolve hostname to IP address (prefer IPv4, fall back to IPv6)
    // This is a workaround for SMTP providers like iCloud that have connection issues
    let addr = (config.smtp_host.as_str(), config.smtp_port)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("DNS lookup failed: {}", e))?
        .find(|a| a.is_ipv4())
        .or_else(|| {
            // Fall back to IPv6 if no IPv4 available
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

    let creds = Credentials::new(
        config.smtp_username.clone(),
        config.smtp_password.clone(),
    );

    // Use async SMTP transport with explicit TLS parameters
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host_ip)
        .port(config.smtp_port)
        .tls(Tls::Required(TlsParameters::new(config.smtp_host.clone().into())?))
        .credentials(creds)
        .timeout(Some(Duration::from_secs(20)))
        .build();

    match mailer.send(email).await {
        Ok(_) => {
            info!("Purchase email sent successfully to {} for order {}", recipient_email, order_id);
            Ok(())
        }
        Err(e) => {
            error!("Failed to send purchase email to {} for order {}: {:?}", recipient_email, order_id, e);
            Err(e.into())
        }
    }
}

fn build_email_body(config: &Config, order_id: i64, books: &[Book]) -> String {
    let mut body = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <style>
        body {{ font-family: Arial, sans-serif; line-height: 1.6; color: #333; max-width: 600px; margin: 0 auto; padding: 20px; }}
        h1 {{ color: #2c3e50; }}
        h2 {{ color: #34495e; margin-top: 30px; }}
        .book {{ margin-bottom: 30px; padding: 15px; background-color: #f8f9fa; border-radius: 5px; }}
        .book-title {{ font-weight: bold; font-size: 18px; color: #2c3e50; }}
        .book-author {{ color: #7f8c8d; font-style: italic; }}
        .download-links {{ margin-top: 10px; }}
        .download-link {{ display: inline-block; margin: 5px 10px 5px 0; padding: 8px 15px; background-color: #3498db; color: white; text-decoration: none; border-radius: 3px; }}
        .download-link:hover {{ background-color: #2980b9; }}
        .footer {{ margin-top: 40px; padding-top: 20px; border-top: 1px solid #ddd; font-size: 12px; color: #7f8c8d; }}
        .order-ref {{ color: #7f8c8d; font-size: 14px; margin-top: 20px; }}
    </style>
</head>
<body>
    <h1>Thank You for Your Purchase!</h1>
    <p>Your order has been confirmed and your books are ready to download.</p>
    <p class="order-ref">Order Reference: #{}</p>

    <h2>Your Books</h2>
"#,
        order_id
    );

    // Add each book with its download links
    for book in books {
        for edition in &book.editions {
            body.push_str(&format!(
                r#"    <div class="book">
        <div class="book-title">{}</div>
        <div class="book-author">{}</div>
"#,
                edition.title, edition.author_name
            ));

            if let Some(files) = &edition.files {
                if !files.is_empty() {
                    body.push_str("        <div class=\"download-links\">\n");
                    body.push_str("            <strong>Download:</strong><br>\n");
                    for file in files {
                        let format_name = match file.format {
                            crate::models::FileFormat::Epub => "EPUB",
                            crate::models::FileFormat::Kepub => "KEPUB",
                            crate::models::FileFormat::Azw3 => "AZW3",
                            crate::models::FileFormat::Pdf => "PDF",
                        };
                        let download_url = format!("{}{}", config.base_url, file.path);
                        body.push_str(&format!(
                            "            <a href=\"{}\" class=\"download-link\">{}</a>\n",
                            download_url, format_name
                        ));
                    }
                    body.push_str("        </div>\n");
                }
            }

            body.push_str("    </div>\n");
        }
    }

    body.push_str(
        r#"
    <div class="footer">
        <p>If you have any questions or issues with your download, please don't hesitate to contact us.</p>
        <p>Thank you for choosing Dragon Books!</p>
    </div>
</body>
</html>
"#,
    );

    body
}
