//! Window and process lifecycle (ADR-0001 §4.3, ADR-0002 §7.9).
//!
//! * A destroyed window and a page (re)load drop the window's streams. A reload keeps the
//!   window and its webview, so `Channel::send` keeps succeeding into the dead page:
//!   nothing else would end the old stream.
//! * Quitting is a ride end. On macOS exiting never drops [`Core`], so the engine's "every
//!   handle gone" end path would not run and ERG could stay applied (S1). Two paths end it
//!   ([`Core::shutdown`], bounded by [`QUIT_CAP`]):
//!   1. **Exit requests** (last window closed, `app.exit`, the Quit menu item, which is
//!      replaced by one that calls `app.exit(0)`): the first `ExitRequested` is held back,
//!      the ride is ended, and only then does the app exit.
//!   2. **The backstop**: `NSApp terminate:` (Dock Quit, logout, system shutdown) reaches
//!      tao's `applicationWillTerminate:` and produces only [`RunEvent::Exit`], with no
//!      `ExitRequested` to hold back. If the shutdown has not run, `Exit` runs it
//!      synchronously. macOS may kill the app a few seconds after `terminate:` at logout, so
//!      [`QUIT_CAP`] must stay at or below 3 s.
use crank_core::{Core, Shutdown};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tauri::{
    AppHandle, Manager, RunEvent, Runtime, Webview, Window, WindowEvent,
    menu::{Menu, MenuEvent, MenuId},
    webview::{PageLoadEvent, PageLoadPayload},
};

/// Longest a quit waits for the engine to end the ride (the end mode delivered). Above the engine's own 2 s end
/// deadline (ADR-0002 §7.9), so a trainer that never answers is still given up on, and
/// at most the ~3 s macOS grants a terminating app at logout (see the module docs).
pub const QUIT_CAP: Duration = Duration::from_secs(3);

/// Id of the menu item that replaces the default Quit item.
pub const QUIT_ITEM_ID: &str = "crank-quit";

/// The builder registrations [`install`] makes: [`tauri::Builder`]'s own methods, as a
/// trait so a test can see which handlers were registered.
pub trait Wire<R: Runtime>: Sized {
    /// [`tauri::Builder::on_page_load`].
    #[must_use]
    fn on_page_load<F: Fn(&Webview<R>, &PageLoadPayload<'_>) + Send + Sync + 'static>(
        self,
        f: F,
    ) -> Self;
    /// [`tauri::Builder::on_window_event`].
    #[must_use]
    fn on_window_event<F: Fn(&Window<R>, &WindowEvent) + Send + Sync + 'static>(self, f: F)
    -> Self;
    /// [`tauri::Builder::menu`].
    #[must_use]
    fn menu<F: FnOnce(&AppHandle<R>) -> tauri::Result<Menu<R>> + Send + 'static>(
        self,
        f: F,
    ) -> Self;
    /// [`tauri::Builder::on_menu_event`].
    #[must_use]
    fn on_menu_event<F: Fn(&AppHandle<R>, MenuEvent) + Send + Sync + 'static>(self, f: F) -> Self;
}

impl<R: Runtime> Wire<R> for tauri::Builder<R> {
    fn on_page_load<F: Fn(&Webview<R>, &PageLoadPayload<'_>) + Send + Sync + 'static>(
        self,
        f: F,
    ) -> Self {
        Self::on_page_load(self, f)
    }
    fn on_window_event<F: Fn(&Window<R>, &WindowEvent) + Send + Sync + 'static>(
        self,
        f: F,
    ) -> Self {
        Self::on_window_event(self, f)
    }
    fn menu<F: FnOnce(&AppHandle<R>) -> tauri::Result<Menu<R>> + Send + 'static>(
        self,
        f: F,
    ) -> Self {
        Self::menu(self, f)
    }
    fn on_menu_event<F: Fn(&AppHandle<R>, MenuEvent) + Send + Sync + 'static>(self, f: F) -> Self {
        Self::on_menu_event(self, f)
    }
}

/// Register the window, page-load and menu handlers on `builder`. The run-event handler
/// is registered by [`run`].
#[must_use]
pub fn install<R: Runtime, W: Wire<R>>(builder: W) -> W {
    install_with(builder, AppHandle::exit)
}

/// [`install`] with the exit action of the Quit menu item given, so a test can see the
/// handler it registers start the exit (`AppHandle::exit` is unimplemented in the mock
/// runtime). [`install`] passes `app.exit`.
#[must_use]
pub fn install_with<R: Runtime, W: Wire<R>>(
    builder: W,
    exit: impl Fn(&AppHandle<R>, i32) + Send + Sync + 'static,
) -> W {
    builder
        .on_page_load(|webview, payload| {
            page_load(
                webview.app_handle(),
                webview.window().label(),
                payload.event(),
            );
        })
        .on_window_event(window_event)
        .menu(menu)
        .on_menu_event(move |app, event| menu_event(event.id(), |code| exit(app, code)))
}

/// The default menu, with its Quit item (Cmd+Q) replaced by [`QUIT_ITEM_ID`].
///
/// The default item sends `NSApp terminate:`, which exits without an `ExitRequested` the
/// shutdown could hold back; the replacement goes through `app.exit(0)` ([`menu_event`]).
/// Only macOS is replaced: there the Quit item is the last of the first (application)
/// submenu. Other platforms keep the default menu.
///
/// # Errors
/// Any menu error from Tauri.
pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    #[cfg(target_os = "macos")]
    if let Some(tauri::menu::MenuItemKind::Submenu(application)) = menu.items()?.into_iter().next()
        && let Some(quit) = application.items()?.len().checked_sub(1)
    {
        use tauri::menu::{MenuItem, MenuItemKind};
        if let Some(MenuItemKind::Predefined(old)) = application.remove_at(quit)? {
            let title = old.text()?;
            application.append(&MenuItem::with_id(
                app,
                QUIT_ITEM_ID,
                title,
                true,
                Some("CmdOrCtrl+Q"),
            )?)?;
        }
    }
    Ok(menu)
}

/// A menu item was chosen: the Quit item exits through `exit`.
pub fn menu_event(id: &MenuId, exit: impl FnOnce(i32)) {
    if id.as_ref() == QUIT_ITEM_ID {
        exit(0);
    }
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

    /// Whether the ride has not been ended yet: an `Exit` that arrives now must end it.
    pub fn pending(&self) -> bool {
        self.0.load(Ordering::SeqCst) != Self::DONE
    }

    /// End the ride (the engine's end mode), then allow the exit.
    pub async fn shut_down(&self, core: &Core) -> Shutdown {
        let outcome = core.shutdown(QUIT_CAP).await;
        match &outcome {
            Shutdown::Idle | Shutdown::Ended => {}
            Shutdown::TimedOut => tracing::warn!("quit: the ride was not ended within the cap"),
            Shutdown::Failed(e) => tracing::warn!(%e, "quit: ending the ride failed"),
        }
        self.0.store(Self::DONE, Ordering::SeqCst);
        outcome
    }
}

/// An exit was requested (`ExitRequested`): hold the first one back, end the ride, then
/// call `exit(status)`. `prevent` stops the exit being requested.
pub fn on_exit_requested(
    quit: &Arc<Quit>,
    core: &Core,
    status: Option<i32>,
    prevent: impl FnOnce(),
    exit: impl FnOnce(i32) + Send + 'static,
) {
    match quit.request() {
        Decision::Proceed => {}
        Decision::Wait => prevent(),
        Decision::Begin => {
            prevent();
            let (core, quit) = (core.clone(), quit.clone());
            tauri::async_runtime::spawn(async move {
                quit.shut_down(&core).await;
                exit(status.unwrap_or(0));
            });
        }
    }
}

/// The app is exiting (`Exit`): the backstop for exits that never asked (Dock Quit, logout,
/// system shutdown). Unless the ride was ended already, end it here, synchronously and
/// bounded by [`QUIT_CAP`]: the process is going away as soon as this returns.
pub fn on_exit(quit: &Quit, core: &Core) {
    if quit.pending() {
        tauri::async_runtime::block_on(quit.shut_down(core));
    }
}

/// What [`handle_exit_requested`] needs of the `ExitRequested` event's api: Tauri's
/// [`tauri::ExitRequestApi`], as a trait because only the event loop can make one.
pub trait ExitApi {
    /// [`tauri::ExitRequestApi::prevent_exit`].
    fn prevent_exit(&self);
}

impl ExitApi for tauri::ExitRequestApi {
    fn prevent_exit(&self) {
        Self::prevent_exit(self);
    }
}

/// `ExitRequested` on the managed core: [`on_exit_requested`] with `api` to hold the exit back.
pub fn handle_exit_requested(
    quit: &Arc<Quit>,
    core: &Core,
    status: Option<i32>,
    api: &impl ExitApi,
    exit: impl FnOnce(i32) + Send + 'static,
) {
    on_exit_requested(quit, core, status, || api.prevent_exit(), exit);
}

/// The `App::run` callback: [`on_exit_requested`] and [`on_exit`] on the managed core.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent, quit: &Arc<Quit>) {
    // Without a core (startup failed) there is no ride to end.
    let Some(core) = app.try_state::<Core>() else {
        return;
    };
    match event {
        RunEvent::ExitRequested { code, api, .. } => {
            let app = app.clone();
            handle_exit_requested(quit, core.inner(), *code, api, move |code| app.exit(code));
        }
        RunEvent::Exit => on_exit(quit, core.inner()),
        _ => {}
    }
}

/// What [`run`] needs of the app: [`tauri::App::run`], as a trait so a test can deliver
/// events.
pub trait Runner<R: Runtime> {
    /// [`tauri::App::run`].
    fn run<F: FnMut(&AppHandle<R>, RunEvent) + 'static>(self, callback: F);
}

impl<R: Runtime> Runner<R> for tauri::App<R> {
    fn run<F: FnMut(&AppHandle<R>, RunEvent) + 'static>(self, callback: F) {
        Self::run(self, callback);
    }
}

/// Run `app` with the quit handling of this module registered.
pub fn run<R: Runtime>(app: impl Runner<R>) {
    let quit = Arc::new(Quit::default());
    app.run(move |handle, event| on_run_event(handle, &event, &quit));
}
