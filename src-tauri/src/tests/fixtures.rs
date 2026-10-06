//! Sample MIME messages and model builders.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::db::NewEmail;
use crate::models::*;

/// Normalizes line endings to CRLF as they appear on the wire.
pub fn crlf(s: &str) -> Vec<u8> {
    s.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes()
}

pub fn simple_text(subject: &str, message_id: &str) -> Vec<u8> {
    crlf(&format!(
        "From: Alice Example <alice@example.com>
To: Bob <bob@example.com>
Subject: {subject}
Date: Tue, 01 Sep 2026 10:00:00 +0000
Message-ID: <{message_id}>
Content-Type: text/plain; charset=utf-8

Hi Bob,
How are you?
"
    ))
}

pub const COMPLEX_SUBJECT: &str = "Отчёт за квартал";
pub const COMPLEX_FROM_NAME: &str = "Иван Петров";
pub const PNG_B64: &str = "iVBORw0KGgo=";
pub const PDF_B64: &str = "JVBERi0xLjQK";

/// multipart/mixed → related → alternative(QP text + hostile HTML) + inline
/// PNG (cid) + base64 PDF attachment, with RFC 2047 encoded headers.
pub fn complex_message() -> Vec<u8> {
    let subject = STANDARD.encode(COMPLEX_SUBJECT);
    let from = STANDARD.encode(COMPLEX_FROM_NAME);
    crlf(&format!(
        r#"From: =?UTF-8?B?{from}?= <ivan@example.ru>
To: bob@example.com, "Carol" <carol@example.com>
Cc: dave@example.com
Subject: =?UTF-8?B?{subject}?=
Date: Wed, 02 Sep 2026 12:30:00 +0300
Message-ID: <complex-1@example.ru>
In-Reply-To: <simple-1@example.com>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="mixed"

--mixed
Content-Type: multipart/related; boundary="related"

--related
Content-Type: multipart/alternative; boundary="alt"

--alt
Content-Type: text/plain; charset=utf-8
Content-Transfer-Encoding: quoted-printable

Quarterly report attached. Caf=C3=A9 =E2=9C=93
--alt
Content-Type: text/html; charset=utf-8

<html><head><style>body{{color:red}}</style><script>alert(1)</script></head><body><p onclick="steal()" style="color:#333">Quarterly <b>report</b></p><img src="cid:logo@x"><a href="javascript:alert(1)">bad</a><a href="https://example.com">good</a><div style="background:expression(alert(1))">x</div><iframe src="https://evil.example"></iframe></body></html>
--alt--
--related
Content-Type: image/png
Content-Transfer-Encoding: base64
Content-ID: <logo@x>
Content-Disposition: inline

{PNG_B64}
--related--
--mixed
Content-Type: application/pdf; name="report.pdf"
Content-Transfer-Encoding: base64
Content-Disposition: attachment; filename="report.pdf"

{PDF_B64}
--mixed--
"#
    ))
}

pub fn account(kind: AccountKind, email: &str) -> Account {
    Account {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        display_name: "Test User".into(),
        email: email.into(),
        incoming: Some(ServerConfig { host: "127.0.0.1".into(), port: 993, security: Security::Tls }),
        outgoing: Some(ServerConfig { host: "127.0.0.1".into(), port: 465, security: Security::Tls }),
        username: None,
        pop3_download_days: None,
        color: "#2563eb".into(),
        created_at: 1_700_000_000,
    }
}

pub fn new_email(account_id: &str, folder_id: &str, remote_id: &str, subject: &str, date: i64) -> NewEmail {
    NewEmail {
        account_id: account_id.into(),
        folder_id: folder_id.into(),
        remote_id: remote_id.into(),
        message_id: Some(format!("<{remote_id}@test>")),
        from: Address { name: Some("Alice".into()), address: "alice@example.com".into() },
        to: vec![Address { name: None, address: "bob@example.com".into() }],
        subject: subject.into(),
        snippet: format!("snippet of {subject}"),
        body_text: Some(format!("body of {subject}")),
        date,
        ..Default::default()
    }
}

pub fn plain_server(port: u16) -> ServerConfig {
    ServerConfig { host: "127.0.0.1".into(), port, security: Security::None }
}
