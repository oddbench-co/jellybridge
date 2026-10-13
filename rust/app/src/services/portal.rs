//! The desktop's own file chooser, through xdg-desktop-portal: the dialog
//! the user's desktop draws (GNOME, KDE, Hyprland's), no toolkit of ours.

use std::collections::HashMap;
use std::path::PathBuf;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[derive(Debug, thiserror::Error)]
pub enum PortalError {
    #[error("the desktop file chooser is unavailable: {0}")]
    Bus(#[from] zbus::Error),
    #[error("the desktop file chooser gave an unreadable answer")]
    Answer,
}

/// Asks the user for one image file. `None` when they cancel. Blocks until
/// the dialog closes, so it runs on a blocking thread.
pub fn pick_image(title: &str) -> Result<Option<PathBuf>, PortalError> {
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
fn file_path(uri: &str) -> Option<PathBuf> {
    let url = url::Url::parse(uri).ok()?;
    url.to_file_path().ok()
}

#[cfg(test)]
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
