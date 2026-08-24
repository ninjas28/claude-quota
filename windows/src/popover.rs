//! The popover, laid out like Claude's own usage screen.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/MenuContentView.swift`: a label
//! column with a countdown beneath it, a capsule bar, and "N% used" on the
//! right, grouped into the session window and the weekly ones.

use chrono::{DateTime, Utc};
use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, Ui, Vec2};

use crate::gauge;
use crate::model::State;
use crate::relative_time;
use crate::settings::ColorTheme;
use crate::snapshot::{UsageEntry, UsageError};

/// The popover's fixed width, matching the macOS one.
pub const WIDTH: f32 = 420.0;

/// Fixed columns so every bar starts and ends on the same x, however long the
/// labels are -- the alignment is most of what makes Claude's version read as a
/// table rather than a list.
mod column {
    /// Wide enough for "You haven't used Fable yet" on one line. Letting that
    /// wrap makes one row taller than the others and the table stops reading as
    /// a table.
    pub const LABEL: f32 = 152.0;
    pub const PERCENTAGE: f32 = 64.0;
}

const BAR_HEIGHT: f32 = 7.0;
const PADDING: f32 = 16.0;
/// The breathing room either side of the bar, between it and the label column
/// on the left and the percentage column on the right.
const GAP: f32 = 10.0;

/// How wide the bar can be, given what is left of the row when it starts.
///
/// Everything still to be placed after the bar has to be reserved here: the gap
/// before the percentage column, the column itself, *and* the right margin.
/// Missing any one of them does not clip anything -- it silently eats into the
/// margin, and the popover ends up with less padding on the right than on every
/// other side.
fn bar_width(available: f32) -> f32 {
    (available - GAP - column::PERCENTAGE - PADDING).max(20.0)
}

/// What the user clicked, handed back for the caller to act on -- the popover
/// itself owns no model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Refresh,
    OpenSettings,
    Quit,
}

/// What one frame of the popover produced: what the user clicked, and how tall
/// the content actually turned out to be.
///
/// The height matters because the window is sized before the content exists.
/// `height_for` is a decent estimate, but only egui knows what a wrapped error
/// message really came to -- so the app measures this and corrects the window
/// on the following frame.
pub struct Rendered {
    pub action: Option<Action>,
    pub height: f32,
}

pub fn show(ui: &mut Ui, state: &State, now: DateTime<Utc>) -> Rendered {
    let top = ui.min_rect().top();
    let mut action = None;
    let stale = state.is_stale();
    let theme = state.settings.color_theme;

    ui.spacing_mut().item_spacing = Vec2::ZERO;

    // Header.
    ui.horizontal(|ui| {
        ui.add_space(PADDING);
        ui.vertical(|ui| {
            ui.add_space(11.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Plan usage limits").size(13.0).strong());
                if let Some(plan) = state.snapshot.as_ref().and_then(|s| s.plan.as_deref()) {
                    ui.add_space(6.0);
                    ui.label(RichText::new(plan).size(12.0).weak());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(PADDING);
                    // A frameless button still carries its padding, which would
                    // hold the glyph a few points further in than the heading
                    // on the left sits out.
                    ui.spacing_mut().button_padding = Vec2::ZERO;
                    let refresh = ui.add_enabled(
                        !state.is_refreshing,
                        egui::Button::new(RichText::new("\u{27f3}").size(14.0)).frame(false),
                    );
                    if refresh.on_hover_text("Refresh now").clicked() {
                        action = Some(Action::Refresh);
                    }
                });
            });
            ui.add_space(11.0);
        });
    });
    separator(ui);

    match &state.snapshot {
        Some(snapshot) => {
            ui.add_space(PADDING);
            for entry in snapshot.session_windows() {
                window_row(ui, &entry, stale, theme, now);
                ui.add_space(18.0);
            }

            let weekly = snapshot.weekly_windows();
            if !weekly.is_empty() {
                indented(ui, |ui| {
                    ui.label(RichText::new("Weekly limits").size(12.0).strong());
                });
                ui.add_space(14.0);
                for entry in weekly {
                    window_row(ui, &entry, stale, theme, now);
                    ui.add_space(14.0);
                }
            }

            // Only shown when we actually know: the status line payload doesn't
            // report it, so this stays hidden on cached data.
            if let Some(extra) = snapshot.extra_usage_enabled {
                indented(ui, |ui| {
                    let text = if extra {
                        "\u{2295}  Extra usage on \u{2014} work continues past 100%"
                    } else {
                        "\u{2296}  Extra usage off \u{2014} requests stop at 100%"
                    };
                    ui.label(RichText::new(text).size(10.0).weak());
                });
                ui.add_space(6.0);
            }
            ui.add_space(PADDING - 6.0);
        }
        None => {
            ui.add_space(PADDING);
            indented(ui, |ui| {
                ui.label(RichText::new("No usage data yet").size(12.0).strong());
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Start a Claude Code session, or enable the usage API fallback in Settings.",
                    )
                    .size(11.0)
                    .weak(),
                );
            });
            ui.add_space(PADDING);
        }
    }

    if let Some(error) = state.visible_error() {
        separator(ui);
        ui.add_space(10.0);
        error_banner(ui, error);
        ui.add_space(10.0);
    }

    separator(ui);
    ui.add_space(10.0);
    if let Some(footer_action) = footer(ui, state, stale) {
        action = Some(footer_action);
    }
    ui.add_space(10.0);

    Rendered { action, height: ui.min_rect().bottom() - top }
}

fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(PADDING);
        ui.vertical(add);
    });
}

fn separator(ui: &mut Ui) {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, stroke);
}

fn window_row(ui: &mut Ui, entry: &UsageEntry, stale: bool, theme: ColorTheme, now: DateTime<Utc>) {
    let percentage = entry.window.clamped_percentage();

    ui.horizontal(|ui| {
        ui.add_space(PADDING);

        ui.allocate_ui_with_layout(
            Vec2::new(column::LABEL, 30.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.label(RichText::new(entry.label()).size(12.0));
                if let Some(subtitle) = entry.subtitle(now) {
                    ui.add_space(2.0);
                    ui.label(RichText::new(subtitle).size(10.0).weak());
                }
            },
        );

        ui.add_space(GAP);
        usage_bar(ui, percentage, stale, theme, bar_width(ui.available_width()));

        ui.add_space(GAP);
        ui.allocate_ui_with_layout(
            Vec2::new(column::PERCENTAGE, 16.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                // Right to left, so "used" is placed first and the number ends
                // up to its left. Only the digits are monospaced -- monospacing
                // the whole string opens a visible gap before the word, and
                // proportional digits are what make the column jitter as the
                // percentage changes. The spacing has to be set explicitly
                // because `show` zeroes it for the page as a whole, and a
                // leading space inside a label is laid out away to nothing.
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new("used").size(11.0).weak());
                ui.label(
                    RichText::new(format!("{}%", percentage.round() as i64))
                        .size(11.0)
                        .monospace()
                        .weak(),
                );
            },
        );
    });
}

/// The capsule bar used in the popover rows. Same rules as the tray icon: the
/// track is tinted with the fill colour, dropping to neutral grey when the
/// window is untouched -- that contrast is what makes an unused row read as
/// "nothing here" rather than "something, very small".
fn usage_bar(ui: &mut Ui, percentage: f64, stale: bool, theme: ColorTheme, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT), Sense::hover());
    let painter = ui.painter();
    let radius = CornerRadius::same((BAR_HEIGHT / 2.0) as u8);

    let [r, g, b] = gauge::rgb(percentage, theme);
    let track = if percentage > 0.0 {
        Color32::from_rgba_unmultiplied(r, g, b, 61)
    } else {
        // Neutral, and derived from the theme so it reads on light and dark.
        ui.visuals().weak_text_color().gamma_multiply(0.35)
    };
    painter.rect_filled(rect, radius, track);

    if percentage > 0.0 {
        // Never let a non-zero reading round away to an invisible sliver.
        let filled = (rect.width() * (percentage / 100.0) as f32).max(BAR_HEIGHT);
        let alpha = if stale { 115 } else { 255 };
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(filled, rect.height())),
            radius,
            Color32::from_rgba_unmultiplied(r, g, b, alpha),
        );
    }
}

fn error_banner(ui: &mut Ui, error: &UsageError) {
    ui.horizontal_top(|ui| {
        ui.add_space(PADDING);
        ui.label(RichText::new("\u{26a0}").size(12.0).color(Color32::from_rgb(255, 149, 0)));
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.set_max_width(WIDTH - PADDING * 2.0 - 24.0);
            ui.label(RichText::new(error.title()).size(11.0).strong());
            ui.add_space(2.0);
            ui.label(RichText::new(error.detail()).size(10.0).weak());
        });
    });
}

fn footer(ui: &mut Ui, state: &State, stale: bool) -> Option<Action> {
    let mut action = None;

    if let Some(snapshot) = &state.snapshot {
        ui.horizontal(|ui| {
            ui.add_space(PADDING);
            let dot = if stale {
                ui.visuals().weak_text_color()
            } else {
                Color32::from_rgb(52, 199, 89)
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(5.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 2.5, dot);

            ui.add_space(4.0);
            let mut line =
                format!("{} \u{b7} {}", relative_time::age(snapshot.age()), snapshot.source.label());
            if let Some(model) = state.context.as_ref().and_then(|c| c.model.as_deref()) {
                if snapshot.source == crate::snapshot::UsageSource::StatusLine {
                    line.push_str(&format!(" \u{b7} {model}"));
                }
            }
            ui.label(RichText::new(line).size(10.0).weak());
        });
        ui.add_space(8.0);
    }

    ui.horizontal(|ui| {
        ui.add_space(PADDING);
        if ui
            .add(egui::Button::new(RichText::new("Settings\u{2026}").size(11.0)).frame(false))
            .clicked()
        {
            action = Some(Action::OpenSettings);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(PADDING);
            if ui
                .add(egui::Button::new(RichText::new("Quit").size(11.0).weak()).frame(false))
                .clicked()
            {
                action = Some(Action::Quit);
            }
        });
    });

    action
}

/// How tall the popover needs to be for what it is about to show.
///
/// egui can size a window to its content, but only after a frame has been laid
/// out -- which on Windows means the popover visibly snaps to size the moment
/// it appears. Computing it up front from the row count keeps it still.
pub fn height_for(state: &State, now: DateTime<Utc>) -> f32 {
    let header = 11.0 + 18.0 + 11.0 + 1.0;
    let footer = 1.0 + 10.0 + 8.0 + 15.0 + 20.0 + 10.0;

    let body = match &state.snapshot {
        None => PADDING + 18.0 + 6.0 + 16.0 + PADDING,
        Some(snapshot) => {
            let row = |entry: &UsageEntry| {
                if entry.subtitle(now).is_some() {
                    18.0 + 2.0 + 14.0
                } else {
                    18.0
                }
            };
            let mut height = PADDING;
            for entry in snapshot.session_windows() {
                height += row(&entry) + 18.0;
            }
            let weekly = snapshot.weekly_windows();
            if !weekly.is_empty() {
                height += 17.0 + 14.0;
                for entry in weekly {
                    height += row(&entry) + 14.0;
                }
            }
            if snapshot.extra_usage_enabled.is_some() {
                height += 15.0 + 6.0;
            }
            height + PADDING - 6.0
        }
    };

    let error = match state.visible_error() {
        None => 0.0,
        Some(error) => {
            // The detail line wraps; two lines is the common case and three the
            // worst, so measure roughly rather than guessing one.
            let lines = (error.detail().len() as f32 / 62.0).ceil().max(1.0);
            1.0 + 10.0 + 15.0 + 2.0 + lines * 13.0 + 10.0
        }
    };

    header + body + error + footer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        UsageSnapshot, UsageSource, UsageWindow, UsageWindowKind,
    };
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    fn state_with(windows: Vec<UsageEntry>) -> State {
        State::preview(UsageSnapshot::new(windows, now(), UsageSource::OAuth))
    }

    /// Where a row's right edge lands, laid out the way `window_row` lays it
    /// out: left margin, label column, gap, bar, gap, percentage column.
    fn row_right_edge(total_width: f32) -> f32 {
        let after_label = total_width - PADDING - column::LABEL - GAP;
        PADDING + column::LABEL + GAP + bar_width(after_label) + GAP + column::PERCENTAGE
    }

    #[test]
    fn a_row_leaves_the_same_margin_on_the_right_as_on_the_left() {
        // The bug this pins: reserving PADDING for the right margin but then
        // spending GAP of it on the space before the percentage column, which
        // left 6pt on the right against 16pt everywhere else.
        for width in [WIDTH, 380.0, 500.0] {
            let margin = width - row_right_edge(width);
            assert!(
                (margin - PADDING).abs() < 0.01,
                "at {width}pt wide the right margin was {margin}, not {PADDING}"
            );
        }
    }

    #[test]
    fn a_bar_never_collapses_however_narrow_the_popover_gets() {
        assert!(bar_width(0.0) >= 20.0);
        assert!(bar_width(-500.0) >= 20.0);
    }

    #[test]
    fn the_popover_grows_with_what_it_has_to_show() {
        let resets = now() + chrono::Duration::hours(2);
        let one = state_with(vec![UsageEntry::new(
            UsageWindowKind::FiveHour,
            UsageWindow::new(23.0, Some(resets)),
        )]);
        let three = state_with(vec![
            UsageEntry::new(UsageWindowKind::FiveHour, UsageWindow::new(23.0, Some(resets))),
            UsageEntry::new(UsageWindowKind::SevenDay, UsageWindow::new(41.0, Some(resets))),
            UsageEntry::new(
                UsageWindowKind::WeeklyScoped,
                UsageWindow::scoped(8.0, Some(resets), Some("Fable".into())),
            ),
        ]);

        assert!(height_for(&three, now()) > height_for(&one, now()));
    }

    #[test]
    fn a_row_with_a_countdown_is_taller_than_one_without() {
        let bare = state_with(vec![UsageEntry::new(
            UsageWindowKind::FiveHour,
            UsageWindow::new(23.0, None),
        )]);
        let counting = state_with(vec![UsageEntry::new(
            UsageWindowKind::FiveHour,
            UsageWindow::new(23.0, Some(now() + chrono::Duration::hours(2))),
        )]);
        assert!(height_for(&counting, now()) > height_for(&bare, now()));
    }

    #[test]
    fn the_empty_state_reserves_room_for_both_of_its_lines() {
        // It is two lines of text, so it is legitimately taller than a single
        // bare row -- but it still has to be a sane popover size.
        let height = height_for(&State::default(), now());
        assert!((100.0..250.0).contains(&height), "{height}");
    }

    #[test]
    fn an_error_banner_makes_room_for_itself() {
        let mut state = state_with(vec![UsageEntry::new(
            UsageWindowKind::FiveHour,
            UsageWindow::new(23.0, None),
        )]);
        let without = height_for(&state, now());
        state.last_error = Some(UsageError::CredentialsExpired);
        assert!(height_for(&state, now()) > without);
    }

    #[test]
    fn every_reachable_state_has_a_finite_positive_height() {
        let mut state = State::default();
        assert!(height_for(&state, now()).is_finite() && height_for(&state, now()) > 0.0);

        state.last_error = Some(UsageError::NoData);
        assert!(height_for(&state, now()) > 0.0);
    }
}
