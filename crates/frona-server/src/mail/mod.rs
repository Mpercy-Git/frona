use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::core::config::{MailConfig, SmtpTls};
use crate::core::error::AppError;

/// Outbound SMTP. Constructed only when `mail.smtp_host` is set — every caller
/// holds an `Option<MailService>` and treats `None` as "email features are off"
/// rather than as an error.
#[derive(Clone)]
pub struct MailService {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl MailService {
    pub fn from_config(config: &MailConfig) -> Result<Option<Self>, AppError> {
        if !config.is_configured() {
            return Ok(None);
        }

        let host = config.smtp_host.trim();
        let builder = match config.tls {
            SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                .map_err(|e| AppError::Internal(format!("SMTP STARTTLS setup failed: {e}")))?,
            SmtpTls::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(host)
                .map_err(|e| AppError::Internal(format!("SMTP TLS setup failed: {e}")))?,
            SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
        };

        let builder = builder.port(config.smtp_port);
        let builder = match (&config.smtp_username, &config.smtp_password) {
            (Some(user), Some(pass)) if !user.is_empty() => {
                builder.credentials(Credentials::new(user.clone(), pass.clone()))
            }
            _ => builder,
        };

        let from = format!("{} <{}>", config.from_name, config.from_address)
            .parse::<Mailbox>()
            .map_err(|e| {
                AppError::Internal(format!(
                    "Invalid mail.from_address '{}': {e}",
                    config.from_address
                ))
            })?;

        Ok(Some(Self {
            transport: builder.build(),
            from,
        }))
    }

    pub async fn send(&self, to: &str, subject: &str, body: String) -> Result<(), AppError> {
        let to: Mailbox = to
            .parse()
            .map_err(|e| AppError::Validation(format!("Invalid recipient address: {e}")))?;

        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build message: {e}")))?;

        self.transport
            .send(message)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to send mail: {e}")))?;

        Ok(())
    }
}
