//! The password window.
//!
//! Deliberately isolated behind `prompt()`: everything the rest of askpass mode
//! knows is "a Secret came back, or the user said no". That keeps the door open
//! for a layer-shell surface later without touching the hardening or the
//! password channel.
//!
//! Labels here stay ASCII on purpose. egui's bundled fonts carry no CJK glyphs,
//! and pulling a system CJK font in would cost startup time — the one thing
//! this tool cannot afford, since its whole value is a popup that is already
//! there when you look up. The text that matters most is sudo's own prompt,
//! which is ASCII anyway.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use super::font;
use super::invocation;
use super::secret::Secret;
use super::theme;

/// Wayland app-id. The Hyprland rule installed by `--init` matches on exactly
/// this string, so the two must stay in step.
pub const APP_ID: &str = "sudo-askpass";

const WINDOW_SIZE: [f32; 2] = [400.0, 200.0];

/// How long the window waits before giving up.
///
/// Without a deadline a window that never draws — a broken Wayland connection,
/// say — would leave sudo blocking on a helper that answers nothing, with the
/// `stay_focused` rule making the session awkward to recover.
const TIMEOUT: Duration = Duration::from_secs(90);

fn window_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(APP_ID)
            .with_inner_size(WINDOW_SIZE)
            .with_decorations(false)
            .with_resizable(false),
        ..Default::default()
    }
}

/// Show the window and return what the user decided.
///
/// `warning` is shown under the field when the account is close to locking out;
/// `None` leaves the window uncluttered in the normal case.
///
/// A `None` return covers every refusal — escape, closed window, timeout, or a
/// GUI that could not start at all. The caller turns all of them into a silent
/// exit.
pub fn prompt(prompt_text: &str, warning: Option<&str>) -> Option<Secret> {
    let outcome: Arc<Mutex<Option<Secret>>> = Arc::new(Mutex::new(None));

    let app_prompt = prompt_text.to_owned();
    let app_warning = warning.map(str::to_owned);
    let app_outcome = Arc::clone(&outcome);

    let run = eframe::run_native(
        APP_ID,
        window_options(),
        Box::new(move |cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(PasswordWindow::new(
                app_prompt,
                app_warning,
                app_outcome,
            )))
        }),
    );

    if let Err(e) = run {
        eprintln!("sudo-pop: cannot open the password window: {e}");
        return None;
    }

    outcome.lock().ok()?.take()
}

/// Show a message with no input, for when asking would be pointless.
///
/// Used when the account is already locked out: prompting there would spend an
/// attempt that cannot succeed, and the terminal message explaining why is
/// hidden behind the dim-around rule.
pub fn notice(message: &str) {
    let text = message.to_owned();
    let _ = eframe::run_native(
        APP_ID,
        window_options(),
        Box::new(move |cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(NoticeWindow::new(text)))
        }),
    );
}

struct NoticeWindow {
    message: String,
    deadline: Instant,
}

impl NoticeWindow {
    /// Shorter than the password timeout: there is nothing to read for long.
    const DISMISS_AFTER: Duration = Duration::from_secs(10);

    fn new(message: String) -> Self {
        Self {
            message,
            deadline: Instant::now() + Self::DISMISS_AFTER,
        }
    }
}

impl eframe::App for NoticeWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let dismissed = ctx.input(|i| i.viewport().close_requested() || i.any_touches())
            || ctx.input(|i| !i.keys_down.is_empty());

        if now >= self.deadline || dismissed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint_after(self.deadline - now);

        // eframe hands us a Ui with no background or margin of its own, so the
        // panel frame has to be drawn explicitly or the window stays blank.
        egui::Frame::central_panel(ui.style())
            .inner_margin(24)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(28.0);
                    ui.label(egui::RichText::new(&self.message).size(14.0).strong());
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("Press any key to close")
                            .size(11.0)
                            .weak(),
                    );
                });
            });
    }
}

/// Dress the window in the desktop's current look: Omarchy's palette and the
/// font fontconfig currently resolves.
///
/// The theme is also pinned so egui does not follow the system preference and
/// undo the palette we just applied.
fn apply_theme(ctx: &egui::Context) {
    font::apply(ctx);

    let Some(theme) = theme::load() else {
        return;
    };
    ctx.set_theme(if theme.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_visuals(theme.visuals());
}

struct PasswordWindow {
    prompt: String,
    command: Option<String>,
    warning: Option<String>,
    password: Secret,
    outcome: Arc<Mutex<Option<Secret>>>,
    deadline: Instant,
    focus_set: bool,
}

impl PasswordWindow {
    fn new(prompt: String, warning: Option<String>, outcome: Arc<Mutex<Option<Secret>>>) -> Self {
        Self {
            prompt,
            command: invocation::command(),
            warning,
            password: Secret::new(),
            outcome,
            deadline: Instant::now() + TIMEOUT,
            focus_set: false,
        }
    }

    /// Hand the password over and close.
    fn submit(&mut self, ctx: &egui::Context) {
        let password = std::mem::take(&mut self.password);
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = Some(password);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Close without answering. The typed text is wiped here rather than left
    /// to `Drop`, which never runs when the process exits through `exit()`.
    fn cancel(&mut self, ctx: &egui::Context) {
        self.password.wipe();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for PasswordWindow {
    /// Transparent so the compositor's dim-around rule shows through the
    /// rounded corners rather than a grey rectangle.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        let now = Instant::now();
        if now >= self.deadline {
            self.cancel(&ctx);
            return;
        }
        // Wake up in time to notice the deadline even if nothing else happens.
        ctx.request_repaint_after(self.deadline - now);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()) {
            self.cancel(&ctx);
            return;
        }

        // eframe hands us a Ui with no background or margin of its own, so the
        // panel frame has to be drawn explicitly or the window stays blank.
        egui::Frame::central_panel(ui.style())
            .inner_margin(24)
            .show(ui, |ui| {
                ui.vertical_centered_justified(|ui| {
                    // The command leads, in the theme's accent: it is the one
                    // piece of information here, and the only cue that
                    // something unexpected is what is asking.
                    match &self.command {
                        Some(command) => {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(command)
                                        .size(11.5)
                                        .family(egui::FontFamily::Monospace)
                                        .color(ui.visuals().hyperlink_color),
                                )
                                .truncate(),
                            );
                            ui.add_space(10.0);
                        }
                        None => ui.add_space(8.0),
                    }
                    // Subdued like a field label: the command above is the
                    // information, this line only says what to type.
                    ui.label(
                        egui::RichText::new(&self.prompt)
                            .size(12.5)
                            .color(ui.visuals().text_color().gamma_multiply(0.5)),
                    );
                    ui.add_space(16.0);

                    let field = ui.add(
                        egui::TextEdit::singleline(self.password.buffer_mut())
                            .password(true)
                            .char_limit(super::secret::MAX_CHARS)
                            .font(egui::TextStyle::Monospace)
                            .margin(egui::Margin::symmetric(10, 8))
                            .desired_width(f32::INFINITY),
                    );

                    // The window exists to take one keystroke sequence, so the
                    // caret belongs in the field from the first frame.
                    if !self.focus_set {
                        field.request_focus();
                        self.focus_set = true;
                    }

                    let entered =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    ui.add_space(14.0);
                    match &self.warning {
                        Some(text) => ui.label(
                            egui::RichText::new(text)
                                .size(11.0)
                                .color(ui.visuals().warn_fg_color),
                        ),
                        None => ui.label(
                            egui::RichText::new("Enter to confirm    Esc to cancel")
                                .size(11.0)
                                .weak(),
                        ),
                    };

                    if entered {
                        if self.password.is_empty() {
                            // Sending an empty line would read as a wrong password
                            // and cost the whole retry budget, so an empty Enter
                            // just keeps the window open.
                            field.request_focus();
                        } else {
                            self.submit(&ctx);
                        }
                    }
                });
            });
    }
}
