use super::fixtures::*;
use super::mock_servers::{imap_msg, imap_server, PASS, USER};
use crate::error::AppError;
use crate::models::FolderRole;
use crate::providers::imap::{ImapProvider, ImapState};
use crate::providers::{IncomingProvider, RemoteAction, RemoteFolder};

fn provider(port: u16, pass: &str) -> ImapProvider {
    ImapProvider::new(plain_server(port), USER.into(), pass.into())
}

fn folder(path: &str, role: FolderRole) -> RemoteFolder {
    RemoteFolder { name: path.into(), path: path.into(), role }
}

#[tokio::test]
async fn lists_folders_with_roles_and_skips_noselect() {
    let server = imap_server(vec![
        ("INBOX".into(), vec![]),
        ("Sent".into(), vec![]),
        ("Trash".into(), vec![]),
        ("Projects".into(), vec![]),
    ])
    .await;
    let folders = provider(server.port, PASS).list_folders().await.unwrap();
    let pairs: Vec<_> = folders.iter().map(|f| (f.path.as_str(), f.role)).collect();
    assert_eq!(
        pairs,
        [
            ("INBOX", FolderRole::Inbox),
            ("Sent", FolderRole::Sent),
            ("Trash", FolderRole::Trash),
            ("Projects", FolderRole::Custom)
        ]
    );
}

#[tokio::test]
async fn initial_then_incremental_uid_sync() {
    let server = imap_server(vec![(
        "INBOX".into(),
        vec![
            imap_msg(10, &["\\Seen"], simple_text("One", "1@t")),
            imap_msg(11, &["\\Flagged"], complex_message()),
        ],
    )])
    .await;
    let p = provider(server.port, PASS);
    let inbox = folder("INBOX", FolderRole::Inbox);

    let first = p.fetch(&inbox, None).await.unwrap();
    assert_eq!(first.messages.len(), 2);
    assert_eq!(first.messages[0].remote_id, "10");
    assert!(first.messages[0].is_read && !first.messages[0].is_flagged);
    assert!(!first.messages[1].is_read && first.messages[1].is_flagged);
    assert_eq!(crate::mime::parse_message(&first.messages[1].raw).unwrap().subject, COMPLEX_SUBJECT);
    let state: ImapState = serde_json::from_str(first.new_state.as_deref().unwrap()).unwrap();
    assert_eq!(state, ImapState { uid_validity: 42, last_uid: 11 });

    // `UID 12:*` returns the last message again — it must be ignored.
    let second = p.fetch(&inbox, first.new_state.as_deref()).await.unwrap();
    assert!(second.messages.is_empty());
    assert!(server.log.lock().unwrap().iter().any(|l| l.contains("UID FETCH 12:*")));

    server.mailboxes.lock().unwrap()[0].1.push(imap_msg(12, &[], simple_text("Three", "3@t")));
    let third = p.fetch(&inbox, second.new_state.as_deref()).await.unwrap();
    assert_eq!(third.messages.iter().map(|m| m.remote_id.as_str()).collect::<Vec<_>>(), ["12"]);
}

#[tokio::test]
async fn uidvalidity_change_triggers_full_resync() {
    let server = imap_server(vec![("INBOX".into(), vec![imap_msg(5, &[], simple_text("A", "a@t"))])]).await;
    let stale = serde_json::to_string(&ImapState { uid_validity: 7, last_uid: 100 }).unwrap();
    let res = provider(server.port, PASS).fetch(&folder("INBOX", FolderRole::Inbox), Some(&stale)).await.unwrap();
    assert_eq!(res.messages.len(), 1);
}

#[tokio::test]
async fn login_failure_is_auth_error() {
    let server = imap_server(vec![("INBOX".into(), vec![])]).await;
    assert!(matches!(provider(server.port, "bad").test().await, Err(AppError::Auth(_))));
}

#[tokio::test]
async fn flags_move_and_delete_are_replayed() {
    let server = imap_server(vec![
        ("INBOX".into(), vec![imap_msg(1, &[], simple_text("A", "a@t")), imap_msg(2, &[], simple_text("B", "b@t"))]),
        ("Trash".into(), vec![]),
    ])
    .await;
    let p = provider(server.port, PASS);

    p.apply("INBOX", "1", &RemoteAction::MarkRead(true)).await.unwrap();
    p.apply("INBOX", "1", &RemoteAction::Flag(true)).await.unwrap();
    {
        let mbs = server.mailboxes.lock().unwrap();
        let flags = &mbs[0].1[0].flags;
        assert!(flags.contains(&"\\Seen".to_string()) && flags.contains(&"\\Flagged".to_string()));
    }
    p.apply("INBOX", "1", &RemoteAction::MarkRead(false)).await.unwrap();
    assert!(!server.mailboxes.lock().unwrap()[0].1[0].flags.contains(&"\\Seen".to_string()));

    p.apply("INBOX", "1", &RemoteAction::Trash { trash_path: Some("Trash".into()) }).await.unwrap();
    {
        let mbs = server.mailboxes.lock().unwrap();
        assert_eq!(mbs[0].1.len(), 1);
        assert_eq!(mbs[1].1.len(), 1);
    }

    p.apply("INBOX", "2", &RemoteAction::Delete).await.unwrap();
    assert!(server.mailboxes.lock().unwrap()[0].1.is_empty());
    assert!(matches!(p.apply("INBOX", "abc", &RemoteAction::Delete).await, Err(AppError::Invalid(_))));
}

#[tokio::test]
async fn append_stores_sent_copy() {
    let server = imap_server(vec![("INBOX".into(), vec![]), ("Sent".into(), vec![])]).await;
    let raw = simple_text("Sent copy", "s@t");
    provider(server.port, PASS).append("Sent", &raw).await.unwrap();
    let mbs = server.mailboxes.lock().unwrap();
    assert_eq!(mbs[1].1.len(), 1);
    assert_eq!(mbs[1].1[0].raw, raw);
}
