//! The settings pages' server side: the user's configuration and
//! jellyfin-web's synced display preferences, read and written whole.

use reqwest::Method;

use super::{ApiError, JellyfinApi, check_status, encode_path_segment};
use crate::features::pages::LibraryRef;
use crate::features::settings::{SettingsData, SettingsSave};

impl JellyfinApi {
    /// Everything the settings pages show, fetched at once.
    pub async fn settings_data(&self) -> Result<SettingsData, ApiError> {
        let user_path = format!("Users/{}", encode_path_segment(&self.session.user_id));
        let user = self.send_json::<serde_json::Value>(
            self.request(Method::GET, &user_path)?,
            "loading user settings",
        );
        let display = self.send_json::<serde_json::Value>(
            self.request(Method::GET, "DisplayPreferences/usersettings")?
                .query(&[
                    ("userId", self.session.user_id.as_str()),
                    ("client", "emby"),
                ]),
            "loading display preferences",
        );
        let views = self.send_json::<serde_json::Value>(
            self.request(Method::GET, &format!("{user_path}/Views"))?
                .query(&[("IncludeHidden", "true")]),
            "loading libraries",
        );
        let grouping = self.send_json::<serde_json::Value>(
            self.request(Method::GET, &format!("{user_path}/GroupingOptions"))?,
            "loading library folders",
        );
        let cultures = self.send_json::<serde_json::Value>(
            self.request(Method::GET, "Localization/Cultures")?,
            "loading languages",
        );
        let (user, display, views, grouping, cultures) =
            tokio::join!(user, display, views, grouping, cultures);
        let user = user?;
        let named = |value: &serde_json::Value| -> Vec<(String, String)> {
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    Some((
                        item["Id"].as_str()?.to_owned(),
                        item["Name"].as_str()?.to_owned(),
                    ))
                })
                .collect()
        };
        let libraries = views
            .map(|views| {
                views["Items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|view| {
                        Some(LibraryRef {
                            id: view["Id"].as_str()?.to_owned(),
                            name: view["Name"].as_str()?.to_owned(),
                            collection_type: view["CollectionType"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut languages: Vec<(String, String)> = cultures
            .map(|cultures| {
                cultures
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|culture| {
                        Some((
                            culture["DisplayName"].as_str()?.to_owned(),
                            culture["ThreeLetterISOLanguageName"].as_str()?.to_owned(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        languages.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(SettingsData {
            user_name: user["Name"].as_str().unwrap_or_default().to_owned(),
            image_tag: user["PrimaryImageTag"].as_str().map(str::to_owned),
            config: user["Configuration"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            display: display?,
            libraries,
            folders: grouping
                .map(|grouping| named(&grouping))
                .unwrap_or_default(),
            languages,
            device: Default::default(),
        })
    }

    /// Changes the user's password (Profile).
    pub async fn change_password(&self, current: &str, new: &str) -> Result<(), ApiError> {
        let response = self
            .request(
                Method::POST,
                &format!(
                    "Users/{}/Password",
                    encode_path_segment(&self.session.user_id)
                ),
            )?
            .json(&serde_json::json!({ "CurrentPw": current, "NewPw": new }))
            .send()
            .await?;
        check_status(response, "saving the password")?;
        Ok(())
    }

    /// Lets another device in with the Quick Connect code it shows.
    pub async fn authorize_quick_connect(&self, code: &str) -> Result<bool, ApiError> {
        self.send_json(
            self.request(Method::POST, "QuickConnect/Authorize")?
                .query(&[("code", code), ("userId", self.session.user_id.as_str())]),
            "authorizing the device",
        )
        .await
    }

    /// Sets the user's picture: the file's bytes, base64 encoded, as
    /// jellyfin-web uploads them.
    pub async fn upload_user_image(&self, bytes: &[u8], mime: &str) -> Result<(), ApiError> {
        use base64::Engine as _;
        let body = base64::engine::general_purpose::STANDARD.encode(bytes);
        let response = self
            .request(
                Method::POST,
                &format!(
                    "Users/{}/Images/Primary",
                    encode_path_segment(&self.session.user_id)
                ),
            )?
            .header(reqwest::header::CONTENT_TYPE, mime)
            .body(body)
            .send()
            .await?;
        check_status(response, "uploading the image")?;
        Ok(())
    }

    pub async fn delete_user_image(&self) -> Result<(), ApiError> {
        let response = self
            .request(
                Method::DELETE,
                &format!(
                    "Users/{}/Images/Primary",
                    encode_path_segment(&self.session.user_id)
                ),
            )?
            .send()
            .await?;
        check_status(response, "deleting the image")?;
        Ok(())
    }

    /// The user's picture.
    pub async fn fetch_user_image(&self, tag: &str) -> Result<Vec<u8>, ApiError> {
        let response = self
            .request(Method::GET, "UserImage")?
            .query(&[
                ("userId", self.session.user_id.as_str()),
                ("tag", tag),
                ("quality", "90"),
            ])
            .send()
            .await?;
        let response = check_status(response, "loading the user's image")?;
        Ok(response.bytes().await?.to_vec())
    }

    /// Writes a settings page's configuration and display preferences.
    pub async fn save_settings(&self, save: &SettingsSave) -> Result<(), ApiError> {
        let config = self
            .request(
                Method::POST,
                &format!(
                    "Users/{}/Configuration",
                    encode_path_segment(&self.session.user_id)
                ),
            )?
            .json(&save.config)
            .send();
        let display = self
            .request(Method::POST, "DisplayPreferences/usersettings")?
            .query(&[
                ("userId", self.session.user_id.as_str()),
                ("client", "emby"),
            ])
            .json(&save.display)
            .send();
        let (config, display) = tokio::join!(config, display);
        check_status(config?, "saving user settings")?;
        check_status(display?, "saving display preferences")?;
        Ok(())
    }
}
