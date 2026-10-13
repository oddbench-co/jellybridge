//! Replays a scripted sequence of input events into the
//! window, so the real app can be driven (and screenshotted) on a hidden or
//! virtual display without touching anyone's mouse or keyboard.
//!
//! `JELLYBRIDGE_INPUT_SCRIPT=path` names a file with one event per line,
//! timed in seconds from the first frame:
//!
//! ```text
//! # time  command  arguments
//! 2.0  move   960 700        pointer to (960, 700) in points
//! 2.1  click  960 700        primary click there
//! 2.3  dclick 960 700        double click
//! 3.0  scroll 0 -300         wheel by (x, y) points
//! 3.5  key    Enter          press and release a key (egui key name)
//! 3.6  key    shift+N        with modifiers (shift, ctrl, alt)
//! 4.0  text   hello          type text
//! 5.0  play   <item id>      play that item, as a card's play button does
//! 5.0  open   <item id>      open a card on the current page, as a click does
//! 5.0  detail <Type> <id>    open an item's page (Type as Jellyfin names it)
//! 5.0  tab    <index>        switch the page's tab
//! 5.0  back                  go back
//! 5.0  search [term]         open search (and search for `term`)
//! 6.0  shot   <file.png>     save the window's next frame, pixel exact
//! ```
//!
//! `play` names the item instead of pointing at a card, so a scenario cannot
//! start whatever happens to sit at a spot after the home screen reorders.

use std::collections::VecDeque;

use super::app::UiAction;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, pos2, vec2};

pub struct InputScript {
    steps: VecDeque<(f64, Step)>,
    start: Option<f64>,
    last_now: f64,
    actions: Vec<UiAction>,
    shots: Vec<String>,
}

enum Step {
    Move(Pos2),
    Click(Pos2, u8),
    Scroll(egui::Vec2),
    Key(Key, Modifiers),
    Text(String),
    Play(String),
    Action(Box<UiAction>),
    Shot(String),
}

impl InputScript {
    /// The script named by `JELLYBRIDGE_INPUT_SCRIPT`, if any.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("JELLYBRIDGE_INPUT_SCRIPT")?;
        let text = std::fs::read_to_string(&path)
            .inspect_err(|error| tracing::warn!(%error, "input script unreadable"))
            .ok()?;
        let steps = text.lines().filter_map(parse_line).collect::<VecDeque<_>>();
        tracing::info!(steps = steps.len(), "input script loaded");
        Some(Self {
            steps,
            start: None,
            last_now: 0.0,
            actions: Vec::new(),
            shots: Vec::new(),
        })
    }

    /// Injects every step that is due.
    pub fn inject(&mut self, input: &mut RawInput) {
        // The script stands in for the user at the keyboard: the window
        // counts as focused even on a display nobody is looking at, so
        // focus-dependent drawing (a focused field's border) shows.
        input.focused = true;
        let now = input.time.unwrap_or(0.0);
        let start = *self.start.get_or_insert(now);
        while let Some((at, _)) = self.steps.front() {
            if start + at > now {
                break;
            }
            let (at, step) = self.steps.pop_front().expect("peeked");
            tracing::info!(at, "input script step");
            match step {
                Step::Play(id) => self.actions.push(UiAction::PlayItem(id)),
                Step::Action(action) => self.actions.push(*action),
                Step::Shot(path) => self.shots.push(path),
                step => push(input, step),
            }
        }
        self.last_now = now;
    }

    /// Screenshots due, taken once by the frame that requests them.
    pub fn take_shots(&mut self) -> Vec<String> {
        std::mem::take(&mut self.shots)
    }

    /// App actions due (play, navigate), taken once by the frame that
    /// dispatches them.
    pub fn take_actions(&mut self) -> Vec<UiAction> {
        std::mem::take(&mut self.actions)
    }

    /// Asks for a frame when the next step is due. Called during the frame:
    /// a request made from the input hook, before the pass begins, is lost.
    pub fn schedule(&self, ctx: &egui::Context) {
        if let (Some((at, _)), Some(start)) = (self.steps.front(), self.start) {
            let wait = (start + at - self.last_now).max(0.0);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }
}

fn push(input: &mut RawInput, step: Step) {
    match step {
        Step::Move(at) => input.events.push(Event::PointerMoved(at)),
        Step::Click(at, count) => {
            input.events.push(Event::PointerMoved(at));
            for _ in 0..count {
                for pressed in [true, false] {
                    input.events.push(Event::PointerButton {
                        pos: at,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    });
                }
            }
        }
        Step::Scroll(delta) => input.events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        }),
        Step::Key(key, modifiers) => {
            for pressed in [true, false] {
                input.events.push(Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers,
                });
            }
        }
        Step::Text(text) => input.events.push(Event::Text(text)),
        Step::Play(_) | Step::Action(_) | Step::Shot(_) => {}
    }
}

fn parse_line(line: &str) -> Option<(f64, Step)> {
    let line = line.split('#').next()?.trim();
    let mut parts = line.splitn(3, char::is_whitespace);
    let at: f64 = parts.next()?.parse().ok()?;
    let command = parts.next()?;
    let rest = parts.next().unwrap_or("").trim();
    let numbers = || -> Option<(f32, f32)> {
        let mut values = rest.split_whitespace().map(|value| value.parse::<f32>());
        Some((values.next()?.ok()?, values.next()?.ok()?))
    };
    let step = match command {
        "move" => numbers().map(|(x, y)| Step::Move(pos2(x, y)))?,
        "click" => numbers().map(|(x, y)| Step::Click(pos2(x, y), 1))?,
        "dclick" => numbers().map(|(x, y)| Step::Click(pos2(x, y), 2))?,
        "scroll" => numbers().map(|(x, y)| Step::Scroll(vec2(x, y)))?,
        "text" => Step::Text(rest.to_owned()),
        "play" => Step::Play(rest.to_owned()),
        "open" => Step::Action(Box::new(UiAction::OpenItem(rest.to_owned()))),
        "tab" => Step::Action(Box::new(UiAction::SwitchTab(rest.parse().ok()?))),
        "back" => Step::Action(Box::new(UiAction::GoBack)),
        "search" if rest.is_empty() => Step::Action(Box::new(UiAction::OpenSearch)),
        "settings" => {
            use crate::features::settings::SettingsPage;
            let page = match rest {
                "profile" => SettingsPage::Profile,
                "quickconnect" => SettingsPage::QuickConnect,
                "display" => SettingsPage::Display,
                "home" => SettingsPage::Home,
                "playback" => SettingsPage::Playback,
                "subtitles" => SettingsPage::Subtitles,
                _ => SettingsPage::Menu,
            };
            Step::Action(Box::new(UiAction::OpenSettings(page)))
        }
        "search" => Step::Action(Box::new(UiAction::Search(rest.to_owned()))),
        "detail" => {
            let (kind, id) = rest.split_once(char::is_whitespace)?;
            Step::Action(Box::new(UiAction::OpenDetail {
                id: id.trim().to_owned(),
                kind: kind.to_owned(),
            }))
        }
        "shot" => Step::Shot(rest.trim().to_owned()),
        "key" => {
            let mut modifiers = Modifiers::NONE;
            let mut name = rest;
            while let Some((modifier, remainder)) = name.split_once('+') {
                match modifier {
                    "shift" => modifiers.shift = true,
                    "ctrl" => modifiers.ctrl = true,
                    "alt" => modifiers.alt = true,
                    _ => return None,
                }
                name = remainder;
            }
            Step::Key(Key::from_name(name)?, modifiers)
        }
        _ => return None,
    };
    Some((at, step))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_lines_parse_into_timed_steps() {
        assert!(
            matches!(parse_line("1.5 click 10 20"), Some((at, Step::Click(_, 1))) if at == 1.5)
        );
        assert!(
            matches!(parse_line("2 key shift+N"), Some((_, Step::Key(Key::N, modifiers))) if modifiers.shift)
        );
        assert!(
            matches!(parse_line("3 text hello world"), Some((_, Step::Text(text))) if text == "hello world")
        );
        assert!(parse_line("# a comment").is_none());
        assert!(parse_line("1 bogus").is_none());
    }
}
