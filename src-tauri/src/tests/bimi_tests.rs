//! BIMI: record parsing, DMARC gating, VMC verification against real chains
//! (eBay/DigiCert, GlobalSign, Sectigo/SSL.com, captured October 2026).

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use rustls_pki_types::UnixTime;

use crate::bimi::{self, dmarc_enforced, parse_bimi, verify_vmc, Dns, Fetch};
use crate::error::{AppError, AppResult};
use crate::mime::dmarc_pass_domain;

const EBAY_PEM: &[u8] = include_bytes!("fixtures/bimi/ebay.pem");
const EBAY_SVG: &[u8] = include_bytes!("fixtures/bimi/ebay.svg");
const GLOBALSIGN_PEM: &[u8] = include_bytes!("fixtures/bimi/globalsign.pem");
const SECTIGO_PEM: &[u8] = include_bytes!("fixtures/bimi/sectigo.pem");
const SECTIGO_SVG: &[u8] = include_bytes!("fixtures/bimi/sectigo.svg");

/// Inside every fixture's validity window.
fn oct_2026() -> UnixTime {
    UnixTime::since_unix_epoch(Duration::from_secs(1_791_300_000))
}

#[derive(Default)]
struct FakeNet {
    txt: HashMap<String, Vec<String>>,
    files: HashMap<String, Vec<u8>>,
}

impl FakeNet {
    fn txt(mut self, name: &str, value: &str) -> Self {
        self.txt.entry(name.into()).or_default().push(value.into());
        self
    }
    fn file(mut self, url: &str, body: &[u8]) -> Self {
        self.files.insert(url.into(), body.to_vec());
        self
    }
}

#[async_trait]
impl Dns for FakeNet {
    async fn txt(&self, name: &str) -> AppResult<Vec<String>> {
        Ok(self.txt.get(name).cloned().unwrap_or_default())
    }
}

#[async_trait]
impl Fetch for FakeNet {
    async fn get(&self, url: &str, _max: usize) -> AppResult<Vec<u8>> {
        self.files.get(url).cloned().ok_or_else(|| AppError::Network(format!("404 {url}")))
    }
}

fn ebay_net() -> FakeNet {
    FakeNet::default()
        .txt("_dmarc.ebay.com", "v=DMARC1; p=reject; pct=100")
        .txt("default._bimi.ebay.com", "v=BIMI1;l=https://vmc.example/ebay.svg;a=https://vmc.example/ebay.pem")
        .file("https://vmc.example/ebay.svg", EBAY_SVG)
        .file("https://vmc.example/ebay.pem", EBAY_PEM)
}

#[test]
fn parses_bimi_and_dmarc_records() {
    assert_eq!(parse_bimi("v=BIMI1; l=https://x/l.svg; a=https://x/a.pem;"), Some((Some("https://x/l.svg".into()), Some("https://x/a.pem".into()))));
    assert_eq!(parse_bimi("v=BIMI1;l=https://x/l.svg;"), Some((Some("https://x/l.svg".into()), None)));
    assert_eq!(parse_bimi("v=BIMI1; l=; a=;"), Some((None, None)), "declined");
    assert_eq!(parse_bimi("v=spf1 -all"), None);

    assert!(dmarc_enforced("v=DMARC1; p=reject", false));
    assert!(dmarc_enforced("v=DMARC1; p=quarantine; pct=100", false));
    assert!(!dmarc_enforced("v=DMARC1; p=none", false));
    assert!(!dmarc_enforced("v=DMARC1; p=quarantine; pct=50", false));
    assert!(!dmarc_enforced("v=DMARC1; p=reject; sp=none", true), "subdomain policy wins");
}

#[test]
fn dmarc_result_is_read_from_authentication_results() {
    let ar = "mx.google.com;\r\n       dkim=pass header.i=@ebay.com;\r\n       spf=pass smtp.mailfrom=ebay.com;\r\n       dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=ebay.com";
    assert_eq!(dmarc_pass_domain(ar, "ebay@ebay.com").as_deref(), Some("ebay.com"));
    assert_eq!(dmarc_pass_domain(ar, "x@paypal.com"), None, "header.from must be the sender");
    assert_eq!(dmarc_pass_domain("mx; dmarc=fail header.from=ebay.com", "a@ebay.com"), None);
    assert_eq!(dmarc_pass_domain("mx; dmarc=pass", "a@ebay.com"), None);
}

#[test]
fn only_the_top_authentication_results_header_counts() {
    // A forged "pass" further down is ignored: the receiver's own (top) says fail.
    let raw = b"Authentication-Results: mx.receiver.example; dmarc=fail header.from=paypal.com\r\n\
Authentication-Results: forged; dmarc=pass header.from=paypal.com\r\n\
From: PayPal <service@paypal.com>\r\nSubject: hi\r\n\r\nbody";
    assert_eq!(crate::mime::parse_message(raw).unwrap().logo_domain, None);
    let raw = b"Authentication-Results: mx.receiver.example; dmarc=pass header.from=paypal.com\r\nFrom: service@paypal.com\r\nSubject: hi\r\n\r\nbody";
    assert_eq!(crate::mime::parse_message(raw).unwrap().logo_domain.as_deref(), Some("paypal.com"));
}

#[test]
fn real_vmcs_verify_against_the_bundled_roots() {
    for (pem, domain, issuer) in [
        (EBAY_PEM, "ebay.com", "DigiCert"),
        (GLOBALSIGN_PEM, "globalsign.com", "GlobalSign"),
        (SECTIGO_PEM, "sectigo.com", "Sectigo"),
    ] {
        let vmc = verify_vmc(pem, &[domain], oct_2026()).unwrap_or_else(|e| panic!("{domain}: {e}"));
        assert!(vmc.svg.contains("<svg"), "{domain}");
        assert!(vmc.issuer.as_deref().unwrap_or_default().contains(issuer), "{domain}: {:?}", vmc.issuer);
    }
}

#[test]
fn vmc_is_refused_for_another_domain_or_time() {
    assert!(verify_vmc(EBAY_PEM, &["paypal.com"], oct_2026()).is_err());
    let expired = UnixTime::since_unix_epoch(Duration::from_secs(1_900_000_000));
    assert!(verify_vmc(EBAY_PEM, &["ebay.com"], expired).is_err());
}

#[test]
fn a_chain_without_its_intermediate_or_with_a_foreign_one_fails() {
    // Leaf only: the chain cannot reach a bundled root.
    let leaf_end = std::str::from_utf8(EBAY_PEM).unwrap().find("-----END CERTIFICATE-----").unwrap() + 25;
    assert!(verify_vmc(&EBAY_PEM[..leaf_end], &["ebay.com"], oct_2026()).is_err());
    // eBay's leaf followed by GlobalSign's chain: issuer does not match.
    let mut mixed = EBAY_PEM[..leaf_end].to_vec();
    let gs = std::str::from_utf8(GLOBALSIGN_PEM).unwrap();
    mixed.extend_from_slice(&GLOBALSIGN_PEM[gs.find("-----END CERTIFICATE-----").unwrap() + 25..]);
    assert!(verify_vmc(&mixed, &["ebay.com"], oct_2026()).is_err());
}

#[tokio::test]
async fn lookup_returns_the_certified_logo() {
    let net = ebay_net();
    let logo = bimi::lookup(&net, &net, "EBay.com", oct_2026()).await.unwrap();
    assert!(logo.verified);
    assert!(logo.issuer.unwrap().contains("DigiCert"));
    assert!(logo.svg.unwrap().starts_with("data:image/svg+xml;base64,"));
}

#[tokio::test]
async fn subdomains_use_the_organisational_records() {
    let net = ebay_net();
    let logo = bimi::lookup(&net, &net, "reply.ebay.com", oct_2026()).await.unwrap();
    assert!(logo.verified, "SAN names ebay.com, where the record was found");
}

#[tokio::test]
async fn no_logo_without_enforced_dmarc() {
    let net = FakeNet::default()
        .txt("_dmarc.ebay.com", "v=DMARC1; p=none")
        .txt("default._bimi.ebay.com", "v=BIMI1;l=https://vmc.example/ebay.svg")
        .file("https://vmc.example/ebay.svg", EBAY_SVG);
    assert_eq!(bimi::lookup(&net, &net, "ebay.com", oct_2026()).await.unwrap().svg, None);
}

#[tokio::test]
async fn logo_without_a_certificate_is_shown_unverified() {
    let net = FakeNet::default()
        .txt("_dmarc.example.org", "v=DMARC1; p=quarantine")
        .txt("default._bimi.example.org", "v=BIMI1; l=https://cdn.example/logo.svg")
        .file("https://cdn.example/logo.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" version="1.2" baseProfile="tiny-ps"><title>Ex</title></svg>"#);
    let logo = bimi::lookup(&net, &net, "example.org", oct_2026()).await.unwrap();
    assert!(logo.svg.is_some());
    assert!(!logo.verified);
}

#[tokio::test]
async fn a_certificate_for_another_logo_does_not_verify() {
    // Sectigo's logo served next to eBay's certificate.
    let net = ebay_net().file("https://vmc.example/ebay.svg", SECTIGO_SVG);
    let logo = bimi::lookup(&net, &net, "ebay.com", oct_2026()).await.unwrap();
    assert!(!logo.verified);
    assert!(logo.svg.is_some(), "still shown, without the check mark");
}

#[tokio::test]
async fn scripted_svg_is_refused() {
    let net = FakeNet::default()
        .txt("_dmarc.evil.example", "v=DMARC1; p=reject")
        .txt("default._bimi.evil.example", "v=BIMI1; l=https://evil.example/l.svg")
        .file("https://evil.example/l.svg", b"<svg><script>alert(1)</script></svg>");
    assert!(bimi::lookup(&net, &net, "evil.example", oct_2026()).await.is_err());
}

#[tokio::test]
async fn invalid_domains_are_rejected() {
    let net = FakeNet::default();
    for d in ["", "localhost", "a..b", "x_y.com", "evil.com/path"] {
        assert!(bimi::lookup(&net, &net, d, oct_2026()).await.is_err(), "{d}");
    }
}

/// Real DNS + HTTPS: `cargo test live_bimi -- --ignored --nocapture`, or
/// `BIMI_DOMAINS=a.com,b.org` for other domains.
#[tokio::test]
#[ignore]
async fn live_bimi() {
    let dns = bimi::SystemDns::new().unwrap();
    let http = bimi::HttpsFetch::new().unwrap();
    let now = UnixTime::now();
    let list = std::env::var("BIMI_DOMAINS").unwrap_or_else(|_| "ebay.com,globalsign.com,sectigo.com,paypal.com,apple.com".into());
    for d in list.split(',').map(str::trim) {
        let logo = bimi::lookup(&dns, &http, d, now).await;
        println!("{d}: {:?}", logo.as_ref().map(|l| (l.svg.is_some(), l.verified, l.issuer.clone())));
        let icon = crate::site_icon::lookup(&http, d).await;
        println!("  site icon: {:?}", icon.as_ref().map(|i| i.as_deref().map(|u| &u[..u.find(',').unwrap_or(0)])));
    }
}

#[tokio::test]
async fn logos_are_served_from_the_cache() {
    let (ctx, ..) = super::test_context();
    let logo = crate::models::SenderLogo { svg: Some("data:image/svg+xml;base64,PHN2Zy8+".into()), verified: true, issuer: Some("DigiCert, Inc.".into()) };
    ctx.db.store_logo("ebay.com", &logo, chrono::Utc::now().timestamp()).unwrap();
    // No network is touched: the fresh cache entry answers.
    assert_eq!(ctx.sender_logo("EBAY.com").await.unwrap(), logo);
}

// ───────────────────────────── site icons ─────────────────────────────

const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 64, 0, 0, 0, 64, 8, 6, 0, 0, 0, 0xaa, 0x69, 0x71, 0xde,
    0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
];

#[test]
fn site_domain_is_the_registrable_one() {
    use crate::site_icon::site_domain;
    assert_eq!(site_domain("mail.anthropic.com").as_deref(), Some("anthropic.com"));
    assert_eq!(site_domain("al.mail.deepseek.com").as_deref(), Some("deepseek.com"));
    assert_eq!(site_domain("news.shop.co.uk").as_deref(), Some("shop.co.uk"));
    assert_eq!(site_domain("co.uk"), None);
}

#[test]
fn icon_links_prefer_svg_then_the_largest() {
    let base = url::Url::parse("https://www.example.com/en/").unwrap();
    let html = r#"<html><head>
        <link rel="icon" href="/favicon-16.png" sizes="16x16">
        <link rel="apple-touch-icon" href="touch.png">
        <link data-rel="icon" href="/nope.png">
        <link rel='icon' type='image/png' sizes='192x192' href='https://cdn.example.com/i192.png'>
        <link rel="shortcut icon" href="/f.ico">
        <link rel="icon" href="http://insecure.example.com/x.png">
        <link rel="icon" href="https://192.168.1.1/router.png">
        <link rel="mask-icon" href="/mask.svg">
        <link rel="icon" href="/logo.svg" type="image/svg+xml">
        <link rel="stylesheet" href="/s.css">
        <link rel="icon" sizes="48x48" href="data:image/png;base64,iVBORw0KGgo=">
        <link rel="icon" sizes="64x64" href="data:text/html;base64,PGh0bWw+">
    </head><body><link rel="icon" href="/late.png"></body></html>"#;
    assert_eq!(
        crate::site_icon::icon_links(html, &base),
        [
            "https://www.example.com/logo.svg",
            "https://cdn.example.com/i192.png",
            "https://www.example.com/en/touch.png",
            "data:image/png;base64,iVBORw0KGgo=",
            "https://www.example.com/f.ico",
            "https://www.example.com/favicon-16.png",
        ]
    );
}

#[test]
fn only_real_images_become_icons() {
    use crate::site_icon::image_data_url;
    assert!(image_data_url(PNG).unwrap().starts_with("data:image/png;base64,"));
    assert!(image_data_url(b"<!DOCTYPE html><html><body><svg></svg></body></html>").is_none());
    assert!(image_data_url(b"<svg xmlns='http://www.w3.org/2000/svg'><script>x</script></svg>").is_none());
    assert!(image_data_url(br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#).unwrap().starts_with("data:image/svg+xml;base64,"));
    assert!(image_data_url(&PNG[..20]).is_none(), "too small to be an icon");
}

#[tokio::test]
async fn site_icon_comes_from_the_organisations_home_page() {
    let net = FakeNet::default()
        .file("https://anthropic.com/", br#"<head><link rel="apple-touch-icon" href="/touch.png"></head>"#)
        .file("https://anthropic.com/touch.png", PNG);
    let icon = crate::site_icon::lookup(&net, "mail.anthropic.com").await.unwrap();
    assert!(icon.unwrap().starts_with("data:image/png;base64,"));
}

#[tokio::test]
async fn site_icon_falls_back_to_favicon_ico() {
    let ico = [&[0u8, 0, 1, 0][..], &[7u8; 100][..]].concat();
    let net = FakeNet::default()
        .file("https://stripe.com/", br#"<head><link rel="icon" href="/missing.png"></head>"#)
        .file("https://stripe.com/favicon.ico", &ico);
    let icon = crate::site_icon::lookup(&net, "stripe.com").await.unwrap();
    assert!(icon.unwrap().starts_with("data:image/x-icon;base64,"));
    // Nothing usable at all.
    assert_eq!(crate::site_icon::lookup(&FakeNet::default(), "nothing.example").await.unwrap(), None);
}
