//! The desktop's own file chooser: on Linux through xdg-desktop-portal (the
//! dialog the user's desktop draws: GNOME, KDE, Hyprland's), on Windows the
//! system open-file dialog. No toolkit of ours either way.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum PortalError {
    #[cfg(target_os = "linux")]
    #[error("the desktop file chooser is unavailable: {0}")]
    Bus(#[from] zbus::Error),
    #[cfg(target_os = "linux")]
    #[error("the desktop file chooser gave an unreadable answer")]
    Answer,
    #[allow(dead_code)]
    #[error("this system has no file chooser the app can use")]
    Unsupported,
}

/// Asks the user for one image file. `None` when they cancel. Blocks until
/// the dialog closes, so it runs on a blocking thread.
#[cfg(windows)]
pub fn pick_image(title: &str) -> Result<Option<PathBuf>, PortalError> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OPENFILENAMEW,
    };
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    // Pairs of description and pattern, each nul-terminated, the list ended
    // by an extra nul.
    let filter: Vec<u16> = "Images\0*.png;*.jpg;*.jpeg;*.webp\0\0"
        .encode_utf16()
        .collect();
    let title = wide(title);
    let mut file = vec![0u16; 32 * 1024];
    // SAFETY: OPENFILENAMEW is plain data; zeroed is its documented
    // starting state.
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR;
    // SAFETY: every pointer in `dialog` outlives the call.
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        // Cancelled (or failed, which the user sees as nothing happening).
        return Ok(None);
    }
    let length = file
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(file.len());
    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
        &file[..length],
    ))))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn pick_image(_title: &str) -> Result<Option<PathBuf>, PortalError> {
    Err(PortalError::Unsupported)
}

#[cfg(target_os = "linux")]
pub fn pick_image(title: &str) -> Result<Option<PathBuf>, PortalError> {
    use std::collections::HashMap;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    let connection = Connection::session()?;
    // The portal answers on a request object whose path is derived from our
    // bus name and a token we choose; subscribe before asking so the
    // answer cannot be missed.
    let token = format!("jellybridge{}", std::process::id());
    let sender = connection
        .unique_name()
        .ok_or(PortalError::Answer)?
        .trim_start_matches(':')
        .replace('.', "_");
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let request = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        request_path.as_str(),
        "org.freedesktop.portal.Request",
    )?;
    let mut responses = request.receive_signal("Response")?;
    let chooser = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
    )?;
    let images: Vec<(u32, &str)> = ["*.png", "*.jpg", "*.jpeg", "*.webp"]
        .into_iter()
        .map(|pattern| (0, pattern))
        .collect();
    let filters = vec![("Images", images)];
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("modal", Value::from(true));
    options.insert("filters", Value::from(filters));
    let _: OwnedObjectPath = chooser.call("OpenFile", &("", title, options))?;
    let message = responses.next().ok_or(PortalError::Answer)?;
    let (response, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|_| PortalError::Answer)?;
    if response != 0 {
        return Ok(None);
    }
    let uris: Vec<String> = results
        .get("uris")
        .and_then(|uris| uris.try_clone().ok())
        .and_then(|uris| Vec::<String>::try_from(uris).ok())
        .ok_or(PortalError::Answer)?;
    Ok(uris.first().and_then(|uri| file_path(uri)))
}

/// A `file://` URI as a path, percent escapes decoded.
/// A blocking error dialog for failures that happen before (or instead of)
/// the window, where nothing else could show them. Best effort: without a
/// dialog tool the message only reaches the log.
#[cfg(windows)]
pub fn show_error_dialog(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("JellyBridge"));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call;
    // a null owner window is allowed.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// A blocking error dialog (see the Windows version). Linux has no dialog
/// API below the toolkits, so it asks the desktop's own tool.
#[cfg(not(windows))]
pub fn show_error_dialog(message: &str) {
    let attempts: [(&str, &[&str]); 2] = [
        (
            "zenity",
            &[
                "--error",
                "--title=JellyBridge",
                "--no-markup",
                "--width=420",
                "--text",
            ],
        ),
        ("kdialog", &["--title", "JellyBridge", "--error"]),
    ];
    for (program, args) in attempts {
        let shown = std::process::Command::new(program)
            .args(args)
            .arg(message)
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if shown {
            return;
        }
    }
    eprintln!("{message}");
}

#[cfg(target_os = "linux")]
fn file_path(uri: &str) -> Option<PathBuf> {
    let url = url::Url::parse(uri).ok()?;
    url.to_file_path().ok()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(
            file_path("file:///home/me/My%20Pictures/a.png"),
            Some(PathBuf::from("/home/me/My Pictures/a.png"))
        );
        assert_eq!(file_path("https://example.com/a.png"), None);
    }
}
