//! Conversations: messages linked by Message-ID / In-Reply-To across folders.

use super::fixtures::*;
use crate::db::Database;
use crate::models::*;

struct Mailbox {
    db: Database,
    acc: Account,
    inbox: Folder,
    sent: Folder,
}

fn mailbox() -> Mailbox {
    let db = Database::open_in_memory().unwrap();
    let acc = account(AccountKind::Imap, "me@example.com");
    db.insert_account(&acc).unwrap();
    let inbox = db.ensure_folder(&acc.id, FolderRole::Inbox).unwrap();
    let sent = db.ensure_folder(&acc.id, FolderRole::Sent).unwrap();
    Mailbox { db, acc, inbox, sent }
}

impl Mailbox {
    fn put(&self, folder: &Folder, remote: &str, date: i64, mid: Option<&str>, irt: Option<&str>) -> String {
        let mut e = new_email(&self.acc.id, &folder.id, remote, remote, date);
        e.message_id = mid.map(str::to_string);
        e.in_reply_to = irt.map(str::to_string);
        self.db.insert_email(&e).unwrap().unwrap()
    }

    fn thread(&self, id: &str) -> Vec<String> {
        self.db.thread(id).unwrap().into_iter().map(|e| e.subject).collect()
    }
}

#[test]
fn replies_in_inbox_and_sent_form_one_conversation_oldest_first() {
    let m = mailbox();
    m.put(&m.inbox, "question", 100, Some("<q@x>"), None);
    // Our reply in Sent; brackets and case differ from the original's id.
    m.put(&m.sent, "my answer", 200, Some("<a@me>"), Some("Q@X"));
    let last = m.put(&m.inbox, "their follow-up", 300, Some("<f@x>"), Some("<a@me>"));
    m.put(&m.inbox, "unrelated", 150, Some("<u@x>"), None);

    assert_eq!(m.thread(&last), ["question", "my answer", "their follow-up"]);
    // Opening the first message finds the whole chain too.
    let first = m.db.list_emails(&EmailQuery { folder_id: Some(m.inbox.id.clone()), ..Default::default() }).unwrap();
    let q = first.items.iter().find(|e| e.subject == "question").unwrap();
    assert_eq!(m.thread(&q.id), ["question", "my answer", "their follow-up"]);
}

#[test]
fn a_message_alone_is_its_own_thread() {
    let m = mailbox();
    let id = m.put(&m.inbox, "hello", 100, Some("<h@x>"), None);
    assert_eq!(m.thread(&id), ["hello"]);
    let bare = m.put(&m.inbox, "no ids", 100, None, None);
    assert_eq!(m.thread(&bare), ["no ids"]);
}

#[test]
fn copies_in_two_folders_appear_once_and_trash_is_left_out() {
    let m = mailbox();
    let archive = m.db.ensure_folder(&m.acc.id, FolderRole::Archive).unwrap();
    let trash = m.db.ensure_folder(&m.acc.id, FolderRole::Trash).unwrap();
    m.put(&m.inbox, "original", 100, Some("<o@x>"), None);
    let copy = m.put(&archive, "original", 100, Some("<o@x>"), None);
    m.put(&trash, "deleted reply", 150, Some("<d@x>"), Some("<o@x>"));
    let reply = m.put(&m.inbox, "reply", 200, Some("<r@x>"), Some("<o@x>"));

    assert_eq!(m.thread(&reply), ["original", "reply"]);
    // The opened copy is the one listed.
    assert!(m.db.thread(&copy).unwrap().iter().any(|e| e.id == copy));
}

#[test]
fn unknown_email_is_not_found() {
    assert!(mailbox().db.thread("nope").is_err());
}
