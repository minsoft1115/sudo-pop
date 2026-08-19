//! The password window.
//!
//! One window per authentication request, not per attempt. PAM asks through
//! the helper as many times as it likes -- a wrong password, an extra prompt,
//! a message to show -- and the window stays put while the text on it changes.
//! That is the difference from the sudo path, where sudo forked a fresh
//! askpass process for every attempt.
//!
//! The event loop therefore cannot block: the helper conversation runs on
//! another thread and the two sides talk over channels. winit allows exactly
//! one event loop per process (`EventLoopError::RecreationAttempt`), so this is
//! the only place in the program that draws.
//!
//! Labels stay ASCII on purpose; see `font` for what that costs and why.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::font;
use crate::secret::Secret;
use crate::theme;

/// Wayland app-id. The Hyprland rule installed by `--init` matches on exactly
/// this string, so the two must stay in step.
pub const APP_ID: &str = "sudo-askpass";

const WINDOW_SIZE: [f32; 2] = [400.0, 200.0];

/// Give up a little after the caller does.
///
/// polkit callers stop waiting at 25 seconds (sd-bus method timeout) and
/// polkitd then cancels the request, which closes this window on its own. This
/// is only a backstop for a cancel that never arrives.
const TIMEOUT: Duration = Duration::from_secs(30);

/// What the helper thread tells the window.
pub enum ToUi {
    Prompt { text: String, echo: bool },
    Info(String),
    Error(String),
    /// The conversation is over; close.
    Done,
}

/// What the window tells the helper thread.
pub enum FromUi {
    Answer(Secret),
    Cancel,
}

/// What the request is about, shown above the field.
pub struct Subject {
    /// The command behind the request, if it could be established.
    pub command: Option<String>,
    /// polkit's own wording, used when there is no command to show.
    pub message: String,
}

/// Show the window and pump it until the conversation ends.
pub fn run(subject: Subject, to_ui: Receiver<ToUi>, from_ui: Sender<FromUi>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(APP_ID)
            .with_inner_size(WINDOW_SIZE)
            .with_decorations(false)
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| {
            font::apply(&cc.egui_ctx);
            if let Some(theme) = theme::load() {
                cc.egui_ctx.set_theme(if theme.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                cc.egui_ctx.set_visuals(theme.visuals());
            }
            Ok(Box::new(Window::new(subject, to_ui, from_ui)))
        }),
    )
    .map_err(|e| format!("cannot open the password window: {e}"))
}

struct Window {
    subject: Subject,
    to_ui: Receiver<ToUi>,
    from_ui: Sender<FromUi>,
    prompt: String,
    echo: bool,
    notice: Option<(String, bool)>,
    password: Secret,
    /// An answer is with the helper; the field is inert until it comes back.
    waiting: bool,
    focus_set: bool,
    deadline: Instant,
}

impl Window {
    fn new(subject: Subject, to_ui: Receiver<ToUi>, from_ui: Sender<FromUi>) -> Self {
        Self {
            subject,
            to_ui,
            from_ui,
            prompt: "Password:".into(),
            echo: false,
            notice: None,
            password: Secret::new(),
            waiting: false,
            focus_set: false,
            deadline: Instant::now() + TIMEOUT,
        }
    }

    /// Take everything the helper thread has queued. Returns false when the
    /// conversation is over and the window should close.
    fn drain(&mut self) -> bool {
        loop {
            match self.to_ui.try_recv() {
                Ok(ToUi::Prompt { text, echo }) => {
                    self.prompt = text;
                    self.echo = echo;
                    self.waiting = false;
                    self.focus_set = false;
                    // A new question deserves a fresh deadline.
                    self.deadline = Instant::now() + TIMEOUT;
                }
                Ok(ToUi::Info(text)) => self.notice = Some((text, false)),
                Ok(ToUi::Error(text)) => {
                    self.notice = Some((text, true));
                    self.waiting = false;
                }
                Ok(ToUi::Done) | Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    fn submit(&mut self) {
        let password = std::mem::take(&mut self.password);
        // Sending moves the buffer; nothing is copied and nothing is left here.
        let _ = self.from_ui.send(FromUi::Answer(password));
        self.password = Secret::new();
        self.waiting = true;
        self.notice = None;
    }

    fn cancel(&mut self, ctx: &egui::Context) {
        self.password.wipe();
        let _ = self.from_ui.send(FromUi::Cancel);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for Window {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if !self.drain() {
            self.password.wipe();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // Nothing wakes this loop when the helper speaks, so look often.
        ctx.request_repaint_after(Duration::from_millis(50));

        if Instant::now() >= self.deadline
            || ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested())
        {
            self.cancel(&ctx);
            return;
        }

        egui::Frame::central_panel(ui.style())
            .inner_margin(24)
            .show(ui, |ui| {
                ui.vertical_centered_justified(|ui| {
                    // The command leads: it is the one piece of information
                    // here, and the only cue that something unexpected is what
                    // is asking. polkit's own message says nothing useful for
                    // run0, so it is the fallback rather than the headline.
                    let headline = self
                        .subject
                        .command
                        .clone()
                        .unwrap_or_else(|| self.subject.message.clone());
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(headline)
                                .size(11.5)
                                .family(egui::FontFamily::Monospace)
                                .color(ui.visuals().hyperlink_color),
                        )
                        .truncate(),
                    );
                    ui.add_space(10.0);

                    ui.label(
                        egui::RichText::new(&self.prompt)
                            .size(12.5)
                            .color(ui.visuals().text_color().gamma_multiply(0.5)),
                    );
                    ui.add_space(16.0);

                    let field = ui.add_enabled(
                        !self.waiting,
                        egui::TextEdit::singleline(self.password.buffer_mut())
                            .password(!self.echo)
                            .char_limit(crate::secret::MAX_CHARS)
                            .font(egui::TextStyle::Monospace)
                            .margin(egui::Margin::symmetric(10, 8))
                            .desired_width(f32::INFINITY),
                    );

                    if !self.focus_set && !self.waiting {
                        field.request_focus();
                        self.focus_set = true;
                    }

                    let entered =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    ui.add_space(14.0);
                    match (&self.notice, self.waiting) {
                        (Some((text, true)), _) => ui.label(
                            egui::RichText::new(text)
                                .size(11.0)
                                .color(ui.visuals().error_fg_color),
                        ),
                        (Some((text, false)), _) => {
                            ui.label(egui::RichText::new(text).size(11.0).weak())
                        }
                        (None, true) => {
                            ui.label(egui::RichText::new("Checking...").size(11.0).weak())
                        }
                        (None, false) => ui.label(
                            egui::RichText::new("Enter to confirm    Esc to cancel")
                                .size(11.0)
                                .weak(),
                        ),
                    };

                    if entered && !self.waiting {
                        if self.password.is_empty() {
                            // An empty line reads as a wrong password and costs
                            // an attempt, so it just keeps the window open.
                            field.request_focus();
                        } else {
                            self.submit();
                        }
                    }
                });
            });
    }
}
