//! WebView2 memory-usage target: ask the WebView to trim its memory while
//! nobody is looking at it (window minimized, or hidden while a game runs),
//! and return to normal when it is shown again.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{WebviewWindow, Window, WindowEvent};

static LOW: AtomicBool = AtomicBool::new(false);

/// Window events: low while minimized, normal otherwise (resizes are
/// frequent, so `set_low` ignores repeats).
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::Resized(_) = event
        && let Some(w) = tauri::Manager::get_webview_window(window, window.label())
    {
        let hidden = !w.is_visible().unwrap_or(true);
        set_low(&w, w.is_minimized().unwrap_or(false) || hidden);
    }
}

/// Set the low (true) or normal (false) memory target. Best effort: older
/// WebView2 runtimes without `ICoreWebView2_19` are left as they are.
pub fn set_low(window: &WebviewWindow, low: bool) {
    if LOW.swap(low, Ordering::AcqRel) == low {
        return;
    }
    tracing::info!(low, "webview memory target");
    #[cfg(windows)]
    {
        let result = window.with_webview(move |wv| {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
            };
            use windows::core::Interface;
            let level = if low {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            };
            // SAFETY: called on the WebView's own thread by `with_webview`;
            // the controller and its interfaces are live COM objects owned
            // by wry for the window's lifetime.
            #[allow(unsafe_code)]
            let r = unsafe {
                wv.controller()
                    .CoreWebView2()
                    .and_then(|core| core.cast::<ICoreWebView2_19>())
                    .and_then(|c| c.SetMemoryUsageTargetLevel(level))
            };
            match r {
                Ok(()) => tracing::debug!(low, "webview memory target set"),
                Err(e) => tracing::debug!(error = %e, "webview memory target unavailable"),
            }
        });
        if let Err(e) = result {
            tracing::debug!(error = %e, "webview not available for memory target");
        }
    }
    #[cfg(not(windows))]
    let _ = (window, low);
}
