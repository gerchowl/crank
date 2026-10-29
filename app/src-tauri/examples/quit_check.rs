//! macOS quit check: a real Wry app is terminated with `NSApp terminate:` (what Dock Quit,
//! logout and system shutdown do) in the middle of a sim ride, and the ride must be
//! recorded as `Finished`, not `Aborted`. Run it with `just quit-test`.
//!
//! The process runs itself twice: the parent starts a child (`CRANK_QUIT_CHECK_DIR` set) that
//! rides and gets terminated, then reopens the child's store. It needs a window server (a
//! GUI session), so it cannot run in the sandboxed `nix flake check`.
//!
//! The last trainer mode (`Resistance{0}`, ADR-0002 §7.9) cannot be observed out of process:
//! the simulator keeps no command log. The `crank-core` tests with a recording radio assert
//! it for the same `Core::shutdown` this check proves is reached.

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("quit_check: macOS only, skipped");
}

#[cfg(target_os = "macos")]
fn main() {
    match std::env::var_os("CRANK_QUIT_CHECK_DIR") {
        Some(dir) => macos::child(dir.as_ref()),
        None => macos::parent(),
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use crank_core::{Core, CoreConfig, DeviceMode, DeviceRole, RideOutcome};
    use crank_desktop::lifecycle::{self, Wire};
    use crank_devices::sim::SimConfig;
    use std::{
        ffi::{c_char, c_void},
        path::Path,
        process::Command,
        time::{Duration, Instant},
    };
    use tauri::{
        AppHandle, Manager, Webview, Window, WindowEvent, Wry,
        menu::{Menu, MenuEvent, MenuItemKind},
        test::{mock_context, noop_assets},
        webview::PageLoadPayload,
    };

    /// How long the child rides before it is terminated.
    const RIDE_FOR: Duration = Duration::from_secs(4);
    /// How long the parent waits for the child to go away.
    const CHILD_LIMIT: Duration = Duration::from_secs(30);

    fn core(dir: &Path) -> Core {
        tauri::async_runtime::block_on(async {
            Core::open(CoreConfig {
                data_dir: dir.into(),
                devices: DeviceMode::Sim(SimConfig::default()),
                clock: None,
            })
        })
        .expect("core opens")
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    /// `[[NSApplication sharedApplication] performSelector:@selector(terminate:) withObject:nil
    /// afterDelay:0]`, on the main thread: `terminate:` then arrives from the run loop, as it
    /// does from the Dock or at logout. (Sending it directly would run it inside tao's own
    /// event handler, which holds a lock `applicationWillTerminate:` takes: a self-deadlock
    /// of the check, not of the app.)
    fn terminate() {
        // SAFETY: the casts give `objc_msgSend` the signatures of the messages sent with them
        // (the arguments are passed as in a plain call on arm64 and x86-64); the class and
        // selector names are NUL-terminated literals; AppKit requires the main thread, which
        // `run_on_main_thread` provides.
        unsafe {
            let send = objc_msgSend as *const ();
            let get: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(send);
            let perform: unsafe extern "C" fn(
                *mut c_void,
                *mut c_void,
                *mut c_void,
                *mut c_void,
                f64,
            ) = std::mem::transmute(send);
            let app = get(
                objc_getClass(c"NSApplication".as_ptr()),
                sel_registerName(c"sharedApplication".as_ptr()),
            );
            perform(
                app,
                sel_registerName(c"performSelector:withObject:afterDelay:".as_ptr()),
                sel_registerName(c"terminate:".as_ptr()),
                std::ptr::null_mut(),
                0.0,
            );
        }
    }

    type MenuBuilder = Box<dyn FnOnce(&AppHandle<Wry>) -> tauri::Result<Menu<Wry>> + Send>;

    /// A builder that keeps the menu `install` registers and ignores the other handlers.
    struct MenuOnly(Option<MenuBuilder>);
    impl Wire<Wry> for MenuOnly {
        fn on_page_load<F: Fn(&Webview<Wry>, &PageLoadPayload<'_>) + Send + Sync + 'static>(
            self,
            _: F,
        ) -> Self {
            self
        }
        fn on_window_event<F: Fn(&Window<Wry>, &WindowEvent) + Send + Sync + 'static>(
            self,
            _: F,
        ) -> Self {
            self
        }
        fn menu<F: FnOnce(&AppHandle<Wry>) -> tauri::Result<Menu<Wry>> + Send + 'static>(
            mut self,
            f: F,
        ) -> Self {
            self.0 = Some(Box::new(f));
            self
        }
        fn on_menu_event<F: Fn(&AppHandle<Wry>, MenuEvent) + Send + Sync + 'static>(
            self,
            _: F,
        ) -> Self {
            self
        }
    }

    /// Ride, then get terminated by `NSApp terminate:` after [`RIDE_FOR`].
    pub fn child(dir: &Path) {
        let app = lifecycle::install(tauri::Builder::<tauri::Wry>::new())
            .build(mock_context(noop_assets()))
            .expect("app builds");
        // The Quit item (Cmd+Q) is ours, not the predefined one that sends `terminate:`: check
        // the menu `install` registered, not `lifecycle::menu` alone.
        let build = lifecycle::install(MenuOnly(None))
            .0
            .expect("install registers a menu");
        let menu = build(app.handle()).expect("menu");
        let Some(MenuItemKind::Submenu(application)) =
            menu.items().expect("items").into_iter().next()
        else {
            panic!("the application menu comes first");
        };
        let last = application
            .items()
            .expect("items")
            .pop()
            .expect("a Quit item");
        assert!(
            matches!(&last, MenuItemKind::MenuItem(i) if i.id().as_ref() == lifecycle::QUIT_ITEM_ID),
            "the application menu's last item is not the replacement Quit item"
        );

        let core = core(dir);
        tauri::async_runtime::block_on(async {
            core.device_pair("trainer".into(), DeviceRole::Trainer)
                .await
                .expect("pair");
            core.ride_load(Some("builtin-sensor-check".into()))
                .await
                .expect("load");
            core.ride_start().await.expect("start");
        });
        app.manage(core);
        let handle = app.handle().clone();
        std::thread::spawn(move || {
            std::thread::sleep(RIDE_FOR);
            handle.run_on_main_thread(terminate).expect("main thread");
        });
        lifecycle::run(app);
    }

    /// Run the child, then read what it left in the store.
    pub fn parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut child = Command::new(std::env::current_exe().expect("exe"))
            .env("CRANK_QUIT_CHECK_DIR", dir.path())
            .spawn()
            .expect("child starts");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("wait") {
                break status;
            }
            if started.elapsed() > CHILD_LIMIT {
                child.kill().expect("kill the child this process started");
                panic!("the child did not exit within {CHILD_LIMIT:?} of NSApp terminate:");
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        println!("child exited: {status}");
        // The child is gone, so `ride.lock` is free again.
        let rides = tauri::async_runtime::block_on(core(dir.path()).rides_list()).expect("rides");
        let outcomes: Vec<RideOutcome> = rides.iter().map(|r| r.outcome).collect();
        assert_eq!(
            outcomes,
            [RideOutcome::Finished],
            "NSApp terminate: must end the ride"
        );
        println!("quit_check: ok, the ride was recorded as Finished");
    }
}
