//! Site icons — the fallback sender logo for domains without BIMI.
//!
//! Like BIMI logos they are only looked up for a sender domain that passed
//! DMARC (see `mime::dmarc_pass_domain`), so a forged From never borrows a
//! brand's icon. They are never "verified": nothing certifies them.
//!
//! The icon comes from the organisation's own site (`mail.example.co.uk` →
//! `https://example.co.uk/`): the largest `apple-touch-icon` / `icon` the home
//! page links to, else `/favicon.ico`. Only real images are accepted, and the
//! UI shows them as `<img src="data:…">`.

use base64::Engine;
use url::Url;

use crate::bimi::{clean_svg, Fetch};
use crate::error::{AppError, AppResult};
use crate::net::is_private_host;

const MAX_PAGE: usize = 512 * 1024;
const MAX_ICON: usize = 256 * 1024;
/// Icons tried before giving up (besides `/favicon.ico`).
const MAX_TRIES: usize = 3;

/// The registrable domain (`news.mail.example.co.uk` → `example.co.uk`).
pub fn site_domain(domain: &str) -> Option<String> {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let site = psl::domain_str(&d)?;
    site.contains('.').then(|| site.to_string())
}

/// The domain's site icon as a data URL; `None` if it has none we accept.
pub async fn lookup(http: &dyn Fetch, domain: &str) -> AppResult<Option<String>> {
    let site = site_domain(domain).ok_or_else(|| AppError::Invalid(format!("'{domain}' has no site")))?;
    let home = format!("https://{site}/");

    let mut urls = match http.get_page(&home, MAX_PAGE).await {
        Ok((base, page)) => Url::parse(&base).map(|b| icon_links(&String::from_utf8_lossy(&page), &b)).unwrap_or_default(),
        Err(AppError::Network(e)) if unreachable(&e) => return Err(AppError::Network(e)),
        Err(_) => vec![],
    };
    urls.truncate(MAX_TRIES);
    urls.push(format!("{home}favicon.ico"));
    urls.dedup();

    for url in urls {
        let bytes = match inline_image(&url) {
            Some(b) => Ok(b),
            None => http.get(&url, MAX_ICON).await,
        };
        if let Ok(bytes) = bytes {
            if let Some(data) = image_data_url(&bytes) {
                return Ok(Some(data));
            }
        }
    }
    Ok(None)
}

/// Offline rather than "the site said no": worth retrying later.
fn unreachable(e: &str) -> bool {
    let e = e.to_ascii_lowercase();
    ["dns error", "error sending request", "timed out"].iter().any(|m| e.contains(m))
}

/// Icon URLs a page links to, best first: SVG, then by declared size.
pub fn icon_links(html: &str, base: &Url) -> Vec<String> {
    let mut found: Vec<(u32, String)> = vec![];
    let lower = html.to_ascii_lowercase();
    // Icons live in <head>; don't scan megabytes of body.
    let end = lower.find("</head").unwrap_or(lower.len());
    let mut at = 0;
    while let Some(i) = lower[at..end].find("<link") {
        let start = at + i;
        let Some(close) = lower[start..end].find('>') else { break };
        let tag = &html[start..start + close];
        at = start + close;
        let attr = |name: &str| attribute(tag, name);
        let rel = attr("rel").unwrap_or_default().to_ascii_lowercase();
        let rels: Vec<&str> = rel.split_whitespace().collect();
        let touch = rels.iter().any(|r| r.starts_with("apple-touch-icon"));
        if !(touch || rels.contains(&"icon")) {
            continue;
        }
        let Some(href) = attr("href") else { continue };
        let Ok(url) = base.join(href.trim()) else { continue };
        let inline = url.scheme() == "data" && inline_image(url.as_str()).is_some();
        if !inline && (url.scheme() != "https" || url.host_str().map_or(true, is_private_host)) {
            continue;
        }
        let kind = attr("type").unwrap_or_default().to_ascii_lowercase();
        let sizes = attr("sizes").unwrap_or_default().to_ascii_lowercase();
        let score = if kind.contains("svg") || url.path().ends_with(".svg") || sizes == "any" {
            10_000
        } else if let Some(px) = sizes.split_whitespace().filter_map(|s| s.split_once('x')?.0.parse::<u32>().ok()).max() {
            px
        } else if touch {
            180
        } else {
            32
        };
        found.push((score, url.to_string()));
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, u)| u).collect()
}

/// The bytes of a `data:image/…;base64,` icon embedded in the page.
fn inline_image(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:image/")?;
    let (meta, data) = rest.split_once(',')?;
    if !meta.ends_with(";base64") || data.len() > MAX_ICON * 4 / 3 + 4 {
        return None;
    }
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

/// Value of `name="…"` / `name='…'` / `name=…` in a tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let pos = from + i;
        from = pos + name.len();
        // Whole attribute name only (`rel`, not `data-rel` or `prel`).
        let before = lower[..pos].chars().next_back();
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let rest = lower[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let skip = tag.len() - rest.len();
        let value = tag[skip..].trim_start();
        let v = match value.chars().next()? {
            q @ ('"' | '\'') => value[1..].split(q).next()?,
            _ => value.split(|c: char| c.is_whitespace() || c == '/').next()?,
        };
        return Some(html_unescape(v));
    }
    None
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#x2F;", "/").replace("&#47;", "/")
}

/// A data URL for an image file, judged by its content (not its name or
/// Content-Type). Anything else — an HTML error page, say — is refused.
pub fn image_data_url(bytes: &[u8]) -> Option<String> {
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        "image/x-icon"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        // An SVG document — not an HTML page that merely contains one.
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
        if head.contains("<html") || head.contains("<!doctype html") {
            return None;
        }
        return clean_svg(bytes).ok().map(|svg| crate::bimi::data_url(&svg));
    };
    // A 1×1 placeholder or empty file is not an icon.
    (bytes.len() > 64).then(|| format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}
