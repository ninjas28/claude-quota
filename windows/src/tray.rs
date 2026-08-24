//! The notification-area icon: what `MenuBarExtra` is on macOS.
//!
//! `tray-icon` needs a win32 message loop on whichever thread creates the icon.
//! `eframe` runs one, so the icon is built inside its creation closure and its
//! events are forwarded into a channel the frame loop drains -- the same shape
//! as the Swift app's `onChange` hop to the main actor.

use std::sync::mpsc::{Receiver, Sender};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::gauge;
use crate::model::State;
use crate::relative_time;
use crate::settings::{ColorTheme, IndicatorStyle};
use crate::snapshot::UsageWindowKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    TogglePopover,
    Refresh,
    OpenSettings,
    /// The check item has already flipped its own state; the app reads it
    /// back and applies whatever it now says.
    ToggleAutostart,
    Quit,
}

/// Everything that changes what the icon looks like. Re-rasterizing on every
/// frame would be wasteful and, worse, makes the shell redraw the tray
/// constantly; this is what keeps updates to the frames that need one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IconKey {
    /// Rounded to whole percent: the icon cannot show finer than that anyway.
    percentage: Option<u8>,
    stale: bool,
    style: IndicatorStyle,
    theme: ColorTheme,
    size: u32,
}

pub struct Tray {
    icon: TrayIcon,
    autostart_item: CheckMenuItem,
    rendered: Option<IconKey>,
    tooltip: String,
}

struct Ids {
    refresh: MenuId,
    settings: MenuId,
    autostart: MenuId,
    quit: MenuId,
}

impl Tray {
    pub fn new(
        repaint: impl Fn() + Send + Sync + 'static,
        autostart: bool,
    ) -> Result<(Self, Receiver<TrayCommand>), String> {
        let (sender, receiver) = std::sync::mpsc::channel();

        let refresh = MenuItem::new("Refresh now", true, None);
        let settings = MenuItem::new("Settings\u{2026}", true, None);
        let autostart_item = CheckMenuItem::new("Launch at login", true, autostart, None);
        let quit = MenuItem::new("Quit", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &refresh,
            &settings,
            &PredefinedMenuItem::separator(),
            &autostart_item,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|error| error.to_string())?;

        let ids = Ids {
            refresh: refresh.id().clone(),
            settings: settings.id().clone(),
            autostart: autostart_item.id().clone(),
            quit: quit.id().clone(),
        };

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            // Left-click has to reach us as an event; the menu is the
            // right-click affordance Windows users expect.
            .with_menu_on_left_click(false)
            .with_tooltip("Claude Quota")
            .with_icon(to_icon(&gauge::render(
                None,
                true,
                IndicatorStyle::Ring,
                ColorTheme::Claude,
                crate::platform::tray_icon_size(),
            ))?)
            .build()
            .map_err(|error| error.to_string())?;

        install_handlers(sender, ids, repaint);

        Ok((
            Self {
                icon,
                autostart_item,
                rendered: None,
                tooltip: String::new(),
            },
            receiver,
        ))
    }

    /// Redraw the icon and tooltip if -- and only if -- something they show has
    /// actually changed.
    pub fn sync(&mut self, state: &State) {
        let displayed = state
            .snapshot
            .as_ref()
            .and_then(|s| state.settings.bar_display_mode.entry(s));
        let percentage = displayed.as_ref().map(|e| e.window.clamped_percentage());

        let key = IconKey {
            percentage: percentage.map(|p| p.round() as u8),
            stale: state.is_stale(),
            style: state.settings.indicator_style,
            theme: state.settings.color_theme,
            size: crate::platform::tray_icon_size(),
        };

        if self.rendered != Some(key) {
            let icon = gauge::render(percentage, key.stale, key.style, key.theme, key.size);
            if let Ok(icon) = to_icon(&icon) {
                let _ = self.icon.set_icon(Some(icon));
                self.rendered = Some(key);
            }
        }

        let tooltip = tooltip(state);
        if tooltip != self.tooltip {
            let _ = self.icon.set_tooltip(Some(&tooltip));
            self.tooltip = tooltip;
        }
    }

    pub fn set_autostart_checked(&self, checked: bool) {
        self.autostart_item.set_checked(checked);
    }

    pub fn autostart_checked(&self) -> bool {
        self.autostart_item.is_checked()
    }
}

/// Everything the macOS app can put beside the icon as text, plus the freshness
/// the popover footer shows. On Windows this is the only place a number can go
/// without opening the popover.
fn tooltip(state: &State) -> String {
    let Some(snapshot) = &state.snapshot else {
        return "Claude Quota \u{2014} no usage data yet".to_string();
    };

    let mut parts = Vec::new();
    if let Some(window) = snapshot.window(UsageWindowKind::FiveHour) {
        parts.push(format!(
            "Session {}%",
            window.clamped_percentage().round() as i64
        ));
    }
    if let Some(window) = snapshot.window(UsageWindowKind::SevenDay) {
        parts.push(format!(
            "Weekly {}%",
            window.clamped_percentage().round() as i64
        ));
    }
    for entry in snapshot.weekly_windows() {
        if let Some(scope) = &entry.window.scope_label {
            parts.push(format!(
                "{} {}%",
                scope,
                entry.window.clamped_percentage().round() as i64
            ));
        }
    }
    if parts.is_empty() {
        parts.push("Claude Quota".to_string());
    }
    parts.push(relative_time::age(snapshot.age()));
    parts.join(" \u{b7} ")
}

fn to_icon(icon: &gauge::Icon) -> Result<tray_icon::Icon, String> {
    tray_icon::Icon::from_rgba(icon.rgba.clone(), icon.size, icon.size)
        .map_err(|error| error.to_string())
}

/// Both handlers fire on the shell's thread, so they only post a message and
/// wake the frame loop -- no state is touched here.
/// The menu items themselves are `Rc`-backed and cannot cross a thread, so the
/// handler carries only their ids -- which are plain strings.
fn install_handlers(
    sender: Sender<TrayCommand>,
    ids: Ids,
    repaint: impl Fn() + Send + Sync + 'static,
) {
    let repaint = std::sync::Arc::new(repaint);

    let click_sender = sender.clone();
    let click_repaint = repaint.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        crate::debug_log!("tray event: {event:?}");
        // Act on release, not press: pressing is also how a drag starts, and
        // Windows 11 lets you drag the icon out of the overflow flyout.
        if let TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
        {
            let _ = click_sender.send(TrayCommand::TogglePopover);
            click_repaint();
        }
    }));

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = if event.id == ids.refresh {
            TrayCommand::Refresh
        } else if event.id == ids.settings {
            TrayCommand::OpenSettings
        } else if event.id == ids.autostart {
            TrayCommand::ToggleAutostart
        } else if event.id == ids.quit {
            TrayCommand::Quit
        } else {
            return;
        };
        let _ = sender.send(command);
        repaint();
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{UsageEntry, UsageSnapshot, UsageSource, UsageWindow};
    use chrono::Utc;

    #[test]
    fn the_tooltip_carries_every_window_and_how_fresh_it_is() {
        let state = State::preview(UsageSnapshot::new(
            vec![
                UsageEntry::new(UsageWindowKind::FiveHour, UsageWindow::new(23.4, None)),
                UsageEntry::new(UsageWindowKind::SevenDay, UsageWindow::new(41.6, None)),
                UsageEntry::new(
                    UsageWindowKind::WeeklyScoped,
                    UsageWindow::scoped(8.0, None, Some("Fable".into())),
                ),
            ],
            Utc::now(),
            UsageSource::StatusLine,
        ));

        let tooltip = tooltip(&state);
        assert!(tooltip.contains("Session 23%"), "{tooltip}");
        assert!(tooltip.contains("Weekly 42%"), "{tooltip}");
        assert!(tooltip.contains("Fable 8%"), "{tooltip}");
        assert!(tooltip.ends_with("just now"), "{tooltip}");
    }

    #[test]
    fn the_tooltip_says_so_when_there_is_nothing_to_show() {
        assert!(tooltip(&State::default()).contains("no usage data yet"));
    }

    #[test]
    fn a_windows_tray_tooltip_stays_within_the_shells_limit() {
        // Shell_NotifyIcon truncates a tooltip past 127 characters, and a
        // truncated one loses the freshness on the end -- the part that says
        // whether to trust the numbers.
        let mut windows = vec![
            UsageEntry::new(UsageWindowKind::FiveHour, UsageWindow::new(100.0, None)),
            UsageEntry::new(UsageWindowKind::SevenDay, UsageWindow::new(100.0, None)),
        ];
        for model in ["Opus 4.5", "Sonnet 4.5", "Fable 5"] {
            windows.push(UsageEntry::new(
                UsageWindowKind::WeeklyScoped,
                UsageWindow::scoped(100.0, None, Some(model.to_string())),
            ));
        }
        let state = State::preview(UsageSnapshot::new(windows, Utc::now(), UsageSource::OAuth));
        let tooltip = tooltip(&state);
        assert!(
            tooltip.chars().count() <= 127,
            "{} chars: {tooltip}",
            tooltip.chars().count()
        );
    }
}
