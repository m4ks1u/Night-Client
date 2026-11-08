//! Window integration that the custom title bar needs.

use tauri::WebviewWindow;

use super::{CmdError, Result};

/// Show the Windows 11 snap-layout flyout, as hovering a native maximize
/// button would.
///
/// A borderless window's WebView covers the non-client area, so Windows
/// never sees the hover itself. The title bar calls this after the pointer
/// has rested on its maximize button for half a second, and Night Client
/// sends the same shortcut Windows provides for it (Win+Z). It only acts
/// while Night Client's own window has focus, so it can never send keys to
/// another application.
#[tauri::command]
pub fn show_snap_layouts(window: WebviewWindow) -> Result<()> {
    if !window.is_focused().unwrap_or(false) || window.is_maximized().unwrap_or(false) {
        return Ok(());
    }
    send_win_z().map_err(|e| CmdError::new("internal", e))
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn send_win_z() -> std::result::Result<(), String> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeybdInput {
        vk: u16,
        scan: u16,
        flags: u32,
        time: u32,
        extra: usize,
    }
    /// `INPUT` with the keyboard arm of the union; padded to the union's
    /// size (MOUSEINPUT is the largest member: 32 bytes on x64).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Input {
        kind: u32,
        ki: KeybdInput,
        _pad: [u8; 8],
    }
    const INPUT_KEYBOARD: u32 = 1;
    const KEYEVENTF_KEYUP: u32 = 0x0002;
    const VK_LWIN: u16 = 0x5B;
    const VK_Z: u16 = 0x5A;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    }

    let key = |vk: u16, up: bool| Input {
        kind: INPUT_KEYBOARD,
        ki: KeybdInput {
            vk,
            scan: 0,
            flags: if up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            extra: 0,
        },
        _pad: [0; 8],
    };
    let seq = [
        key(VK_LWIN, false),
        key(VK_Z, false),
        key(VK_Z, true),
        key(VK_LWIN, true),
    ];
    debug_assert_eq!(std::mem::size_of::<Input>(), 40);
    // SAFETY: `seq` is a valid array of correctly laid-out INPUT structures
    // and the size argument matches the structure size.
    let sent = unsafe {
        SendInput(
            seq.len() as u32,
            seq.as_ptr(),
            std::mem::size_of::<Input>() as i32,
        )
    };
    if sent as usize == seq.len() {
        Ok(())
    } else {
        Err("SendInput was blocked".into())
    }
}

#[cfg(not(windows))]
fn send_win_z() -> std::result::Result<(), String> {
    Ok(())
}
