//! MIME parsing and HTML sanitizing.

use std::borrow::Cow;
use std::collections::HashSet;

use base64::Engine;
use mail_parser::{MessageParser, MimeHeaders, PartType};

use crate::db::{NewAttachment, NewEmail};
use crate::error::{AppError, AppResult};
use crate::models::Address;

const SNIPPET_LEN: usize = 180;

#[derive(Debug, Clone, Default)]
pub struct ParsedEmail {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub subject: String,
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub date: i64,
    pub body_text: Option<String>,
    /// Already sanitized.
    pub body_html: Option<String>,
    pub snippet: String,
    pub attachments: Vec<NewAttachment>,
    /// Sender domain that passed DMARC (BIMI logos are shown only for it).
    pub logo_domain: Option<String>,
}

impl ParsedEmail {
    pub fn into_new_email(
        self,
        account_id: &str,
        folder_id: &str,
        remote_id: &str,
        is_read: bool,
        is_flagged: bool,
    ) -> NewEmail {
        NewEmail {
            account_id: account_id.into(),
            folder_id: folder_id.into(),
            remote_id: remote_id.into(),
            message_id: self.message_id,
            in_reply_to: self.in_reply_to,
            from: self.from,
            to: self.to,
            cc: self.cc,
            reply_to: self.reply_to,
            subject: self.subject,
            snippet: self.snippet,
            body_text: self.body_text,
            body_html: self.body_html,
            date: self.date,
            is_read,
            is_flagged,
            attachments: self.attachments,
            logo_domain: self.logo_domain,
        }
    }
}

/// Parses a raw RFC 5322 message (any nesting of multipart/alternative,
/// related and mixed; base64 / quoted-printable transfer encodings; RFC 2047
/// encoded headers).
pub fn parse_message(raw: &[u8]) -> AppResult<ParsedEmail> {
    let msg = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| AppError::Protocol("unparseable MIME message".into()))?;

    let html = msg.html_part(0).and_then(|p| match &p.body {
        PartType::Html(h) => Some(h.to_string()),
        _ => None,
    });
    let text = msg
        .text_part(0)
        .and_then(|p| match &p.body {
            PartType::Text(t) => Some(t.to_string()),
            _ => None,
        })
        .or_else(|| html.as_deref().map(html_to_text));

    let mut attachments = Vec::new();
    for part in msg.attachments() {
        let is_inline_disposition = part
            .content_disposition()
            .map(|d| d.ctype().eq_ignore_ascii_case("inline"))
            .unwrap_or(true);
        let content_id = part
            .content_id()
            .filter(|_| is_inline_disposition)
            .map(|c| c.trim_matches(|ch| ch == '<' || ch == '>').to_string());
        let mime_type = part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".into())
            .to_lowercase();
        let filename = part
            .attachment_name()
            .map(str::to_string)
            .unwrap_or_else(|| default_filename(&mime_type, attachments.len()));
        attachments.push(NewAttachment {
            filename,
            mime_type,
            content_id,
            data: part.contents().to_vec(),
        });
    }

    let in_reply_to = msg
        .in_reply_to()
        .as_text()
        .map(str::to_string)
        .or_else(|| msg.in_reply_to().as_text_list().and_then(|l| l.first().map(|s| s.to_string())));

    let snippet = make_snippet(text.as_deref().unwrap_or_default());
    let from = msg.from().map(convert_addresses).and_then(|v| v.into_iter().next()).unwrap_or_default();
    // Only the top-most Authentication-Results counts: it is the one our own
    // mail server added on receipt. Lower ones may be forged by the sender.
    let auth_results = msg
        .headers_raw()
        .find(|(name, _)| name.eq_ignore_ascii_case("Authentication-Results"))
        .map(|(_, value)| value);
    let logo_domain = auth_results.and_then(|v| dmarc_pass_domain(v, &from.address));
    Ok(ParsedEmail {
        logo_domain,
        message_id: msg.message_id().map(str::to_string),
        in_reply_to,
        subject: msg.subject().unwrap_or("").trim().to_string(),
        from,
        to: msg.to().map(convert_addresses).unwrap_or_default(),
        cc: msg.cc().map(convert_addresses).unwrap_or_default(),
        reply_to: msg.reply_to().map(convert_addresses).unwrap_or_default(),
        date: msg
            .date()
            .map(|d| d.to_timestamp())
            .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        body_html: html.map(|h| sanitize_html(&h)),
        body_text: text,
        snippet,
        attachments,
    })
}

/// The From domain when an `Authentication-Results` value says DMARC passed
/// for exactly that domain (`dmarc=pass … header.from=example.com`).
pub fn dmarc_pass_domain(auth_results: &str, from_address: &str) -> Option<String> {
    let from_domain = from_address.rsplit_once('@')?.1.trim().trim_end_matches('>').to_ascii_lowercase();
    if from_domain.is_empty() {
        return None;
    }
    let flat = auth_results.split_whitespace().collect::<Vec<_>>().join(" ");
    // The first element is the authserv-id; each later one is a method result.
    flat.split(';').skip(1).find_map(|res| {
        let res = res.trim();
        res.get(..6).filter(|p| p.eq_ignore_ascii_case("dmarc="))?;
        let verdict = res[6..].split(|c: char| c.is_whitespace() || c == '(').next()?;
        if !verdict.eq_ignore_ascii_case("pass") {
            return None;
        }
        let domain = res
            .split_whitespace()
            .find_map(|t| t.get(..12).filter(|p| p.eq_ignore_ascii_case("header.from=")).map(|_| &t[12..]))?
            .trim_matches(|c| c == '"' || c == ';')
            .to_ascii_lowercase();
        (domain == from_domain).then_some(domain)
    })
}

fn convert_addresses(a: &mail_parser::Address<'_>) -> Vec<Address> {
    let addrs: Vec<&mail_parser::Addr<'_>> = match a {
        mail_parser::Address::List(list) => list.iter().collect(),
        mail_parser::Address::Group(groups) => groups.iter().flat_map(|g| g.addresses.iter()).collect(),
    };
    addrs
        .into_iter()
        .filter_map(|addr| {
            Some(Address {
                name: addr.name.as_ref().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
                address: addr.address.as_ref()?.trim().to_string(),
            })
        })
        .collect()
}

fn default_filename(mime: &str, index: usize) -> String {
    let ext = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "message/rfc822" => "eml",
        _ => "bin",
    };
    format!("attachment-{}.{ext}", index + 1)
}

/// Strips everything that could execute code or exfiltrate data on render:
/// scripts, event handlers, forms, iframes, `javascript:` URLs and dangerous
/// inline CSS. The document's `<title>` (and `<noscript>`) are dropped *with
/// their text* — otherwise the title shows up as stray text above the body.
/// `<style>` blocks are kept: newsletters hide their preheader and lay
/// themselves out with them. That is safe because the frontend renders the
/// result in a sandboxed, script-less iframe whose CSP blocks every remote
/// load (`@import`, `url()`) unless remote images are allowed.
pub fn sanitize_html(html: &str) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .clean_content_tags(HashSet::from(["script", "title", "noscript", "template"]))
        .add_tags(["font", "center", "style"])
        .add_tag_attributes("font", ["face", "size", "color"])
        .add_generic_attributes([
            "style", "class", "align", "valign", "bgcolor", "width", "height", "border",
            "cellpadding", "cellspacing", "dir", "color",
        ])
        .url_schemes(HashSet::from(["http", "https", "mailto", "tel", "cid", "data"]))
        .link_rel(Some("noopener noreferrer nofollow"))
        .set_tag_attribute_value("a", "target", "_blank")
        .attribute_filter(|_element, attribute, value| {
            if attribute == "style" {
                let v = value.to_ascii_lowercase();
                let dangerous = ["expression", "javascript:", "behavior", "-moz-binding", "@import", "vbscript:"];
                if dangerous.iter().any(|d| v.contains(d)) {
                    return None;
                }
            }
            Some(Cow::Borrowed(value))
        });
    builder.clean(html).to_string()
}

/// Replaces `cid:` references with data URLs so inline images render offline.
pub fn inline_cid_images(html: &str, parts: &[(String, String, Vec<u8>)]) -> String {
    let mut out = html.to_string();
    for (cid, mime, data) in parts {
        let needle = format!("cid:{cid}");
        if out.contains(&needle) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            out = out.replace(&needle, &format!("data:{mime};base64,{b64}"));
        }
    }
    out
}

/// Very small HTML → text converter used for snippets and text fallbacks.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut skip_depth: Option<&str> = None;
    let lower = html.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if let Some(end_tag) = skip_depth {
            if lb[i..].starts_with(end_tag.as_bytes()) {
                skip_depth = None;
                i += end_tag.len();
            } else {
                i += utf8_len(bytes[i]);
            }
            continue;
        }
        let c = bytes[i];
        if c == b'<' {
            // <head> holds <title>, <style>, <meta>: nothing of it is text.
            if lb[i..].starts_with(b"<head") && lb.get(i + 5).is_some_and(|b| *b == b'>' || *b == b'/' || b.is_ascii_whitespace()) {
                skip_depth = Some("</head>");
                continue;
            }
            if lb[i..].starts_with(b"<title") {
                skip_depth = Some("</title>");
                continue;
            }
            if lb[i..].starts_with(b"<style") {
                skip_depth = Some("</style>");
                continue;
            }
            if lb[i..].starts_with(b"<script") {
                skip_depth = Some("</script>");
                continue;
            }
            if lb[i..].starts_with(b"<br") || lb[i..].starts_with(b"<p") || lb[i..].starts_with(b"<div") || lb[i..].starts_with(b"</tr") {
                out.push('\n');
            }
            in_tag = true;
        } else if c == b'>' && in_tag {
            in_tag = false;
        } else if !in_tag {
            // Copy the whole UTF-8 char.
            let ch_len = utf8_len(c);
            out.push_str(&html[i..i + ch_len]);
            i += ch_len;
            continue;
        }
        i += utf8_len(c);
    }
    decode_entities(&out)
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

pub fn make_snippet(text: &str) -> String {
    let collapsed = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .flat_map(|l| l.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= SNIPPET_LEN {
        collapsed
    } else {
        let mut s: String = collapsed.chars().take(SNIPPET_LEN).collect();
        s.push('…');
        s
    }
}
