//! Startup compatibility checks must work before any WebView can render UI.

// Tauri's generated runtime context omits minimumWebview2Version. Keep an
// explicit startup threshold; the configuration test prevents installer drift.
pub const MINIMUM_VERSION: &str = "111.0.1661.62";

/// Compare four numeric components instead of treating browser versions as
/// decimal numbers or lexical strings. Preview-channel suffixes are metadata.
fn version_parts(version: &str) -> Option<[u32; 4]> {
    let mut parts = version.split_ascii_whitespace().next()?.split('.');
    let mut result = [0; 4];
    for part in &mut result {
        let text = parts.next()?;
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *part = text.parse().ok()?;
    }
    parts.next().is_none().then_some(result)
}

pub fn requires_update(current: Option<&str>, minimum: Option<&str>) -> bool {
    let (Some(current), Some(minimum)) = (current, minimum) else {
        // A failed query is already logged by startup. Missing diagnostics do
        // not prove that the installed runtime is too old.
        return false;
    };
    match (version_parts(current), version_parts(minimum)) {
        (Some(current), Some(minimum)) => current < minimum,
        // Startup records the queried version after logging is initialized.
        // An unfamiliar format is diagnostic evidence, not proof of old age.
        _ => false,
    }
}

#[cfg(target_os = "windows")]
pub fn prepare_host_thread() {
    use std::ptr;
    use windows_sys::{
        core::w,
        Win32::{
            Foundation::GetLastError,
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                CreateWindowExW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
                WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
            },
        },
    };

    // Hidden WebView2 windows initialize before any foreground surface is shown.
    // Endpoint hooks that wrap GetMessageA with PeekMessageA + GetLastError can
    // fail with ERROR_INVALID_WINDOW_HANDLE (0x80070578) when the UI thread has
    // no visible top-level window while synchronous script setup pumps the queue.
    // A zero-size off-screen tool window keeps a valid visible handle on the UI
    // thread without appearing on screen, in the taskbar, or in focus traversal.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            w!("Static"),
            ptr::null(),
            WS_POPUP | WS_VISIBLE,
            -32000,
            -32000,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        log::warn!(
            "webview_runtime_host_window_failed native_code={}",
            unsafe { GetLastError() }
        );
    }
}

#[cfg(target_os = "windows")]
pub fn show_update_prompt(app: &tauri::AppHandle) {
    use crate::services::native_labels::NativeLabels;
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    use tauri_plugin_opener::OpenerExt;

    const DOWNLOAD_URL: &str = "https://developer.microsoft.com/en-us/microsoft-edge/webview2/";

    let labels = NativeLabels::load(app);
    let app = app.clone();
    // The native dialog stays readable even when the runtime cannot evaluate
    // the application's CSS. Its callback leaves the main event loop free.
    app.dialog()
        .message(labels.text("/webviewRuntime/updateRequired"))
        .title("MangoDisk")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            labels.text("/webviewRuntime/update").into(),
            labels.text("/webviewRuntime/exit").into(),
        ))
        .show(move |update| {
            if update {
                if let Err(error) = app.opener().open_url(DOWNLOAD_URL, None::<&str>) {
                    log::warn!(
                        "webview_runtime_update_link_failed error={}",
                        mangodisk_platform::diagnostics::text(&error)
                    );
                    app.dialog()
                        .message(
                            labels
                                .text("/webviewRuntime/openFailed")
                                .replace("{url}", DOWNLOAD_URL),
                        )
                        .title("MangoDisk")
                        .buttons(MessageDialogButtons::OkCustom(
                            labels.text("/webviewRuntime/exit").into(),
                        ))
                        .show(move |_| app.exit(0));
                    return;
                }
                log::info!("webview_runtime_update_link_opened");
            } else {
                log::info!("webview_runtime_update_dismissed");
            }
            app.exit(0);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_version_rejects_older_components_and_accepts_the_boundary() {
        let minimum = Some("111.0.1661.62");
        for version in ["110.0.1587.69", "111.0.1660.99", "111.0.1661.61"] {
            assert!(requires_update(Some(version), minimum), "{version}");
        }
        for version in [
            "111.0.1661.62",
            "111.0.1661.63",
            "111.0.1662.0",
            "153.0.4234.32",
            "111.0.1661.62 beta",
        ] {
            assert!(!requires_update(Some(version), minimum), "{version}");
        }
    }

    #[test]
    fn unavailable_or_malformed_versions_are_not_reported_as_outdated() {
        for current in [
            None,
            Some(""),
            Some("unknown"),
            Some("110.0"),
            Some("110.0.1.2.3"),
            Some("+110.0.1.2"),
            Some("4294967296.0.0.0"),
        ] {
            assert!(!requires_update(current, Some("111.0.1661.62")));
        }
        assert!(!requires_update(Some("110.0.1587.69"), None));
        assert!(!requires_update(Some("110.0.1587.69"), Some("invalid")));
    }

    #[test]
    fn installer_and_startup_share_a_valid_minimum_version() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let minimum = config["bundle"]["windows"]["minimumWebview2Version"]
            .as_str()
            .expect("the installer must declare the runtime baseline");
        assert_eq!(minimum, MINIMUM_VERSION);
        assert_eq!(version_parts(minimum), Some([111, 0, 1661, 62]));
        assert!(requires_update(Some("110.0.1587.69"), Some(minimum)));
    }
}
