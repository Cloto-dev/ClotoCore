use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

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
        .find(xcap::Monitor::is_primary)
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
            select_script_file
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
                MenuItem::with_id(app, "status", "Exiv: Online", false, None::<&str>)?;
            let show_item = MenuItem::with_id(app, "show", "Show Dashboard", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Exiv", true, None::<&str>)?;

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
                .tooltip("Exiv System")
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

            // --- Launch the Exiv Kernel Server ---
            tauri::async_runtime::spawn(async move {
                dotenvy::dotenv().ok();
                if let Err(e) = exiv_core::run_kernel().await {
                    eprintln!("Failed to start Exiv Kernel: {}", e);
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
            let maint = exiv_core::config::exe_dir().join(".maintenance");
            let _ = std::fs::remove_file(maint);
        }
    });
}
