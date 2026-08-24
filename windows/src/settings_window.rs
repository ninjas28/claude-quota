//! The Settings window.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/SettingsView.swift` -- same rows, in
//! the same order, saying the same things. Rendered into whatever `Ui` the
//! caller hands it, so the same code serves the standalone window.

use egui::{RichText, Ui};

use crate::settings::{poll_interval, BarDisplayMode, ColorTheme, IndicatorStyle, Settings};

pub const SIZE: [f32; 2] = [460.0, 430.0];

/// True when the user changed something this frame, so the caller knows to
/// persist and re-render the tray icon.
pub fn show(ui: &mut Ui, settings: &mut Settings, launch_at_login: &mut bool) -> bool {
    let mut changed = false;

    ui.add_space(6.0);

    row(ui, "Indicator", None, |ui| {
        ui.horizontal(|ui| {
            for style in IndicatorStyle::ALL {
                changed |= ui
                    .selectable_value(&mut settings.indicator_style, style, style.label())
                    .changed();
            }
        });
        if settings.indicator_style == IndicatorStyle::Number {
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "A tray icon carries no text label, so the percentage is drawn inside it.",
                )
                .size(10.0)
                .weak(),
            );
        }
    });

    row(ui, "Colours", None, |ui| {
        ui.horizontal(|ui| {
            for theme in ColorTheme::ALL {
                changed |=
                    ui.selectable_value(&mut settings.color_theme, theme, theme.label()).changed();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new(settings.color_theme.detail()).size(10.0).weak());
    });

    row(ui, "Tray shows", None, |ui| {
        egui::ComboBox::from_id_salt("bar-display-mode")
            .selected_text(settings.bar_display_mode.label())
            .width(220.0)
            .show_ui(ui, |ui| {
                for mode in BarDisplayMode::ALL {
                    changed |= ui
                        .selectable_value(&mut settings.bar_display_mode, mode, mode.label())
                        .changed();
                }
            });
    });

    row(ui, "Check every", Some("Only when the status line cache has gone stale."), |ui| {
        egui::ComboBox::from_id_salt("poll-interval")
            .selected_text(poll_interval::label(settings.poll_interval_seconds))
            .width(220.0)
            .show_ui(ui, |ui| {
                for seconds in poll_interval::CHOICES {
                    changed |= ui
                        .selectable_value(
                            &mut settings.poll_interval_seconds,
                            seconds,
                            poll_interval::label(seconds),
                        )
                        .changed();
                }
            });
    });

    row(
        ui,
        "Usage API",
        Some(
            "Falls back to the endpoint /usage uses when no Claude Code session is running. \
             Costs no quota, but it is a network call.",
        ),
        |ui| {
            changed |= ui
                .checkbox(&mut settings.oauth_fallback_enabled, "Use the usage API fallback")
                .changed();
        },
    );

    row(ui, "Launch at login", Some("Registers under the per-user Run key."), |ui| {
        changed |= ui.checkbox(launch_at_login, "Start Claude Quota with Windows").changed();
    });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(
        RichText::new(match &settings.chain {
            Some(chain) => format!("Status line chains to: {chain}"),
            None => "Status line: no previous command to chain to.".to_string(),
        })
        .size(10.0)
        .weak(),
    );

    changed
}

fn row(ui: &mut Ui, label: &str, detail: Option<&str>, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::Vec2::new(110.0, 20.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.label(RichText::new(label).size(12.0).strong());
            },
        );
        ui.vertical(|ui| {
            ui.set_max_width(300.0);
            add(ui);
            if let Some(detail) = detail {
                ui.add_space(4.0);
                ui.label(RichText::new(detail).size(10.0).weak());
            }
        });
    });
    ui.add_space(16.0);
}
