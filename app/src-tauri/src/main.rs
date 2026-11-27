// Prevent additional console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `claude-usage-app usage|recommend|api|help` talks to the running
    // widget and exits before Tauri (and its single-instance hand-off) starts.
    if let Some(code) = claude_usage_app_lib::cli::run_from_args() {
        std::process::exit(code);
    }
    claude_usage_app_lib::run()
}
