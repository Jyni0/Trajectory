use crate::error::AppError;
use crate::models::AccountKind;
use crate::oauth::{parse_callback, OAuthProvider, TokenCache};
use crate::secrets::MemoryStore;

#[test]
fn parses_successful_redirect() {
    let r = parse_callback("GET /callback?code=4%2F0Ab&state=xyz&scope=email HTTP/1.1").unwrap();
    assert_eq!(r, Some(("4/0Ab".to_string(), "xyz".to_string())));
}

#[test]
fn ignores_unrelated_paths() {
    assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1").unwrap(), None);
}

#[test]
fn reports_denied_consent() {
    let err = parse_callback("GET /callback?error=access_denied&error_description=User+cancelled HTTP/1.1").unwrap_err();
    match err {
        AppError::Auth(msg) => assert!(msg.contains("access_denied") && msg.contains("User cancelled")),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn rejects_incomplete_redirect() {
    assert!(matches!(parse_callback("GET /callback?code=abc HTTP/1.1"), Err(AppError::Protocol(_))));
    assert!(matches!(parse_callback("garbage"), Err(AppError::Protocol(_))));
}

#[test]
fn provider_mapping_and_scopes() {
    assert_eq!(OAuthProvider::for_kind(AccountKind::Gmail).unwrap(), OAuthProvider::Google);
    assert_eq!(OAuthProvider::for_kind(AccountKind::Outlook).unwrap(), OAuthProvider::Microsoft);
    assert!(OAuthProvider::for_kind(AccountKind::Imap).is_err());
    assert!(OAuthProvider::Microsoft.scopes().contains(&"offline_access"));
    assert!(OAuthProvider::Google.scopes().iter().any(|s| s.ends_with("gmail.modify")));
}

#[tokio::test]
async fn cached_tokens_are_reused_without_refresh() {
    let cache = TokenCache::default();
    cache.put(
        "acc",
        &crate::oauth::OAuthGrant {
            access_token: "live-token".into(),
            refresh_token: None,
            expires_at: chrono::Utc::now().timestamp() + 3600,
        },
    );
    // No refresh token is stored, so any refresh attempt would fail.
    let t = cache.access_token("acc", AccountKind::Gmail, &MemoryStore::default()).await.unwrap();
    assert_eq!(t, "live-token");
}

#[tokio::test]
async fn expired_token_without_refresh_token_fails() {
    let cache = TokenCache::default();
    cache.put(
        "acc",
        &crate::oauth::OAuthGrant { access_token: "old".into(), refresh_token: None, expires_at: 0 },
    );
    let err = cache.access_token("acc", AccountKind::Gmail, &MemoryStore::default()).await.unwrap_err();
    assert!(matches!(err, AppError::Secret(_)), "{err:?}");
}

// ───────────────────────── sign-in apps (Settings) ─────────────────────────

const GOOGLE_ID: &str = "123456789-abc123def.apps.googleusercontent.com";
const MS_ID: &str = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";

#[test]
fn an_app_saved_in_settings_is_used_and_its_secret_never_shown() {
    use crate::oauth::AppSource;
    let store = MemoryStore::default();
    OAuthProvider::Google.save_app(&store, &format!("  {GOOGLE_ID} "), Some("GOCSPX-supersecretvalue")).unwrap();

    let app = OAuthProvider::Google.app(&store).unwrap();
    assert_eq!(app.client_id, GOOGLE_ID);
    assert_eq!(app.client_secret.as_deref(), Some("GOCSPX-supersecretvalue"));
    assert_eq!(app.source, AppSource::Settings);
    assert!(!format!("{app:?}").contains("supersecret"), "Debug must not leak the secret");

    let info = OAuthProvider::Google.info(&store).unwrap();
    assert_eq!(info.client_id.as_deref(), Some(GOOGLE_ID));
    assert_eq!(info.secret_hint.as_deref(), Some("GOCSPX-••••••"));
    assert_eq!(info.source, Some(AppSource::Settings));
    assert!(!serde_json::to_string(&info).unwrap().contains("supersecret"));
}

#[test]
fn saving_again_without_a_secret_keeps_the_old_one() {
    let store = MemoryStore::default();
    OAuthProvider::Google.save_app(&store, GOOGLE_ID, Some("GOCSPX-first")).unwrap();
    let other = "987-zzz.apps.googleusercontent.com";
    OAuthProvider::Google.save_app(&store, other, None).unwrap();
    let app = OAuthProvider::Google.app(&store).unwrap();
    assert_eq!((app.client_id.as_str(), app.client_secret.as_deref()), (other, Some("GOCSPX-first")));
}

#[test]
fn invalid_ids_and_a_missing_google_secret_are_refused() {
    let store = MemoryStore::default();
    assert!(OAuthProvider::Google.save_app(&store, "not-an-id", Some("GOCSPX-x")).is_err());
    assert!(OAuthProvider::Google.save_app(&store, GOOGLE_ID, None).is_err(), "Google desktop clients need the secret");
    assert!(OAuthProvider::Google.save_app(&store, GOOGLE_ID, Some("has space")).is_err());
    assert!(OAuthProvider::Microsoft.save_app(&store, "0f1e2d3c-4b5a", None).is_err());
    assert!(OAuthProvider::Microsoft.save_app(&store, "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1fg", None).is_err());
    // Microsoft public clients have no secret.
    OAuthProvider::Microsoft.save_app(&store, MS_ID, None).unwrap();
    let ms = OAuthProvider::Microsoft.info(&store).unwrap();
    assert_eq!((ms.client_id.as_deref(), ms.secret_hint), (Some(MS_ID), None));
}

#[test]
fn clearing_forgets_the_saved_app() {
    let store = MemoryStore::default();
    OAuthProvider::Microsoft.save_app(&store, MS_ID, None).unwrap();
    OAuthProvider::Microsoft.clear_app(&store).unwrap();
    // Unless the environment or the build provides one, sign-in now explains where to set it up.
    if std::env::var("TRAJECTORY_MS_CLIENT_ID").is_err() && option_env!("TRAJECTORY_MS_CLIENT_ID").is_none() {
        let err = OAuthProvider::Microsoft.app(&store).unwrap_err().to_string();
        assert!(err.contains("Settings"), "{err}");
    }
}

#[test]
fn providers_parse_from_their_ids() {
    for p in [OAuthProvider::Google, OAuthProvider::Microsoft] {
        assert_eq!(OAuthProvider::parse(p.id()).unwrap(), p);
    }
    assert!(OAuthProvider::parse("yahoo").is_err());
}
