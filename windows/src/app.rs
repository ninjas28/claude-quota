//! The tray app: an `eframe` event loop that owns the tray icon, the popover,
//! and the settings window.
//!
//! Port of `ClaudeQuotaBarApp.swift`. `MenuBarExtra` gives macOS the popover
//! for free -- placement, dismissal, and all. Here the window is an ordinary
//! viewport that we position, show, and hide by hand.
//!
//! There is one window, not two. The obvious design -- popover in the main
//! viewport, settings in a child viewport -- cannot work: egui only builds a
//! child viewport while the parent is painting, and a hidden window is never
//! asked to paint, so choosing Settings from a closed popover would open
//! nothing at all. Instead the single window changes shape: undecorated and
//! always-on-top by the tray for the popover, decorated and centred with a
//! title bar for settings.
//!
//! egui 0.35 splits a frame into `logic` (no drawing) and `ui` (drawing only),
//! which happens to match the split this app already wanted: everything that
//! talks to the shell -- tray events, showing and hiding the window -- belongs
//! in `logic`, and `ui` only renders whatever state that left behind.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::Utc;
use egui::{
    containers::Frame, Color32, CornerRadius, Margin, Ui, ViewportBuilder, ViewportCommand,
    WindowLevel,
};

use crate::model::UsageModel;
use crate::platform;
use crate::popover;
use crate::settings::Settings;
use crate::settings_window;
use crate::tray::{Tray, TrayCommand};

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([popover::WIDTH, 320.0])
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_always_on_top()
            // No taskbar button and no Alt-Tab entry: this is a status
            // indicator, not an app you switch to. The macOS equivalent is
            // LSUIElement / `.accessory`.
            .with_taskbar(false)
            .with_visible(false),
        ..Default::default()
    };

    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

/// The window's name outside settings. Not shown anywhere in the popover --
/// it is undecorated -- but it is what Alt-Tab, Task Manager, and anything
/// enumerating windows sees.
const WINDOW_TITLE: &str = "Claude Quota";

/// What the one window is currently being.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Hidden,
    Popover,
    Settings,
}

struct App {
    model: UsageModel,
    tray: Option<Tray>,
    commands: Option<std::sync::mpsc::Receiver<TrayCommand>>,
    exe: PathBuf,
    autostart: bool,
    mode: Mode,
    /// Guards the click that closes the popover from immediately reopening it:
    /// clicking the tray icon takes focus away from the popover, which hides
    /// it, and then the click event arrives and would toggle it back on.
    hidden_at: Option<Instant>,
    /// The window is not focused on the frame it becomes visible, and treating
    /// that as focus loss would close it before it ever appeared.
    shown_at: Option<Instant>,
    /// Frames left to keep asking for the foreground. One attempt is usually
    /// enough, but the shell is still finishing its own click when the first
    /// one lands, so a few retries make it reliable.
    focus_attempts: u8,
    /// Whether this showing has ever actually held focus. Dismiss-on-focus-loss
    /// is only armed once it has -- otherwise a popover that was refused the
    /// foreground would close itself on the very next frame, which is exactly
    /// the bug the retries above are there to avoid.
    ever_focused: bool,
    /// Where the tray click was, so the popover can be re-placed whenever its
    /// height changes without waiting for another click.
    anchor: (f32, f32),
    /// The height the popover is currently sized to, in points.
    placed_height: f32,
    /// What the last frame measured the popover content to actually be.
    measured_height: Option<f32>,
    /// The last size and position actually sent to the window, so a repeat is
    /// dropped instead of becoming another `SetWindowPos` the user can see.
    placed: Option<(f32, f32, f32)>,
    /// Set while the popover is parked off-screen being measured. Carries the
    /// estimate it was parked at, so the measurement can be filed against it.
    measuring: Option<(Instant, f32)>,
    /// The last (estimate, measured) pair. When a later open estimates the same
    /// height, the content has the same shape and we already know exactly how
    /// tall it comes out -- so it can be placed correctly first time.
    last_measurement: Option<(f32, f32)>,
    /// Set when the user really means to exit, so the close request that
    /// follows is allowed through instead of being turned into "hide".
    quitting: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let exe = std::env::current_exe().unwrap_or_default();
        let autostart = autostart_enabled();

        let ctx = cc.egui_ctx.clone();
        let model = UsageModel::start(settings, move || ctx.request_repaint());

        // Must be built here: `tray-icon` requires the thread that creates the
        // icon to be running a win32 message loop, and this closure runs on
        // eframe's.
        let ctx = cc.egui_ctx.clone();
        let (tray, commands) = match Tray::new(move || ctx.request_repaint(), autostart) {
            Ok((tray, commands)) => (Some(tray), Some(commands)),
            Err(error) => {
                eprintln!("claude-quota: could not create the tray icon: {error}");
                (None, None)
            }
        };

        Self {
            model,
            tray,
            commands,
            exe,
            autostart,
            mode: Mode::Hidden,
            hidden_at: None,
            shown_at: None,
            focus_attempts: 0,
            ever_focused: false,
            anchor: (0.0, 0.0),
            placed_height: 0.0,
            measured_height: None,
            placed: None,
            measuring: None,
            last_measurement: None,
            quitting: false,
        }
    }

    /// Size the popover and put it where the tray click was.
    ///
    /// Order matters. A window that starts life hidden comes back from
    /// `Visible(true)` still iconic -- `ShowWindow(SW_SHOW)` restores whatever
    /// show state it had -- and Windows ignores move and resize requests aimed
    /// at a minimized window. So un-minimize first, and only then say where it
    /// goes.
    /// Move and size the window, skipping the call when nothing would change.
    ///
    /// Every one of these is a `SetWindowPos` the user can see, so sending the
    /// same one twice is a visible stutter rather than a harmless no-op.
    fn place_at(&mut self, ctx: &egui::Context, height: f32, position: (f32, f32)) {
        if self.placed == Some((height, position.0, position.1)) {
            return;
        }
        let scale = ctx.pixels_per_point().max(0.5);
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(
            popover::WIDTH,
            height,
        )));
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(
            position.0 / scale,
            position.1 / scale,
        )));
        self.placed = Some((height, position.0, position.1));
        self.placed_height = height;
        crate::debug_log!("place at {:?} h={height} ppp={scale}", position);
    }

    /// Where a popover of this height belongs, relative to the tray click.
    fn popover_position(&self, ctx: &egui::Context, height: f32) -> (f32, f32) {
        let scale = ctx.pixels_per_point().max(0.5);
        let physical = (popover::WIDTH * scale, height * scale);
        platform::popover_position(self.anchor, physical, platform::work_area())
    }

    /// Somewhere the window can be visible -- so egui is asked to paint it, and
    /// its real height can be measured -- without the user seeing it.
    ///
    /// This exists because a hidden window is never painted, so there is no way
    /// to lay the popover out before showing it. Parking it below the desktop
    /// for one frame is what stops the first open of a given shape from
    /// appearing at the estimated height and then visibly jumping to the real
    /// one.
    fn offscreen(&self) -> (f32, f32) {
        let (left, _, _, bottom) = platform::work_area();
        (left, bottom + 2_000.0)
    }

    fn show_popover(&mut self, ctx: &egui::Context) {
        let estimate = {
            let state = self.model.state();
            popover::height_for(&state, Utc::now())
        };

        self.anchor = platform::cursor_position();
        // Every one of these undoes something `open_settings` did. The title in
        // particular has no visible home in an undecorated window, which is
        // exactly why forgetting it is easy: the popover would keep reading as
        // "Claude Quota Settings" in Alt-Tab and Task Manager for the rest of
        // the session, having been to settings once.
        ctx.send_viewport_cmd(ViewportCommand::Title(WINDOW_TITLE.to_string()));
        ctx.send_viewport_cmd(ViewportCommand::Decorations(false));
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));

        self.mode = Mode::Popover;
        self.measured_height = None;
        self.placed = None;

        // A shape we have already measured can go straight to its final place.
        let known = self
            .last_measurement
            .filter(|(cached, _)| (cached - estimate).abs() < 0.5)
            .map(|(_, measured)| measured);

        match known {
            Some(height) => {
                let position = self.popover_position(ctx, height);
                self.place_at(ctx, height, position);
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                self.reveal(ctx);
            }
            None => {
                let offscreen = self.offscreen();
                self.place_at(ctx, estimate, offscreen);
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                self.measuring = Some((Instant::now(), estimate));
                ctx.request_repaint();
            }
        }

        // Opening it is a good moment to check we are not showing something old.
        self.model.refresh(false);
    }

    /// The popover is now where it belongs; let it take focus and start
    /// counting as shown.
    fn reveal(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.shown_at = Some(Instant::now());
        self.focus_attempts = 3;
        self.ever_focused = false;
    }

    /// Turn the window into a normal, decorated, centred settings window.
    fn open_settings(&mut self, ctx: &egui::Context) {
        let scale = ctx.pixels_per_point().max(0.5);
        let (width, height) = (settings_window::SIZE[0], settings_window::SIZE[1]);
        let (left, top, right, bottom) = platform::work_area();
        let x = (left + right) / 2.0 - width * scale / 2.0;
        let y = (top + bottom) / 2.0 - height * scale / 2.0;

        ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(WindowLevel::Normal));
        ctx.send_viewport_cmd(ViewportCommand::Title("Claude Quota Settings".to_string()));
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(width, height)));
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(
            x / scale,
            y / scale,
        )));
        ctx.send_viewport_cmd(ViewportCommand::Focus);

        crate::debug_log!("open settings at ({x}, {y})");
        self.mode = Mode::Settings;
        self.shown_at = Some(Instant::now());
        self.focus_attempts = 6;
        self.ever_focused = false;
    }

    fn hide(&mut self, ctx: &egui::Context) {
        if self.mode == Mode::Hidden {
            return;
        }
        crate::debug_log!("hide ({:?})", self.mode);
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.mode = Mode::Hidden;
        self.hidden_at = Some(Instant::now());
        self.focus_attempts = 0;
        self.measuring = None;
        self.measured_height = None;
        self.placed = None;
    }

    fn toggle_popover(&mut self, ctx: &egui::Context) {
        match self.mode {
            Mode::Popover => self.hide(ctx),
            // A tray click while settings is open should bring settings
            // forward rather than swap the window out from under it.
            Mode::Settings => {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                self.focus_attempts = 4;
            }
            Mode::Hidden => {
                // The focus-loss hide has just run for this very click.
                if self
                    .hidden_at
                    .map(|at| at.elapsed() < Duration::from_millis(300))
                    .unwrap_or(false)
                {
                    return;
                }
                self.show_popover(ctx);
            }
        }
    }

    fn set_autostart(&mut self, enabled: bool) {
        self.autostart = enabled;
        if let Err(error) = set_autostart(enabled, &self.exe) {
            eprintln!("claude-quota: could not update launch at login: {error}");
            self.autostart = autostart_enabled();
        }
        if let Some(tray) = &self.tray {
            tray.set_autostart_checked(self.autostart);
        }
    }

    fn handle(&mut self, command: TrayCommand, ctx: &egui::Context) {
        match command {
            TrayCommand::TogglePopover => self.toggle_popover(ctx),
            TrayCommand::Refresh => self.model.refresh_now(),
            TrayCommand::OpenSettings => self.open_settings(ctx),
            TrayCommand::ToggleAutostart => {
                // The check item flipped itself when it was clicked, so what it
                // now reads is what the user asked for.
                let wanted = self.tray.as_ref().map(|tray| tray.autostart_checked());
                if let Some(wanted) = wanted {
                    self.set_autostart(wanted);
                }
            }
            TrayCommand::Quit => self.quit(ctx),
        }
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    fn draw_popover(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let visuals = ctx.global_style().visuals.clone();
        let frame = Frame::default()
            .fill(visuals.window_fill)
            .stroke(visuals.window_stroke)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::ZERO);

        let rendered = {
            let state = self.model.state();
            egui::CentralPanel::default()
                .frame(frame)
                .show(ui, |ui| popover::show(ui, &state, Utc::now()))
                .inner
        };
        self.measured_height = Some(rendered.height);

        match rendered.action {
            Some(popover::Action::Refresh) => self.model.refresh_now(),
            Some(popover::Action::OpenSettings) => self.open_settings(&ctx),
            Some(popover::Action::Quit) => self.quit(&ctx),
            None => {}
        }
    }

    fn draw_settings(&mut self, ui: &mut Ui) {
        let mut settings = self.model.state().settings.clone();
        let mut autostart = self.autostart;

        let fill = ui.ctx().global_style().visuals.window_fill;
        let changed = egui::CentralPanel::default()
            .frame(Frame::default().inner_margin(Margin::same(18)).fill(fill))
            .show(ui, |ui| {
                settings_window::show(ui, &mut settings, &mut autostart)
            })
            .inner;

        if changed {
            self.model.update_settings(|current| *current = settings);
        }
        if autostart != self.autostart {
            self.set_autostart(autostart);
        }
    }
}

impl eframe::App for App {
    /// Transparent so the popover's rounded corners are actually round rather
    /// than sitting on a square of window background.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let hwnd = native_handle(frame);

        if let Some(commands) = &self.commands {
            let pending: Vec<TrayCommand> = commands.try_iter().collect();
            for command in pending {
                crate::debug_log!("command: {command:?}");
                self.handle(command, ctx);
            }
        }

        if let Some(tray) = &mut self.tray {
            let state = self.model.state();
            tray.sync(&state);
        }

        // Closing the settings window means "close the window", not "quit the
        // app" -- the tray icon is the app. Only an explicit Quit gets through.
        if ctx.input(|input| input.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.hide(ctx);
        }

        if self.mode == Mode::Hidden {
            return;
        }

        // Parked off-screen waiting to be measured. Nothing else applies until
        // it has a real height and a real position -- in particular it must not
        // be dismissed for not having focus it was never offered.
        if let Some((since, estimate)) = self.measuring {
            let measured = self.measured_height.take();
            let timed_out = since.elapsed() > Duration::from_millis(300);

            match measured {
                Some(height) => {
                    self.last_measurement = Some((estimate, height));
                    let position = self.popover_position(ctx, height);
                    self.place_at(ctx, height, position);
                    self.measuring = None;
                    self.reveal(ctx);
                }
                // Never got a frame. Show it at the estimate rather than leave
                // it parked where nobody can see it.
                None if timed_out => {
                    let position = self.popover_position(ctx, estimate);
                    self.place_at(ctx, estimate, position);
                    self.measuring = None;
                    self.reveal(ctx);
                }
                None => ctx.request_repaint(),
            }
            return;
        }

        // Correct the popover if the content changed height while open. The
        // 2pt deadband is what stops a resize from feeding back into a
        // remeasure and oscillating forever.
        if self.mode == Mode::Popover {
            if let Some(measured) = self.measured_height.take() {
                if (measured - self.placed_height).abs() > 2.0 {
                    crate::debug_log!(
                        "resize: measured {measured} vs placed {}",
                        self.placed_height
                    );
                    let position = self.popover_position(ctx, measured);
                    self.place_at(ctx, measured, position);
                    ctx.request_repaint();
                }
            }
        }

        let focused = ctx.input(|input| input.viewport().focused).unwrap_or(false);
        if focused {
            self.ever_focused = true;
            self.focus_attempts = 0;
        } else if self.focus_attempts > 0 {
            self.focus_attempts -= 1;
            if let Some(hwnd) = hwnd {
                platform::take_foreground(hwnd);
            }
            // Spaced rather than spun: `take_foreground` calls
            // `BringWindowToTop`, and repeating that as fast as the frame loop
            // will go is visible as a flicker.
            ctx.request_repaint_after(Duration::from_millis(30));
        }

        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            crate::debug_log!("dismiss: escape");
            self.hide(ctx);
            return;
        }

        // Dismiss on focus loss, the way a menu bar popover does -- but not
        // before the window has had a chance to take focus, and never for
        // settings, which is a window you are meant to leave open while you
        // look at something else.
        if self.mode == Mode::Popover {
            let settled = self
                .shown_at
                .map(|at| at.elapsed() > Duration::from_millis(250))
                .unwrap_or(false);
            if settled && self.ever_focused && !focused {
                crate::debug_log!("dismiss: focus lost");
                self.hide(ctx);
                return;
            }

            // Countdowns tick once a second, and only while the popover is open.
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        match self.mode {
            Mode::Hidden => {}
            Mode::Popover => self.draw_popover(ui),
            Mode::Settings => self.draw_settings(ui),
        }
    }
}

/// eframe hands out the platform window handle on the `Frame`, which is the
/// only place it is reachable from inside a frame.
fn native_handle(frame: &eframe::Frame) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

#[cfg(windows)]
fn autostart_enabled() -> bool {
    crate::autostart::is_enabled()
}

#[cfg(windows)]
fn set_autostart(enabled: bool, exe: &std::path::Path) -> std::io::Result<()> {
    crate::autostart::set(enabled, exe)
}

#[cfg(not(windows))]
fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
fn set_autostart(_enabled: bool, _exe: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
