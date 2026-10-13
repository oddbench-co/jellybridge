use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSession {
    pub server_url: String,
    pub token: String,
    pub user_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSettings {
    /// Each library grid's sort and view, by page key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub grid_settings: HashMap<String, StoredGridSettings>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub playback_preferences: HashMap<String, StoredPlaybackPreference>,
    /// Settings jellyfin-web keeps per device (its browser storage): the
    /// settings pages' device-only fields, by jellyfin-web's own key.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub device: HashMap<String, String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// How one grid is sorted and drawn, in jellyfin-web's own vocabulary
/// ("SortName", "Descending", "posterCard").
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoredGridSettings {
    pub sort_by: String,
    pub descending: bool,
    pub view: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoredPlaybackPreference {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_title: Option<String>,
    pub subtitles_disabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("Could not locate the user configuration directory")]
    MissingConfigDirectory,
    #[error("Refusing to use a symbolic link for session storage")]
    SymbolicLink,
    #[error("Session storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Could not encode the saved session: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct SessionStore {
    path: PathBuf,
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn for_current_user() -> Result<Self, SessionError> {
        let directory = config_directory()?;
        Ok(Self {
            path: directory.join("session.json"),
        })
    }

    #[cfg(test)]
    fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Option<StoredSession>, SessionError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(SessionError::SymbolicLink);
        }
        if !metadata.is_file() {
            return Ok(None);
        }

        let mut data = String::new();
        File::open(&self.path)?.read_to_string(&mut data)?;
        match serde_json::from_str::<StoredSession>(&data) {
            Ok(session)
                if !session.server_url.trim().is_empty()
                    && !session.token.trim().is_empty()
                    && !session.user_id.trim().is_empty() =>
            {
                Ok(Some(session))
            }
            Ok(_) | Err(_) => {
                self.quarantine_malformed()?;
                Ok(None)
            }
        }
    }

    pub fn save(&self, session: &StoredSession) -> Result<(), SessionError> {
        let parent = self
            .path
            .parent()
            .ok_or(SessionError::MissingConfigDirectory)?;
        secure_directory(parent)?;
        refuse_symlink(&self.path)?;

        let temporary = parent.join(format!(
            ".session.json.tmp-{}-{}",
            std::process::id(),
            timestamp_nanos()
        ));
        let result = (|| -> Result<(), SessionError> {
            let mut file = secure_create(&temporary)?;
            serde_json::to_writer(&mut file, session)?;
            file.write_all(b"\n")?;
            file.flush()?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            sync_directory(parent)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn clear(&self) -> Result<(), SessionError> {
        refuse_symlink(&self.path)?;
        match fs::remove_file(&self.path) {
            Ok(()) => {
                if let Some(parent) = self.path.parent() {
                    sync_directory(parent)?;
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn quarantine_malformed(&self) -> Result<(), SessionError> {
        let parent = self
            .path
            .parent()
            .ok_or(SessionError::MissingConfigDirectory)?;
        let quarantine = parent.join(format!("session.json.invalid-{}", timestamp_nanos()));
        fs::rename(&self.path, quarantine)?;
        sync_directory(parent)?;
        Ok(())
    }
}

impl SettingsStore {
    pub fn for_current_user() -> Result<Self, SessionError> {
        Ok(Self {
            path: config_directory()?.join("settings.json"),
        })
    }

    #[cfg(test)]
    fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<StoredSettings, SessionError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoredSettings::default());
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(SessionError::SymbolicLink);
        }
        if !metadata.is_file() {
            return Ok(StoredSettings::default());
        }

        let mut data = String::new();
        File::open(&self.path)?.read_to_string(&mut data)?;
        match serde_json::from_str(&data) {
            Ok(settings) => Ok(settings),
            Err(_) => {
                self.quarantine_malformed()?;
                Ok(StoredSettings::default())
            }
        }
    }

    pub fn save_grid_settings(
        &self,
        grids: HashMap<String, StoredGridSettings>,
    ) -> Result<(), SessionError> {
        let mut settings = self.load()?;
        settings.grid_settings = grids;
        self.save(&settings)
    }

    /// Replaces the device-only settings with `device`.
    pub fn save_device_settings(
        &self,
        device: HashMap<String, String>,
    ) -> Result<(), SessionError> {
        let mut settings = self.load()?;
        settings.device = device;
        self.save(&settings)
    }

    pub fn save_playback_preference(
        &self,
        key: &str,
        preference: StoredPlaybackPreference,
    ) -> Result<(), SessionError> {
        let mut settings = self.load()?;
        settings
            .playback_preferences
            .insert(key.to_owned(), preference);
        self.save(&settings)
    }

    fn save(&self, settings: &StoredSettings) -> Result<(), SessionError> {
        let parent = self
            .path
            .parent()
            .ok_or(SessionError::MissingConfigDirectory)?;
        secure_directory(parent)?;
        refuse_symlink(&self.path)?;
        let temporary = parent.join(format!(
            ".settings.json.tmp-{}-{}",
            std::process::id(),
            timestamp_nanos()
        ));
        let result = (|| -> Result<(), SessionError> {
            let mut file = secure_create(&temporary)?;
            serde_json::to_writer_pretty(&mut file, settings)?;
            file.write_all(b"\n")?;
            file.flush()?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            sync_directory(parent)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn quarantine_malformed(&self) -> Result<(), SessionError> {
        let parent = self
            .path
            .parent()
            .ok_or(SessionError::MissingConfigDirectory)?;
        let quarantine = parent.join(format!("settings.json.invalid-{}", timestamp_nanos()));
        fs::rename(&self.path, quarantine)?;
        sync_directory(parent)?;
        Ok(())
    }
}

fn config_directory() -> Result<PathBuf, SessionError> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        Ok(PathBuf::from(path).join("jellybridge"))
    } else {
        let home = std::env::var_os("HOME").ok_or(SessionError::MissingConfigDirectory)?;
        Ok(PathBuf::from(home).join(".config/jellybridge"))
    }
}

pub fn cache_directory() -> Result<PathBuf, SessionError> {
    let directory = if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(path).join("jellybridge/images")
    } else {
        let home = std::env::var_os("HOME").ok_or(SessionError::MissingConfigDirectory)?;
        PathBuf::from(home).join(".cache/jellybridge/images")
    };
    secure_directory(&directory)?;
    Ok(directory)
}

fn refuse_symlink(path: &Path) -> Result<(), SessionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(SessionError::SymbolicLink),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn secure_directory(path: &Path) -> Result<(), SessionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(SessionError::SymbolicLink);
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "configuration path is not a directory",
            )
            .into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(path)?;
            }
            #[cfg(not(unix))]
            fs::create_dir_all(path)?;
        }
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn secure_create(path: &Path) -> Result<File, std::io::Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    File::open(path)?.sync_all()
}

fn timestamp_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "jellybridge-session-test-{name}-{}-{}",
            std::process::id(),
            timestamp_nanos()
        ))
    }

    #[test]
    fn atomically_round_trips_the_kotlin_compatible_session_shape() {
        let directory = test_directory("round-trip");
        let store = SessionStore::at(directory.join("session.json"));
        let session = StoredSession {
            server_url: "https://jellyfin.example".into(),
            token: "secret-token".into(),
            user_id: "user-1".into(),
        };
        store.save(&session).unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.server_url, session.server_url);
        assert_eq!(loaded.token, session.token);
        assert_eq!(loaded.user_id, session.user_id);
        let json = fs::read_to_string(directory.join("session.json")).unwrap();
        assert!(json.contains("\"serverUrl\""));
        assert!(!json.contains("server_url"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn quarantines_a_malformed_session() {
        let directory = test_directory("malformed");
        secure_directory(&directory).unwrap();
        let store = SessionStore::at(directory.join("session.json"));
        fs::write(directory.join("session.json"), b"not-json").unwrap();
        assert!(store.load().unwrap().is_none());
        assert!(fs::read_dir(&directory).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("session.json.invalid-")
        }));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn settings_preserve_kotlin_and_unknown_fields_when_catalog_preferences_change() {
        let directory = test_directory("settings-compatibility");
        secure_directory(&directory).unwrap();
        let store = SettingsStore::at(directory.join("settings.json"));
        fs::write(
            directory.join("settings.json"),
            br#"{
                "cardWidthDp": 220,
                "librarySortKey": "name_desc",
                "libraryPageSize": 120,
                "playbackPreferences": {"movie": {"audioLanguage": "jpn"}},
                "futureSetting": true
            }"#,
        )
        .unwrap();

        let grids = HashMap::from([(
            "lib:movies".to_owned(),
            StoredGridSettings {
                sort_by: "DateCreated".into(),
                descending: true,
                view: "posterCard".into(),
            },
        )]);
        store.save_grid_settings(grids).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(directory.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(value["gridSettings"]["lib:movies"]["sortBy"], "DateCreated");
        assert_eq!(value["gridSettings"]["lib:movies"]["view"], "posterCard");
        // Settings this version no longer reads survive a save.
        assert_eq!(value["cardWidthDp"], 220);
        assert_eq!(value["libraryPageSize"], 120);
        assert_eq!(
            value["playbackPreferences"]["movie"]["audioLanguage"],
            "jpn"
        );
        assert_eq!(value["futureSetting"], true);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn playback_preferences_round_trip_in_the_kotlin_compatible_shape() {
        let directory = test_directory("playback-preferences");
        let store = SettingsStore::at(directory.join("settings.json"));
        let preference = StoredPlaybackPreference {
            audio_language: Some("jpn".into()),
            audio_title: Some("5.1".into()),
            subtitle_language: Some("eng".into()),
            subtitle_title: Some("Signs & Songs".into()),
            subtitles_disabled: false,
        };

        store
            .save_playback_preference("series:show-1", preference.clone())
            .unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.playback_preferences.get("series:show-1"),
            Some(&preference)
        );
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(directory.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            value["playbackPreferences"]["series:show-1"]["audioLanguage"],
            "jpn"
        );
        assert_eq!(
            value["playbackPreferences"]["series:show-1"]["subtitlesDisabled"],
            false
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_settings_are_quarantined() {
        let directory = test_directory("malformed-settings");
        secure_directory(&directory).unwrap();
        let store = SettingsStore::at(directory.join("settings.json"));
        fs::write(directory.join("settings.json"), b"not-json").unwrap();
        let settings = store.load().unwrap();
        assert!(settings.grid_settings.is_empty());
        assert!(fs::read_dir(&directory).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("settings.json.invalid-")
        }));
        fs::remove_dir_all(directory).unwrap();
    }
}
