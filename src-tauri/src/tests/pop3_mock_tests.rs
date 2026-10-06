use super::fixtures::*;
use super::mock_servers::{pop3_server, PASS, USER};
use crate::error::AppError;
use crate::models::FolderRole;
use crate::providers::pop3::{Pop3Client, Pop3Provider};
use crate::providers::{IncomingProvider, RemoteAction, RemoteFolder};

fn inbox() -> RemoteFolder {
    RemoteFolder { name: "Inbox".into(), path: "INBOX".into(), role: FolderRole::Inbox }
}

fn dated(subject: &str, date: &str) -> Vec<u8> {
    crlf(&format!("From: a@b.c\nSubject: {subject}\nDate: {date}\nMessage-ID: <{subject}@t>\n\nbody of {subject}\n"))
}

#[tokio::test]
async fn delta_sync_only_downloads_new_messages() {
    let server = pop3_server(vec![
        ("uid-a".into(), simple_text("First", "a@t")),
        ("uid-b".into(), simple_text("Second", "b@t")),
    ])
    .await;
    let p = Pop3Provider::new(plain_server(server.port), USER.into(), PASS.into(), None);

    p.test().await.unwrap();
    assert_eq!(p.list_folders().await.unwrap(), vec![inbox()]);

    let first = p.fetch(&inbox(), None).await.unwrap();
    let ids: Vec<_> = first.messages.iter().map(|m| m.remote_id.as_str()).collect();
    assert_eq!(ids, ["uid-a", "uid-b"]);
    assert!(crate::mime::parse_message(&first.messages[1].raw).unwrap().subject == "Second");

    // Nothing new → nothing downloaded.
    let second = p.fetch(&inbox(), first.new_state.as_deref()).await.unwrap();
    assert!(second.messages.is_empty());

    // New mail arrives.
    server.mailbox.lock().unwrap().push(("uid-c".into(), simple_text("Third", "c@t")));
    let third = p.fetch(&inbox(), second.new_state.as_deref()).await.unwrap();
    assert_eq!(third.messages.len(), 1);
    assert_eq!(third.messages[0].remote_id, "uid-c");
    let retr_count = server.log.lock().unwrap().iter().filter(|l| l.starts_with("RETR")).count();
    assert_eq!(retr_count, 3, "each message downloaded exactly once");
}

#[tokio::test]
async fn dot_stuffed_lines_are_restored() {
    let raw = crlf("From: a@b.c\nSubject: dots\n\n.leading dot\n..two dots\nend\n");
    let server = pop3_server(vec![("u1".into(), raw.clone())]).await;
    let p = Pop3Provider::new(plain_server(server.port), USER.into(), PASS.into(), None);
    let res = p.fetch(&inbox(), None).await.unwrap();
    assert_eq!(res.messages[0].raw, raw);
}

#[tokio::test]
async fn download_period_skips_old_messages() {
    let recent = chrono::Utc::now().to_rfc2822();
    let server = pop3_server(vec![
        ("old".into(), dated("Old", "Mon, 01 Jan 2001 00:00:00 +0000")),
        ("new".into(), dated("New", &recent)),
    ])
    .await;
    let p = Pop3Provider::new(plain_server(server.port), USER.into(), PASS.into(), Some(30));
    let res = p.fetch(&inbox(), None).await.unwrap();
    assert_eq!(res.messages.len(), 1);
    assert_eq!(res.messages[0].remote_id, "new");
    // The old message is remembered and not checked again.
    let again = p.fetch(&inbox(), res.new_state.as_deref()).await.unwrap();
    assert!(again.messages.is_empty());
    let tops = server.log.lock().unwrap().iter().filter(|l| l.starts_with("TOP")).count();
    assert_eq!(tops, 2);
}

#[tokio::test]
async fn bad_password_is_an_auth_error() {
    let server = pop3_server(vec![]).await;
    let p = Pop3Provider::new(plain_server(server.port), USER.into(), "nope".into(), None);
    assert!(matches!(p.test().await, Err(AppError::Auth(_))));
}

#[tokio::test]
async fn permanent_delete_issues_dele_for_matching_uidl() {
    let server = pop3_server(vec![("x".into(), simple_text("X", "x@t")), ("y".into(), simple_text("Y", "y@t"))]).await;
    let p = Pop3Provider::new(plain_server(server.port), USER.into(), PASS.into(), None);
    p.apply("INBOX", "y", &RemoteAction::MarkRead(true)).await.unwrap();
    p.apply("INBOX", "y", &RemoteAction::Delete).await.unwrap();
    assert!(server.log.lock().unwrap().contains(&"DELE 2".to_string()));
    let remaining: Vec<_> = server.mailbox.lock().unwrap().iter().map(|(u, _)| u.clone()).collect();
    assert_eq!(remaining, ["x"]);
}

#[tokio::test]
async fn raw_client_stat_and_uidl() {
    let server = pop3_server(vec![("only".into(), b"Subject: s\r\n\r\nhi\r\n".to_vec())]).await;
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", server.port)).await.unwrap();
    let mut c = Pop3Client::connect(tcp).await.unwrap();
    c.login(USER, PASS).await.unwrap();
    assert_eq!(c.stat().await.unwrap(), (1, 18));
    assert_eq!(c.uidl().await.unwrap(), vec![(1, "only".to_string())]);
    assert!(c.retr(5).await.is_err(), "-ERR is surfaced");
    c.quit().await.unwrap();
}
