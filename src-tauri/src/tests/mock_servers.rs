//! Minimal scripted SMTP / POP3 / IMAP servers for integration tests.
//! Each server listens on an ephemeral localhost port, records the commands it
//! receives and serves an in-memory mailbox.

use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

pub const USER: &str = "user@example.com";
pub const PASS: &str = "secret";

pub type Log = Arc<Mutex<Vec<String>>>;
pub type Pop3Mailbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
pub type ImapMailboxes = Arc<Mutex<Vec<(String, Vec<ImapMessage>)>>>;

async fn listen() -> (TcpListener, u16) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    (l, port)
}

async fn read_line(r: &mut BufReader<TcpStream>) -> Option<String> {
    let mut line = String::new();
    match r.read_line(&mut line).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
    }
}

async fn write(r: &mut BufReader<TcpStream>, s: &str) {
    let _ = r.get_mut().write_all(s.as_bytes()).await;
}

// ─────────────────────────────── SMTP ───────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct SmtpCapture {
    pub auth_ok: bool,
    pub mail_from: String,
    pub rcpt_to: Vec<String>,
    pub data: String,
}

pub struct SmtpMock {
    pub port: u16,
    pub captured: Arc<Mutex<Vec<SmtpCapture>>>,
}

pub async fn smtp_server() -> SmtpMock {
    let (listener, port) = listen().await;
    let captured: Arc<Mutex<Vec<SmtpCapture>>> = Arc::default();
    let cap = captured.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            let cap = cap.clone();
            tokio::spawn(async move {
                let mut r = BufReader::new(sock);
                let mut cur = SmtpCapture::default();
                write(&mut r, "220 mock.local ESMTP ready\r\n").await;
                while let Some(line) = read_line(&mut r).await {
                    let upper = line.to_ascii_uppercase();
                    if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        write(&mut r, "250-mock.local\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n").await;
                    } else if let Some(b64) = line.strip_prefix("AUTH PLAIN ") {
                        let decoded = STANDARD.decode(b64.trim()).unwrap_or_default();
                        let parts: Vec<&[u8]> = decoded.split(|b| *b == 0).collect();
                        let ok = parts.len() == 3 && parts[1] == USER.as_bytes() && parts[2] == PASS.as_bytes();
                        cur.auth_ok = ok;
                        write(&mut r, if ok { "235 2.7.0 Authenticated\r\n" } else { "535 5.7.8 Bad credentials\r\n" }).await;
                    } else if upper.starts_with("MAIL FROM:") {
                        cur.mail_from = line[10..].trim().to_string();
                        write(&mut r, "250 OK\r\n").await;
                    } else if upper.starts_with("RCPT TO:") {
                        cur.rcpt_to.push(line[8..].trim().to_string());
                        write(&mut r, "250 OK\r\n").await;
                    } else if upper == "DATA" {
                        write(&mut r, "354 End data with <CR><LF>.<CR><LF>\r\n").await;
                        let mut data = String::new();
                        while let Some(l) = read_line(&mut r).await {
                            if l == "." {
                                break;
                            }
                            data.push_str(l.strip_prefix('.').filter(|_| l.starts_with("..")).unwrap_or(&l));
                            data.push_str("\r\n");
                        }
                        cur.data = data;
                        cap.lock().unwrap().push(std::mem::take(&mut cur));
                        write(&mut r, "250 OK queued\r\n").await;
                    } else if upper == "QUIT" {
                        write(&mut r, "221 Bye\r\n").await;
                        return;
                    } else {
                        // NOOP, RSET, ...
                        write(&mut r, "250 OK\r\n").await;
                    }
                }
            });
        }
    });
    SmtpMock { port, captured }
}

// ─────────────────────────────── POP3 ───────────────────────────────

pub struct Pop3Mock {
    pub port: u16,
    pub log: Log,
    /// (uidl, raw message). Message numbers are 1-based positions.
    pub mailbox: Pop3Mailbox,
}

pub async fn pop3_server(messages: Vec<(String, Vec<u8>)>) -> Pop3Mock {
    let (listener, port) = listen().await;
    let log: Log = Arc::default();
    let mailbox = Arc::new(Mutex::new(messages));
    let (log2, mb2) = (log.clone(), mailbox.clone());
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            let (log, mb) = (log2.clone(), mb2.clone());
            tokio::spawn(async move {
                let mut r = BufReader::new(sock);
                let mut deleted: Vec<usize> = Vec::new();
                let mut user_ok = false;
                write(&mut r, "+OK POP3 mock ready\r\n").await;
                while let Some(line) = read_line(&mut r).await {
                    log.lock().unwrap().push(line.clone());
                    let mut it = line.split_whitespace();
                    let cmd = it.next().unwrap_or_default().to_ascii_uppercase();
                    let arg = |i: Option<&str>| i.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                    match cmd.as_str() {
                        "USER" => {
                            user_ok = it.next() == Some(USER);
                            write(&mut r, "+OK\r\n").await;
                        }
                        "PASS" => {
                            if user_ok && it.next() == Some(PASS) {
                                write(&mut r, "+OK logged in\r\n").await;
                            } else {
                                write(&mut r, "-ERR invalid credentials\r\n").await;
                            }
                        }
                        "STAT" => {
                            let mb = mb.lock().unwrap().clone();
                            let size: usize = mb.iter().map(|(_, m)| m.len()).sum();
                            write(&mut r, &format!("+OK {} {size}\r\n", mb.len())).await;
                        }
                        "UIDL" => {
                            let mut out = String::from("+OK\r\n");
                            for (i, (uid, _)) in mb.lock().unwrap().iter().enumerate() {
                                out.push_str(&format!("{} {uid}\r\n", i + 1));
                            }
                            out.push_str(".\r\n");
                            write(&mut r, &out).await;
                        }
                        "RETR" | "TOP" => {
                            let n = arg(it.next());
                            let msg = mb.lock().unwrap().get(n.wrapping_sub(1)).map(|(_, m)| m.clone());
                            match msg {
                                Some(raw) => {
                                    let text = String::from_utf8_lossy(&raw).to_string();
                                    let body = if cmd == "TOP" {
                                        text.split("\r\n\r\n").next().unwrap_or_default().to_string() + "\r\n\r\n"
                                    } else {
                                        text
                                    };
                                    let mut out = String::from("+OK\r\n");
                                    for l in body.split_inclusive("\r\n") {
                                        if l.starts_with('.') {
                                            out.push('.');
                                        }
                                        out.push_str(l);
                                    }
                                    if !out.ends_with("\r\n") {
                                        out.push_str("\r\n");
                                    }
                                    out.push_str(".\r\n");
                                    write(&mut r, &out).await;
                                }
                                None => write(&mut r, "-ERR no such message\r\n").await,
                            }
                        }
                        "DELE" => {
                            deleted.push(arg(it.next()));
                            write(&mut r, "+OK deleted\r\n").await;
                        }
                        "QUIT" => {
                            { let mut mb = mb.lock().unwrap();
                            deleted.sort_unstable_by(|a, b| b.cmp(a));
                            for n in deleted.drain(..) {
                                if n >= 1 && n <= mb.len() {
                                    mb.remove(n - 1);
                                }
                            }
                            }
                            write(&mut r, "+OK bye\r\n").await;
                            return;
                        }
                        _ => write(&mut r, "-ERR unknown command\r\n").await,
                    }
                }
            });
        }
    });
    Pop3Mock { port, log, mailbox }
}

// ─────────────────────────────── IMAP ───────────────────────────────

#[derive(Clone)]
pub struct ImapMessage {
    pub uid: u32,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
}

pub struct ImapMock {
    pub port: u16,
    pub log: Log,
    pub mailboxes: ImapMailboxes,
}

const UID_VALIDITY: u32 = 42;

fn fetch_line(seq: usize, m: &ImapMessage) -> Vec<u8> {
    let mut out = format!(
        "* {seq} FETCH (UID {} FLAGS ({}) BODY[] {{{}}}\r\n",
        m.uid,
        m.flags.join(" "),
        m.raw.len()
    )
    .into_bytes();
    out.extend_from_slice(&m.raw);
    out.extend_from_slice(b")\r\n");
    out
}

/// Serves `mailboxes` as `(name, messages)`; "Sent" carries the \Sent attribute.
pub async fn imap_server(mailboxes: Vec<(String, Vec<ImapMessage>)>) -> ImapMock {
    let (listener, port) = listen().await;
    let log: Log = Arc::default();
    let mailboxes = Arc::new(Mutex::new(mailboxes));
    let (log2, mb2) = (log.clone(), mailboxes.clone());
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            let (log, mbs) = (log2.clone(), mb2.clone());
            tokio::spawn(async move {
                let mut r = BufReader::new(sock);
                let mut selected: Option<String> = None;
                write(&mut r, "* OK [CAPABILITY IMAP4rev1 MOVE] mock ready\r\n").await;
                while let Some(line) = read_line(&mut r).await {
                    log.lock().unwrap().push(line.clone());
                    let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
                    let upper = rest.to_ascii_uppercase();
                    let unquote = |s: &str| s.trim().trim_matches('"').to_string();

                    if upper.starts_with("LOGIN") {
                        let ok = rest.contains(&format!("\"{USER}\"")) && rest.contains(&format!("\"{PASS}\""));
                        let resp = if ok {
                            format!("{tag} OK LOGIN completed\r\n")
                        } else {
                            format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")
                        };
                        write(&mut r, &resp).await;
                    } else if upper.starts_with("CAPABILITY") {
                        write(&mut r, &format!("* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK done\r\n")).await;
                    } else if upper.starts_with("LIST") {
                        let mut out = String::new();
                        for (name, _) in mbs.lock().unwrap().iter() {
                            let attrs = match name.as_str() {
                                "Sent" => "\\HasNoChildren \\Sent",
                                "Trash" => "\\HasNoChildren \\Trash",
                                _ => "\\HasNoChildren",
                            };
                            out.push_str(&format!("* LIST ({attrs}) \"/\" \"{name}\"\r\n"));
                        }
                        out.push_str("* LIST (\\Noselect \\HasChildren) \"/\" \"[Mock]\"\r\n");
                        out.push_str(&format!("{tag} OK LIST completed\r\n"));
                        write(&mut r, &out).await;
                    } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                        let name = unquote(&rest[rest.find(' ').unwrap_or(0)..]);
                        let mbs = mbs.lock().unwrap().clone();
                        match mbs.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                            Some((n, msgs)) => {
                                selected = Some(n.clone());
                                let next = msgs.iter().map(|m| m.uid).max().unwrap_or(0) + 1;
                                write(
                                    &mut r,
                                    &format!(
                                        "* {} EXISTS\r\n* 0 RECENT\r\n* FLAGS (\\Seen \\Flagged \\Deleted)\r\n* OK [UIDVALIDITY {UID_VALIDITY}] UIDs valid\r\n* OK [UIDNEXT {next}] Predicted next UID\r\n{tag} OK [READ-WRITE] SELECT completed\r\n",
                                        msgs.len()
                                    ),
                                )
                                .await;
                            }
                            None => write(&mut r, &format!("{tag} NO no such mailbox\r\n")).await,
                        }
                    } else if upper.starts_with("FETCH") || upper.starts_with("UID FETCH") {
                        let is_uid = upper.starts_with("UID");
                        let set = rest.split_whitespace().nth(if is_uid { 2 } else { 1 }).unwrap_or("1:*");
                        let start: u32 = set.split(':').next().and_then(|s| s.parse().ok()).unwrap_or(1);
                        let msgs = mbs
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|(n, _)| Some(n) == selected.as_ref())
                            .map(|(_, m)| m.clone())
                            .unwrap_or_default();
                        let mut out = Vec::new();
                        let mut any = false;
                        for (i, m) in msgs.iter().enumerate() {
                            let matches = if is_uid { m.uid >= start } else { (i as u32 + 1) >= start };
                            if matches {
                                any = true;
                                out.extend(fetch_line(i + 1, m));
                            }
                        }
                        // Like real servers, `UID n:*` returns the last message when n > max.
                        if is_uid && !any {
                            if let Some(m) = msgs.last() {
                                out.extend(fetch_line(msgs.len(), m));
                            }
                        }
                        out.extend(format!("{tag} OK FETCH completed\r\n").into_bytes());
                        let _ = r.get_mut().write_all(&out).await;
                    } else if upper.starts_with("UID STORE") {
                        let uid: u32 = rest.split_whitespace().nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let add = upper.contains("+FLAGS");
                        let flag = if upper.contains("\\SEEN") {
                            "\\Seen"
                        } else if upper.contains("\\FLAGGED") {
                            "\\Flagged"
                        } else {
                            "\\Deleted"
                        };
                        { let mut mbs = mbs.lock().unwrap();
                        if let Some((_, msgs)) = mbs.iter_mut().find(|(n, _)| Some(n) == selected.as_ref()) {
                            if let Some(m) = msgs.iter_mut().find(|m| m.uid == uid) {
                                m.flags.retain(|f| f != flag);
                                if add {
                                    m.flags.push(flag.to_string());
                                }
                            }
                        }
                        }
                        write(&mut r, &format!("{tag} OK STORE completed\r\n")).await;
                    } else if upper.starts_with("UID MOVE") {
                        let mut parts = rest.split_whitespace();
                        let uid: u32 = parts.nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let dest = unquote(&parts.collect::<Vec<_>>().join(" "));
                        { let mut mbs = mbs.lock().unwrap();
                        let moved = mbs
                            .iter_mut()
                            .find(|(n, _)| Some(n) == selected.as_ref())
                            .and_then(|(_, msgs)| msgs.iter().position(|m| m.uid == uid).map(|p| msgs.remove(p)));
                        if let (Some(m), Some((_, d))) = (moved, mbs.iter_mut().find(|(n, _)| *n == dest)) {
                            d.push(ImapMessage { uid: d.iter().map(|m| m.uid).max().unwrap_or(0) + 1, ..m });
                        }
                        }
                        write(&mut r, &format!("{tag} OK MOVE completed\r\n")).await;
                    } else if upper.starts_with("EXPUNGE") {
                        { let mut mbs = mbs.lock().unwrap();
                        if let Some((_, msgs)) = mbs.iter_mut().find(|(n, _)| Some(n) == selected.as_ref()) {
                            msgs.retain(|m| !m.flags.iter().any(|f| f == "\\Deleted"));
                        }
                        }
                        write(&mut r, &format!("{tag} OK EXPUNGE completed\r\n")).await;
                    } else if upper.starts_with("APPEND") {
                        let len: usize = rest
                            .rsplit_once('{')
                            .and_then(|(_, n)| n.trim_end_matches('}').parse().ok())
                            .unwrap_or(0);
                        let name = unquote(rest.split_whitespace().nth(1).unwrap_or_default());
                        write(&mut r, "+ Ready for literal data\r\n").await;
                        let mut buf = vec![0u8; len];
                        let _ = r.read_exact(&mut buf).await;
                        let _ = read_line(&mut r).await; // trailing CRLF
                        { let mut mbs = mbs.lock().unwrap();
                        if let Some((_, msgs)) = mbs.iter_mut().find(|(n, _)| *n == name) {
                            let uid = msgs.iter().map(|m| m.uid).max().unwrap_or(0) + 1;
                            msgs.push(ImapMessage { uid, flags: vec!["\\Seen".into()], raw: buf });
                        }
                        }
                        write(&mut r, &format!("{tag} OK APPEND completed\r\n")).await;
                    } else if upper.starts_with("LOGOUT") {
                        write(&mut r, &format!("* BYE logging out\r\n{tag} OK LOGOUT completed\r\n")).await;
                        return;
                    } else {
                        write(&mut r, &format!("{tag} OK\r\n")).await;
                    }
                }
            });
        }
    });
    ImapMock { port, log, mailboxes }
}

pub fn imap_msg(uid: u32, flags: &[&str], raw: Vec<u8>) -> ImapMessage {
    ImapMessage { uid, flags: flags.iter().map(|s| s.to_string()).collect(), raw }
}
