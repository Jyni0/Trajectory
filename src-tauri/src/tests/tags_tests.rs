//! Tags, notes, list ordering, and the security guards added with them.

use super::fixtures::*;
use super::test_context;
use crate::db::Database;
use crate::error::AppError;
use crate::models::*;
use crate::net::is_private_host;
use crate::service::safe_filename;

fn db_with_mail() -> (Database, Account, Folder, Vec<String>) {
    let db = Database::open_in_memory().unwrap();
    let acc = account(AccountKind::Imap, "me@example.com");
    db.insert_account(&acc).unwrap();
    let inbox = db.ensure_folder(&acc.id, FolderRole::Inbox).unwrap();
    let mut ids = vec![];
    for (i, subject) in ["Beta", "alpha", "Gamma"].iter().enumerate() {
        let mut e = new_email(&acc.id, &inbox.id, &i.to_string(), subject, 100 + i as i64);
        e.is_read = i != 1;
        e.is_flagged = i == 0;
        e.from = Address { name: Some(["Zed", "Amy", "Bob"][i].into()), address: format!("{i}@x.io") };
        ids.push(db.insert_email(&e).unwrap().unwrap());
    }
    (db, acc, inbox, ids)
}

fn subjects(db: &Database, q: EmailQuery) -> Vec<String> {
    db.list_emails(&q).unwrap().items.into_iter().map(|e| e.subject).collect()
}

#[test]
fn tags_are_created_ordered_renamed_and_deleted() {
    let (db, ..) = db_with_mail();
    let work = db.create_tag("  Work  ", "#2563EB").unwrap();
    let home = db.create_tag("Home", "#16a34a").unwrap();
    assert_eq!((work.name.as_str(), work.color.as_str(), work.position), ("Work", "#2563eb", 0));
    assert_eq!(home.position, 1);

    assert!(matches!(db.create_tag("work", "#000000"), Err(AppError::Invalid(_))), "names are unique ignoring case");
    assert!(matches!(db.create_tag("   ", "#000000"), Err(AppError::Invalid(_))));
    assert!(matches!(db.create_tag("x", "red"), Err(AppError::Invalid(_))));
    assert!(matches!(db.create_tag("x", "#12345g"), Err(AppError::Invalid(_))));
    assert!(matches!(db.create_tag(&"x".repeat(41), "#000000"), Err(AppError::Invalid(_))));

    db.reorder_tags(std::slice::from_ref(&home.id)).unwrap();
    let names: Vec<_> = db.list_tags(None).unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(names, ["Home", "Work"]);

    let renamed = db.update_tag(&work.id, "Office", "#ea580c").unwrap();
    assert_eq!((renamed.name.as_str(), renamed.color.as_str()), ("Office", "#ea580c"));
    assert!(matches!(db.update_tag(&work.id, "home", "#ea580c"), Err(AppError::Invalid(_))));

    db.delete_tag(&home.id).unwrap();
    assert_eq!(db.list_tags(None).unwrap().len(), 1);
    assert!(matches!(db.delete_tag(&home.id), Err(AppError::NotFound(_))));
}

#[test]
fn tagging_filters_counts_and_cascades() {
    let (db, acc, inbox, ids) = db_with_mail();
    let tag = db.create_tag("Work", "#2563eb").unwrap();
    db.set_email_tag(&ids[0], &tag.id, true).unwrap();
    db.set_email_tag(&ids[0], &tag.id, true).unwrap(); // idempotent
    db.set_email_tag(&ids[2], &tag.id, true).unwrap();

    assert_eq!(db.list_tags(Some(&acc.id)).unwrap()[0].count, 2);
    assert_eq!(db.list_tags(Some("other")).unwrap()[0].count, 0);

    let page = db
        .list_emails(&EmailQuery { account_id: Some(acc.id.clone()), tag_id: Some(tag.id.clone()), ..Default::default() })
        .unwrap();
    assert_eq!(page.total, 2);
    assert!(page.items.iter().all(|e| e.tags == vec![tag.id.clone()]));

    db.set_email_tag(&ids[0], &tag.id, false).unwrap();
    assert_eq!(db.list_tags(None).unwrap()[0].count, 1);
    assert!(matches!(db.set_email_tag("nope", &tag.id, true), Err(AppError::NotFound(_))));
    assert!(matches!(db.set_email_tag(&ids[0], "nope", true), Err(AppError::NotFound(_))));

    // Moving keeps the tag; deleting the message or the tag drops the link.
    let archive = db.ensure_folder(&acc.id, FolderRole::Archive).unwrap();
    db.move_email(&ids[2], &archive.id).unwrap();
    assert_eq!(db.get_email(&ids[2]).unwrap().summary.tags, vec![tag.id.clone()]);
    db.delete_tag(&tag.id).unwrap();
    assert!(db.get_email(&ids[2]).unwrap().summary.tags.is_empty());
    let _ = inbox;
}

#[test]
fn notes_are_saved_cleared_and_searchable() {
    let (db, acc, _, ids) = db_with_mail();
    db.set_note(&ids[1], Some("  call back on Friday  ")).unwrap();
    let e = db.get_email(&ids[1]).unwrap();
    assert_eq!(e.note.as_deref(), Some("call back on Friday"));
    assert!(e.summary.has_note);

    let found = subjects(&db, EmailQuery { account_id: Some(acc.id.clone()), search: Some("friday".into()), ..Default::default() });
    assert_eq!(found, ["alpha"]);

    db.set_note(&ids[1], Some("   ")).unwrap();
    let e = db.get_email(&ids[1]).unwrap();
    assert_eq!(e.note, None);
    assert!(!e.summary.has_note);

    assert!(matches!(db.set_note(&ids[1], Some(&"x".repeat(10_001))), Err(AppError::Invalid(_))));
    assert!(matches!(db.set_note("nope", Some("x")), Err(AppError::NotFound(_))));
}

#[test]
fn list_follows_the_chosen_sort() {
    let (db, acc, ..) = db_with_mail();
    let q = |sort| EmailQuery { account_id: Some(acc.id.clone()), sort, ..Default::default() };
    assert_eq!(subjects(&db, q(MailSort::Newest)), ["Gamma", "alpha", "Beta"]);
    assert_eq!(subjects(&db, q(MailSort::Oldest)), ["Beta", "alpha", "Gamma"]);
    assert_eq!(subjects(&db, q(MailSort::Unread))[0], "alpha");
    assert_eq!(subjects(&db, q(MailSort::Flagged))[0], "Beta");
    assert_eq!(subjects(&db, q(MailSort::Subject)), ["alpha", "Beta", "Gamma"]);
    assert_eq!(subjects(&db, q(MailSort::Sender)), ["alpha", "Gamma", "Beta"]); // Amy, Bob, Zed
}

#[test]
fn v1_database_is_migrated_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        // A schema-1 database as shipped before tags existed.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE emails (id TEXT PRIMARY KEY, account_id TEXT, folder_id TEXT, remote_id TEXT, message_id TEXT, in_reply_to TEXT, from_name TEXT, from_address TEXT NOT NULL, to_json TEXT NOT NULL, cc_json TEXT NOT NULL, reply_to_json TEXT NOT NULL, subject TEXT NOT NULL, snippet TEXT NOT NULL, body_text TEXT, body_html TEXT, date INTEGER NOT NULL, is_read INTEGER NOT NULL DEFAULT 0, is_flagged INTEGER NOT NULL DEFAULT 0, has_attachments INTEGER NOT NULL DEFAULT 0); CREATE TABLE sync_state (folder_id TEXT PRIMARY KEY, state TEXT NOT NULL); PRAGMA user_version = 1;").unwrap();
    }
    let db = Database::open(&path).unwrap();
    assert!(db.list_tags(None).unwrap().is_empty());
    drop(db);
    // Re-opening a migrated database is a no-op.
    Database::open(&path).unwrap();
}

#[test]
fn private_hosts_are_recognised() {
    for h in ["127.0.0.1", "localhost", "10.1.2.3", "192.168.1.5", "172.16.0.1", "100.101.102.103", "::1", "[fd00::1]", "nas.local", "mail.lan", "mailbox"] {
        assert!(is_private_host(h), "{h} should be private");
    }
    for h in ["imap.gmail.com", "8.8.8.8", "100.128.0.1", "172.32.0.1", "2001:db8::1", "example.com", ""] {
        assert!(!is_private_host(h), "{h} should be public");
    }
}

#[tokio::test]
async fn unencrypted_connection_to_a_public_server_is_refused() {
    let (ctx, ..) = test_context();
    let insecure = ServerConfig { host: "imap.example.com".into(), port: 143, security: Security::None };
    let new = NewAccount {
        kind: AccountKind::Imap,
        display_name: "Me".into(),
        email: "me@example.com".into(),
        incoming: Some(insecure),
        outgoing: Some(ServerConfig { host: "smtp.example.com".into(), port: 465, security: Security::Tls }),
        username: None,
        password: Some("pw".into()),
        pop3_download_days: None,
    };
    let err = ctx.test_connection(&new).await.unwrap_err();
    assert!(matches!(err, AppError::Invalid(ref m) if m.contains("imap.example.com")), "{err:?}");
    assert!(ctx.add_account(new, false).await.is_err());
}

#[test]
fn attachment_names_cannot_escape_or_hit_devices() {
    assert_eq!(safe_filename("../../etc/passwd"), ".._.._etc_passwd");
    assert_eq!(safe_filename(".."), "attachment");
    assert_eq!(safe_filename("   "), "attachment");
    assert_eq!(safe_filename("report.pdf. . "), "report.pdf");
    assert_eq!(safe_filename("CON"), "_CON");
    assert_eq!(safe_filename("nul.txt"), "_nul.txt");
    assert_eq!(safe_filename("com1.log"), "_com1.log");
    assert_eq!(safe_filename("console.txt"), "console.txt");
    assert_eq!(safe_filename("a\u{0}b\nc.txt"), "a_b_c.txt");
    assert!(safe_filename(&"x".repeat(500)).chars().count() <= 180);
}

#[test]
fn saving_an_attachment_never_overwrites() {
    let (ctx, ..) = test_context();
    let acc = account(AccountKind::Imap, "me@example.com");
    ctx.db.insert_account(&acc).unwrap();
    let inbox = ctx.db.ensure_folder(&acc.id, FolderRole::Inbox).unwrap();
    let mut e = new_email(&acc.id, &inbox.id, "1", "Hi", 1);
    e.attachments.push(crate::db::NewAttachment { filename: "a.txt".into(), mime_type: "text/plain".into(), content_id: None, data: b"new".to_vec() });
    let id = ctx.db.insert_email(&e).unwrap().unwrap();
    let att = ctx.db.get_email(&id).unwrap().attachments[0].id.clone();

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"old").unwrap();
    let saved = ctx.save_attachment(&att, dir.path()).unwrap();
    assert_eq!(saved.file_name().unwrap(), "a (1).txt");
    assert_eq!(std::fs::read(dir.path().join("a.txt")).unwrap(), b"old");
    assert_eq!(std::fs::read(saved).unwrap(), b"new");
}

#[test]
fn refetched_mail_gets_the_new_body_and_keeps_user_state() {
    let (db, acc, inbox, ids) = db_with_mail();
    let tag = db.create_tag("Keep", "#2563eb").unwrap();
    db.set_email_tag(&ids[0], &tag.id, true).unwrap();
    db.set_note(&ids[0], Some("mine")).unwrap();
    db.set_flagged(&ids[0], false).unwrap();

    let mut again = new_email(&acc.id, &inbox.id, "0", "Beta", 100);
    again.body_html = Some("<p>clean</p>".into());
    assert_eq!(db.insert_email(&again).unwrap(), None, "still a duplicate");
    let e = db.get_email(&ids[0]).unwrap();
    assert_eq!(e.body_html.as_deref(), Some("<p>clean</p>"));
    assert_eq!((e.summary.tags, e.note.as_deref(), e.summary.is_flagged), (vec![tag.id], Some("mine"), false));
}

#[test]
fn older_database_forgets_sync_cursors_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let acc = account(AccountKind::Imap, "me@example.com");
    let folder_id = {
        let db = Database::open(&path).unwrap();
        db.insert_account(&acc).unwrap();
        let f = db.ensure_folder(&acc.id, FolderRole::Inbox).unwrap();
        db.set_sync_state(&f.id, "cursor").unwrap();
        f.id
    };
    // Pretend the file was written by the previous version (schema 3).
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("ALTER TABLE emails DROP COLUMN logo_domain; DROP TABLE sender_logos; PRAGMA user_version = 3;")
        .unwrap();
    let db = Database::open(&path).unwrap();
    assert_eq!(db.get_sync_state(&folder_id).unwrap(), None);
    db.set_sync_state(&folder_id, "cursor2").unwrap();
    drop(db);
    let db = Database::open(&path).unwrap();
    assert_eq!(db.get_sync_state(&folder_id).unwrap().as_deref(), Some("cursor2"), "only once");
}

#[test]
fn system_folders_show_the_apps_name_not_the_servers() {
    let db = Database::open_in_memory().unwrap();
    let acc = account(AccountKind::Imap, "me@example.com");
    db.insert_account(&acc).unwrap();
    let inbox = db.upsert_remote_folder(&acc.id, "INBOX", "INBOX", FolderRole::Inbox).unwrap();
    let sent = db.upsert_remote_folder(&acc.id, "SENT ITEMS", "Sent Items", FolderRole::Sent).unwrap();
    let label = db.upsert_remote_folder(&acc.id, "VIP", "VIP", FolderRole::Custom).unwrap();
    assert_eq!((inbox.name.as_str(), sent.name.as_str(), label.name.as_str()), ("Inbox", "Sent", "VIP"));
    assert_eq!(inbox.path, "INBOX", "the server path is untouched");
}
