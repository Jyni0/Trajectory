//! BIMI — brand logos next to authenticated mail.
//!
//! A domain publishes `default._bimi.<domain> TXT "v=BIMI1; l=<svg>; a=<pem>"`.
//! The logo is shown only for mail whose sender domain passed DMARC (see
//! `mime::dmarc_pass_domain`) and whose DMARC policy is enforced
//! (quarantine/reject at 100 %), as the BIMI spec requires — otherwise anyone
//! could put a bank's logo on a forged message.
//!
//! The logo counts as *verified* only when the `a=` Verified Mark Certificate
//!  * chains to one of the VMC roots bundled in `certs/vmc` (DigiCert,
//!    GlobalSign, SSL.com — Sectigo issues under the latter),
//!  * carries the BIMI extended key usage and is valid today,
//!  * names the domain in its subject alternative names, and
//!  * embeds the very same SVG that `l=` serves.
//!
//! The certified SVG is what gets shown then. Logos are rendered by the UI as
//! `<img src="data:image/svg+xml…">`, where SVG can run no script and load
//! nothing.

use std::io::Read;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use webpki::{EndEntityCert, KeyUsage};

use crate::error::{AppError, AppResult};
use crate::models::SenderLogo;

/// VMC roots, DER. Fetched from the CAs' own AIA URLs and cross-checked
/// (SHA-256) against the roots shipped in live VMC chains:
///  * DigiCert Verified Mark Root CA — 504386C9…33C46B42
///  * GlobalSign Verified Mark Root R42 — CD122CB8…524E7E0B
///  * SSL.com VMC RSA Root CA 2024 — 8F9D1B76…154BD30D
const VMC_ROOTS: &[&[u8]] = &[
    include_bytes!("../certs/vmc/DigiCertVerifiedMarkRootCA.crt"),
    include_bytes!("../certs/vmc/gsverifiedmarkrootr42.crt"),
    include_bytes!("../certs/vmc/SSL.com-VMC-Root-2024-RSA.cer"),
];

/// id-kp-BrandIndicatorforMessageIdentification (1.3.6.1.5.5.7.3.31), DER value.
const EKU_BIMI: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x1f];
/// id-pe-logotype (1.3.6.1.5.5.7.1.12).
const OID_LOGOTYPE: &str = "1.3.6.1.5.5.7.1.12";

const MAX_SVG: usize = 64 * 1024;
const MAX_PEM: usize = 64 * 1024;
/// Gunzipped certificate logo cap (zip-bomb guard).
const MAX_CERT_SVG: u64 = 256 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// TXT lookups, abstracted for tests.
#[async_trait]
pub trait Dns: Send + Sync {
    /// The TXT strings of `name` (each record's chunks joined); empty if none.
    async fn txt(&self, name: &str) -> AppResult<Vec<String>>;
}

/// HTTPS GET with a size cap, abstracted for tests.
#[async_trait]
pub trait Fetch: Send + Sync {
    async fn get(&self, url: &str, max: usize) -> AppResult<Vec<u8>>;
    /// The first `max` bytes of a page (the rest is dropped, not an error) and
    /// the URL after redirects, which is the base for its links.
    async fn get_page(&self, url: &str, max: usize) -> AppResult<(String, Vec<u8>)> {
        Ok((url.to_string(), self.get(url, max).await?))
    }
}

/// The system resolver.
pub struct SystemDns(hickory_resolver::TokioResolver);

impl SystemDns {
    pub fn new() -> AppResult<Self> {
        let r = hickory_resolver::TokioResolver::builder_tokio()
            .map_err(|e| AppError::Network(format!("DNS: {e}")))?
            .build()
            .map_err(|e| AppError::Network(format!("DNS: {e}")))?;
        Ok(Self(r))
    }
}

#[async_trait]
impl Dns for SystemDns {
    async fn txt(&self, name: &str) -> AppResult<Vec<String>> {
        use hickory_resolver::proto::rr::RData;
        let lookup = match self.0.txt_lookup(format!("{name}.")).await {
            Ok(l) => l,
            // NXDOMAIN / no records are an answer, not an error.
            Err(e) if e.is_no_records_found() || e.is_nx_domain() => return Ok(vec![]),
            Err(e) => return Err(AppError::Network(format!("DNS {name}: {e}"))),
        };
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|r| match &r.data {
                RData::TXT(t) => Some(t.txt_data.iter().map(|c| String::from_utf8_lossy(c)).collect::<String>()),
                _ => None,
            })
            .collect())
    }
}

/// HTTPS only, no redirects off HTTPS, bounded size and time.
pub struct HttpsFetch(reqwest::Client);

impl HttpsFetch {
    pub fn new() -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(3))
            .timeout(FETCH_TIMEOUT)
            // Some sites refuse clients that do not say who they are.
            .user_agent(concat!("Mozilla/5.0 (compatible; Trajectory/", env!("CARGO_PKG_VERSION"), ")"))
            .build()
            .map_err(|e| AppError::Network(e.to_string()))?;
        Ok(Self(client))
    }
}

#[async_trait]
impl Fetch for HttpsFetch {
    async fn get(&self, url: &str, max: usize) -> AppResult<Vec<u8>> {
        Ok(self.fetch(url, max, false).await?.1)
    }

    async fn get_page(&self, url: &str, max: usize) -> AppResult<(String, Vec<u8>)> {
        self.fetch(url, max, true).await
    }
}

impl HttpsFetch {
    /// Over `max` bytes: an error, or with `truncate` the first `max` bytes.
    async fn fetch(&self, url: &str, max: usize, truncate: bool) -> AppResult<(String, Vec<u8>)> {
        let mut resp = self
            .0
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| AppError::Network(format!("{url}: {e}")))?;
        let final_url = resp.url().to_string();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| AppError::Network(e.to_string()))? {
            body.extend_from_slice(&chunk);
            if body.len() > max {
                if truncate {
                    body.truncate(max);
                    break;
                }
                return Err(AppError::Invalid(format!("{url} is larger than {max} bytes")));
            }
        }
        Ok((final_url, body))
    }
}

/// `v=BIMI1; l=…; a=…` → (logo URL, certificate URL). `None` for anything else.
pub fn parse_bimi(txt: &str) -> Option<(Option<String>, Option<String>)> {
    let mut tags = txt.split(';').map(str::trim).filter(|t| !t.is_empty());
    let version = tags.next()?.split_once('=')?;
    if !(version.0.trim().eq_ignore_ascii_case("v") && version.1.trim() == "BIMI1") {
        return None;
    }
    let (mut l, mut a) = (None, None);
    for t in tags {
        if let Some((k, v)) = t.split_once('=') {
            let v = v.trim();
            let url = (!v.is_empty()).then(|| v.to_string());
            match k.trim().to_ascii_lowercase().as_str() {
                "l" => l = url,
                "a" => a = url,
                _ => {}
            }
        }
    }
    Some((l, a))
}

/// Whether a DMARC record is enforced: `p` (or `sp` for a subdomain) is
/// quarantine/reject and `pct`, if given, is 100.
pub fn dmarc_enforced(txt: &str, subdomain: bool) -> bool {
    let tags: Vec<(String, String)> = txt
        .split(';')
        .filter_map(|t| t.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase()))
        .collect();
    let get = |k: &str| tags.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    if get("v") != Some("dmarc1") {
        return false;
    }
    let policy = if subdomain { get("sp").or(get("p")) } else { get("p") };
    matches!(policy, Some("quarantine" | "reject")) && get("pct").map_or(true, |p| p == "100")
}

/// `mail.news.example.com` → itself, then its parents down to two labels.
fn candidates(domain: &str) -> Vec<String> {
    let labels: Vec<&str> = domain.split('.').collect();
    (0..labels.len().saturating_sub(1)).map(|i| labels[i..].join(".")).collect()
}

fn valid_domain(d: &str) -> bool {
    d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

async fn first_record(dns: &dyn Dns, name: &str, prefix: &str) -> AppResult<Option<String>> {
    Ok(dns.txt(name).await?.into_iter().find(|t| t.trim_start().to_ascii_lowercase().starts_with(prefix)))
}

/// Looks a domain's logo up. `Ok(SenderLogo::default())` = it has none.
pub async fn lookup(dns: &dyn Dns, http: &dyn Fetch, domain: &str, now: UnixTime) -> AppResult<SenderLogo> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if !valid_domain(&domain) {
        return Err(AppError::Invalid(format!("'{domain}' is not a domain")));
    }
    let names = candidates(&domain);

    // DMARC must be enforced (own record, or the parent's `sp`).
    let mut enforced = false;
    for (i, name) in names.iter().enumerate() {
        if let Some(rec) = first_record(dns, &format!("_dmarc.{name}"), "v=dmarc1").await? {
            enforced = dmarc_enforced(&rec, i > 0);
            break;
        }
    }
    if !enforced {
        return Ok(SenderLogo::default());
    }

    let mut found = None;
    for name in &names {
        if let Some(rec) = first_record(dns, &format!("default._bimi.{name}"), "v=bimi1").await? {
            found = parse_bimi(&rec).map(|r| (name.clone(), r));
            break;
        }
    }
    let Some((record_domain, (logo_url, cert_url))) = found else { return Ok(SenderLogo::default()) };

    let served = match &logo_url {
        Some(url) => Some(clean_svg(&http.get(url, MAX_SVG).await?)?),
        None => None,
    };

    if let Some(url) = cert_url {
        let pem = http.get(&url, MAX_PEM).await?;
        let names = [domain.as_str(), record_domain.as_str()];
        match verify_vmc(&pem, &names, now) {
            Ok(vmc) if served.as_deref().map_or(true, |s| same_svg(s, &vmc.svg)) => {
                return Ok(SenderLogo { svg: Some(data_url(&vmc.svg)), verified: true, issuer: vmc.issuer });
            }
            Ok(_) => log::info!("BIMI logo of {domain} differs from the one in its certificate"),
            Err(e) => log::info!("VMC of {domain} not accepted: {e}"),
        }
    }
    Ok(SenderLogo { svg: served.as_deref().map(data_url), verified: false, issuer: None })
}

pub(crate) fn data_url(svg: &str) -> String {
    format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg))
}

fn same_svg(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
}

/// Accepts only an SVG document with no script or external reference.
pub(crate) fn clean_svg(bytes: &[u8]) -> AppResult<String> {
    let s = std::str::from_utf8(bytes).map_err(|_| AppError::Invalid("logo is not UTF-8".into()))?;
    let lower = s.to_ascii_lowercase();
    if !lower.contains("<svg") {
        return Err(AppError::Invalid("logo is not an SVG".into()));
    }
    for bad in ["<script", "javascript:", "<foreignobject", "<!entity"] {
        if lower.contains(bad) {
            return Err(AppError::Invalid(format!("logo contains {bad}")));
        }
    }
    Ok(s.to_string())
}

/// What a valid VMC vouches for.
pub struct Vmc {
    pub svg: String,
    pub issuer: Option<String>,
}

/// Validates a VMC chain (PEM, leaf first) for one of `domains`.
pub fn verify_vmc(pem: &[u8], domains: &[&str], now: UnixTime) -> AppResult<Vmc> {
    let bad = |m: String| AppError::Invalid(format!("VMC: {m}"));
    let ders: Vec<Vec<u8>> = x509_parser::pem::Pem::iter_from_buffer(pem)
        .filter_map(Result::ok)
        .filter(|p| p.label == "CERTIFICATE")
        .map(|p| p.contents)
        .collect();
    let (leaf_der, rest) = ders.split_first().ok_or_else(|| bad("no certificate".into()))?;

    let anchors: Vec<_> = VMC_ROOTS
        .iter()
        .filter_map(|der| webpki::anchor_from_trusted_cert(&CertificateDer::from(*der)).ok().map(|a| a.to_owned()))
        .collect();
    let intermediates: Vec<CertificateDer<'_>> = rest.iter().map(|d| CertificateDer::from(d.as_slice())).collect();
    let leaf_cert = CertificateDer::from(leaf_der.as_slice());
    let leaf = EndEntityCert::try_from(&leaf_cert).map_err(|e| bad(e.to_string()))?;
    leaf.verify_for_usage(
        webpki::ALL_VERIFICATION_ALGS,
        &anchors,
        &intermediates,
        now,
        KeyUsage::required(EKU_BIMI),
        None,
        None,
    )
    .map_err(|e| bad(format!("chain: {e}")))?;

    let named = domains.iter().any(|d| {
        ServerName::try_from(d.to_string()).is_ok_and(|n| leaf.verify_is_valid_for_subject_name(&n).is_ok())
    });
    if !named {
        return Err(bad(format!("not issued for {}", domains.join(" / "))));
    }

    let (_, cert) = x509_parser::parse_x509_certificate(leaf_der).map_err(|e| bad(e.to_string()))?;
    let issuer = cert.issuer().iter_organization().next().and_then(|o| o.as_str().ok()).map(str::to_string);
    let ext = cert
        .extensions()
        .iter()
        .find(|e| e.oid.to_id_string() == OID_LOGOTYPE)
        .ok_or_else(|| bad("no logotype".into()))?;
    let svg = logo_from_extension(ext.value).ok_or_else(|| bad("logotype has no SVG".into()))?;
    Ok(Vmc { svg: clean_svg(svg.as_bytes())?, issuer })
}

/// The SVG inside a logotype extension: a `data:image/svg+xml;base64,` URI,
/// usually gzip-compressed (RFC 3709 / RFC 6170).
fn logo_from_extension(value: &[u8]) -> Option<String> {
    const PREFIX: &[u8] = b"data:image/svg+xml;base64,";
    let start = value.windows(PREFIX.len()).position(|w| w == PREFIX)? + PREFIX.len();
    let end = value[start..]
        .iter()
        .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')))
        .map_or(value.len(), |p| start + p);
    let raw = base64::engine::general_purpose::STANDARD.decode(&value[start..end]).ok()?;
    let bytes = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(raw.as_slice()).take(MAX_CERT_SVG).read_to_end(&mut out).ok()?;
        out
    } else {
        raw
    };
    String::from_utf8(bytes).ok()
}
