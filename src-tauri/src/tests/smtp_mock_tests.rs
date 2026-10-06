use base64::Engine;

use super::fixtures::*;
use super::mock_servers::{smtp_server, PASS, USER};
use crate::models::{AccountKind, OutgoingAttachment, OutgoingEmail};
use crate::providers::smtp::SmtpProvider;
use crate::providers::OutgoingProvider;
use crate::service::AppContext;

fn outgoing() -> OutgoingEmail {
    OutgoingEmail {
        account_id: "acc".into(),
        to: vec!["Bob <bob@example.com>".into()],
        cc: vec!["carol@example.com".into()],
        bcc: vec!["hidden@example.com".into()],
        subject: "Quarterly numbers".into(),
        body_text: "See attached.".into(),
        body_html: Some("<p>See <b>attached</b>.</p>".into()),
        attachments: vec![OutgoingAttachment {
            filename: "numbers.csv".into(),
            mime_type: "text/csv".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode("a,b\n1,2\n"),
        }],
        in_reply_to: Some("<orig@example.com>".into()),
        draft_id: None,
    }
}

#[tokio::test]
async fn sends_authenticated_multipart_message() {
    let server = smtp_server().await;
    let provider = SmtpProvider::new(plain_server(server.port), USER.into(), PASS.into());
    let account = account(AccountKind::Imap, "me@example.com");
    let msg = AppContext::build_message(&account, &outgoing()).unwrap();

    provider.send(&msg).await.unwrap();

    let cap = server.captured.lock().unwrap().clone();
    assert_eq!(cap.len(), 1);
    let c = &cap[0];
    assert!(c.auth_ok, "AUTH PLAIN must be performed");
    assert_eq!(c.mail_from, "<me@example.com>");
    assert_eq!(c.rcpt_to, ["<bob@example.com>", "<carol@example.com>", "<hidden@example.com>"]);
    assert!(c.data.contains("Subject: Quarterly numbers"));
    assert!(c.data.contains("In-Reply-To: <orig@example.com>"));
    assert!(c.data.contains("multipart/mixed"));
    assert!(c.data.contains("multipart/alternative"));
    assert!(c.data.contains("filename=\"numbers.csv\""));
    assert!(!c.data.contains("hidden@example.com"), "Bcc must not leak into headers");

    // What went over the wire parses back with our own MIME parser.
    let parsed = crate::mime::parse_message(c.data.as_bytes()).unwrap();
    assert_eq!(parsed.subject, "Quarterly numbers");
    // Text attachments are canonicalized to CRLF line endings on the wire.
    assert_eq!(parsed.attachments[0].data, b"a,b\r\n1,2\r\n");
}

#[tokio::test]
async fn rejects_bad_credentials() {
    let server = smtp_server().await;
    let provider = SmtpProvider::new(plain_server(server.port), USER.into(), "wrong".into());
    assert!(provider.test().await.is_err());
    let account = account(AccountKind::Imap, "me@example.com");
    let msg = AppContext::build_message(&account, &outgoing()).unwrap();
    assert!(provider.send(&msg).await.is_err());
    assert!(server.captured.lock().unwrap().is_empty());
}

#[tokio::test]
async fn connection_test_succeeds_with_valid_credentials() {
    let server = smtp_server().await;
    let provider = SmtpProvider::new(plain_server(server.port), USER.into(), PASS.into());
    provider.test().await.unwrap();
}

#[tokio::test]
async fn unreachable_server_is_a_network_error() {
    // Port 1 on localhost is closed.
    let provider = SmtpProvider::new(plain_server(1), USER.into(), PASS.into());
    assert!(provider.test().await.is_err());
}

#[test]
fn build_message_validates_recipients() {
    let account = account(AccountKind::Imap, "me@example.com");
    let mut out = outgoing();
    out.to = vec![];
    assert!(AppContext::build_message(&account, &out).is_err());
    out.to = vec!["not an address".into()];
    assert!(AppContext::build_message(&account, &out).is_err());
}

#[test]
fn plain_text_message_without_html_is_single_part() {
    let account = account(AccountKind::Imap, "me@example.com");
    let out = OutgoingEmail { body_html: None, attachments: vec![], ..outgoing() };
    let raw = String::from_utf8(AppContext::build_message(&account, &out).unwrap().formatted()).unwrap();
    assert!(!raw.contains("multipart/"));
    assert!(raw.contains("text/plain"));
}
