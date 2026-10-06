//! Gmail API and Microsoft Graph providers against `wiremock`.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::fixtures::*;
use crate::error::AppError;
use crate::models::{AccountKind, FolderRole, OutgoingEmail};
use crate::providers::gmail::{GmailProvider, GmailState};
use crate::providers::graph::{GraphProvider, GraphState};
use crate::providers::{IncomingProvider, OutgoingProvider, RemoteAction, RemoteFolder};
use crate::service::AppContext;

fn inbox(path: &str) -> RemoteFolder {
    RemoteFolder { name: "Inbox".into(), path: path.into(), role: FolderRole::Inbox }
}

// ─────────────────────────────── Gmail ──────────────────────────────

async fn gmail() -> (MockServer, GmailProvider) {
    let server = MockServer::start().await;
    let p = GmailProvider::with_base("tok".into(), server.uri()).without_pacing();
    (server, p)
}

#[tokio::test]
async fn gmail_labels_map_to_folders() {
    let (server, p) = gmail().await;
    Mock::given(method("GET"))
        .and(path("/labels"))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "labels": [
            { "id": "INBOX", "name": "INBOX", "type": "system" },
            { "id": "SENT", "name": "SENT", "type": "system" },
            { "id": "CATEGORY_SOCIAL", "name": "CATEGORY_SOCIAL", "type": "system" },
            { "id": "UNREAD", "name": "UNREAD", "type": "system" },
            { "id": "Label_1", "name": "Receipts", "type": "user" }
        ]})))
        .mount(&server)
        .await;
    let folders = p.list_folders().await.unwrap();
    let got: Vec<_> = folders.iter().map(|f| (f.name.as_str(), f.path.as_str(), f.role)).collect();
    assert_eq!(
        got,
        [("Inbox", "INBOX", FolderRole::Inbox), ("Sent", "SENT", FolderRole::Sent), ("Receipts", "Label_1", FolderRole::Custom)]
    );
}

#[tokio::test]
async fn gmail_initial_and_history_sync() {
    let (server, p) = gmail().await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "emailAddress": "me@gmail.com", "historyId": "100" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/messages"))
        .and(query_param("labelIds", "INBOX"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [{ "id": "m1" }] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/messages/m1"))
        .and(query_param("format", "raw"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "m1", "labelIds": ["INBOX", "UNREAD", "STARRED"],
            "raw": URL_SAFE_NO_PAD.encode(complex_message())
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/history"))
        .and(query_param("startHistoryId", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "historyId": "105",
            "history": [{ "messagesAdded": [{ "message": { "id": "m2" } }] }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/messages/m2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "m2", "labelIds": ["INBOX"], "raw": URL_SAFE_NO_PAD.encode(simple_text("Later", "l@t"))
        })))
        .mount(&server)
        .await;

    let first = p.fetch(&inbox("INBOX"), None).await.unwrap();
    assert_eq!(first.messages.len(), 1);
    let m = &first.messages[0];
    assert!(!m.is_read && m.is_flagged);
    assert_eq!(crate::mime::parse_message(&m.raw).unwrap().subject, COMPLEX_SUBJECT);
    let st: GmailState = serde_json::from_str(first.new_state.as_deref().unwrap()).unwrap();
    assert_eq!(st.history_id, "100");

    let second = p.fetch(&inbox("INBOX"), first.new_state.as_deref()).await.unwrap();
    assert_eq!(second.messages[0].remote_id, "m2");
    assert!(second.messages[0].is_read);
    let st: GmailState = serde_json::from_str(second.new_state.as_deref().unwrap()).unwrap();
    assert_eq!(st.history_id, "105");
}

#[tokio::test]
async fn gmail_expired_history_falls_back_to_listing() {
    let (server, p) = gmail().await;
    Mock::given(method("GET")).and(path("/history")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "emailAddress": "me@gmail.com", "historyId": "900" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [] })))
        .mount(&server)
        .await;
    let res = p.fetch(&inbox("INBOX"), Some(r#"{"historyId":"1"}"#)).await.unwrap();
    assert!(res.messages.is_empty());
    assert!(res.new_state.unwrap().contains("900"));
}

#[tokio::test]
async fn gmail_actions_and_send() {
    let (server, p) = gmail().await;
    Mock::given(method("POST"))
        .and(path("/messages/m1/modify"))
        .and(body_partial_json(json!({ "removeLabelIds": ["UNREAD"] })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/messages/m1/modify"))
        .and(body_partial_json(json!({ "removeLabelIds": ["INBOX"] })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/messages/m1/trash"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/messages/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "sent1" })))
        .expect(1)
        .mount(&server)
        .await;

    p.apply("INBOX", "m1", &RemoteAction::MarkRead(true)).await.unwrap();
    p.apply("INBOX", "m1", &RemoteAction::Archive { archive_path: None }).await.unwrap();
    p.apply("INBOX", "m1", &RemoteAction::Trash { trash_path: None }).await.unwrap();

    let account = account(AccountKind::Gmail, "me@gmail.com");
    let out = OutgoingEmail {
        to: vec!["bob@example.com".into()],
        subject: "Via API".into(),
        body_text: "hello".into(),
        ..Default::default()
    };
    p.send(&AppContext::build_message(&account, &out).unwrap()).await.unwrap();

    let sent = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path() == "/messages/send")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    let raw = URL_SAFE_NO_PAD.decode(body["raw"].as_str().unwrap()).unwrap();
    assert_eq!(crate::mime::parse_message(&raw).unwrap().subject, "Via API");
}

#[tokio::test]
async fn gmail_unauthorized_maps_to_auth_error() {
    let (server, p) = gmail().await;
    Mock::given(method("GET")).and(path("/profile")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    assert!(matches!(IncomingProvider::test(&p).await, Err(AppError::Auth(_))));
}

// ─────────────────────────────── Graph ──────────────────────────────

async fn graph() -> (MockServer, GraphProvider) {
    let server = MockServer::start().await;
    let p = GraphProvider::with_base("gtok".into(), server.uri());
    (server, p)
}

#[tokio::test]
async fn graph_folders_resolve_well_known_roles() {
    let (server, p) = graph().await;
    for (wk, id) in [("inbox", "F-IN"), ("sentitems", "F-SENT"), ("deleteditems", "F-DEL")] {
        Mock::given(method("GET"))
            .and(path(format!("/mailFolders/{wk}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": id, "displayName": wk })))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/mailFolders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "value": [
            { "id": "F-IN", "displayName": "Inbox" },
            { "id": "F-SENT", "displayName": "Sent Items" },
            { "id": "F-DEL", "displayName": "Deleted Items" },
            { "id": "F-X", "displayName": "Projects" }
        ]})))
        .mount(&server)
        .await;
    let folders = p.list_folders().await.unwrap();
    let roles: Vec<_> = folders.iter().map(|f| (f.path.as_str(), f.role)).collect();
    assert_eq!(
        roles,
        [("F-IN", FolderRole::Inbox), ("F-SENT", FolderRole::Sent), ("F-DEL", FolderRole::Trash), ("F-X", FolderRole::Custom)]
    );
}

#[tokio::test]
async fn graph_delta_sync_follows_pages_and_stores_delta_link() {
    let (server, p) = graph().await;
    let next = format!("{}/delta-page-2", server.uri());
    let delta = format!("{}/delta-final?token=abc", server.uri());
    Mock::given(method("GET"))
        .and(path("/mailFolders/F-IN/messages/delta"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{ "id": "g1", "isRead": true, "flag": { "flagStatus": "notFlagged" } }],
            "@odata.nextLink": next
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/delta-page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [
                { "id": "g2", "isRead": false, "flag": { "flagStatus": "flagged" } },
                { "id": "gone", "@removed": { "reason": "deleted" } }
            ],
            "@odata.deltaLink": delta
        })))
        .mount(&server)
        .await;
    for (id, subj) in [("g1", "G one"), ("g2", "G two")] {
        Mock::given(method("GET"))
            .and(path(format!("/messages/{id}/$value")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(simple_text(subj, &format!("{id}@t"))))
            .mount(&server)
            .await;
    }

    let res = p.fetch(&inbox("F-IN"), None).await.unwrap();
    let got: Vec<_> = res.messages.iter().map(|m| (m.remote_id.as_str(), m.is_read, m.is_flagged)).collect();
    assert_eq!(got, [("g1", true, false), ("g2", false, true)]);
    let st: GraphState = serde_json::from_str(res.new_state.as_deref().unwrap()).unwrap();
    assert_eq!(st.delta_link, delta);
}

#[tokio::test]
async fn graph_actions_and_send() {
    let (server, p) = graph().await;
    Mock::given(method("PATCH"))
        .and(path("/messages/g1"))
        .and(body_partial_json(json!({ "isRead": true })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/messages/g1"))
        .and(body_partial_json(json!({ "flag": { "flagStatus": "flagged" } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/messages/g1/move"))
        .and(body_partial_json(json!({ "destinationId": "deleteditems" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sendMail"))
        .and(header("content-type", "text/plain"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    p.apply("F-IN", "g1", &RemoteAction::MarkRead(true)).await.unwrap();
    p.apply("F-IN", "g1", &RemoteAction::Flag(true)).await.unwrap();
    p.apply("F-IN", "g1", &RemoteAction::Trash { trash_path: None }).await.unwrap();

    let account = account(AccountKind::Outlook, "me@outlook.com");
    let out = OutgoingEmail { to: vec!["x@y.z".into()], subject: "Graph send".into(), body_text: "b".into(), ..Default::default() };
    p.send(&AppContext::build_message(&account, &out).unwrap()).await.unwrap();

    let req = server.received_requests().await.unwrap().into_iter().find(|r| r.url.path() == "/sendMail").unwrap();
    let mime = STANDARD.decode(&req.body).unwrap();
    assert_eq!(crate::mime::parse_message(&mime).unwrap().subject, "Graph send");
}

#[tokio::test]
async fn graph_profile_prefers_mail_then_upn() {
    let (server, p) = graph().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "mail": null, "userPrincipalName": "me@contoso.com" })))
        .mount(&server)
        .await;
    assert_eq!(p.profile_email().await.unwrap(), "me@contoso.com");
}

#[tokio::test]
async fn api_errors_carry_the_servers_reason() {
    let (server, p) = gmail().await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "error": {
            "code": 403,
            "message": "Gmail API has not been used in project 123 before or it is disabled.",
            "status": "PERMISSION_DENIED"
        }})))
        .mount(&server)
        .await;
    let err = p.profile_email().await.unwrap_err();
    assert!(matches!(err, AppError::Auth(_)), "{err:?}");
    assert!(err.to_string().contains("Gmail API has not been used in project 123"), "{err}");

    let scopes = crate::providers::api_error(403, "/profile", r#"{"error":{"message":"Request had insufficient authentication scopes."}}"#);
    assert!(scopes.to_string().contains("allow every permission"), "{scopes}");
    let plain = crate::providers::api_error(500, "/x", "<html>oops</html>");
    assert!(matches!(plain, AppError::Network(_)) && plain.to_string().contains("Internal Server Error"), "{plain}");
}

const QUOTA_BODY: &str = r#"{"error":{"code":403,"message":"Quota exceeded for quota metric 'Total Query Cost' and limit 'Units per minute per user' of service 'gmail.googleapis.com'.","errors":[{"reason":"rateLimitExceeded"}]}}"#;

#[tokio::test]
async fn gmail_rate_limits_are_retried_not_treated_as_auth_failures() {
    let (server, p) = gmail().await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(403).set_body_raw(QUOTA_BODY, "application/json"))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "emailAddress": "me@gmail.com", "historyId": "1" })))
        .mount(&server)
        .await;
    assert_eq!(p.profile_email().await.unwrap(), "me@gmail.com");

    let err = crate::providers::api_error(403, "/messages/x", QUOTA_BODY);
    assert!(matches!(err, AppError::Network(_)), "a quota is not a credentials problem: {err:?}");
}

#[tokio::test]
async fn gmail_skips_stored_messages_and_keeps_progress_when_throttled() {
    let (server, p) = gmail().await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "emailAddress": "me@gmail.com", "historyId": "7" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [{ "id": "m1" }, { "id": "m2" }, { "id": "m3" }] })))
        .mount(&server)
        .await;
    // m1 is already stored: it must not be downloaded (it would fail the test).
    Mock::given(method("GET")).and(path("/messages/m1")).respond_with(ResponseTemplate::new(500)).expect(0).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/messages/m2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "m2", "labelIds": ["INBOX"], "raw": URL_SAFE_NO_PAD.encode(simple_text("Two", "two@t"))
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/messages/m3"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let known: std::collections::HashSet<String> = ["m1".to_string()].into();
    let res = p.fetch_new(&inbox("INBOX"), None, &known).await.unwrap();
    assert_eq!(res.messages.iter().map(|m| m.remote_id.as_str()).collect::<Vec<_>>(), ["m2"]);
    assert!(res.new_state.is_none(), "cursor stays so the rest is fetched next time");
}

#[tokio::test]
async fn gmail_first_sync_takes_one_page_and_older_mail_comes_on_demand() {
    let (server, p) = gmail().await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "emailAddress": "me@gmail.com", "historyId": "9" })))
        .mount(&server)
        .await;
    // Page 2 of the listing.
    Mock::given(method("GET"))
        .and(path("/messages"))
        .and(query_param("pageToken", "p2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [{ "id": "old2" }] })))
        .mount(&server)
        .await;
    // Page 1: the newest.
    Mock::given(method("GET"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "messages": [{ "id": "new" }, { "id": "old1" }], "nextPageToken": "p2" })))
        .mount(&server)
        .await;
    for id in ["new", "old1", "old2"] {
        Mock::given(method("GET"))
            .and(path(format!("/messages/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": id, "labelIds": ["INBOX"], "raw": URL_SAFE_NO_PAD.encode(simple_text(id, &format!("{id}@t")))
            })))
            .mount(&server)
            .await;
    }

    let first = p.fetch(&inbox("INBOX"), None).await.unwrap();
    assert_eq!(first.messages.len(), 2, "one listing page, nothing older yet");
    let st: GmailState = serde_json::from_str(first.new_state.as_deref().unwrap()).unwrap();
    assert_eq!((st.older_page.as_deref(), st.exhausted), (Some("p2"), false));

    // Scrolled to the end: the next page, skipping what is stored.
    let known: std::collections::HashSet<String> = ["new".to_string(), "old1".to_string()].into();
    let older = p.fetch_older(&inbox("INBOX"), first.new_state.as_deref(), &known, 50).await.unwrap();
    assert_eq!(older.messages.iter().map(|m| m.remote_id.as_str()).collect::<Vec<_>>(), ["old2"]);
    assert!(older.exhausted);

    // Nothing left: answered without calling the server.
    let again = p.fetch_older(&inbox("INBOX"), older.new_state.as_deref(), &known, 50).await.unwrap();
    assert!(again.exhausted && again.messages.is_empty());
}
