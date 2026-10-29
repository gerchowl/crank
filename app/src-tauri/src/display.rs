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
//!   (`CGGetActiveDisplayList`) with their bounds and `CGDisplayScreenSize` (mm, from the
//!   EDID). Tauri's [`tauri::Monitor`] does not expose its `CGDirectDisplayID`, so the display
//!   is found by position: tao reports a monitor's position as the display's
//!   `CGDisplayBounds` origin (points) times the scale factor, so the display whose origin
//!   times the monitor's scale factor lands on the monitor's position is the one.
//! * **Linux:** always `None`. The only source is GDK's `width_mm`, which X11 (`RandR`) and
//!   Wayland derive from EDID but which is routinely wrong or a made-up 16:9 default
//!   (virtual machines, docks, KVMs); a wrong width would give wrongly small type, so
//!   "unknown" with the notice is the honest answer there. The override setting covers it.
//! * **Elsewhere:** `None`.
use tauri::{Runtime, Window};

/// One display as the platform reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayInfo {
    /// Top-left corner in the global desktop space, in points (logical pixels).
    pub origin: (f64, f64),
    /// Physical width in millimetres as the platform reports it; 0 when there is no EDID.
    pub width_mm: f64,
}

/// A window's monitor as Tauri reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMonitor {
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
#[must_use]
pub fn width_mm(monitor: &WindowMonitor, displays: &dyn Displays) -> Option<f64> {
    if !monitor.scale_factor.is_finite() || monitor.scale_factor <= 0.0 {
        return None;
    }
    let near = |a: f64, b: f64| (a - b).abs() <= POSITION_TOLERANCE_PX;
    displays
        .all()
        .into_iter()
        .find(|d| {
            near(d.origin.0 * monitor.scale_factor, monitor.position_px.0)
                && near(d.origin.1 * monitor.scale_factor, monitor.position_px.1)
        })
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
    let monitor = WindowMonitor {
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
            origin: (x, y),
            width_mm,
        }
    }
    fn monitor(x: f64, y: f64, scale_factor: f64) -> WindowMonitor {
        WindowMonitor {
            position_px: (x, y),
            scale_factor,
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
}
