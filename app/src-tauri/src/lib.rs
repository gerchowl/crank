//! Thin desktop shell and bindings registry (ADR-0001 §4).
//!
//! The process owns one [`crank_core::Core`] (and with it `ride.lock`), registered as
//! Tauri state, and exposes its methods one-to-one as commands ([`commands`]). Streams
//! and link channels belong to the calling window's label; destroying a window drops them.
//! Devices and the data directory follow [`RuntimeConfig`]. Window and quit handling is
//! [`lifecycle`].
pub mod commands;
mod config;
pub mod display;
pub mod lifecycle;
pub mod logging;
pub mod report;

use commands::{
    compile_workout, core_ready, device_forget, device_links, device_pair, devices_scan, links_ack,
    links_intent, links_open, open_stream, parse_dsl, print_dsl, ride_end, ride_load, ride_pause,
    ride_resume, ride_skip, ride_start, ride_status, ride_summary, rides_list, stream_ack,
    stream_snapshot, stream_subscribe, stream_unsubscribe, workout_get, workouts_list,
};
pub use config::RuntimeConfig;
use crank_core::{ApiError, Core, CoreConfig, DeviceMode, EngineHost, ErrorCode};
use crank_devices::sim::SimConfig;
use display::display_width_mm;
use std::path::Path;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// Registry shared by the desktop and export binary to prevent drift.
#[must_use]
pub fn bindings() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::new()
        .commands(tauri_specta::collect_commands![
            core_ready,
            devices_scan,
            device_pair,
            device_forget,
            device_links,
            links_intent,
            links_open::<tauri::Wry>,
            links_ack::<tauri::Wry>,
            workouts_list,
            workout_get,
            parse_dsl,
            print_dsl,
            compile_workout,
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
            display_width_mm::<tauri::Wry>,
        ])
        // The rider tuning and its defaults (ADR-0001 §7.2), so the UI and its test backends
        // take the ADR-0002 §7.4 values from Rust instead of copying them (#589).
        .typ::<crank_engine::Tuning>()
        .constant("TUNING_DEFAULT", crank_engine::Tuning::default())
}

/// The devices `config` asks for: the simulator, or Bluetooth with the capture directory
/// of `CRANK_CAPTURE_DIR`.
#[must_use]
pub fn device_mode(config: &RuntimeConfig) -> DeviceMode {
    if config.sim {
        DeviceMode::Sim(SimConfig::default())
    } else {
        DeviceMode::Ble {
            capture_dir: config.capture_dir.clone(),
        }
    }
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
            devices: device_mode(config),
            clock: None,
            engine_host: EngineHost::Thread,
        })
    })
}

/// What the user is told when the core cannot open: a second crank holding `ride.lock`
/// gets its own sentence, anything else the store's message.
#[must_use]
pub fn startup_failure(e: &ApiError) -> String {
    match e.code {
        ErrorCode::Conflict => "crank is already running.".into(),
        _ => format!("crank could not start: {}", e.message),
    }
}

/// Launch the desktop window.
///
/// If the core cannot open (another crank holds `ride.lock`, the store fails) the user
/// gets a native error dialog and the process exits with status 1: no panic.
/// Quitting ends a ride in progress first ([`lifecycle`]).
///
/// # Errors
/// Returns an error if Tauri cannot initialize or run the app.
pub fn run() -> tauri::Result<()> {
    let bindings = bindings();
    let builder = lifecycle::install(
        tauri::Builder::default()
            .plugin(tauri_plugin_dialog::init())
            .invoke_handler(bindings.invoke_handler())
            .setup(|app| {
                let config = RuntimeConfig::from_env();
                let plan =
                    logging::LogPlan::new(config.capture_dir.as_deref(), app.path().app_log_dir()?);
                // Asked for a capture, the rider must not ride without one unnoticed; the
                // default log is a courtesy and never stops the app. A failed install has
                // shut its pipeline down already (`process::exit` skips vigil's flush).
                match plan.install(std::env::var(logging::FILTER_ENV).ok().as_deref()) {
                    Ok(guard) => app.manage(logging::Logging::new(guard)),
                    Err(e) if plan.capture => {
                        app.dialog()
                            .message(format!("crank could not open its capture log: {e}"))
                            .kind(MessageDialogKind::Error)
                            .title("crank")
                            .blocking_show();
                        std::process::exit(1)
                    }
                    Err(e) => {
                        eprintln!("crank: no log file: {e}");
                        app.manage(logging::Logging::default())
                    }
                };
                let data_dir = match &config.data_dir {
                    Some(dir) => dir.clone(),
                    None => app.path().app_data_dir()?,
                };
                std::fs::create_dir_all(&data_dir)?;
                match open_core(&config, &data_dir) {
                    Ok(core) => {
                        app.manage(core);
                        Ok(())
                    }
                    Err(e) => {
                        tracing::error!(%e, "the core did not open");
                        app.dialog()
                            .message(startup_failure(&e))
                            .kind(MessageDialogKind::Error)
                            .title("crank")
                            .blocking_show();
                        // Nothing else is running that needs a clean-up: write the log, leave now.
                        app.state::<logging::Logging>()
                            .shut_down(lifecycle::QUIT_CAP);
                        std::process::exit(1)
                    }
                }
            }),
    );
    // The plugin opens an automation HTTP endpoint: it exists only in the `e2e` build (ADR-0001 §8.3).
    #[cfg(feature = "e2e")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    let app = builder.build(tauri::generate_context!())?;
    lifecycle::run(app);
    Ok(())
}
