use super::fixtures::*;
use crate::db::{Database, NewAttachment};
use crate::error::AppError;
use crate::models::*;

fn db_with_account() -> (Database, Account, Folder) {
    let db = Database::open_in_memory().unwrap();
    let acc = account(AccountKind::Imap, "me@example.com");
    db.insert_account(&acc).unwrap();
    let inbox = db.ensure_folder(&acc.id, FolderRole::Inbox).unwrap();
    (db, acc, inbox)
}

#[test]
fn account_roundtrip_and_lookup() {
    let (db, acc, _) = db_with_account();
    assert_eq!(db.list_accounts().unwrap(), vec![acc.clone()]);
    assert_eq!(db.get_account(&acc.id).unwrap(), acc);
    assert!(db.find_account_by_email("ME@example.com").unwrap().is_some());
    assert!(matches!(db.get_account("nope"), Err(AppError::NotFound(_))));
}

#[test]
fn file_backed_database_persists_between_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let acc = account(AccountKind::Pop3, "pop@example.com");
    {
        let db = Database::open(&path).unwrap();
        db.insert_account(&acc).unwrap();
    }
    let db = Database::open(&path).unwrap();
    assert_eq!(db.get_account(&acc.id).unwrap().email, "pop@example.com");
}

#[test]
fn deleting_account_cascades_to_folders_emails_and_attachments() {
    let (db, acc, inbox) = db_with_account();
    let mut e = new_email(&acc.id, &inbox.id, "1", "Hello", 100);
    e.attachments.push(NewAttachment { filename: "a.txt".into(), mime_type: "text/plain".into(), content_id: None, data: b"x".to_vec() });
    let id = db.insert_email(&e).unwrap().unwrap();
    let att_id = db.get_email(&id).unwrap().attachments[0].id.clone();

    db.delete_account(&acc.id).unwrap();
    assert!(db.list_folders(&acc.id).unwrap().is_empty());
    assert!(matches!(db.get_email(&id), Err(AppError::NotFound(_))));
    assert!(matches!(db.get_attachment(&att_id), Err(AppError::NotFound(_))));
}

#[test]
fn folders_are_sorted_by_role_and_counted() {
    let (db, acc, inbox) = db_with_account();
    db.upsert_remote_folder(&acc.id, "Zeta", "Zeta", FolderRole::Custom).unwrap();
    db.upsert_remote_folder(&acc.id, "Alpha", "Alpha", FolderRole::Custom).unwrap();
    db.ensure_folder(&acc.id, FolderRole::Trash).unwrap();
    db.ensure_folder(&acc.id, FolderRole::Sent).unwrap();
    db.insert_email(&new_email(&acc.id, &inbox.id, "1", "a", 1)).unwrap();
    let mut read = new_email(&acc.id, &inbox.id, "2", "b", 2);
    read.is_read = true;
    db.insert_email(&read).unwrap();

    let folders = db.list_folders(&acc.id).unwrap();
    let names: Vec<_> = folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Inbox", "Sent", "Trash", "Alpha", "Zeta"]);
    assert_eq!((folders[0].unread_count, folders[0].total_count), (1, 2));
}

#[test]
fn remote_folder_promotes_local_placeholder_and_keeps_mail() {
    let (db, acc, _) = db_with_account();
    let local_sent = db.ensure_folder(&acc.id, FolderRole::Sent).unwrap();
    assert!(local_sent.is_local());
    db.insert_email(&new_email(&acc.id, &local_sent.id, "local-1", "sent", 1)).unwrap();

    let remote = db.upsert_remote_folder(&acc.id, "Sent Items", "Sent Items", FolderRole::Sent).unwrap();
    assert_eq!(remote.id, local_sent.id);
    assert_eq!(remote.path, "Sent Items");
    assert_eq!(remote.total_count, 1);

    // A second folder claiming the same role becomes a plain label.
    let dup = db.upsert_remote_folder(&acc.id, "Sent2", "Sent2", FolderRole::Sent).unwrap();
    assert_eq!(dup.role, FolderRole::Custom);
    // Upserting an existing path is idempotent.
    let again = db.upsert_remote_folder(&acc.id, "Sent Items", "Sent Items", FolderRole::Sent).unwrap();
    assert_eq!(again.id, local_sent.id);
}

#[test]
fn insert_deduplicates_by_remote_id_and_message_id() {
    let (db, acc, inbox) = db_with_account();
    let e = new_email(&acc.id, &inbox.id, "10", "Hello", 1);
    let id = db.insert_email(&e).unwrap().expect("first insert");
    assert!(db.insert_email(&e).unwrap().is_none(), "same remote id is a duplicate");

    // Same Message-ID under a new UID (e.g. after a server-side move) refreshes the remote id.
    let moved = crate::db::NewEmail { remote_id: "99".into(), ..e.clone() };
    assert!(db.insert_email(&moved).unwrap().is_none());
    assert_eq!(db.email_location(&id).unwrap().remote_id, "99");
    assert_eq!(db.list_emails(&EmailQuery::default()).unwrap().total, 1);
}

#[test]
fn pagination_returns_newest_first() {
    let (db, acc, inbox) = db_with_account();
    for i in 0..25 {
        db.insert_email(&new_email(&acc.id, &inbox.id, &i.to_string(), &format!("m{i}"), i)).unwrap();
    }
    let q = |offset| EmailQuery { folder_id: Some(inbox.id.clone()), limit: Some(10), offset: Some(offset), ..Default::default() };

    let p1 = db.list_emails(&q(0)).unwrap();
    assert_eq!(p1.total, 25);
    assert!(p1.has_more);
    assert_eq!(p1.items.first().unwrap().subject, "m24");
    assert_eq!(p1.items.len(), 10);

    let p3 = db.list_emails(&q(20)).unwrap();
    assert_eq!(p3.items.len(), 5);
    assert!(!p3.has_more);
    assert_eq!(p3.items.last().unwrap().subject, "m0");
}

#[test]
fn filters_and_search() {
    let (db, acc, inbox) = db_with_account();
    let mut a = new_email(&acc.id, &inbox.id, "1", "Invoice March", 1);
    a.is_flagged = true;
    a.is_read = true;
    let mut b = new_email(&acc.id, &inbox.id, "2", "Lunch?", 2);
    b.from = Address { name: Some("Zoë Kitchen".into()), address: "zoe@food.io".into() };
    b.body_text = Some("Pizza at 100% discount_today".into());
    db.insert_email(&a).unwrap();
    db.insert_email(&b).unwrap();

    let q = |filter, search: Option<&str>| EmailQuery {
        account_id: Some(acc.id.clone()),
        filter,
        search: search.map(str::to_string),
        ..Default::default()
    };
    let subjects = |p: EmailPage| p.items.into_iter().map(|e| e.subject).collect::<Vec<_>>();

    assert_eq!(subjects(db.list_emails(&q(MailFilter::Unread, None)).unwrap()), ["Lunch?"]);
    assert_eq!(subjects(db.list_emails(&q(MailFilter::Flagged, None)).unwrap()), ["Invoice March"]);
    assert_eq!(subjects(db.list_emails(&q(MailFilter::All, Some("invoice"))).unwrap()), ["Invoice March"]);
    assert_eq!(subjects(db.list_emails(&q(MailFilter::All, Some("zoe@food"))).unwrap()), ["Lunch?"]);
    assert_eq!(subjects(db.list_emails(&q(MailFilter::All, Some("Zoë"))).unwrap()), ["Lunch?"]);
    // LIKE wildcards in the search term are matched literally.
    assert_eq!(subjects(db.list_emails(&q(MailFilter::All, Some("100%"))).unwrap()), ["Lunch?"]);
    assert_eq!(db.list_emails(&q(MailFilter::All, Some("%"))).unwrap().total, 1);
    assert_eq!(db.list_emails(&q(MailFilter::All, Some("_"))).unwrap().total, 1);
    assert_eq!(db.list_emails(&q(MailFilter::All, Some("   "))).unwrap().total, 2);
}

#[test]
fn flags_move_and_delete() {
    let (db, acc, inbox) = db_with_account();
    let trash = db.ensure_folder(&acc.id, FolderRole::Trash).unwrap();
    let id = db.insert_email(&new_email(&acc.id, &inbox.id, "1", "x", 1)).unwrap().unwrap();

    db.set_read(&id, true).unwrap();
    db.set_flagged(&id, true).unwrap();
    let e = db.get_email(&id).unwrap();
    assert!(e.summary.is_read && e.summary.is_flagged);

    db.move_email(&id, &trash.id).unwrap();
    assert_eq!(db.get_email(&id).unwrap().summary.folder_id, trash.id);

    db.delete_email(&id).unwrap();
    assert!(matches!(db.delete_email(&id), Err(AppError::NotFound(_))));
    assert!(matches!(db.set_read("missing", true), Err(AppError::NotFound(_))));
}

#[test]
fn attachments_and_inline_parts() {
    let (db, acc, inbox) = db_with_account();
    let mut e = new_email(&acc.id, &inbox.id, "1", "files", 1);
    e.attachments = vec![
        NewAttachment { filename: "logo.png".into(), mime_type: "image/png".into(), content_id: Some("logo".into()), data: vec![1, 2] },
        NewAttachment { filename: "doc.pdf".into(), mime_type: "application/pdf".into(), content_id: None, data: vec![3; 10] },
    ];
    let id = db.insert_email(&e).unwrap().unwrap();
    let email = db.get_email(&id).unwrap();
    assert!(email.summary.has_attachments);
    assert_eq!(email.attachments.len(), 2);
    let pdf = email.attachments.iter().find(|a| a.filename == "doc.pdf").unwrap();
    let (meta, data) = db.get_attachment(&pdf.id).unwrap();
    assert_eq!((meta.size, data.len()), (10, 10));
    assert_eq!(db.inline_parts(&id).unwrap(), vec![("logo".into(), "image/png".into(), vec![1, 2])]);
}

#[test]
fn inline_only_images_do_not_mark_has_attachments() {
    let (db, acc, inbox) = db_with_account();
    let mut e = new_email(&acc.id, &inbox.id, "1", "inline", 1);
    e.attachments = vec![NewAttachment { filename: "i.png".into(), mime_type: "image/png".into(), content_id: Some("i".into()), data: vec![0] }];
    let id = db.insert_email(&e).unwrap().unwrap();
    assert!(!db.get_email(&id).unwrap().summary.has_attachments);
}

#[test]
fn sync_state_upserts() {
    let (db, _acc, inbox) = db_with_account();
    assert_eq!(db.get_sync_state(&inbox.id).unwrap(), None);
    db.set_sync_state(&inbox.id, "a").unwrap();
    db.set_sync_state(&inbox.id, "b").unwrap();
    assert_eq!(db.get_sync_state(&inbox.id).unwrap().as_deref(), Some("b"));
}

#[test]
fn unread_total_counts_only_inbox() {
    let (db, acc, inbox) = db_with_account();
    let spam = db.ensure_folder(&acc.id, FolderRole::Spam).unwrap();
    db.insert_email(&new_email(&acc.id, &inbox.id, "1", "a", 1)).unwrap();
    db.insert_email(&new_email(&acc.id, &spam.id, "2", "b", 1)).unwrap();
    assert_eq!(db.unread_total().unwrap(), 1);
}
