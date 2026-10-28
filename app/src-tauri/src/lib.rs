//! Thin desktop shell and bindings registry (ADR-0001 §4).
//!
//! The process owns one [`crank_core::Core`] (and with it `ride.lock`), registered as
//! Tauri state, and exposes its methods one-to-one as commands ([`commands`]). Streams
//! belong to the calling window's label; destroying a window drops them. Devices and
//! the data directory follow [`RuntimeConfig`].
pub mod commands;
mod config;

use commands::{
    core_ready, device_forget, device_links, device_pair, devices_scan, open_stream, ride_end,
    ride_load, ride_pause, ride_resume, ride_skip, ride_start, ride_status, ride_summary,
    rides_list, stream_ack, stream_snapshot, stream_subscribe, stream_unsubscribe, workout_get,
    workouts_list,
};
pub use config::RuntimeConfig;
use crank_core::{ApiError, Core, CoreConfig, DeviceMode};
use crank_devices::sim::SimConfig;
use std::path::Path;
use tauri::Manager;

/// Registry shared by the desktop and export binary to prevent drift.
#[must_use]
pub fn bindings() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::new().commands(tauri_specta::collect_commands![
        core_ready,
        devices_scan,
        device_pair,
        device_forget,
        device_links,
        workouts_list,
        workout_get,
        ride_load,
        ride_start,
        ride_pause,
        ride_resume,
        ride_skip,
        ride_end,
        ride_status,
        open_stream::<tauri::Wry>,
        stream_subscribe::<tauri::Wry>,
        stream_unsubscribe::<tauri::Wry>,
        stream_ack::<tauri::Wry>,
        stream_snapshot::<tauri::Wry>,
        rides_list,
        ride_summary,
    ])
}

/// Open the core in `data_dir` for `config`. Runs inside the async runtime, which the
/// device layer spawns its tasks on.
///
/// # Errors
/// `Conflict` when another process holds `ride.lock`; `Storage` for any store failure.
pub fn open_core(config: &RuntimeConfig, data_dir: &Path) -> Result<Core, ApiError> {
    tauri::async_runtime::block_on(async {
        Core::open(CoreConfig {
            data_dir: data_dir.into(),
            devices: if config.sim {
                DeviceMode::Sim(SimConfig::default())
            } else {
                DeviceMode::Ble
            },
            clock: None,
        })
    })
}

/// Launch the desktop window.
///
/// # Errors
/// Returns an error if Tauri cannot initialize or run the app, or the core cannot open
/// (for instance because another crank process holds `ride.lock`).
pub fn run() -> tauri::Result<()> {
    let bindings = bindings();
    let builder = tauri::Builder::default()
        .invoke_handler(bindings.invoke_handler())
        .setup(|app| {
            let config = RuntimeConfig::from_env();
            let data_dir = match &config.data_dir {
                Some(dir) => dir.clone(),
                None => app.path().app_data_dir()?,
            };
            std::fs::create_dir_all(&data_dir)?;
            app.manage(open_core(&config, &data_dir)?);
            Ok(())
        })
        // A destroyed window drops its streams (a reload keeps the window: its old
        // channel fails on the next frame and `open_stream` replaces the streams).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let core = window.state::<Core>().inner().clone();
                let label = window.label().to_owned();
                tauri::async_runtime::spawn(async move { core.close_window(label).await });
            }
        });
    // The plugin opens an automation HTTP endpoint: it exists only in the `e2e` build (ADR-0001 §8.3).
    #[cfg(feature = "e2e")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    builder.run(tauri::generate_context!())
}
