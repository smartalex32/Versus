//! Native-only coordinates used while a Windows shell drag is active.
//!
//! `winit` normally provides the pointer position. During shell drags, use
//! the live cursor when forwarded pointer events are stale or unavailable.
//! Reading it leaves the system cursor unchanged.

#[cfg(windows)]
use eframe::egui::Pos2;

/// Return the Windows cursor position in this window's egui coordinate space.
///
/// The value is only intended as a fallback while a native drag is active.
/// Invalid window handles and unavailable cursor information are treated as
/// unavailable rather than guessing a target pane.
#[cfg(windows)]
pub(crate) fn cursor_position(window: isize, pixels_per_point: f32) -> Option<Pos2> {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

    if window == 0 || !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return None;
    }

    let mut screen_position = POINT { x: 0, y: 0 };
    // SAFETY: `screen_position` is valid writable memory for the duration of
    // the call. GetCursorPos only reads the system cursor; it does not modify
    // global cursor state.
    if unsafe { GetCursorPos(&mut screen_position) } == 0 {
        return None;
    }

    screen_point_to_egui_position(window, screen_position, pixels_per_point)
}

#[cfg(windows)]
fn screen_point_to_egui_position(
    window: isize,
    mut screen_position: windows_sys::Win32::Foundation::POINT,
    pixels_per_point: f32,
) -> Option<Pos2> {
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;

    if window == 0 || !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return None;
    }

    // SAFETY: `screen_position` is valid writable memory and `window` was
    // validated for the null handle case. Windows reports invalid handles by
    // returning zero, which we turn into no position.
    if unsafe { ScreenToClient(window, &mut screen_position) } == 0 {
        return None;
    }

    Some(Pos2::new(
        screen_position.x as f32 / pixels_per_point,
        screen_position.y as f32 / pixels_per_point,
    ))
}

#[cfg(all(test, windows))]
#[path = "../tests/unit/native_drop/mod.rs"]
mod tests;
