use super::fixtures::*;
use crate::mime::*;
use crate::providers::imap::decode_modified_utf7;

#[test]
fn parses_simple_plain_text() {
    let p = parse_message(&simple_text("Hello there", "simple-1@example.com")).unwrap();
    assert_eq!(p.subject, "Hello there");
    assert_eq!(p.from.name.as_deref(), Some("Alice Example"));
    assert_eq!(p.from.address, "alice@example.com");
    assert_eq!(p.to.len(), 1);
    assert_eq!(p.message_id.as_deref(), Some("simple-1@example.com"));
    assert_eq!(p.date, 1_788_256_800); // 2026-09-01T10:00:00Z
    assert!(p.body_html.is_none());
    assert!(p.body_text.unwrap().contains("How are you?"));
    assert_eq!(p.snippet, "Hi Bob, How are you?");
    assert!(p.attachments.is_empty());
}

#[test]
fn parses_complex_multipart_message() {
    let p = parse_message(&complex_message()).unwrap();
    assert_eq!(p.subject, COMPLEX_SUBJECT);
    assert_eq!(p.from.name.as_deref(), Some(COMPLEX_FROM_NAME));
    assert_eq!(p.from.address, "ivan@example.ru");
    let to: Vec<_> = p.to.iter().map(|a| a.address.as_str()).collect();
    assert_eq!(to, ["bob@example.com", "carol@example.com"]);
    assert_eq!(p.to[1].name.as_deref(), Some("Carol"));
    assert_eq!(p.cc[0].address, "dave@example.com");
    assert_eq!(p.in_reply_to.as_deref(), Some("simple-1@example.com"));

    // quoted-printable UTF-8 text part
    assert_eq!(p.body_text.as_deref().map(str::trim), Some("Quarterly report attached. Café ✓"));

    // base64 attachments: inline image keeps its content-id, the PDF does not
    assert_eq!(p.attachments.len(), 2);
    let png = p.attachments.iter().find(|a| a.mime_type == "image/png").unwrap();
    assert_eq!(png.content_id.as_deref(), Some("logo@x"));
    assert_eq!(png.data, b"\x89PNG\r\n\x1a\n");
    let pdf = p.attachments.iter().find(|a| a.filename == "report.pdf").unwrap();
    assert_eq!(pdf.mime_type, "application/pdf");
    assert_eq!(pdf.content_id, None);
    assert_eq!(pdf.data, b"%PDF-1.4\n");
}

#[test]
fn html_part_is_sanitized() {
    let html = parse_message(&complex_message()).unwrap().body_html.unwrap();
    for bad in ["<script", "alert(1)", "onclick", "javascript:", "expression(", "<iframe", "evil.example"] {
        assert!(!html.contains(bad), "sanitized HTML still contains {bad:?}: {html}");
    }
    // Style blocks stay: the sandboxed, script-less viewer applies them.
    assert!(html.contains("<style>body{color:red}</style>"), "{html}");
    assert!(html.contains("<b>report</b>"));
    assert!(html.contains("style=\"color:#333\""), "benign inline CSS is kept: {html}");
    assert!(html.contains("src=\"cid:logo@x\""));
    assert!(html.contains("href=\"https://example.com\""));
    assert!(html.contains("rel=\"noopener noreferrer nofollow\""));
    assert!(html.contains("target=\"_blank\""));
}

#[test]
fn sanitizer_strips_event_handlers_and_forms() {
    let out = sanitize_html(r#"<img src=x onerror="alert(1)"><form action="https://x"><input name=p></form><a href="vbscript:x">v</a><svg onload=alert(1)></svg>"#);
    assert!(!out.contains("onerror"));
    assert!(!out.contains("<form"));
    assert!(!out.contains("<input"));
    assert!(!out.contains("vbscript"));
    assert!(!out.contains("onload"));
}

#[test]
fn cid_images_become_data_urls() {
    let html = r#"<img src="cid:logo@x"><img src="cid:missing">"#;
    let out = inline_cid_images(html, &[("logo@x".into(), "image/png".into(), b"\x89PNG\r\n\x1a\n".to_vec())]);
    assert!(out.contains(&format!("data:image/png;base64,{PNG_B64}")));
    assert!(out.contains("cid:missing"));
}

#[test]
fn html_only_message_gets_text_fallback() {
    let raw = crlf("From: a@b.c\nSubject: H\nContent-Type: text/html; charset=utf-8\n\n<style>p{}</style><p>Привет&nbsp;<b>мир</b></p><br>Line&amp;two");
    let p = parse_message(&raw).unwrap();
    let text = p.body_text.unwrap();
    assert!(text.contains("Привет мир"), "{text}");
    assert!(text.contains("Line&two"));
    assert!(!text.contains("p{}"));
    assert!(p.body_html.is_some());
}

#[test]
fn html_to_text_handles_multibyte_inside_tags() {
    let text = html_to_text(r#"<p title="заголовок">Текст ✓</p><script>var x="тест"</script>"#);
    assert_eq!(text.trim(), "Текст ✓");
}

#[test]
fn snippet_collapses_whitespace_skips_quotes_and_truncates() {
    assert_eq!(make_snippet("Hello\n\n   world\n> quoted reply\n"), "Hello world");
    let long = "word ".repeat(100);
    let s = make_snippet(&long);
    assert!(s.ends_with('…'));
    assert_eq!(s.chars().count(), 181);
}

#[test]
fn missing_headers_get_defaults() {
    let p = parse_message(&crlf("Content-Type: text/plain\n\nbody only")).unwrap();
    assert_eq!(p.subject, "");
    assert_eq!(p.from.address, "");
    assert!(p.date > 0);
}

#[test]
fn decodes_imap_modified_utf7_names() {
    assert_eq!(decode_modified_utf7("INBOX"), "INBOX");
    assert_eq!(decode_modified_utf7("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"), "Отправленные");
    assert_eq!(decode_modified_utf7("Tom &- Jerry"), "Tom & Jerry");
}

const NEWSLETTER: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>My Newsletter Title</title><style>.pre{display:none} p{color:red}</style><!--[if mso]><style>x{}</style><![endif]--></head><body><noscript><img src="https://t.example/p.gif"></noscript><div class="pre" style="display:none">Preheader</div><p class="x">Hello <b>there</b></p><script>alert(1)</script></body></html>"#;

#[test]
fn sanitizer_drops_the_title_text_and_keeps_styles() {
    let out = crate::mime::sanitize_html(NEWSLETTER);
    assert!(!out.contains("My Newsletter Title"), "{out}");
    assert!(!out.contains("t.example"), "{out}");
    assert!(!out.contains("alert"), "{out}");
    assert!(!out.contains("mso"), "{out}");
    assert!(out.starts_with("<style>.pre{display:none} p{color:red}</style>"), "{out}");
    assert!(out.contains(r#"<p class="x">Hello <b>there</b></p>"#), "{out}");
}

#[test]
fn style_content_cannot_break_out() {
    let out = crate::mime::sanitize_html("<style>p{}</style><img src=x onerror=alert(1)></style><script>alert(2)</script>");
    assert!(!out.contains("onerror") && !out.contains("<script"), "{out}");
}

#[test]
fn text_of_an_html_only_mail_has_no_head() {
    let text = crate::mime::html_to_text(NEWSLETTER);
    assert!(!text.contains("My Newsletter Title"), "{text}");
    assert!(!text.contains("color:red"), "{text}");
    assert!(text.contains("Hello there"), "{text}");
    // <header> is content, not <head>.
    assert!(crate::mime::html_to_text("<header>Top</header><p>Body</p>").contains("Top"));
}
