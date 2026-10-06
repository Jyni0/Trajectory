//! SMTP submission via `lettre`.

use std::time::Duration;

use async_trait::async_trait;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use super::OutgoingProvider;
use crate::error::{AppError, AppResult};
use crate::models::{Security, ServerConfig};

pub struct SmtpProvider {
    cfg: ServerConfig,
    username: String,
    password: String,
}

impl SmtpProvider {
    pub fn new(cfg: ServerConfig, username: String, password: String) -> Self {
        Self { cfg, username, password }
    }

    fn transport(&self) -> AppResult<AsyncSmtpTransport<Tokio1Executor>> {
        let host = self.cfg.host.as_str();
        let builder = match self.cfg.security {
            Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host)?,
            Security::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?,
            Security::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
        };
        let mut builder = builder.port(self.cfg.port).timeout(Some(Duration::from_secs(30)));
        if !self.username.is_empty() {
            builder = builder.credentials(Credentials::new(self.username.clone(), self.password.clone()));
        }
        Ok(builder.build())
    }
}

#[async_trait]
impl OutgoingProvider for SmtpProvider {
    async fn test(&self) -> AppResult<()> {
        match self.transport()?.test_connection().await? {
            true => Ok(()),
            false => Err(AppError::Network(format!("SMTP server {} did not respond", self.cfg.host))),
        }
    }

    async fn send(&self, message: &lettre::Message) -> AppResult<()> {
        self.transport()?.send(message.clone()).await?;
        Ok(())
    }
}
