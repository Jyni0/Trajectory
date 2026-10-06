//! TCP / TLS plumbing shared by the IMAP and POP3 clients.

use std::fmt::Debug;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::error::{AppError, AppResult};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Any duplex byte stream (plain TCP or TLS) usable by the protocol clients.
pub trait MailStream: AsyncRead + AsyncWrite + Unpin + Send + Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Debug> MailStream for T {}

pub type BoxStream = Box<dyn MailStream>;

fn connector() -> TlsConnector {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        Arc::new(
            ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("ring supports the default TLS versions")
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    });
    TlsConnector::from(config.clone())
}

/// Loopback, private (RFC 1918 / ULA), link-local or CGNAT/Tailscale
/// (100.64.0.0/10) address, `localhost`, or a single-label / `.local` / `.lan`
/// / `.home.arpa` name — hosts that never route over the public internet.
pub fn is_private_host(host: &str) -> bool {
    use std::net::IpAddr;
    let h = host.trim().trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    if let Ok(ip) = h.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || (o[0] == 100 && (o[1] & 0xc0) == 64)
            }
            IpAddr::V6(v6) => {
                let seg = v6.segments()[0];
                v6.is_loopback() || (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80
            }
        };
    }
    !h.is_empty()
        && (h == "localhost"
            || h.ends_with(".localhost")
            || h.ends_with(".local")
            || h.ends_with(".lan")
            || h.ends_with(".home.arpa")
            || h.ends_with(".internal")
            || !h.contains('.'))
}

pub async fn tcp_connect(host: &str, port: u16) -> AppResult<TcpStream> {
    tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| AppError::Network(format!("timed out connecting to {host}:{port}")))?
        .map_err(|e| AppError::Network(format!("cannot connect to {host}:{port}: {e}")))
}

pub async fn tls_upgrade<S>(host: &str, stream: S) -> AppResult<tokio_rustls::client::TlsStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| AppError::Invalid(format!("invalid host name '{host}'")))?;
    tokio::time::timeout(CONNECT_TIMEOUT, connector().connect(name, stream))
        .await
        .map_err(|_| AppError::Network(format!("TLS handshake with {host} timed out")))?
        .map_err(|e| AppError::Network(format!("TLS handshake with {host} failed: {e}")))
}
