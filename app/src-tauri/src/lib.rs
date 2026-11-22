//! Thin desktop shell and bindings registry (ADR-0001 §4).
//!
//! The process owns one [`crank_core::Core`] (and with it `ride.lock`), registered as
//! Tauri state, and exposes its methods one-to-one as commands ([`commands`]). Streams
//! and link channels belong to the calling window's label; destroying a window drops them.
//! Devices and the data directory follow [`RuntimeConfig`]. Window and quit handling is
//! [`lifecycle`]; the Mac's sleep releases the links ([`sleep`]).
mod build_rev;
pub mod commands;
mod config;
pub mod display;
pub mod lifecycle;
pub mod logging;
pub mod report;
pub mod sleep;
pub mod version;

use commands::{
    compile_workout, core_ready, device_forget, device_links, device_pair, devices_scan, links_ack,
    links_intent, links_open, open_stream, parse_dsl, print_dsl, ride_end, ride_load, ride_pause,
    ride_resume, ride_skip, ride_start, ride_status, ride_summary, rides_list, stream_ack,
    stream_snapshot, stream_subscribe, stream_unsubscribe, ui_ack, ui_open, workout_get,
    workouts_list,
};
pub use config::RuntimeConfig;
use crank_core::{ApiError, Core, CoreConfig, DeviceMode, EngineHost, ErrorCode};
use crank_devices::sim::SimConfig;
use display::display_width_mm;
use std::path::Path;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// The command list of [`bindings`], then the build's test-only commands. One list, so the
/// production and the `e2e` registries cannot drift apart.
macro_rules! registry {
    ($($extra:tt)*) => {
        tauri_specta::collect_commands![
            core_ready,
            devices_scan,
            device_pair,
            device_forget,
            device_links,
            links_intent,
            links_open::<tauri::Wry>,
            links_ack::<tauri::Wry>,
            ui_open::<tauri::Wry>,
            ui_ack::<tauri::Wry>,
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
            version::app_build,
            $($extra)*
        ]
    };
}

/// Registry shared by the desktop and export binary to prevent drift.
///
/// The `e2e` build adds the test hooks `sim_fault` (`commands::sim_fault`, #668) and
/// `sim_latency` (`commands::sim_latency`, #681), so a
/// production build has no such command and the exported bindings (built without `e2e`)
/// never name it. The invoke handler and the exported function list come from the same
/// `collect_commands!`, so `tests/commands.rs` `only_the_e2e_build_registers_sim_fault`
/// checks the export of this registry in CI; the `deps-edges` flake check proves that
/// production never enables `e2e`.
#[must_use]
pub fn bindings() -> tauri_specta::Builder<tauri::Wry> {
    #[cfg(not(feature = "e2e"))]
    let commands = registry!();
    #[cfg(feature = "e2e")]
    let commands = registry!(commands::sim_fault, commands::sim_latency);
    tauri_specta::Builder::new()
        .commands(commands)
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
                // The build's string in the title (#552, DESIGN §5.9); tauri.conf.json names the one window.
                app.get_webview_window("main")
                    .ok_or("the main window is missing")?
                    .set_title(&version::window_title(version::BUILD_LABEL))?;
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
    // The setup above has run, so the core is managed; the observer lives until the quit starts.
    #[cfg(target_os = "macos")]
    {
        let handle = app.handle().clone();
        let observer = sleep::Observer::new(move || sleep::on_will_sleep(&handle));
        lifecycle::run(app, observer);
    }
    #[cfg(not(target_os = "macos"))]
    lifecycle::run(app, ());
    Ok(())
}
