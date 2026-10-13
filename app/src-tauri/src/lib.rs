//! Thin desktop shell and bindings registry (ADR-0001 §4).
#![cfg_attr(test, allow(clippy::unwrap_used))]

#[tauri::command]
#[specta::specta]
fn scaffold_info() -> crank_core::ScaffoldInfo {
    crank_core::ScaffoldInfo {
        app_name: "crank".into(),
    }
}

/// Registry shared by the desktop and export binary to prevent drift.
#[must_use]
pub fn bindings() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::new().commands(tauri_specta::collect_commands![scaffold_info])
}

/// Launch the placeholder desktop window.
///
/// # Errors
/// Returns an error if Tauri cannot initialize or run the app.
pub fn run() -> tauri::Result<()> {
    let bindings = bindings();
    tauri::Builder::default()
        .invoke_handler(bindings.invoke_handler())
        .run(tauri::generate_context!())
}
