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
mod tests {
    use super::{cursor_position, screen_point_to_egui_position};
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP},
    };

    struct HiddenWindow(isize);

    impl HiddenWindow {
        fn new() -> Self {
            const STATIC: &[u16] = &[
                b'S' as u16,
                b'T' as u16,
                b'A' as u16,
                b'T' as u16,
                b'I' as u16,
                b'C' as u16,
                0,
            ];
            // SAFETY: STATIC is a predefined system window class. We create a
            // hidden, unparented window for coordinate conversion only.
            let handle = unsafe {
                CreateWindowExW(
                    0,
                    STATIC.as_ptr(),
                    STATIC.as_ptr(),
                    WS_POPUP,
                    100,
                    100,
                    200,
                    200,
                    0,
                    0,
                    0,
                    std::ptr::null(),
                )
            };
            assert_ne!(
                handle, 0,
                "the hidden coordinate-test window must be created"
            );
            Self(handle)
        }
    }

    impl Drop for HiddenWindow {
        fn drop(&mut self) {
            // SAFETY: this guard owns the successfully-created window handle.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    fn screen_position(window: isize, x: i32, y: i32) -> POINT {
        let mut point = POINT { x, y };
        // SAFETY: `point` is valid writable memory and `window` is owned by
        // the test guard.
        assert_ne!(unsafe { ClientToScreen(window, &mut point) }, 0);
        point
    }

    #[test]
    fn converts_real_client_and_screen_coordinates_at_supported_scales() {
        let window = HiddenWindow::new();

        for (x, y, scale) in [(-30, 18, 1.0), (75, -45, 1.5), (140, 80, 2.0)] {
            let position =
                screen_point_to_egui_position(window.0, screen_position(window.0, x, y), scale)
                    .expect("a valid hidden window should convert its screen coordinate");

            assert!((position.x - x as f32 / scale).abs() < f32::EPSILON);
            assert!((position.y - y as f32 / scale).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn rejects_invalid_window_handles_and_scale_values() {
        let screen = POINT { x: 10, y: 20 };
        assert_eq!(screen_point_to_egui_position(0, screen, 1.0), None);
        // A non-null handle that was not created by Windows makes the native
        // ScreenToClient query fail.
        assert_eq!(screen_point_to_egui_position(1, screen, 1.0), None);
        assert_eq!(cursor_position(0, 1.0), None);

        let window = HiddenWindow::new();
        assert_eq!(screen_point_to_egui_position(window.0, screen, 0.0), None);
        assert_eq!(
            screen_point_to_egui_position(window.0, screen, f32::NAN),
            None
        );
    }
}
