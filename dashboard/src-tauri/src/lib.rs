use std::sync::OnceLock;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Holds the auto-generated API key (only set when CLOTO_API_KEY was absent from .env).
static AUTO_GENERATED_KEY: OnceLock<String> = OnceLock::new();

/// Generate a cryptographically random API key (64 hex chars).
fn generate_api_key() -> String {
    use rand::rngs::OsRng;
    use rand::Rng;
    use std::fmt::Write;
    let bytes: [u8; 32] = OsRng.gen();
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Returns the kernel HTTP port (used by frontend to construct API URLs).
#[tauri::command]
fn get_kernel_port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8081)
}

/// Capture the primary screen and return a base64-encoded PNG.
#[tauri::command]
fn capture_screen() -> Result<String, String> {
    use base64::Engine;
    use xcap::Monitor;

    let monitors = Monitor::all().map_err(|e| format!("Failed to enumerate monitors: {}", e))?;
    let primary = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| Monitor::all().ok().and_then(|m| m.into_iter().next()))
        .ok_or_else(|| "No monitor found".to_string())?;

    let image = primary
        .capture_image()
        .map_err(|e| format!("Screen capture failed: {}", e))?;

    let mut buf = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encoding failed: {}", e))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(buf.into_inner()))
}

/// Select a file within the scripts/ directory. Returns a relative path.
#[tauri::command]
fn select_script_file(base_dir: String) -> Result<Option<String>, String> {
    // This is a synchronous helper; the actual dialog is done via tauri-plugin-dialog on the frontend.
    // This command validates a proposed path against security constraints.
    let path = std::path::Path::new(&base_dir);
    if !path.exists() || !path.is_dir() {
        return Err(format!("Directory does not exist: {}", base_dir));
    }
    Ok(None)
}

/// Read a text file and return its contents.
#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

// ── Language Pack Management ──

/// Resolve the languages directory path, creating it if needed.
fn get_languages_dir_path() -> Result<std::path::PathBuf, String> {
    let home = if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE")
    } else {
        std::env::var("HOME")
    }
    .map_err(|_| "Cannot determine home directory".to_string())?;

    let dir = std::path::PathBuf::from(home)
        .join("Documents")
        .join("ClotoCore")
        .join("languages");

    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Return the path to `Documents/ClotoCore/languages`, creating it if needed.
#[tauri::command]
fn get_languages_dir() -> Result<String, String> {
    get_languages_dir_path().map(|p| p.to_string_lossy().into_owned())
}

/// Scan the languages directory and return all .json files as (filename, content) pairs.
#[tauri::command]
fn scan_languages_dir() -> Result<Vec<(String, String)>, String> {
    let dir = get_languages_dir_path()?;
    let mut results = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                results.push((name, content));
            }
        }
    }
    Ok(results)
}

/// Save a language pack JSON file to the languages directory.
#[tauri::command]
fn save_language_pack(filename: String, content: String) -> Result<(), String> {
    let dir = get_languages_dir_path()?;
    let path = dir.join(format!("{}.json", filename));
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

/// Remove a language pack file from the languages directory.
#[tauri::command]
fn remove_language_pack(filename: String) -> Result<(), String> {
    let dir = get_languages_dir_path()?;
    let path = dir.join(format!("{}.json", filename));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Install or update bundled default language packs.
///
/// Uses a snapshot file (`.ja.bundled`) to detect user edits:
/// - If `ja.json` matches the snapshot → user hasn't edited → safe to overwrite
/// - If `ja.json` differs from the snapshot → user customized → skip update
/// - If `ja.json` or snapshot doesn't exist → fresh install → write both
///
/// Returns the number of packs installed or updated.
const DEFAULT_JA_PACK: &str = include_str!("../resources/ja.json");

#[tauri::command]
fn install_default_packs() -> Result<u32, String> {
    let dir = get_languages_dir_path()?;
    let mut installed = 0u32;

    let ja_path = dir.join("ja.json");
    let snapshot_path = dir.join(".ja.bundled");

    let needs_write = if ja_path.exists() {
        let existing = std::fs::read_to_string(&ja_path).unwrap_or_default();
        let snapshot = std::fs::read_to_string(&snapshot_path).unwrap_or_default();

        if existing.trim() == DEFAULT_JA_PACK.trim() {
            // Already up to date
            false
        } else if !snapshot_path.exists() || existing.trim() == snapshot.trim() {
            // No snapshot (pre-update install) or file matches snapshot → user hasn't edited
            true
        } else {
            // File differs from snapshot → user has customized, skip
            false
        }
    } else {
        true
    };

    if needs_write {
        std::fs::write(&ja_path, DEFAULT_JA_PACK).map_err(|e| e.to_string())?;
        std::fs::write(&snapshot_path, DEFAULT_JA_PACK).map_err(|e| e.to_string())?;
        installed += 1;
    }

    Ok(installed)
}

/// Returns the auto-generated API key, or None if the user configured their own in .env.
#[tauri::command]
fn get_auto_api_key() -> Option<String> {
    AUTO_GENERATED_KEY.get().cloned()
}

#[allow(clippy::too_many_lines)]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Tauri desktop mode: bind kernel to loopback only for security
    std::env::set_var("BIND_ADDRESS", "127.0.0.1");

    // Add Tauri WebView origins to CORS allowlist
    let existing_cors = std::env::var("CORS_ORIGINS").unwrap_or_default();
    let tauri_origins = "tauri://localhost,http://tauri.localhost";
    let combined = if existing_cors.is_empty() {
        format!(
            "http://localhost:1420,http://127.0.0.1:1420,{}",
            tauri_origins
        )
    } else {
        format!("{},{}", existing_cors, tauri_origins)
    };
    std::env::set_var("CORS_ORIGINS", combined);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_kernel_port,
            capture_screen,
            select_script_file,
            get_auto_api_key,
            read_text_file,
            get_languages_dir,
            scan_languages_dir,
            save_language_pack,
            remove_language_pack,
            install_default_packs
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // --- System Tray ---
            let status_item =
                MenuItem::with_id(app, "status", "Cloto: Online", false, None::<&str>)?;
            let show_item = MenuItem::with_id(app, "show", "Show Dashboard", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Cloto", true, None::<&str>)?;

            let tray_menu = Menu::with_items(
                app,
                &[
                    &status_item,
                    &PredefinedMenuItem::separator(app)?,
                    &show_item,
                    &PredefinedMenuItem::separator(app)?,
                    &quit_item,
                ],
            )?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Cloto System")
                .menu(&tray_menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // --- Global Shortcut: CmdOrCtrl+Shift+E to toggle dashboard ---
            app.global_shortcut()
                .on_shortcut(
                    "CmdOrCtrl+Shift+E",
                    |app_handle: &tauri::AppHandle,
                     _shortcut: &tauri_plugin_global_shortcut::Shortcut,
                     event: tauri_plugin_global_shortcut::ShortcutEvent| {
                        if event.state == ShortcutState::Pressed {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                if window.is_visible().unwrap_or(false) {
                                    let _ = window.hide();
                                } else {
                                    let _ = window.show();
                                    let _ = window.set_focus();
                                }
                            }
                        }
                    },
                )
                .ok();

            // --- Launch the Cloto Kernel Server ---
            // Load .env before spawn so we can inspect CLOTO_API_KEY synchronously.
            dotenvy::dotenv().ok();

            // Auto-generate API key if not configured in .env
            if std::env::var("CLOTO_API_KEY").is_err() {
                let key = generate_api_key();
                std::env::set_var("CLOTO_API_KEY", &key);
                let _ = AUTO_GENERATED_KEY.set(key);
            }

            tauri::async_runtime::spawn(async move {
                if let Err(e) = cloto_core::run_kernel().await {
                    eprintln!("Failed to start Cloto Kernel: {}", e);
                }
            });

            Ok(())
        })
        // Intercept window close: minimize to tray instead of quitting
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Run with cleanup on exit
    app.run(|_app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            // Clean up stale maintenance file if present
            let maint = cloto_core::config::exe_dir().join(".maintenance");
            let _ = std::fs::remove_file(maint);
        }
    });
}
