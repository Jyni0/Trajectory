//! Server settings autodetection from an email address: a table of well-known
//! providers with a `imap.<domain>` / `pop.<domain>` / `smtp.<domain>` fallback.

use crate::error::{AppError, AppResult};
use crate::models::{AccountKind, DetectedSettings, Security, ServerConfig};

struct Known {
    domains: &'static [&'static str],
    provider: &'static str,
    suggested: Option<AccountKind>,
    imap: (&'static str, u16, Security),
    pop3: (&'static str, u16, Security),
    smtp: (&'static str, u16, Security),
}

const KNOWN: &[Known] = &[
    Known {
        domains: &["gmail.com", "googlemail.com"],
        provider: "Google",
        suggested: Some(AccountKind::Gmail),
        imap: ("imap.gmail.com", 993, Security::Tls),
        pop3: ("pop.gmail.com", 995, Security::Tls),
        smtp: ("smtp.gmail.com", 465, Security::Tls),
    },
    Known {
        domains: &["outlook.com", "hotmail.com", "live.com", "msn.com", "office365.com"],
        provider: "Microsoft",
        suggested: Some(AccountKind::Outlook),
        imap: ("outlook.office365.com", 993, Security::Tls),
        pop3: ("outlook.office365.com", 995, Security::Tls),
        smtp: ("smtp.office365.com", 587, Security::StartTls),
    },
    Known {
        domains: &["yahoo.com", "ymail.com"],
        provider: "Yahoo",
        suggested: None,
        imap: ("imap.mail.yahoo.com", 993, Security::Tls),
        pop3: ("pop.mail.yahoo.com", 995, Security::Tls),
        smtp: ("smtp.mail.yahoo.com", 465, Security::Tls),
    },
    Known {
        domains: &["icloud.com", "me.com", "mac.com"],
        provider: "iCloud",
        suggested: None,
        imap: ("imap.mail.me.com", 993, Security::Tls),
        pop3: ("imap.mail.me.com", 995, Security::Tls),
        smtp: ("smtp.mail.me.com", 587, Security::StartTls),
    },
    Known {
        domains: &["yandex.ru", "yandex.com", "ya.ru"],
        provider: "Yandex",
        suggested: None,
        imap: ("imap.yandex.com", 993, Security::Tls),
        pop3: ("pop.yandex.com", 995, Security::Tls),
        smtp: ("smtp.yandex.com", 465, Security::Tls),
    },
    Known {
        domains: &["mail.ru", "bk.ru", "inbox.ru", "list.ru"],
        provider: "Mail.ru",
        suggested: None,
        imap: ("imap.mail.ru", 993, Security::Tls),
        pop3: ("pop.mail.ru", 995, Security::Tls),
        smtp: ("smtp.mail.ru", 465, Security::Tls),
    },
    Known {
        domains: &["fastmail.com", "fastmail.fm"],
        provider: "Fastmail",
        suggested: None,
        imap: ("imap.fastmail.com", 993, Security::Tls),
        pop3: ("pop.fastmail.com", 995, Security::Tls),
        smtp: ("smtp.fastmail.com", 465, Security::Tls),
    },
    Known {
        domains: &["zoho.com", "zohomail.com"],
        provider: "Zoho",
        suggested: None,
        imap: ("imap.zoho.com", 993, Security::Tls),
        pop3: ("pop.zoho.com", 995, Security::Tls),
        smtp: ("smtp.zoho.com", 465, Security::Tls),
    },
    Known {
        domains: &["gmx.com", "gmx.net", "gmx.de"],
        provider: "GMX",
        suggested: None,
        imap: ("imap.gmx.com", 993, Security::Tls),
        pop3: ("pop.gmx.com", 995, Security::Tls),
        smtp: ("mail.gmx.com", 587, Security::StartTls),
    },
];

fn cfg((host, port, security): (&str, u16, Security)) -> ServerConfig {
    ServerConfig { host: host.to_string(), port, security }
}

pub fn domain_of(email: &str) -> AppResult<String> {
    let email = email.trim();
    let (local, domain) = email
        .rsplit_once('@')
        .ok_or_else(|| AppError::Invalid(format!("'{email}' is not an email address")))?;
    if local.is_empty() || domain.is_empty() || !domain.contains('.') || domain.contains(char::is_whitespace) {
        return Err(AppError::Invalid(format!("'{email}' is not an email address")));
    }
    Ok(domain.to_ascii_lowercase())
}

pub fn detect(email: &str) -> AppResult<DetectedSettings> {
    let domain = domain_of(email)?;
    if let Some(k) = KNOWN.iter().find(|k| k.domains.contains(&domain.as_str())) {
        return Ok(DetectedSettings {
            provider: Some(k.provider.to_string()),
            suggested_kind: k.suggested,
            imap: cfg(k.imap),
            pop3: cfg(k.pop3),
            smtp: cfg(k.smtp),
        });
    }
    Ok(DetectedSettings {
        provider: None,
        suggested_kind: None,
        imap: ServerConfig { host: format!("imap.{domain}"), port: 993, security: Security::Tls },
        pop3: ServerConfig { host: format!("pop.{domain}"), port: 995, security: Security::Tls },
        smtp: ServerConfig { host: format!("smtp.{domain}"), port: 465, security: Security::Tls },
    })
}
