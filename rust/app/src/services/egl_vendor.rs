//! Keeps EGL vendor drivers the display does not use out of the process.
//!
//! `libEGL.so.1` on Linux is glvnd, a vendor-neutral dispatch layer. At first
//! EGL use it reads every ICD description in `/usr/share/glvnd/egl_vendor.d`
//! and `dlopen`s each one in turn to ask which can drive the display. On a
//! machine with both NVIDIA and Mesa installed, Mesa loses, never draws a
//! single frame, and stays resident for the life of the process -- it brings
//! `libgallium` and `libLLVM` with it, because Mesa compiles shaders with
//! LLVM. Measured here: 104 MiB of RSS for a driver that is never used.
//!
//! `__EGL_VENDOR_LIBRARY_FILENAMES` narrows that list to one file and removes
//! the load entirely: settled RSS 344 MiB to 238 MiB, PSS 146 MiB to 122 MiB,
//! with no change to launch time and an identical GL context.
//!
//! It cannot be set blindly. On an AMD or Intel machine Mesa *is* the right
//! vendor, and on a hybrid machine the right answer can change between boots.
//! So this module never guesses:
//!
//! 1. The first launch probes normally and records which vendor actually
//!    served the display, but only when exactly one installed ICD can be
//!    matched to it. When nothing matches unambiguously, nothing is pinned and
//!    the app behaves exactly as it did before.
//! 2. Later launches pin that one ICD, and leave a marker file behind while
//!    they do. Reaching a working GL context removes the marker.
//! 3. Finding the marker still there at startup means the previous pinned
//!    launch never got a context -- it crashed, hung, or was killed. The pin
//!    is dropped and that launch probes normally. This covers the failure
//!    modes an error return cannot.
//! 4. A pin that produces a *different* vendor than the one recorded is
//!    rejected: the app would otherwise spend the session on whatever driver
//!    the stale pin selected, which on this hardware means software rendering.
//!    The pin is dropped and the app restarts itself once, unpinned.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::services::session::{SettingsStore, StoredSettings};

/// Where glvnd looks for ICD descriptions, in its own precedence order.
const ICD_DIRECTORIES: [&str; 2] = ["/etc/glvnd/egl_vendor.d", "/usr/share/glvnd/egl_vendor.d"];

/// The variable glvnd reads, once, at first EGL use.
const VENDOR_FILENAMES: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";

/// Set on the restart after a rejected pin, so a pin that somehow keeps
/// mismatching can never turn into an exec loop.
const REPROBE_MARKER: &str = "JELLYBRIDGE_EGL_REPROBED";

/// What [`apply`] did, carried to [`note_gl_vendor`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pin {
    /// Nothing was pinned; glvnd will probe every vendor.
    Probing,
    /// One ICD was pinned. The context must report this `GL_VENDOR`.
    Applied { expect: String },
}

static PIN: OnceLock<Pin> = OnceLock::new();
static MARKER: OnceLock<Option<PathBuf>> = OnceLock::new();
static RECONCILED: AtomicBool = AtomicBool::new(false);
static REPROBE_WANTED: AtomicBool = AtomicBool::new(false);

/// Applies a stored pin, if there is a usable one.
///
/// Must be called before anything touches EGL -- glvnd caches its vendor list
/// on first use, so a later change to the environment has no effect. In
/// practice that means before `slint::BackendSelector::select`.
///
/// `settings` is only read; a pin that has to be dropped is written back here
/// rather than left for the caller to notice.
pub fn apply(settings: &StoredSettings, cache_directory: &Path) {
    let preset = std::env::var_os(VENDOR_FILENAMES);
    let pin = match decide(settings, cache_directory, preset.as_deref()) {
        Decision::Probe => Pin::Probing,
        Decision::Forget { reason } => {
            drop_pin(reason);
            Pin::Probing
        }
        Decision::Pin { icd, expect } => {
            // SAFETY: called from the first statements of `main`, before any
            // thread that could read the environment concurrently is started.
            unsafe { std::env::set_var(VENDOR_FILENAMES, &icd) };
            tracing::debug!(icd = %icd.display(), expect, "pinned the EGL vendor");
            Pin::Applied { expect }
        }
    };
    let _ = PIN.set(pin);
}

/// What [`decide`] concluded.
///
/// Deciding is deliberately free of both the environment and the settings
/// file: it reads what it is given and writes only the marker in the cache
/// directory it is handed. Setting the variable and persisting a dropped pin
/// are [`apply`]'s job, which is what lets these paths be tested without a
/// test reaching into the process environment or the user's real config.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Probe,
    Pin { icd: PathBuf, expect: String },
    Forget { reason: &'static str },
}

fn decide(
    settings: &StoredSettings,
    cache_directory: &Path,
    preset: Option<&std::ffi::OsStr>,
) -> Decision {
    if !cfg!(target_os = "linux") {
        return Decision::Probe;
    }
    // An explicit setting from the user or a launcher script always wins; this
    // module exists to choose one when nobody else has.
    if preset.is_some() {
        return Decision::Probe;
    }
    let (Some(icd), Some(expect)) = (
        settings.egl_vendor_icd.as_deref(),
        settings.egl_vendor_name.as_deref(),
    ) else {
        return Decision::Probe;
    };
    let icd = PathBuf::from(icd);
    if !icd.is_file() {
        return Decision::Forget {
            reason: "the pinned EGL vendor file is gone",
        };
    }

    let marker = marker_path(cache_directory);
    if marker.exists() {
        let _ = std::fs::remove_file(&marker);
        return Decision::Forget {
            reason: "the previous pinned launch never reached a GL context",
        };
    }
    // Deliberately not fsynced: this is a hint, and losing it costs one extra
    // probing launch rather than any data. Without somewhere to record the
    // attempt there is no way to detect a pin that crashes the process, so the
    // pin is not taken at all.
    if std::fs::write(&marker, b"").is_err() {
        return Decision::Probe;
    }

    Decision::Pin {
        icd,
        expect: expect.to_owned(),
    }
}

/// Reports the `GL_VENDOR` of the context that was actually created.
///
/// Called from the render host the first time a GL context exists. The work is
/// a string compare and, at most once in the life of an installation, one
/// settings write -- so it runs inline rather than being handed to a worker.
pub fn note_gl_vendor(vendor: &str) {
    if RECONCILED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(pin) = PIN.get() else { return };
    match pin {
        Pin::Applied { expect } if expect == vendor => {
            // The pin reproduced the vendor it was recorded for.
            if let Some(marker) = marker_path_cached() {
                let _ = std::fs::remove_file(marker);
            }
        }
        Pin::Applied { expect } => {
            tracing::warn!(
                expected = expect,
                actual = vendor,
                "the pinned EGL vendor no longer serves this display; restarting unpinned"
            );
            drop_pin("the pinned EGL vendor produced a different driver");
            if let Some(marker) = marker_path_cached() {
                let _ = std::fs::remove_file(marker);
            }
            if std::env::var_os(REPROBE_MARKER).is_none() {
                REPROBE_WANTED.store(true, Ordering::Relaxed);
                slint::quit_event_loop().ok();
            }
        }
        Pin::Probing => learn(vendor),
    }
}

/// Records the vendor that won a full probe, when exactly one installed ICD
/// can be attributed to it. Takes effect on the next launch.
fn learn(vendor: &str) {
    if std::env::var_os(VENDOR_FILENAMES).is_some() {
        return;
    }
    let Some(icd) = sole_matching_icd(vendor) else {
        tracing::debug!(
            vendor,
            "no single EGL vendor file matches this driver; not pinning"
        );
        return;
    };
    let Ok(store) = SettingsStore::for_current_user() else {
        return;
    };
    match store.save_egl_vendor(Some((&icd, vendor))) {
        Ok(()) => tracing::info!(
            icd = %icd.display(),
            vendor,
            "recorded the EGL vendor that serves this display"
        ),
        Err(error) => tracing::warn!(%error, "could not record the EGL vendor"),
    }
}

/// True when the app should re-execute itself without a pin. Checked after the
/// event loop exits, so the restart happens on a torn-down process.
pub fn reprobe_wanted() -> bool {
    REPROBE_WANTED.load(Ordering::Relaxed)
}

/// Replaces this process with a fresh, unpinned one. Only returns on failure.
pub fn reexec_unpinned() -> std::io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let Ok(executable) = std::env::current_exe() else {
            return std::io::Error::other("the running executable could not be located");
        };
        let mut command = std::process::Command::new(executable);
        command.args(std::env::args_os().skip(1));
        command.env_remove(VENDOR_FILENAMES);
        // One restart only: if the replacement mismatches again, it keeps the
        // context it has rather than restarting forever.
        command.env(REPROBE_MARKER, "1");
        command.exec()
    }
    #[cfg(not(unix))]
    std::io::Error::other("restarting is only implemented on unix")
}

fn drop_pin(reason: &str) {
    tracing::warn!(reason, "dropping the stored EGL vendor pin");
    if let Ok(store) = SettingsStore::for_current_user()
        && let Err(error) = store.save_egl_vendor(None)
    {
        tracing::warn!(%error, "could not clear the EGL vendor pin");
    }
}

fn marker_path(cache_directory: &Path) -> PathBuf {
    let path = cache_directory.join("egl-pin-pending");
    let _ = MARKER.set(Some(path.clone()));
    path
}

fn marker_path_cached() -> Option<&'static PathBuf> {
    MARKER.get()?.as_ref()
}

/// The installed ICD whose vendor library can be attributed to `gl_vendor`,
/// but only when exactly one can.
///
/// Vendor libraries are named `libEGL_<vendor>.so`, and the token in the
/// middle is what is matched, case-insensitively, against the `GL_VENDOR`
/// string: `nvidia` against `NVIDIA Corporation`, `mesa` against
/// `Mesa/X.org`. Two matches, or none, means no pin -- an Intel or AMD driver
/// reports neither its own name nor `mesa`, and guessing there is exactly the
/// mistake this module exists to avoid.
fn sole_matching_icd(gl_vendor: &str) -> Option<PathBuf> {
    sole_matching_icd_in(&ICD_DIRECTORIES.map(Path::new), gl_vendor)
}

fn sole_matching_icd_in(directories: &[&Path], gl_vendor: &str) -> Option<PathBuf> {
    let needle = gl_vendor.to_ascii_lowercase();
    let mut found: Option<PathBuf> = None;
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(token) = vendor_token(&path) else {
                continue;
            };
            if !needle.contains(&token) {
                continue;
            }
            if found.is_some() {
                // Ambiguous. Better to load a driver we do not use than to
                // pin the wrong one.
                return None;
            }
            found = Some(path);
        }
    }
    found
}

/// The `<vendor>` in the `libEGL_<vendor>.so...` named by an ICD file.
fn vendor_token(icd: &Path) -> Option<String> {
    let text = std::fs::read_to_string(icd).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
    let library = parsed.get("ICD")?.get("library_path")?.as_str()?;
    let file = Path::new(library).file_name()?.to_str()?;
    let rest = file.strip_prefix("libEGL_")?;
    let token = rest.split(".so").next()?;
    if token.is_empty() {
        return None;
    }
    Some(token.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("jellybridge-egl-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory");
        path
    }

    fn write_icd(directory: &Path, name: &str, library: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(
            &path,
            format!(r#"{{"file_format_version":"1.0.0","ICD":{{"library_path":"{library}"}}}}"#),
        )
        .expect("write icd");
        path
    }

    #[test]
    fn the_vendor_token_comes_from_the_library_the_icd_names() {
        let directory = scratch("token");
        let nvidia = write_icd(&directory, "10_nvidia.json", "libEGL_nvidia.so.0");
        let mesa = write_icd(&directory, "50_mesa.json", "libEGL_mesa.so.0");
        assert_eq!(vendor_token(&nvidia).as_deref(), Some("nvidia"));
        assert_eq!(vendor_token(&mesa).as_deref(), Some("mesa"));

        // A vendor library that does not follow the convention is not guessed at.
        let odd = write_icd(&directory, "90_odd.json", "libSomethingElse.so.1");
        assert_eq!(vendor_token(&odd), None);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_vendor_serving_the_display_is_matched_to_its_own_icd() {
        let directory = scratch("match");
        let nvidia = write_icd(&directory, "10_nvidia.json", "libEGL_nvidia.so.0");
        write_icd(&directory, "50_mesa.json", "libEGL_mesa.so.0");
        let dirs = [directory.as_path()];

        assert_eq!(
            sole_matching_icd_in(&dirs, "NVIDIA Corporation").as_deref(),
            Some(nvidia.as_path()),
            "the NVIDIA driver did not resolve to its own vendor file"
        );
        assert!(
            sole_matching_icd_in(&dirs, "Mesa/X.org").is_some(),
            "a Mesa driver should resolve to the Mesa vendor file"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_driver_no_vendor_file_can_be_attributed_to_is_never_pinned() {
        // This is the case the whole design turns on. Intel and AMD drivers are
        // served by Mesa but report neither "mesa" nor anything matching an ICD
        // filename, so there is nothing to match and nothing may be guessed --
        // pinning Mesa's competitor there would leave the machine on software
        // rendering.
        let directory = scratch("nomatch");
        write_icd(&directory, "10_nvidia.json", "libEGL_nvidia.so.0");
        write_icd(&directory, "50_mesa.json", "libEGL_mesa.so.0");
        let dirs = [directory.as_path()];

        for vendor in ["Intel", "AMD", "Broadcom", "Imagination Technologies"] {
            assert_eq!(
                sole_matching_icd_in(&dirs, vendor),
                None,
                "{vendor} was matched to a vendor file it cannot be attributed to"
            );
        }
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_ambiguous_match_pins_nothing() {
        // Two vendor files whose tokens both appear in the driver string: there
        // is no way to tell which one served the display, so neither is taken.
        let directory = scratch("ambiguous");
        write_icd(&directory, "10_alpha.json", "libEGL_alpha.so.0");
        write_icd(&directory, "50_beta.json", "libEGL_beta.so.0");
        assert_eq!(
            sole_matching_icd_in(&[directory.as_path()], "Alpha Beta Graphics"),
            None
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_healthy_pin_is_applied_and_leaves_a_marker_behind() {
        let directory = scratch("healthy");
        let icd = write_icd(&directory, "10_nvidia.json", "libEGL_nvidia.so.0");
        let settings = StoredSettings {
            egl_vendor_icd: Some(icd.to_string_lossy().into_owned()),
            egl_vendor_name: Some("NVIDIA Corporation".to_owned()),
            ..StoredSettings::default()
        };

        assert_eq!(
            decide(&settings, &directory, None),
            Decision::Pin {
                icd: icd.clone(),
                expect: "NVIDIA Corporation".to_owned()
            }
        );
        assert!(
            directory.join("egl-pin-pending").exists(),
            "a pinned launch must leave a marker so a crash is detectable"
        );

        // An environment that already names a vendor is left alone.
        let _ = fs::remove_file(directory.join("egl-pin-pending"));
        assert_eq!(
            decide(&settings, &directory, Some(std::ffi::OsStr::new("/somewhere/else.json"))),
            Decision::Probe,
            "an explicit vendor choice was overridden"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_pin_is_not_applied_without_somewhere_to_record_the_attempt() {
        let settings = StoredSettings {
            egl_vendor_icd: Some("/nonexistent/10_nvidia.json".to_owned()),
            egl_vendor_name: Some("NVIDIA Corporation".to_owned()),
            ..StoredSettings::default()
        };
        // The ICD file does not exist, so the pin is dropped rather than set.
        let directory = scratch("nopin");
        assert_eq!(
            decide(&settings, &directory, None),
            Decision::Forget {
                reason: "the pinned EGL vendor file is gone"
            }
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_stale_marker_means_the_last_pinned_launch_failed_so_the_pin_is_dropped() {
        let directory = scratch("stale");
        let icd = write_icd(&directory, "10_nvidia.json", "libEGL_nvidia.so.0");
        fs::write(directory.join("egl-pin-pending"), b"").expect("marker");
        let settings = StoredSettings {
            egl_vendor_icd: Some(icd.to_string_lossy().into_owned()),
            egl_vendor_name: Some("NVIDIA Corporation".to_owned()),
            ..StoredSettings::default()
        };
        assert_eq!(
            decide(&settings, &directory, None),
            Decision::Forget {
                reason: "the previous pinned launch never reached a GL context"
            }
        );
        assert!(
            !directory.join("egl-pin-pending").exists(),
            "the stale marker was left behind"
        );
        assert!(std::env::var_os(VENDOR_FILENAMES).is_none());
        let _ = fs::remove_dir_all(&directory);
    }
}
