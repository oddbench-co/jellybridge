use slint::platform::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerUiState {
    pub fullscreen: bool,
    pub normal_chrome_visible: bool,
    pub overlay_visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerHotkeyAction {
    TogglePlayback,
    SeekRelative(f64),
    SeekToStart,
    SeekToEnd,
    AdjustVolume(f32),
    ToggleMute,
    ToggleFullscreen,
    ExitFullscreen,
    CycleAudio,
    CycleSubtitle,
    LeavePlayer,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HotkeyDecision {
    pub consumed: bool,
    pub action: Option<PlayerHotkeyAction>,
}

pub fn resolve_player_hotkey(
    text: &str,
    repeat: bool,
    shift: bool,
    command_modifier: bool,
    fullscreen: bool,
) -> HotkeyDecision {
    if command_modifier {
        return HotkeyDecision::default();
    }

    let action = if is_key(text, Key::Space)
        || text.eq_ignore_ascii_case("k")
        || text.eq_ignore_ascii_case("p")
        || is_key(text, Key::Pause)
    {
        Some(PlayerHotkeyAction::TogglePlayback)
    } else if is_key(text, Key::LeftArrow) {
        Some(PlayerHotkeyAction::SeekRelative(if shift {
            -30.0
        } else {
            -5.0
        }))
    } else if is_key(text, Key::RightArrow) {
        Some(PlayerHotkeyAction::SeekRelative(if shift {
            30.0
        } else {
            5.0
        }))
    } else if is_key(text, Key::UpArrow) {
        Some(PlayerHotkeyAction::AdjustVolume(5.0))
    } else if is_key(text, Key::DownArrow) {
        Some(PlayerHotkeyAction::AdjustVolume(-5.0))
    } else if is_key(text, Key::Home) {
        Some(PlayerHotkeyAction::SeekToStart)
    } else if is_key(text, Key::End) {
        Some(PlayerHotkeyAction::SeekToEnd)
    } else if text == "0" {
        Some(PlayerHotkeyAction::AdjustVolume(2.0))
    } else if text == "9" {
        Some(PlayerHotkeyAction::AdjustVolume(-2.0))
    } else if text.eq_ignore_ascii_case("m") {
        Some(PlayerHotkeyAction::ToggleMute)
    } else if text.eq_ignore_ascii_case("f") {
        Some(PlayerHotkeyAction::ToggleFullscreen)
    } else if is_key(text, Key::Escape) && fullscreen {
        Some(PlayerHotkeyAction::ExitFullscreen)
    } else if text.eq_ignore_ascii_case("a") {
        Some(PlayerHotkeyAction::CycleAudio)
    } else if text.eq_ignore_ascii_case("j") {
        Some(PlayerHotkeyAction::CycleSubtitle)
    } else if text.eq_ignore_ascii_case("q") {
        Some(PlayerHotkeyAction::LeavePlayer)
    } else {
        None
    };

    let Some(action) = action else {
        return HotkeyDecision::default();
    };
    let repeats = matches!(
        action,
        PlayerHotkeyAction::SeekRelative(_) | PlayerHotkeyAction::AdjustVolume(_)
    );
    HotkeyDecision {
        consumed: true,
        action: (!repeat || repeats).then_some(action),
    }
}

pub fn next_track_index(current: i32, count: usize) -> Option<i32> {
    let count = i32::try_from(count).ok()?;
    if count < 2 {
        return None;
    }
    Some(if current < 0 || current + 1 >= count {
        0
    } else {
        current + 1
    })
}

fn is_key(text: &str, key: Key) -> bool {
    let mut characters = text.chars();
    characters.next() == Some(char::from(key)) && characters.next().is_none()
}

impl Default for PlayerUiState {
    fn default() -> Self {
        Self {
            fullscreen: false,
            normal_chrome_visible: true,
            overlay_visible: true,
        }
    }
}

impl PlayerUiState {
    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = fullscreen;
        self.normal_chrome_visible = !fullscreen;
        self.overlay_visible = true;
    }

    pub fn pointer_activity(&mut self) {
        self.overlay_visible = true;
    }

    pub fn auto_hide_overlay(&mut self) {
        if self.fullscreen {
            self.overlay_visible = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_removes_normal_chrome_and_uses_only_the_overlay() {
        let mut state = PlayerUiState::default();
        state.set_fullscreen(true);

        assert!(state.fullscreen);
        assert!(!state.normal_chrome_visible);
        assert!(state.overlay_visible);
    }

    #[test]
    fn fullscreen_overlay_hides_until_the_next_pointer_activity() {
        let mut state = PlayerUiState::default();
        state.set_fullscreen(true);
        state.auto_hide_overlay();
        assert!(!state.overlay_visible);

        state.pointer_activity();
        assert!(state.overlay_visible);

        state.set_fullscreen(false);
        state.auto_hide_overlay();
        assert!(state.normal_chrome_visible);
        assert!(state.overlay_visible);
    }

    #[test]
    fn standard_player_hotkeys_map_to_expected_actions() {
        assert_eq!(
            resolve_player_hotkey(" ", false, false, false, false).action,
            Some(PlayerHotkeyAction::TogglePlayback)
        );
        assert_eq!(
            resolve_player_hotkey(
                &char::from(Key::LeftArrow).to_string(),
                false,
                false,
                false,
                false
            )
            .action,
            Some(PlayerHotkeyAction::SeekRelative(-5.0))
        );
        assert_eq!(
            resolve_player_hotkey(
                &char::from(Key::RightArrow).to_string(),
                false,
                true,
                false,
                false
            )
            .action,
            Some(PlayerHotkeyAction::SeekRelative(30.0))
        );
        assert_eq!(
            resolve_player_hotkey("M", false, false, false, false).action,
            Some(PlayerHotkeyAction::ToggleMute)
        );
        assert_eq!(
            resolve_player_hotkey("j", false, false, false, false).action,
            Some(PlayerHotkeyAction::CycleSubtitle)
        );
    }

    #[test]
    fn hotkeys_preserve_shortcuts_and_suppress_repeated_toggles() {
        assert!(!resolve_player_hotkey("f", false, false, true, false).consumed);

        let repeat = resolve_player_hotkey(" ", true, false, false, false);
        assert!(repeat.consumed);
        assert_eq!(repeat.action, None);

        assert!(
            !resolve_player_hotkey(
                &char::from(Key::Escape).to_string(),
                false,
                false,
                false,
                false
            )
            .consumed
        );
        assert_eq!(
            resolve_player_hotkey(
                &char::from(Key::Escape).to_string(),
                false,
                false,
                false,
                true
            )
            .action,
            Some(PlayerHotkeyAction::ExitFullscreen)
        );
    }

    #[test]
    fn track_cycling_wraps_and_ignores_single_track_lists() {
        assert_eq!(next_track_index(0, 3), Some(1));
        assert_eq!(next_track_index(2, 3), Some(0));
        assert_eq!(next_track_index(-1, 3), Some(0));
        assert_eq!(next_track_index(0, 1), None);
    }
}
