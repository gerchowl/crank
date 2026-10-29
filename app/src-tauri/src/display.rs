//! The physical width of the display a window is on (#286, DESIGN §6.2).
//!
//! A webview cannot read a display's physical size, so the ride screen's type tiers (#91)
//! need it from the shell. [`display_width_mm`] answers for the calling window's current
//! monitor; `None` means "unknown" and the tiers fall back (and the settings override, #90,
//! still wins in the UI).
//!
//! The mapping from a window's monitor to a display is pure ([`width_mm`]) over a
//! [`Displays`] source, so it is tested without a screen; only the source is platform code.
//!
//! * **macOS:** CoreGraphics. `CoreGraphicsDisplays` lists the active displays
//!   (`CGGetActiveDisplayList`) with their id, bounds and `CGDisplayScreenSize` (mm, from the
//!   EDID). The window's own display is found **exactly**, by id: `NSWindow.screen` ->
//!   `deviceDescription["NSScreenNumber"]` is the `CGDirectDisplayID` ([`WindowMonitor::display_id`]),
//!   which also tells mirrored displays apart (they share an origin). Only on the main thread, where
//!   `AppKit` may be asked; Tauri runs a synchronous command there. If the id is unavailable (another
//!   thread, a window without a screen, the key missing) the display is found by position instead:
//!   tao reports a monitor's position as the display's `CGDisplayBounds` origin (points) times the
//!   scale factor, so the display whose origin times the monitor's scale factor lands on the monitor's
//!   position is the one. That fallback is ambiguous for mirrored displays and takes the first.
//! * **Linux:** always `None`. The only source is GDK's `width_mm`, which X11 (`RandR`) and
//!   Wayland derive from EDID but which is routinely wrong or a made-up 16:9 default
//!   (virtual machines, docks, KVMs); a wrong width would give wrongly small type, so
//!   "unknown" with the notice is the honest answer there. The override setting covers it.
//! * **Elsewhere:** `None`.
use tauri::{Runtime, Window};

/// One display as the platform reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayInfo {
    /// The platform's display id (`CGDirectDisplayID` on macOS).
    pub id: u32,
    /// Top-left corner in the global desktop space, in points (logical pixels).
    pub origin: (f64, f64),
    /// Physical width in millimetres as the platform reports it; 0 when there is no EDID.
    pub width_mm: f64,
}

/// A window's monitor as Tauri reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMonitor {
    /// The window's own display id, when the platform could say; an exact match for it is preferred
    /// over position (mirrored displays share a position).
    pub display_id: Option<u32>,
    /// Top-left corner in physical pixels.
    pub position_px: (f64, f64),
    /// Device pixels per point.
    pub scale_factor: f64,
}

/// A source of the displays currently attached.
pub trait Displays {
    /// Every active display.
    fn all(&self) -> Vec<DisplayInfo>;
}

/// Positions closer than this many physical pixels are the same corner (rounding in the
/// point-to-pixel conversion).
const POSITION_TOLERANCE_PX: f64 = 1.0;

/// The physical width in mm of the display `monitor` is, or `None` when no display matches
/// or the reported width is not a positive finite number (0 is "no EDID", never a width).
///
/// With the window's own `display_id` the match is exact, and an id that is not among the displays is
/// unknown, never a guess by position. Without one the match is by position (see the module doc).
#[must_use]
pub fn width_mm(monitor: &WindowMonitor, displays: &dyn Displays) -> Option<f64> {
    let all = displays.all();
    let found = if let Some(id) = monitor.display_id {
        all.into_iter().find(|d| d.id == id)
    } else {
        if !monitor.scale_factor.is_finite() || monitor.scale_factor <= 0.0 {
            return None;
        }
        let near = |a: f64, b: f64| (a - b).abs() <= POSITION_TOLERANCE_PX;
        all.into_iter().find(|d| {
            near(d.origin.0 * monitor.scale_factor, monitor.position_px.0)
                && near(d.origin.1 * monitor.scale_factor, monitor.position_px.1)
        })
    };
    found
        .map(|d| d.width_mm)
        .filter(|mm| mm.is_finite() && *mm > 0.0)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{DisplayInfo, Displays};
    use core_graphics::display::CGDisplay;

    /// The displays CoreGraphics lists as active.
    pub struct CoreGraphicsDisplays;
    impl Displays for CoreGraphicsDisplays {
        fn all(&self) -> Vec<DisplayInfo> {
            CGDisplay::active_displays()
                .unwrap_or_default()
                .into_iter()
                .map(|id| {
                    let display = CGDisplay::new(id);
                    let bounds = display.bounds();
                    DisplayInfo {
                        id,
                        origin: (bounds.origin.x, bounds.origin.y),
                        width_mm: display.screen_size().width,
                    }
                })
                .collect()
        }
    }
}
#[cfg(target_os = "macos")]
pub use platform::CoreGraphicsDisplays;

/// The `CGDirectDisplayID` of the display the window is on: `NSWindow.screen`'s
/// `deviceDescription["NSScreenNumber"]`. `None` off the main thread (`AppKit` is main-thread only), for
/// a window without a screen, or when the key is missing; the caller then matches by position.
#[cfg(target_os = "macos")]
fn window_display_id<R: Runtime>(window: &Window<R>) -> Option<u32> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSNumber, ns_string};
    MainThreadMarker::new()?;
    let ptr = window.ns_window().ok()?;
    // SAFETY: Tauri hands out the window's live `NSWindow` pointer, and `MainThreadMarker::new()` above
    // proved this is the main thread, the only thread that may use it. It is not retained past this call.
    let ns_window: &NSWindow = unsafe { ptr.cast::<NSWindow>().as_ref()? };
    let description = ns_window.screen()?.deviceDescription();
    let number = description.objectForKey(ns_string!("NSScreenNumber"))?;
    Some(number.downcast_ref::<NSNumber>()?.unsignedIntValue())
}

/// No display source on this platform (see the module doc for Linux).
#[cfg(not(target_os = "macos"))]
pub struct NoDisplays;
#[cfg(not(target_os = "macos"))]
impl Displays for NoDisplays {
    fn all(&self) -> Vec<DisplayInfo> {
        Vec::new()
    }
}

/// The physical width in mm of the display the calling window is on, `None` when unknown
/// (no EDID, no platform source, or the window has no monitor).
#[tauri::command]
#[specta::specta]
// Tauri hands a command its extractors by value.
#[allow(clippy::needless_pass_by_value)]
#[must_use]
pub fn display_width_mm<R: Runtime>(window: Window<R>) -> Option<f64> {
    let monitor = window.current_monitor().ok().flatten()?;
    let position = monitor.position();
    #[cfg(target_os = "macos")]
    let display_id = window_display_id(&window);
    #[cfg(not(target_os = "macos"))]
    let display_id = None;
    let monitor = WindowMonitor {
        display_id,
        position_px: (f64::from(position.x), f64::from(position.y)),
        scale_factor: monitor.scale_factor(),
    };
    #[cfg(target_os = "macos")]
    let displays = CoreGraphicsDisplays;
    #[cfg(not(target_os = "macos"))]
    let displays = NoDisplays;
    width_mm(&monitor, &displays)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<DisplayInfo>);
    impl Displays for Fake {
        fn all(&self) -> Vec<DisplayInfo> {
            self.0.clone()
        }
    }
    fn display(x: f64, y: f64, width_mm: f64) -> DisplayInfo {
        DisplayInfo {
            id: 0,
            origin: (x, y),
            width_mm,
        }
    }
    fn with_id(id: u32, d: DisplayInfo) -> DisplayInfo {
        DisplayInfo { id, ..d }
    }
    fn monitor(x: f64, y: f64, scale_factor: f64) -> WindowMonitor {
        WindowMonitor {
            display_id: None,
            position_px: (x, y),
            scale_factor,
        }
    }
    fn on_display(id: u32, m: WindowMonitor) -> WindowMonitor {
        WindowMonitor {
            display_id: Some(id),
            ..m
        }
    }

    #[test]
    fn the_display_at_the_monitors_corner_answers() {
        // A 2x laptop panel at the origin and a 1x external to its right (points).
        let displays = Fake(vec![
            display(0.0, 0.0, 344.0),
            display(1512.0, -200.0, 597.0),
        ]);
        assert_eq!(width_mm(&monitor(0.0, 0.0, 2.0), &displays), Some(344.0));
        // The external's position is its point origin times ITS scale factor, not the laptop's.
        assert_eq!(
            width_mm(&monitor(1512.0, -200.0, 1.0), &displays),
            Some(597.0)
        );
    }

    #[test]
    fn a_pixel_of_rounding_still_matches_but_a_different_corner_does_not() {
        let displays = Fake(vec![display(1512.5, 0.0, 597.0)]);
        assert_eq!(width_mm(&monitor(3025.0, 0.0, 2.0), &displays), Some(597.0));
        assert_eq!(width_mm(&monitor(1500.0, 0.0, 1.0), &displays), None);
    }

    #[test]
    fn no_matching_display_is_unknown() {
        assert_eq!(width_mm(&monitor(0.0, 0.0, 1.0), &Fake(vec![])), None);
        let far = Fake(vec![display(5000.0, 0.0, 600.0)]);
        assert_eq!(width_mm(&monitor(0.0, 0.0, 1.0), &far), None);
    }

    #[test]
    fn zero_negative_and_non_finite_widths_are_unknown() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let displays = Fake(vec![display(0.0, 0.0, bad)]);
            assert_eq!(width_mm(&monitor(0.0, 0.0, 1.0), &displays), None, "{bad}");
        }
    }

    #[test]
    fn a_bad_scale_factor_is_unknown_not_a_wrong_match() {
        let displays = Fake(vec![display(0.0, 0.0, 300.0)]);
        for bad in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(width_mm(&monitor(0.0, 0.0, bad), &displays), None, "{bad}");
        }
    }

    #[test]
    fn mirrored_displays_are_told_apart_by_the_windows_own_display_id() {
        // Two displays mirror each other: same origin, different panels. By position the first would
        // always win; the window's id picks its own.
        let displays = Fake(vec![
            with_id(10, display(0.0, 0.0, 344.0)),
            with_id(11, display(0.0, 0.0, 597.0)),
        ]);
        let here = monitor(0.0, 0.0, 2.0);
        assert_eq!(width_mm(&on_display(11, here), &displays), Some(597.0));
        assert_eq!(width_mm(&on_display(10, here), &displays), Some(344.0));
    }

    #[test]
    fn without_an_id_a_mirrored_pair_falls_back_to_position_and_takes_the_first() {
        // The documented limit of the fallback: ambiguous, the first display wins.
        let displays = Fake(vec![
            with_id(10, display(0.0, 0.0, 344.0)),
            with_id(11, display(0.0, 0.0, 597.0)),
        ]);
        assert_eq!(width_mm(&monitor(0.0, 0.0, 2.0), &displays), Some(344.0));
    }

    #[test]
    fn an_id_that_is_not_among_the_displays_is_unknown_not_a_position_guess() {
        let displays = Fake(vec![with_id(10, display(0.0, 0.0, 344.0))]);
        assert_eq!(
            width_mm(&on_display(99, monitor(0.0, 0.0, 2.0)), &displays),
            None
        );
    }

    #[test]
    fn the_id_match_needs_no_scale_factor_and_still_refuses_a_bad_width() {
        let ok = Fake(vec![with_id(7, display(100.0, 100.0, 300.0))]);
        assert_eq!(
            width_mm(&on_display(7, monitor(0.0, 0.0, f64::NAN)), &ok),
            Some(300.0)
        );
        let no_edid = Fake(vec![with_id(7, display(0.0, 0.0, 0.0))]);
        assert_eq!(
            width_mm(&on_display(7, monitor(0.0, 0.0, 1.0)), &no_edid),
            None
        );
    }
}
