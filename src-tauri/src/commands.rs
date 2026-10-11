//! Tauri IPC commands — thin wrappers over `AppContext`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::autodetect;
use crate::error::{AppError, AppResult};
use crate::mail_engine::{SyncCommand, SyncHandle};
use crate::models::*;
use crate::service::AppContext;

pub struct AppState {
    pub ctx: Arc<AppContext>,
    pub sync: SyncHandle,
    /// The "Desktop notifications" preference, mirrored from the UI.
    pub notify: AtomicBool,
}

type Cmd<'a> = State<'a, AppState>;

#[tauri::command]
pub fn list_accounts(state: Cmd<'_>) -> AppResult<Vec<Account>> {
    state.ctx.list_accounts()
}

#[tauri::command]
pub async fn add_account(state: Cmd<'_>, account: NewAccount, verify: Option<bool>) -> AppResult<Account> {
    let created = state.ctx.add_account(account, verify.unwrap_or(true)).await?;
    state.sync.request(SyncCommand::Account(created.id.clone()));
    Ok(created)
}

/// Google / Microsoft sign-in apps; secrets are reported only masked.
#[tauri::command]
pub fn oauth_apps(state: Cmd<'_>) -> AppResult<Vec<crate::oauth::OAuthAppInfo>> {
    state.ctx.oauth_apps()
}

#[tauri::command]
pub fn set_oauth_app(
    state: Cmd<'_>,
    provider: String,
    client_id: String,
    client_secret: Option<String>,
) -> AppResult<crate::oauth::OAuthAppInfo> {
    state.ctx.set_oauth_app(&provider, &client_id, client_secret.as_deref())
}

#[tauri::command]
pub fn clear_oauth_app(state: Cmd<'_>, provider: String) -> AppResult<crate::oauth::OAuthAppInfo> {
    state.ctx.clear_oauth_app(&provider)
}

#[tauri::command]
pub async fn add_oauth_account(app: AppHandle, state: Cmd<'_>, kind: AccountKind) -> AppResult<Account> {
    let opener = app.clone();
    let created = state
        .ctx
        .add_oauth_account(kind, move |url| {
            opener
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))
        })
        .await?;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_focus();
    }
    state.sync.request(SyncCommand::Account(created.id.clone()));
    Ok(created)
}

#[tauri::command]
pub fn remove_account(state: Cmd<'_>, id: String) -> AppResult<()> {
    state.ctx.remove_account(&id)
}

#[tauri::command]
pub async fn test_connection(state: Cmd<'_>, account: NewAccount) -> AppResult<ConnectionReport> {
    state.ctx.test_connection(&account).await
}

#[tauri::command]
pub fn autodetect_settings(email: String) -> AppResult<DetectedSettings> {
    autodetect::detect(&email)
}

#[tauri::command]
pub fn list_folders(state: Cmd<'_>, account_id: String) -> AppResult<Vec<Folder>> {
    state.ctx.list_folders(&account_id)
}

#[tauri::command]
pub fn list_emails(state: Cmd<'_>, query: EmailQuery) -> AppResult<EmailPage> {
    state.ctx.list_emails(&query)
}

/// Older messages of a folder, fetched when its list is scrolled to the end.
#[tauri::command]
pub async fn load_older(state: Cmd<'_>, folder_id: String) -> AppResult<OlderPage> {
    state.ctx.load_older(&folder_id).await
}

/// Every message of the conversation `id` belongs to, oldest first.
#[tauri::command]
pub fn get_thread(state: Cmd<'_>, id: String) -> AppResult<Vec<EmailSummary>> {
    state.ctx.thread(&id)
}

#[tauri::command]
pub fn get_email(state: Cmd<'_>, id: String) -> AppResult<Email> {
    state.ctx.get_email(&id)
}

#[tauri::command]
pub async fn sender_logo(state: Cmd<'_>, domain: String) -> AppResult<SenderLogo> {
    state.ctx.sender_logo(&domain).await
}

#[tauri::command]
pub fn list_tags(state: Cmd<'_>, account_id: Option<String>) -> AppResult<Vec<Tag>> {
    state.ctx.list_tags(account_id.as_deref())
}

#[tauri::command]
pub fn create_tag(state: Cmd<'_>, name: String, color: String) -> AppResult<Tag> {
    state.ctx.create_tag(&name, &color)
}

#[tauri::command]
pub fn update_tag(state: Cmd<'_>, id: String, name: String, color: String) -> AppResult<Tag> {
    state.ctx.update_tag(&id, &name, &color)
}

#[tauri::command]
pub fn delete_tag(state: Cmd<'_>, id: String) -> AppResult<()> {
    state.ctx.delete_tag(&id)
}

#[tauri::command]
pub fn reorder_tags(state: Cmd<'_>, ids: Vec<String>) -> AppResult<()> {
    state.ctx.reorder_tags(&ids)
}

#[tauri::command]
pub fn set_email_tag(state: Cmd<'_>, email_id: String, tag_id: String, on: bool) -> AppResult<()> {
    state.ctx.set_email_tag(&email_id, &tag_id, on)
}

#[tauri::command]
pub fn set_note(state: Cmd<'_>, email_id: String, note: Option<String>) -> AppResult<()> {
    state.ctx.set_note(&email_id, note.as_deref())
}

#[tauri::command]
pub async fn mark_read(state: Cmd<'_>, id: String, read: bool) -> AppResult<()> {
    state.ctx.mark_read(&id, read)
}

#[tauri::command]
pub async fn set_flagged(state: Cmd<'_>, id: String, flagged: bool) -> AppResult<()> {
    state.ctx.set_flagged(&id, flagged)
}

#[tauri::command]
pub async fn move_email(state: Cmd<'_>, id: String, folder_id: String) -> AppResult<()> {
    state.ctx.move_email(&id, &folder_id)
}

#[tauri::command]
pub async fn archive_email(state: Cmd<'_>, id: String) -> AppResult<()> {
    state.ctx.archive_email(&id)
}

#[tauri::command]
pub async fn delete_email(state: Cmd<'_>, id: String) -> AppResult<()> {
    state.ctx.delete_email(&id)
}

#[tauri::command]
pub async fn send_email(state: Cmd<'_>, email: OutgoingEmail) -> AppResult<String> {
    state.ctx.send_email(email).await
}

#[tauri::command]
pub fn save_draft(state: Cmd<'_>, email: OutgoingEmail) -> AppResult<String> {
    state.ctx.save_draft(email)
}

#[tauri::command]
pub fn sync_now(state: Cmd<'_>, account_id: Option<String>) {
    state.sync.request(account_id.map(SyncCommand::Account).unwrap_or(SyncCommand::All));
}

/// Fetches new mail of one folder (it was just opened).
#[tauri::command]
pub fn sync_folder(state: Cmd<'_>, folder_id: String) {
    state.sync.request(SyncCommand::Folder(folder_id));
}

#[tauri::command]
pub fn set_notifications(state: Cmd<'_>, enabled: bool) {
    state.notify.store(enabled, Ordering::Relaxed);
}

/// Saves an attachment into the Downloads folder and reveals it.
#[tauri::command]
pub fn save_attachment(app: AppHandle, state: Cmd<'_>, id: String) -> AppResult<String> {
    let dir = app
        .path()
        .download_dir()
        .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
    let path = state.ctx.save_attachment(&id, &dir)?;
    let _ = app.opener().reveal_item_in_dir(&path);
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> AppResult<()> {
    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:")) {
        return Err(AppError::Invalid("unsupported link".into()));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))
}
