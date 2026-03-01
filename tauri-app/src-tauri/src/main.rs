// Show console window on Windows for debugging
// #![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Init tracing subscriber for console logs
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    ai_manager_tauri::run();
}
