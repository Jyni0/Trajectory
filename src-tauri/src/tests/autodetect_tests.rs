use crate::autodetect::{detect, domain_of};
use crate::error::AppError;
use crate::models::{AccountKind, FolderRole, Security};
use crate::providers::role_from_name;

#[test]
fn known_providers() {
    let g = detect("someone@gmail.com").unwrap();
    assert_eq!(g.provider.as_deref(), Some("Google"));
    assert_eq!(g.suggested_kind, Some(AccountKind::Gmail));
    assert_eq!((g.imap.host.as_str(), g.imap.port), ("imap.gmail.com", 993));

    let o = detect("Someone@Hotmail.com").unwrap();
    assert_eq!(o.suggested_kind, Some(AccountKind::Outlook));
    assert_eq!(o.smtp.security, Security::StartTls);
    assert_eq!(o.smtp.port, 587);

    let y = detect("x@yandex.ru").unwrap();
    assert_eq!(y.provider.as_deref(), Some("Yandex"));
    assert_eq!(y.pop3.port, 995);
}

#[test]
fn falls_back_to_conventional_hostnames() {
    let d = detect("me@corp.example.org").unwrap();
    assert_eq!(d.provider, None);
    assert_eq!(d.imap.host, "imap.corp.example.org");
    assert_eq!(d.pop3.host, "pop.corp.example.org");
    assert_eq!(d.smtp.host, "smtp.corp.example.org");
    assert_eq!(d.smtp.security, Security::Tls);
}

#[test]
fn rejects_invalid_addresses() {
    for bad in ["", "plain", "@example.com", "me@", "me@localhost", "me@exa mple.com"] {
        assert!(matches!(domain_of(bad), Err(AppError::Invalid(_))), "{bad:?} should be rejected");
    }
}

#[test]
fn folder_roles_from_names() {
    assert_eq!(role_from_name("INBOX"), FolderRole::Inbox);
    assert_eq!(role_from_name("Sent Items"), FolderRole::Sent);
    assert_eq!(role_from_name("[Gmail]/Spam"), FolderRole::Spam);
    assert_eq!(role_from_name("INBOX.Trash"), FolderRole::Trash);
    assert_eq!(role_from_name("Черновики"), FolderRole::Drafts);
    assert_eq!(role_from_name("Projects"), FolderRole::Custom);
}
