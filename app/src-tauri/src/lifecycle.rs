//! Window and process lifecycle (ADR-0001 §4.3, ADR-0002 §7.9).
//!
//! * A destroyed window and a page (re)load drop the window's streams. A reload keeps the
//!   window and its webview, so `Channel::send` keeps succeeding into the dead page:
//!   nothing else would end the old stream.
//! * Quitting is a ride end. On macOS exiting never drops [`Core`], so the engine's "every
//!   handle gone" end path would not run and ERG could stay applied (S1). The first exit
//!   request is held back, the ride is ended ([`Core::shutdown`], bounded by
//!   [`QUIT_CAP`]) and only then does the app exit.
use crank_core::{Core, Shutdown};
use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Manager, RunEvent, Runtime, WindowEvent, webview::PageLoadEvent};

/// Longest a quit waits for the devices to be released. Above the engine's own 2 s end
/// deadline (ADR-0002 §7.9), so a trainer that never answers is still given up on.
pub const QUIT_CAP: Duration = Duration::from_secs(3);

/// Register the window and page-load handlers on `builder`.
#[must_use]
pub fn install<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .on_page_load(|webview, payload| {
            page_load(
                webview.app_handle(),
                webview.window().label(),
                payload.event(),
            );
        })
        .on_window_event(window_event)
}

/// A window event: a destroyed window drops its streams.
pub fn window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    if matches!(event, WindowEvent::Destroyed) {
        drop_streams(window.app_handle(), window.label());
    }
}

/// A page of `window` started loading (the first load, or a reload): streams opened by
/// the page it replaces are dropped.
pub fn page_load<R: Runtime>(app: &AppHandle<R>, window: &str, event: PageLoadEvent) {
    if event == PageLoadEvent::Started {
        drop_streams(app, window);
    }
}

/// Drop the streams of `window` on the managed core, if there is one.
fn drop_streams<R: Runtime>(app: &AppHandle<R>, window: &str) {
    if let Some(core) = app.try_state::<Core>() {
        let (core, window) = (core.inner().clone(), window.to_owned());
        tauri::async_runtime::spawn(async move { core.close_window(window).await });
    }
}

/// Where the quit stands.
#[derive(Default)]
pub struct Quit(AtomicU8);

/// What to do with an exit request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Hold the exit back and start the shutdown.
    Begin,
    /// A shutdown is under way: hold the exit back.
    Wait,
    /// The ride is ended: let the app exit.
    Proceed,
}

impl Quit {
    const IDLE: u8 = 0;
    const SHUTTING_DOWN: u8 = 1;
    const DONE: u8 = 2;

    /// Decide on an exit request.
    pub fn request(&self) -> Decision {
        match self.0.compare_exchange(
            Self::IDLE,
            Self::SHUTTING_DOWN,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => Decision::Begin,
            Err(Self::SHUTTING_DOWN) => Decision::Wait,
            Err(_) => Decision::Proceed,
        }
    }

    /// End the ride and release the devices, then allow the exit.
    pub async fn shut_down(&self, core: &Core) -> Shutdown {
        let outcome = core.shutdown(QUIT_CAP).await;
        match &outcome {
            Shutdown::Idle | Shutdown::Released => {}
            Shutdown::TimedOut => tracing::warn!("quit: devices not released within the cap"),
            Shutdown::Failed(e) => tracing::warn!(%e, "quit: ending the ride failed"),
        }
        self.0.store(Self::DONE, Ordering::SeqCst);
        outcome
    }
}

/// The `App::run` callback: hold back the first exit request until the ride is ended.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent, quit: &std::sync::Arc<Quit>) {
    let RunEvent::ExitRequested {
        code: requested,
        api,
        ..
    } = event
    else {
        return;
    };
    // Without a core (startup failed) there is no ride to end.
    let Some(core) = app.try_state::<Core>() else {
        return;
    };
    match quit.request() {
        Decision::Proceed => {}
        Decision::Wait => api.prevent_exit(),
        Decision::Begin => {
            api.prevent_exit();
            let (app, core, quit) = (app.clone(), core.inner().clone(), quit.clone());
            let status = requested.unwrap_or(0);
            tauri::async_runtime::spawn(async move {
                quit.shut_down(&core).await;
                app.exit(status);
            });
        }
    }
}
