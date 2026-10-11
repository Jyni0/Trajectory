//! Trajectory — Tauri core.

pub mod autodetect;
pub mod bimi;
pub mod commands;
pub mod db;
pub mod error;
pub mod mail_engine;
pub mod mime;
pub mod models;
pub mod net;
pub mod oauth;
pub mod providers;
pub mod secrets;
pub mod service;
pub mod site_icon;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use commands::AppState;
use mail_engine::{SyncCommand, SyncEngine};
use service::{AppContext, EventSink};

const KEYRING_SERVICE: &str = "dev.trajectory.mail";
/// Identifier of the app before it was renamed to Trajectory: its keychain
/// entries and data folder are moved over on first start.
const LEGACY_ID: &str = "dev.singularity.mail";

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if event == service::events::MAIL_NEW {
            if let (Some(tray), Ok(unread)) = (
                self.0.tray_by_id("main"),
                self.0.state::<AppState>().ctx.db.unread_total(),
            ) {
                let _ = tray.set_tooltip(Some(format!("Trajectory — {unread} unread")));
            }
            notify_new_mail(&self.0, &payload);
        }
        let _ = self.0.emit(event, payload);
    }
}

/// A system notification for new inbox mail, unless the user is looking at
/// the window (the UI shows a toast then) or turned notifications off.
fn notify_new_mail(app: &AppHandle, payload: &serde_json::Value) {
    if !app.state::<AppState>().notify.load(Ordering::Relaxed) {
        return;
    }
    let in_view = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
    if in_view {
        return;
    }
    let count = payload["count"].as_u64().unwrap_or(0);
    let first = &payload["latest"][0];
    let (title, body) = if count == 1 && first.is_object() {
        let from = first["from"]["name"]
            .as_str()
            .filter(|n| !n.trim().is_empty())
            .or(first["from"]["address"].as_str())
            .unwrap_or("New mail");
        let subject = first["subject"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or("(no subject)");
        (from.to_string(), subject.to_string())
    } else {
        ("New mail".to_string(), format!("{count} new messages"))
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("cannot show notification: {e}");
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open Trajectory", true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &sync, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Trajectory")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "sync" => app.state::<AppState>().sync.request(SyncCommand::All),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// "Quit" in the title-bar menu: really exits (the window's close button
/// only hides it to the tray).
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Moves the mail cache of the pre-rename app into the new data folder (once,
/// only while the new one has none). Moved, not copied: no second plaintext
/// copy of the mail is left behind.
fn adopt_legacy_data(dir: &std::path::Path) {
    let Some(old) = dir.parent().map(|p| p.join(LEGACY_ID)) else { return };
    if dir.join("mail.db").exists() || !old.join("mail.db").exists() {
        return;
    }
    for name in ["mail.db", "mail.db-wal", "mail.db-shm"] {
        let from = old.join(name);
        if from.exists() {
            if let Err(e) = std::fs::rename(&from, dir.join(name)) {
                log::warn!("could not move {}: {e}", from.display());
            }
        }
    }
    let _ = std::fs::remove_dir(&old);
}

/// The data folder holds every cached message: owner-only on Unix (Windows
/// already keeps %APPDATA% per user).
fn restrict_dir(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            restrict_dir(&dir);
            adopt_legacy_data(&dir);
            let db = db::Database::open(dir.join("mail.db"))?;
            let ctx = AppContext::new(
                db,
                Arc::new(secrets::KeyringStore::new(KEYRING_SERVICE).with_legacy(LEGACY_ID)),
                Arc::new(TauriSink(app.handle().clone())),
            );
            let (engine, sync) = SyncEngine::new(ctx.clone(), mail_engine::DEFAULT_INTERVAL);
            app.manage(AppState { ctx, sync, notify: AtomicBool::new(true) });
            tauri::async_runtime::spawn(engine.run());
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the app (and background sync) in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::add_account,
            commands::add_oauth_account,
            commands::remove_account,
            commands::test_connection,
            commands::autodetect_settings,
            commands::list_folders,
            commands::list_emails,
            commands::get_email,
            commands::get_thread,
            commands::load_older,
            commands::oauth_apps,
            commands::set_oauth_app,
            commands::clear_oauth_app,
            commands::sender_logo,
            commands::list_tags,
            commands::create_tag,
            commands::update_tag,
            commands::delete_tag,
            commands::reorder_tags,
            commands::set_email_tag,
            commands::set_note,
            commands::mark_read,
            commands::set_flagged,
            commands::move_email,
            commands::archive_email,
            commands::delete_email,
            commands::send_email,
            commands::save_draft,
            commands::sync_now,
            commands::sync_folder,
            commands::set_notifications,
            commands::save_attachment,
            commands::open_external,
            quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Trajectory")
        .run(|app, event| {
            // macOS: clicking the Dock icon brings back a window closed to the tray.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
