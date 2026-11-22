//! The Mac is about to sleep: cool the links down (ADR-0002 §5.1.3, `Release(Sleep)`).
//!
//! **What it does.** On `NSWorkspaceWillSleepNotification`, [`on_will_sleep`] sends
//! `links_intent(Release { reason: Sleep })` to the managed [`Core`] ([`will_sleep`]): the
//! keeper releases its warm links, cancels an attempt still connecting, and a loaded ride that
//! has not started is discarded. A started ride is untouched (sleep during a ride is #51).
//!
//! **Where it lives and how long.** [`Observer`] (macOS only) is registered on
//! `NSWorkspace.sharedWorkspace.notificationCenter` by [`crate::run`] once the app is built (the
//! core is managed by then), moved into the run-event callback ([`crate::lifecycle::run`]) and
//! dropped, which unregisters it, when the quit starts (the first `ExitRequested`, or `Exit` on
//! the backstop path): from then on nothing acquires a link (§5.1.3), and a sleep that comes
//! during the quit's 3 s has the quit's own release instead. Other platforms have no observer
//! (#41).
//!
//! **It does not block the main thread.** `NSWorkspace` posts the notification on the main
//! thread and macOS does not wait for its observers, so the handler only spawns the intent on
//! the async runtime, like every other main-thread handler of the app ([`crate::lifecycle`]).
//! The release therefore races the sleep. Losing that race costs no safety: nothing is written
//! while warm (§5.1.3), the system drops the Bluetooth links when it sleeps, and the spawned
//! intent is applied at wake, which still discards an unstarted ride that slept through.
use crank_core::{Core, LinkIntent, ReleaseReason};
use tauri::{AppHandle, Manager, Runtime};

/// The intent the observer sends.
pub const SLEEP: LinkIntent = LinkIntent::Release {
    reason: ReleaseReason::Sleep,
};

/// Send [`SLEEP`] to `core` and wait until it is applied; a refusal is logged, never shown.
///
/// # Errors
/// What [`Core::links_intent`] answers (`Storage` when the keeper cannot read the pairings).
pub async fn will_sleep(core: &Core) -> Result<(), crank_core::ApiError> {
    tracing::info!("sleep: the Mac is about to sleep; releasing the links");
    let answer = core.links_intent(SLEEP).await;
    if let Err(e) = &answer {
        tracing::warn!(%e, "sleep: the links were not released");
    }
    answer
}

/// The observer's handler: [`will_sleep`] on the managed core, spawned so the main thread
/// never waits. Without a core (it failed to open) there is nothing to release.
pub fn on_will_sleep<R: Runtime>(app: &AppHandle<R>) {
    if let Some(core) = app.try_state::<Core>() {
        let core = core.inner().clone();
        tauri::async_runtime::spawn(async move {
            // Logged inside; nobody waits on the answer.
            let _ = will_sleep(&core).await;
        });
    }
}

#[cfg(target_os = "macos")]
pub use mac::Observer;

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send, sel};
    use objc2_app_kit::{NSWorkspace, NSWorkspaceWillSleepNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter};

    /// The target's state: the handler it calls.
    struct Ivars {
        handler: Box<dyn Fn() + Send + Sync>,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and the class implements no `Drop`.
        #[unsafe(super(NSObject))]
        #[name = "CrankSleepTarget"]
        #[ivars = Ivars]
        struct Target;

        impl Target {
            /// `willSleep:`, the selector it is registered with.
            #[unsafe(method(willSleep:))]
            fn will_sleep(&self, _note: &NSNotification) {
                (self.ivars().handler)();
            }
        }

        unsafe impl NSObjectProtocol for Target {}
    );

    impl Target {
        fn new(handler: Box<dyn Fn() + Send + Sync>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { handler });
            // SAFETY: `init` is `NSObject`'s designated initialiser, called once on a fresh allocation.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The registration of a handler for `NSWorkspaceWillSleepNotification` on the workspace's
    /// notification center; dropping it unregisters the handler (the module docs say when).
    pub struct Observer {
        center: Retained<NSNotificationCenter>,
        target: Retained<Target>,
    }

    impl Observer {
        /// Register `handler`: it runs on the posting thread (the main thread, for the system's
        /// notification) each time the Mac is about to sleep, until the observer is dropped.
        #[must_use]
        pub fn new(handler: impl Fn() + Send + Sync + 'static) -> Self {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            let target = Target::new(Box::new(handler));
            // SAFETY: `target` implements `willSleep:` taking one `NSNotification`, the selector's
            // signature; the name is AppKit's constant; no sender filter (`nil`). The center keeps
            // only a weak reference, and `Drop` unregisters before `target` can be freed.
            unsafe {
                center.addObserver_selector_name_object(
                    &target,
                    sel!(willSleep:),
                    Some(NSWorkspaceWillSleepNotification),
                    None,
                );
            }
            Self { center, target }
        }
    }

    impl Drop for Observer {
        fn drop(&mut self) {
            // SAFETY: `target` is the object registered in `new`, still alive (held by `self`).
            unsafe { self.center.removeObserver(&self.target) };
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use objc2_app_kit::NSWorkspaceDidWakeNotification;
        use objc2_foundation::NSNotificationName;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        fn post(name: &NSNotificationName) {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            // SAFETY: no sender object (`nil`).
            unsafe { center.postNotificationName_object(name, None) };
        }

        /// The handler runs on the will-sleep notification of the workspace's center, on no
        /// other, and not once the observer is dropped.
        #[test]
        fn the_observer_runs_its_handler_on_will_sleep_until_dropped() {
            let calls = Arc::new(AtomicUsize::new(0));
            let counted = calls.clone();
            let observer = Observer::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            });
            // SAFETY (both): AppKit's notification-name constants, read only.
            let (sleep, wake) = unsafe {
                (
                    NSWorkspaceWillSleepNotification,
                    NSWorkspaceDidWakeNotification,
                )
            };
            post(wake);
            assert_eq!(calls.load(Ordering::SeqCst), 0, "a wake is not a sleep");
            post(sleep);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            drop(observer);
            post(sleep);
            assert_eq!(calls.load(Ordering::SeqCst), 1, "dropping unregisters it");
        }
    }
}
