//! The handful of Win32 calls the UI needs.
//!
//! macOS hands `MenuBarExtra` a label and it positions the popover itself. A
//! tray icon has no such affordance: the shell gives us a click and a cursor
//! position, and where the window goes is entirely our problem.

/// The tray icon size the shell wants, in physical pixels: 16 logical scaled by
/// the system DPI, so 16 at 100% and 32 at 200%.
#[cfg(windows)]
pub fn tray_icon_size() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    // SAFETY: a pure metric read with no arguments to get wrong.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    if size <= 0 {
        16
    } else {
        size as u32
    }
}

#[cfg(not(windows))]
pub fn tray_icon_size() -> u32 {
    16
}

/// The mouse position in physical pixels.
#[cfg(windows)]
pub fn cursor_position() -> (f32, f32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    // SAFETY: `point` is the POINT the call documents it fills in.
    match unsafe { GetCursorPos(&mut point) } {
        Ok(()) => (point.x as f32, point.y as f32),
        Err(_) => (0.0, 0.0),
    }
}

#[cfg(not(windows))]
pub fn cursor_position() -> (f32, f32) {
    (0.0, 0.0)
}

/// The desktop area excluding the taskbar, in physical pixels.
#[cfg(windows)]
pub fn work_area() -> (f32, f32, f32, f32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut rect = RECT::default();
    // SAFETY: `rect` is the RECT SPI_GETWORKAREA documents it writes.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() {
        return (0.0, 0.0, 1920.0, 1080.0);
    }
    (
        rect.left as f32,
        rect.top as f32,
        rect.right as f32,
        rect.bottom as f32,
    )
}

#[cfg(not(windows))]
pub fn work_area() -> (f32, f32, f32, f32) {
    (0.0, 0.0, 1920.0, 1080.0)
}

/// Take the foreground, the way a tray popup has to.
///
/// Windows only lets the process that owns the most recent input event call
/// `SetForegroundWindow`. A tray click is delivered to *us*, but the input
/// event belongs to the shell -- so the call is silently downgraded to a
/// taskbar flash and the popover comes up unfocused. The documented workaround
/// is to borrow the foreground thread's input queue for the duration of the
/// call, which is what every tray popup has done since Windows 98.
///
/// This is best effort. If it fails the popover is still visible (it is
/// always-on-top); it just will not close by itself when you click elsewhere,
/// which is why the caller waits for real focus before arming that.
#[cfg(windows)]
pub fn take_foreground(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    let window = HWND(hwnd as *mut std::ffi::c_void);

    // SAFETY: `window` is the handle eframe gave us for our own live window,
    // and the thread ids come straight from the calls that produce them. The
    // attach is unconditionally undone below.
    unsafe {
        let foreground = GetForegroundWindow();
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(foreground, None);
        let borrowed =
            theirs != 0 && theirs != ours && AttachThreadInput(theirs, ours, true).as_bool();

        let _ = BringWindowToTop(window);
        let _ = SetForegroundWindow(window);
        let _ = SetActiveWindow(window);
        let _ = SetFocus(Some(window));

        if borrowed {
            let _ = AttachThreadInput(theirs, ours, false);
        }
    }
}

#[cfg(not(windows))]
pub fn take_foreground(_hwnd: isize) {}

/// Make `println!` reach the terminal that launched us.
///
/// The binary is built for the windows subsystem so that launching the tray app
/// never flashes a console window. Inherited handles still work -- which is why
/// the status line bridge prints correctly into the pipe Claude Code gives it --
/// but a human running `claude-quota.exe install-statusline` from a prompt has
/// no handles at all, and every message would vanish. Borrow the parent's
/// console in that case.
#[cfg(windows)]
pub fn attach_parent_console() {
    use windows::core::w;
    use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    // SAFETY: every call below is a handle query or a handle swap on this
    // process, and each result is checked before it is used.
    unsafe {
        let existing = GetStdHandle(STD_OUTPUT_HANDLE);
        if matches!(existing, Ok(handle) if !handle.is_invalid() && handle != HANDLE::default()) {
            // Already redirected -- a pipe from Claude Code, or a shell. Leave
            // it alone; replacing it would break the bridge.
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        let console = CreateFileW(
            w!("CONOUT$"),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        );
        if let Ok(console) = console {
            if console != INVALID_HANDLE_VALUE {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, console);
                let _ = SetStdHandle(STD_ERROR_HANDLE, console);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn attach_parent_console() {}

/// Where to put a popover of this size for a tray click at `cursor`.
///
/// Anchored to the work-area corner nearest the click rather than to the cursor
/// itself: the taskbar can be on any edge, and the window has to end up on the
/// desktop side of it either way.
pub fn popover_position(
    cursor: (f32, f32),
    size: (f32, f32),
    work: (f32, f32, f32, f32),
) -> (f32, f32) {
    let (left, top, right, bottom) = work;
    let (width, height) = size;
    let margin = 8.0;

    // Horizontally centred on the click, then pushed back inside the work area.
    let x = (cursor.0 - width / 2.0).clamp(left + margin, (right - width - margin).max(left));

    // Above the click when the taskbar is at the bottom, below it when at the
    // top. Which it is, is exactly what the work area tells us.
    let above = cursor.1 > (top + bottom) / 2.0;
    let y = if above {
        (bottom - height - margin).max(top + margin)
    } else {
        (top + margin).min((bottom - height - margin).max(top))
    };

    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FHD_BOTTOM_TASKBAR: (f32, f32, f32, f32) = (0.0, 0.0, 1920.0, 1032.0);

    #[test]
    fn a_click_on_a_bottom_taskbar_puts_the_popover_above_it() {
        let (x, y) = popover_position((1700.0, 1050.0), (420.0, 300.0), FHD_BOTTOM_TASKBAR);
        assert_eq!(y, 1032.0 - 300.0 - 8.0);
        assert!(x + 420.0 <= 1920.0);
    }

    #[test]
    fn a_click_on_a_top_taskbar_puts_the_popover_below_it() {
        let top_taskbar = (0.0, 48.0, 1920.0, 1080.0);
        let (_, y) = popover_position((1700.0, 20.0), (420.0, 300.0), top_taskbar);
        assert_eq!(y, 48.0 + 8.0);
    }

    #[test]
    fn the_popover_never_hangs_off_the_edge_of_the_work_area() {
        for cursor_x in [0.0, 5.0, 960.0, 1915.0, 1920.0] {
            let (x, _) = popover_position((cursor_x, 1050.0), (420.0, 300.0), FHD_BOTTOM_TASKBAR);
            assert!(x >= 0.0, "{cursor_x} -> {x}");
            assert!(x + 420.0 <= 1920.0, "{cursor_x} -> {x}");
        }
    }

    #[test]
    fn it_is_centred_on_the_click_when_there_is_room() {
        let (x, _) = popover_position((960.0, 1050.0), (420.0, 300.0), FHD_BOTTOM_TASKBAR);
        assert_eq!(x, 750.0);
    }

    #[test]
    fn a_popover_taller_than_the_screen_is_pinned_rather_than_pushed_off_the_top() {
        let (_, y) = popover_position((960.0, 1050.0), (420.0, 4000.0), FHD_BOTTOM_TASKBAR);
        assert_eq!(y, 8.0);
    }

    #[test]
    fn a_secondary_monitor_to_the_left_keeps_negative_coordinates() {
        // Work areas on a multi-monitor desktop are not anchored at the origin.
        let left_monitor = (-1920.0, 0.0, 0.0, 1032.0);
        let (x, y) = popover_position((-200.0, 1050.0), (420.0, 300.0), left_monitor);
        assert!(x >= -1920.0 && x + 420.0 <= 0.0, "{x}");
        assert_eq!(y, 1032.0 - 308.0);
    }
}
