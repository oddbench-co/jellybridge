use slint::platform::Key;

/// Rewind and fast-forward steps, as in Jellyfin's player.
pub const REWIND_SECONDS: f64 = 10.0;
pub const FAST_FORWARD_SECONDS: f64 = 30.0;

/// The speed menu's choices; `speed-index` in the window indexes this.
pub const PLAYBACK_SPEEDS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

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
    /// Seek to this fraction of the duration (the 0–9 keys).
    SeekToFraction(f64),
    SeekToStart,
    SeekToEnd,
    AdjustVolume(f32),
    /// Step through [`PLAYBACK_SPEEDS`] by this many entries.
    StepSpeed(i32),
    NextChapter,
    PreviousChapter,
    NextItem,
    PreviousItem,
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

/// Jellyfin web's player keyboard map, plus `a`/`c` to cycle tracks and `q`
/// to leave.
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

    let digit = text
        .chars()
        .next()
        .filter(|_| text.chars().count() == 1)
        .and_then(|character| character.to_digit(10));
    let action = if is_key(text, Key::Space)
        || text.eq_ignore_ascii_case("k")
        || is_key(text, Key::Pause)
    {
        Some(PlayerHotkeyAction::TogglePlayback)
    } else if is_key(text, Key::LeftArrow) {
        Some(PlayerHotkeyAction::SeekRelative(if shift {
            -5.0
        } else {
            -REWIND_SECONDS
        }))
    } else if is_key(text, Key::RightArrow) {
        Some(PlayerHotkeyAction::SeekRelative(if shift {
            5.0
        } else {
            FAST_FORWARD_SECONDS
        }))
    } else if text.eq_ignore_ascii_case("j") {
        Some(PlayerHotkeyAction::SeekRelative(-REWIND_SECONDS))
    } else if text.eq_ignore_ascii_case("l") {
        Some(PlayerHotkeyAction::SeekRelative(FAST_FORWARD_SECONDS))
    } else if let Some(digit) = digit {
        Some(PlayerHotkeyAction::SeekToFraction(f64::from(digit) / 10.0))
    } else if is_key(text, Key::UpArrow) {
        Some(PlayerHotkeyAction::AdjustVolume(5.0))
    } else if is_key(text, Key::DownArrow) {
        Some(PlayerHotkeyAction::AdjustVolume(-5.0))
    } else if is_key(text, Key::Home) {
        Some(PlayerHotkeyAction::SeekToStart)
    } else if is_key(text, Key::End) {
        Some(PlayerHotkeyAction::SeekToEnd)
    } else if text == ">" {
        Some(PlayerHotkeyAction::StepSpeed(1))
    } else if text == "<" {
        Some(PlayerHotkeyAction::StepSpeed(-1))
    } else if is_key(text, Key::PageUp) {
        Some(PlayerHotkeyAction::NextChapter)
    } else if is_key(text, Key::PageDown) {
        Some(PlayerHotkeyAction::PreviousChapter)
    } else if text == "N" || (shift && text == "n") {
        Some(PlayerHotkeyAction::NextItem)
    } else if text == "P" || (shift && text == "p") {
        Some(PlayerHotkeyAction::PreviousItem)
    } else if text.eq_ignore_ascii_case("m") {
        Some(PlayerHotkeyAction::ToggleMute)
    } else if text.eq_ignore_ascii_case("f") {
        Some(PlayerHotkeyAction::ToggleFullscreen)
    } else if is_key(text, Key::Escape) {
        Some(if fullscreen {
            PlayerHotkeyAction::ExitFullscreen
        } else {
            PlayerHotkeyAction::LeavePlayer
        })
    } else if text.eq_ignore_ascii_case("a") {
        Some(PlayerHotkeyAction::CycleAudio)
    } else if text.eq_ignore_ascii_case("c") {
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

/// The [`PLAYBACK_SPEEDS`] entry closest to what mpv reports.
pub fn speed_index(speed: f64) -> usize {
    PLAYBACK_SPEEDS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - speed).abs().total_cmp(&(*b - speed).abs()))
        .map_or(2, |(index, _)| index)
}

/// Moves `steps` entries through [`PLAYBACK_SPEEDS`], stopping at either end.
pub fn stepped_speed(current: f64, steps: i32) -> f64 {
    let last = PLAYBACK_SPEEDS.len() - 1;
    let index = speed_index(current).saturating_add_signed(steps as isize);
    PLAYBACK_SPEEDS[index.min(last)]
}

/// Where the chapter keys go: the next chapter start, or the start of the
/// current chapter -- the one before it when already within its first few
/// seconds, so pressing twice keeps going back.
pub fn chapter_target(chapters: &[f64], position: f64, forward: bool) -> Option<f64> {
    const RESTART_GRACE_SECONDS: f64 = 3.0;
    if forward {
        chapters.iter().copied().find(|time| *time > position + 1.0)
    } else {
        chapters
            .iter()
            .copied()
            .rev()
            .find(|time| *time < position - RESTART_GRACE_SECONDS)
            .or(Some(0.0))
    }
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

    /// The idle timer ran out. The overlay hides in windowed and fullscreen
    /// mode alike, unless something needs it to stay (`hold`): playback is
    /// paused, a menu is open, or the pointer rests on the controls.
    pub fn auto_hide_overlay(&mut self, hold: bool) {
        if !hold {
            self.overlay_visible = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key) -> String {
        char::from(key).to_string()
    }

    fn action(text: &str, shift: bool, fullscreen: bool) -> Option<PlayerHotkeyAction> {
        resolve_player_hotkey(text, false, shift, false, fullscreen).action
    }

    #[test]
    fn fullscreen_removes_normal_chrome_and_uses_only_the_overlay() {
        let mut state = PlayerUiState::default();
        state.set_fullscreen(true);

        assert!(state.fullscreen);
        assert!(!state.normal_chrome_visible);
        assert!(state.overlay_visible);
    }

    #[test]
    fn overlay_hides_when_idle_in_either_mode_unless_held() {
        let mut state = PlayerUiState::default();
        state.auto_hide_overlay(false);
        assert!(!state.overlay_visible, "windowed mode hides too");

        state.pointer_activity();
        assert!(state.overlay_visible);

        state.set_fullscreen(true);
        state.auto_hide_overlay(true);
        assert!(state.overlay_visible, "paused or menu open keeps it up");
        state.auto_hide_overlay(false);
        assert!(!state.overlay_visible);
    }

    #[test]
    fn jellyfin_player_hotkeys_map_to_expected_actions() {
        use PlayerHotkeyAction::*;
        assert_eq!(action(" ", false, false), Some(TogglePlayback));
        assert_eq!(action("k", false, false), Some(TogglePlayback));
        assert_eq!(
            action(&key(Key::LeftArrow), false, false),
            Some(SeekRelative(-10.0))
        );
        assert_eq!(
            action(&key(Key::RightArrow), false, false),
            Some(SeekRelative(30.0))
        );
        assert_eq!(
            action(&key(Key::RightArrow), true, false),
            Some(SeekRelative(5.0))
        );
        assert_eq!(action("j", false, false), Some(SeekRelative(-10.0)));
        assert_eq!(action("l", false, false), Some(SeekRelative(30.0)));
        assert_eq!(action("0", false, false), Some(SeekToFraction(0.0)));
        assert_eq!(action("7", false, false), Some(SeekToFraction(0.7)));
        assert_eq!(action(">", true, false), Some(StepSpeed(1)));
        assert_eq!(action("<", true, false), Some(StepSpeed(-1)));
        assert_eq!(action(&key(Key::PageUp), false, false), Some(NextChapter));
        assert_eq!(
            action(&key(Key::PageDown), false, false),
            Some(PreviousChapter)
        );
        assert_eq!(action("N", true, false), Some(NextItem));
        assert_eq!(action("P", true, false), Some(PreviousItem));
        assert_eq!(action("n", false, false), None);
        assert_eq!(action("M", false, false), Some(ToggleMute));
        assert_eq!(action("c", false, false), Some(CycleSubtitle));
        assert_eq!(action("a", false, false), Some(CycleAudio));
    }

    #[test]
    fn escape_exits_fullscreen_first_and_then_leaves() {
        assert_eq!(
            action(&key(Key::Escape), false, true),
            Some(PlayerHotkeyAction::ExitFullscreen)
        );
        assert_eq!(
            action(&key(Key::Escape), false, false),
            Some(PlayerHotkeyAction::LeavePlayer)
        );
    }

    #[test]
    fn hotkeys_preserve_shortcuts_and_suppress_repeated_toggles() {
        assert!(!resolve_player_hotkey("f", false, false, true, false).consumed);

        let repeat = resolve_player_hotkey(" ", true, false, false, false);
        assert!(repeat.consumed);
        assert_eq!(repeat.action, None);

        let repeat = resolve_player_hotkey("l", true, false, false, false);
        assert_eq!(
            repeat.action,
            Some(PlayerHotkeyAction::SeekRelative(30.0))
        );
    }

    #[test]
    fn track_cycling_wraps_and_ignores_single_track_lists() {
        assert_eq!(next_track_index(0, 3), Some(1));
        assert_eq!(next_track_index(2, 3), Some(0));
        assert_eq!(next_track_index(-1, 3), Some(0));
        assert_eq!(next_track_index(0, 1), None);
    }

    #[test]
    fn speed_steps_through_the_menu_and_stops_at_the_ends() {
        assert_eq!(speed_index(1.0), 2);
        assert_eq!(speed_index(1.3), 3);
        assert_eq!(stepped_speed(1.0, 1), 1.25);
        assert_eq!(stepped_speed(1.0, -1), 0.75);
        assert_eq!(stepped_speed(2.0, 1), 2.0);
        assert_eq!(stepped_speed(0.5, -1), 0.5);
    }

    #[test]
    fn chapter_keys_move_between_chapter_starts() {
        let chapters = [0.0, 100.0, 200.0];
        assert_eq!(chapter_target(&chapters, 50.0, true), Some(100.0));
        assert_eq!(chapter_target(&chapters, 250.0, true), None);
        assert_eq!(chapter_target(&chapters, 150.0, false), Some(100.0));
        // Just past a chapter start, going back skips to the one before.
        assert_eq!(chapter_target(&chapters, 101.0, false), Some(0.0));
        assert_eq!(chapter_target(&[], 30.0, false), Some(0.0));
    }
}
