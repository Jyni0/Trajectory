//! End-to-end tests of the service layer and the sync engine against the
//! mock IMAP / POP3 / SMTP servers.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;

use super::fixtures::*;
use super::mock_servers::*;
use super::test_context;
use crate::error::AppError;
use crate::mail_engine::{sync_account, sync_folder, SyncCommand, SyncEngine};
use crate::models::*;
use crate::secrets::{SecretStore, KEY_PASSWORD};
use crate::service::{events, AppContext};

fn new_account(kind: AccountKind, imap_port: u16, smtp_port: u16, password: &str) -> NewAccount {
    NewAccount {
        kind,
        display_name: "Me".into(),
        email: USER.into(),
        incoming: Some(plain_server(imap_port)),
        outgoing: Some(plain_server(smtp_port)),
        username: None,
        password: Some(password.into()),
        pop3_download_days: Some(14),
    }
}

async fn wait_until(mut f: impl FnMut() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("condition not met in time");
}

async fn imap_setup(messages: Vec<ImapMessage>) -> (Arc<AppContext>, Arc<super::RecordingSink>, ImapMock, SmtpMock, Account) {
    let imap = imap_server(vec![("INBOX".into(), messages), ("Sent".into(), vec![]), ("Trash".into(), vec![])]).await;
    let smtp = smtp_server().await;
    let (ctx, sink, _) = test_context();
    let account = ctx.add_account(new_account(AccountKind::Imap, imap.port, smtp.port, PASS), true).await.unwrap();
    (ctx, sink, imap, smtp, account)
}

fn inbox_of(ctx: &AppContext, account: &Account) -> Folder {
    ctx.db.find_folder_by_role(&account.id, FolderRole::Inbox).unwrap().unwrap()
}

#[tokio::test]
async fn add_account_validates_input() {
    let (ctx, _, _) = test_context();
    let mut bad = new_account(AccountKind::Imap, 1, 1, PASS);
    bad.email = "nope".into();
    assert!(matches!(ctx.add_account(bad, false).await, Err(AppError::Invalid(_))));

    let mut no_pass = new_account(AccountKind::Imap, 1, 1, "");
    no_pass.password = None;
    assert!(matches!(ctx.add_account(no_pass, false).await, Err(AppError::Invalid(_))));

    let mut no_host = new_account(AccountKind::Pop3, 1, 1, PASS);
    no_host.incoming = None;
    assert!(matches!(ctx.add_account(no_host, false).await, Err(AppError::Invalid(_))));

    let oauth = NewAccount { kind: AccountKind::Gmail, ..new_account(AccountKind::Imap, 1, 1, PASS) };
    assert!(matches!(ctx.add_account(oauth, false).await, Err(AppError::Invalid(_))));
}

#[tokio::test]
async fn add_account_verifies_credentials_and_stores_secret_in_keychain() {
    let imap = imap_server(vec![("INBOX".into(), vec![])]).await;
    let smtp = smtp_server().await;
    let (ctx, _, secrets) = test_context();

    let err = ctx.add_account(new_account(AccountKind::Imap, imap.port, smtp.port, "wrong"), true).await.unwrap_err();
    assert!(matches!(err, AppError::Auth(_)), "{err:?}");
    assert!(ctx.list_accounts().unwrap().is_empty());

    let report = ctx.test_connection(&new_account(AccountKind::Imap, imap.port, smtp.port, PASS)).await.unwrap();
    assert!(report.ok(), "{report:?}");

    let acc = ctx.add_account(new_account(AccountKind::Imap, imap.port, smtp.port, PASS), true).await.unwrap();
    assert_eq!(secrets.get(&acc.id, KEY_PASSWORD).unwrap().as_deref(), Some(PASS));
    // Passwords are never written to SQLite.
    assert!(!format!("{:?}", ctx.db.get_account(&acc.id).unwrap()).contains(PASS));
    let roles: Vec<_> = ctx.list_folders(&acc.id).unwrap().iter().map(|f| f.role).collect();
    assert_eq!(roles, [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts, FolderRole::Trash]);

    let dup = ctx.add_account(new_account(AccountKind::Imap, imap.port, smtp.port, PASS), false).await;
    assert!(matches!(dup, Err(AppError::Invalid(_))));

    ctx.remove_account(&acc.id).unwrap();
    assert!(ctx.list_accounts().unwrap().is_empty());
    assert_eq!(secrets.get(&acc.id, KEY_PASSWORD).unwrap(), None);
}

#[tokio::test]
async fn connection_report_pinpoints_failing_server() {
    let imap = imap_server(vec![("INBOX".into(), vec![])]).await;
    let (ctx, _, _) = test_context();
    let report = ctx.test_connection(&new_account(AccountKind::Imap, imap.port, 1, PASS)).await.unwrap();
    assert!(report.incoming_ok);
    assert!(!report.outgoing_ok);
    assert!(report.outgoing_error.is_some());
}

#[tokio::test]
async fn imap_sync_populates_cache_and_emits_events() {
    let (ctx, sink, imap, _smtp, account) = imap_setup(vec![
        imap_msg(1, &["\\Seen"], simple_text("Read one", "r@t")),
        imap_msg(2, &[], complex_message()),
    ])
    .await;

    let report = sync_account(&ctx, &account).await.unwrap();
    assert_eq!(report.new_messages, 2);
    assert_eq!(report.folders, 3);

    // Local Sent/Trash placeholders were promoted to the server folders.
    let folders = ctx.list_folders(&account.id).unwrap();
    let sent = folders.iter().find(|f| f.role == FolderRole::Sent).unwrap();
    assert_eq!(sent.path, "Sent");
    assert_eq!(folders.iter().filter(|f| f.role == FolderRole::Sent).count(), 1);

    let inbox = inbox_of(&ctx, &account);
    assert_eq!((inbox.total_count, inbox.unread_count), (2, 1));

    let new = sink.find(events::MAIL_NEW).expect("mail:new emitted");
    assert_eq!(new["count"], 1);
    assert_eq!(new["latest"][0]["subject"], COMPLEX_SUBJECT);

    // Second pass is incremental: nothing new, no event.
    sink.events.lock().unwrap().clear();
    assert_eq!(sync_account(&ctx, &account).await.unwrap().new_messages, 0);
    assert!(sink.find(events::MAIL_NEW).is_none());

    imap.mailboxes.lock().unwrap()[0].1.push(imap_msg(3, &[], simple_text("Fresh", "f@t")));
    assert_eq!(sync_account(&ctx, &account).await.unwrap().new_messages, 1);
}

#[tokio::test]
async fn reading_an_email_inlines_cid_images_and_hides_inline_parts() {
    let (ctx, _sink, _imap, _smtp, account) = imap_setup(vec![imap_msg(1, &[], complex_message())]).await;
    sync_account(&ctx, &account).await.unwrap();
    let page = ctx.list_emails(&EmailQuery { account_id: Some(account.id.clone()), ..Default::default() }).unwrap();
    let email = ctx.get_email(&page.items[0].id).unwrap();
    assert!(email.body_html.unwrap().contains(&format!("data:image/png;base64,{PNG_B64}")));
    assert_eq!(email.attachments.len(), 1);
    assert_eq!(email.attachments[0].filename, "report.pdf");

    let dir = tempfile::tempdir().unwrap();
    let p1 = ctx.save_attachment(&email.attachments[0].id, dir.path()).unwrap();
    let p2 = ctx.save_attachment(&email.attachments[0].id, dir.path()).unwrap();
    assert_eq!(p1.file_name().unwrap(), "report.pdf");
    assert_eq!(p2.file_name().unwrap(), "report (1).pdf");
    assert_eq!(std::fs::read(p1).unwrap(), b"%PDF-1.4\n");
}

#[tokio::test]
async fn user_actions_update_cache_and_replay_on_server() {
    let (ctx, _sink, imap, _smtp, account) = imap_setup(vec![imap_msg(7, &[], simple_text("Act", "act@t"))]).await;
    sync_account(&ctx, &account).await.unwrap();
    let id = ctx.list_emails(&EmailQuery::default()).unwrap().items[0].id.clone();

    ctx.mark_read(&id, true).unwrap();
    ctx.set_flagged(&id, true).unwrap();
    let e = ctx.get_email(&id).unwrap();
    assert!(e.summary.is_read && e.summary.is_flagged);
    wait_until(|| {
        let mbs = imap.mailboxes.lock().unwrap();
        let f = &mbs[0].1[0].flags;
        f.contains(&"\\Seen".to_string()) && f.contains(&"\\Flagged".to_string())
    })
    .await;

    // Delete → Trash (moved on the server too).
    ctx.delete_email(&id).unwrap();
    let trash = ctx.db.find_folder_by_role(&account.id, FolderRole::Trash).unwrap().unwrap();
    assert_eq!(ctx.get_email(&id).unwrap().summary.folder_id, trash.id);
    wait_until(|| imap.mailboxes.lock().unwrap()[2].1.len() == 1).await;
}

#[tokio::test]
async fn archive_creates_local_archive_folder_when_server_has_none() {
    let (ctx, _sink, _imap, _smtp, account) = imap_setup(vec![imap_msg(1, &[], simple_text("Arch", "ar@t"))]).await;
    sync_account(&ctx, &account).await.unwrap();
    let id = ctx.list_emails(&EmailQuery::default()).unwrap().items[0].id.clone();
    ctx.archive_email(&id).unwrap();
    let archive = ctx.db.find_folder_by_role(&account.id, FolderRole::Archive).unwrap().unwrap();
    assert!(archive.is_local());
    assert_eq!(ctx.get_email(&id).unwrap().summary.folder_id, archive.id);
}

#[tokio::test]
async fn moving_between_accounts_is_rejected() {
    let (ctx, _sink, _imap, _smtp, account) = imap_setup(vec![imap_msg(1, &[], simple_text("M", "m@t"))]).await;
    sync_account(&ctx, &account).await.unwrap();
    let other = account_for_db(&ctx);
    let other_inbox = ctx.db.ensure_folder(&other.id, FolderRole::Inbox).unwrap();
    let id = ctx.list_emails(&EmailQuery { account_id: Some(account.id.clone()), ..Default::default() }).unwrap().items[0].id.clone();
    assert!(matches!(ctx.move_email(&id, &other_inbox.id), Err(AppError::Invalid(_))));
}

fn account_for_db(ctx: &AppContext) -> Account {
    let a = account(AccountKind::Pop3, "other@example.com");
    ctx.db.insert_account(&a).unwrap();
    a
}

#[tokio::test]
async fn send_email_delivers_files_sent_copy_and_removes_draft() {
    let (ctx, sink, imap, smtp, account) = imap_setup(vec![]).await;
    sync_account(&ctx, &account).await.unwrap();

    let draft_id = ctx
        .save_draft(OutgoingEmail { account_id: account.id.clone(), subject: "Draft".into(), body_text: "wip".into(), ..Default::default() })
        .unwrap();

    let out = OutgoingEmail {
        account_id: account.id.clone(),
        to: vec!["friend@example.com".into()],
        subject: "Hello friend".into(),
        body_text: "Body".into(),
        body_html: Some("<p>Body</p>".into()),
        attachments: vec![OutgoingAttachment {
            filename: "a.txt".into(),
            mime_type: "text/plain".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode("attached"),
        }],
        draft_id: Some(draft_id.clone()),
        ..Default::default()
    };
    let sent_id = ctx.send_email(out).await.unwrap();

    let cap = smtp.captured.lock().unwrap().clone();
    assert_eq!(cap.len(), 1);
    assert!(cap[0].data.contains("Subject: Hello friend"));

    let sent = ctx.get_email(&sent_id).unwrap();
    assert_eq!(sent.summary.subject, "Hello friend");
    assert!(sent.summary.is_read);
    assert_eq!(sent.attachments[0].filename, "a.txt");
    assert!(matches!(ctx.get_email(&draft_id), Err(AppError::NotFound(_))));
    assert!(sink.names().contains(&events::MAIL_CHANGED.to_string()));

    // A copy is APPENDed to the server's Sent folder in the background,
    // and the next sync recognises it by Message-ID instead of duplicating.
    wait_until(|| imap.mailboxes.lock().unwrap()[1].1.len() == 1).await;
    sync_account(&ctx, &account).await.unwrap();
    let sent_folder = ctx.db.find_folder_by_role(&account.id, FolderRole::Sent).unwrap().unwrap();
    assert_eq!(sent_folder.total_count, 1);
}

#[tokio::test]
async fn send_failure_keeps_draft() {
    let (ctx, _sink, _imap, _smtp, account) = imap_setup(vec![]).await;
    let draft_id = ctx
        .save_draft(OutgoingEmail { account_id: account.id.clone(), subject: "Keep".into(), ..Default::default() })
        .unwrap();
    // No recipients → validation error before anything is sent.
    let res = ctx
        .send_email(OutgoingEmail { account_id: account.id.clone(), draft_id: Some(draft_id.clone()), ..Default::default() })
        .await;
    assert!(matches!(res, Err(AppError::Invalid(_))));
    assert!(ctx.get_email(&draft_id).is_ok());
}

#[tokio::test]
async fn drafts_are_replaced_and_permanently_deleted() {
    let (ctx, _, _) = test_context();
    let acc = account_for_db(&ctx);
    let d1 = ctx
        .save_draft(OutgoingEmail {
            account_id: acc.id.clone(),
            to: vec!["Bob <bob@example.com>".into(), "".into()],
            subject: "v1".into(),
            body_text: "first".into(),
            ..Default::default()
        })
        .unwrap();
    let d2 = ctx
        .save_draft(OutgoingEmail { account_id: acc.id.clone(), subject: "v2".into(), draft_id: Some(d1.clone()), ..Default::default() })
        .unwrap();
    assert!(ctx.get_email(&d1).is_err());
    let drafts = ctx.db.find_folder_by_role(&acc.id, FolderRole::Drafts).unwrap().unwrap();
    assert_eq!(drafts.total_count, 1);

    let first = ctx.save_draft(OutgoingEmail { account_id: acc.id.clone(), to: vec!["Bob <bob@example.com>".into()], ..Default::default() }).unwrap();
    let e = ctx.get_email(&first).unwrap();
    assert_eq!(e.to, vec![Address { name: Some("Bob".into()), address: "bob@example.com".into() }]);

    // Deleting from Drafts is permanent (no trip through Trash).
    ctx.delete_email(&d2).unwrap();
    assert!(matches!(ctx.get_email(&d2), Err(AppError::NotFound(_))));
}

#[tokio::test]
async fn pop3_account_sync_through_engine() {
    let pop = pop3_server(vec![("u1".into(), simple_text("Pop mail", "p1@t"))]).await;
    let smtp = smtp_server().await;
    let (ctx, sink, _) = test_context();
    let mut na = new_account(AccountKind::Pop3, pop.port, smtp.port, PASS);
    na.pop3_download_days = None;
    let account = ctx.add_account(na, true).await.unwrap();

    let (engine, handle) = SyncEngine::new(ctx.clone(), Duration::from_secs(3600));
    let task = tokio::spawn(engine.run());
    // The first interval tick fires immediately; an explicit request also works.
    handle.request(SyncCommand::Account(account.id.clone()));
    wait_until(|| sink.find(events::MAIL_NEW).is_some()).await;

    let page = ctx.list_emails(&EmailQuery { account_id: Some(account.id.clone()), ..Default::default() }).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].subject, "Pop mail");
    let statuses: Vec<_> = sink
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, _)| n == events::SYNC_STATUS)
        .map(|(_, v)| v["state"].as_str().unwrap().to_string())
        .collect();
    assert!(statuses.contains(&"syncing".to_string()) && statuses.contains(&"idle".to_string()));

    // Permanently deleting from Trash removes it from the POP3 maildrop.
    let id = page.items[0].id.clone();
    ctx.delete_email(&id).unwrap();
    ctx.delete_email(&id).unwrap();
    wait_until(|| pop.mailbox.lock().unwrap().is_empty()).await;

    drop(handle);
    tokio::time::timeout(Duration::from_secs(5), task).await.expect("engine stops when handles drop").unwrap();
}

#[tokio::test]
async fn sync_errors_are_reported_as_events() {
    let imap = imap_server(vec![("INBOX".into(), vec![])]).await;
    let smtp = smtp_server().await;
    let (ctx, sink, secrets) = test_context();
    let account = ctx.add_account(new_account(AccountKind::Imap, imap.port, smtp.port, PASS), true).await.unwrap();
    secrets.set(&account.id, KEY_PASSWORD, "rotated-elsewhere").unwrap();

    let (engine, handle) = SyncEngine::new(ctx.clone(), Duration::from_secs(3600));
    tokio::spawn(engine.run());
    handle.request(SyncCommand::All);
    wait_until(|| {
        sink.events
            .lock()
            .unwrap()
            .iter()
            .any(|(n, v)| n == events::SYNC_STATUS && v["state"] == "error")
    })
    .await;
}

#[tokio::test]
async fn opening_a_folder_fetches_only_its_new_mail() {
    let imap = imap_server(vec![
        ("INBOX".into(), vec![imap_msg(1, &[], simple_text("First", "f1@t"))]),
        ("Sent".into(), vec![]),
    ])
    .await;
    let smtp = smtp_server().await;
    let (ctx, sink, _) = test_context();
    let account = ctx.add_account(new_account(AccountKind::Imap, imap.port, smtp.port, PASS), true).await.unwrap();
    sync_account(&ctx, &account).await.unwrap();
    let inbox = ctx.db.list_folders(&account.id).unwrap().into_iter().find(|f| f.role == FolderRole::Inbox).unwrap();
    sink.events.lock().unwrap().clear();

    imap.mailboxes.lock().unwrap()[0].1.push(imap_msg(2, &[], simple_text("Second", "f2@t")));
    assert_eq!(sync_folder(&ctx, &account, &inbox.id).await.unwrap(), 1);
    let new = sink.find(events::MAIL_NEW).expect("new inbox mail is announced");
    assert_eq!(new["count"], 1);
    assert_eq!(new["latest"][0]["subject"], "Second");

    // Through the engine, a folder request reports progress like an account sync.
    let (engine, handle) = SyncEngine::new(ctx.clone(), Duration::from_secs(3600));
    tokio::spawn(engine.run());
    sink.events.lock().unwrap().clear();
    handle.request(SyncCommand::Folder(inbox.id.clone()));
    wait_until(|| sink.events.lock().unwrap().iter().any(|(n, v)| n == events::SYNC_STATUS && v["state"] == "idle")).await;
}
