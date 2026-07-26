// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The uninstall handoff (DEFENDER_DESIGN.md §7) copies *this* binary to a
    // temp directory and re-launches it as `purge-exec`, because this is the
    // binary the kernel lives in on a desktop install. Without this hook the
    // copy would start a second GUI instead of executing the plan. Returns
    // `None` on an ordinary launch, so the app starts exactly as before.
    if let Some(result) = cloto_core::cli::run_detached_helper_if_requested() {
        if let Err(e) = result {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
        return;
    }
    app_lib::run();
}
