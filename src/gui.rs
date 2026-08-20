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
//! Our own labels stay ASCII on purpose. The text we did not write -- the
//! command line, polkit's message, PAM's prompts -- can be in any script, so
//! the font chain grows to meet it; see `font`.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::font;
use crate::secret::Secret;
use crate::theme;

/// Wayland app-id. The Hyprland rule installed by `--init` matches on exactly
/// this string, so the two must stay in step.
pub const APP_ID: &str = "sudo-askpass";

const WINDOW_HEIGHT: f32 = 200.0;

/// The window is as wide as the lines it has to show, between these.
///
/// The command line is why. `run0 pacman -Syu` fits in 400 with room to spare,
/// but a systemd unit path, a desktop app's argv, or `sudo` on a long
/// invocation does not -- and a truncated command is the one thing this window
/// must not do, because that line is what tells you whether to type at all.
/// 800 rather than "as wide as it takes": past that the eye stops reading a
/// line and starts scanning it, and a password box has no business filling a
/// screen.
const MIN_WIDTH: f32 = 400.0;
const MAX_WIDTH: f32 = 800.0;

/// The panel's inner margin, on every side.
const PANEL_MARGIN: f32 = 24.0;

/// What the window costs around its widest line: the panel's inner margin on
/// both sides, and a little slack so a line that just fits is not truncated by
/// a rounding difference between measuring and drawing.
const CHROME_WIDTH: f32 = PANEL_MARGIN * 2.0 + 8.0;

/// The lock glyph and the gap between it and the field.
const LOCK_WIDTH: f32 = 24.0;
const LOCK_GAP: f32 = 6.0;

/// The password row keeps the width it has in the narrowest window and is
/// centred in anything wider.
///
/// It could stretch with the window, and that is what a text field normally
/// does -- but the window only widens to fit a long *command*, and a password
/// box that grows with the command reads as though the command belongs in it.
/// The thing being typed is the same length whatever is being authorised.
const FIELD_ROW_WIDTH: f32 = MIN_WIDTH - PANEL_MARGIN * 2.0;

/// Text sizes, shared between measuring the window and drawing it. Measuring
/// with one size and drawing with another is a bug that only shows up on the
/// long lines nobody tests with.
const HEADLINE_SIZE: f32 = 11.5;
const DETAIL_SIZE: f32 = 11.0;

/// Below this many seconds the countdown turns to the error colour, the same
/// way the attempts line does when its budget runs low.
const HURRY_AT_OR_BELOW: u64 = 5;

/// Seconds remaining, rounded up.
///
/// Up rather than down so the final second reads `1s` for its whole length
/// instead of sitting on `0s`; the number reaches zero only once there really
/// is nothing left. It is already a shade optimistic -- the caller started its
/// clock before polkitd reached us, measured at about a quarter second -- and
/// rounding down would hide a second that is still there.
fn ceil_secs(left: Duration) -> u64 {
    left.as_millis().div_ceil(1000) as u64
}

/// The number to draw, or `None` for "do not draw one".
///
/// Zero is where the estimate stops being one. The 25 seconds belongs to the
/// caller's bus library, not to us, and a caller that sets no timeout of its
/// own -- `pkcheck` waits indefinitely, measured -- is still very much waiting
/// when the count runs out. A red `0s` on a live request is a lie, and the
/// only requests that ever reach zero on screen are the ones it lies about:
/// a caller that really did time out has polkitd cancel it, which closes the
/// window in the same moment.
fn countdown(left: Duration) -> Option<u64> {
    let secs = ceil_secs(left);
    (secs > 0).then_some(secs)
}

/// Nerd Font padlock (nf-fa-lock) -- the same glyph the system polkit dialog
/// uses. The Omarchy monospace font carries it; without a Nerd Font egui draws
/// a box, so this assumes the target platform (Omarchy) it is built for.
const LOCK_GLYPH: &str = "\u{f023}";

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
    /// polkit's own wording, the last thing tried when nothing better exists.
    pub message: String,
    /// What the request will do, from `invocation::purpose`: the second line
    /// under the command, and the headline when there is no command. `None`
    /// where polkit's sentence would add nothing (the `run0` path).
    pub purpose: Option<String>,
    /// Whose password is being asked. The helper's prompt never says.
    pub user: Option<String>,
    /// The standing budget line and whether it is low enough to alarm, from
    /// `attempts::Budget::status`. It does not change while the window is up,
    /// so it is a property of the request rather than a message on the channel.
    pub attempts: Option<(String, bool)>,
    /// When the caller stops waiting, on the paths where one does.
    ///
    /// `None` on the sudo path: sudo waits for askpass however long it takes,
    /// so a countdown there would be inventing a deadline. On the polkit path
    /// the caller really does leave, and the window is the only place that can
    /// say so before it happens.
    pub deadline: Option<Instant>,
}

impl Subject {
    /// The first line: the command if we have one, else what polkit says the
    /// request will do, else its raw wording.
    fn headline(&self) -> String {
        self.command
            .clone()
            .or_else(|| self.purpose.clone())
            .unwrap_or_else(|| self.message.clone())
    }

    /// The second line, which exists only when the first one is a command and
    /// polkit's sentence adds something to it.
    fn detail(&self) -> Option<&str> {
        self.command.is_some().then(|| self.purpose.as_deref())?
    }
}

/// Wide enough for the lines it must show, within bounds.
fn fitted_width(chain: &font::Chain, subject: &Subject) -> f32 {
    let headline = subject.headline();
    let mut lines = vec![(headline.as_str(), egui::FontId::monospace(HEADLINE_SIZE))];
    if let Some(detail) = subject.detail() {
        lines.push((detail, egui::FontId::proportional(DETAIL_SIZE)));
    }
    let text = chain.measure(&lines);
    let width = clamp_width(text);
    if std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty()) {
        eprintln!("sudo-pop: text {text:.0}pt -> window {width:.0}pt");
    }
    width
}

/// Text width to window width.
fn clamp_width(text: f32) -> f32 {
    (text + CHROME_WIDTH).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Show the window and pump it until the conversation ends.
pub fn run(subject: Subject, to_ui: Receiver<ToUi>, from_ui: Sender<FromUi>) -> Result<(), String> {
    // Built before the window rather than inside it: the size depends on how
    // wide these lines come out, and that cannot be measured without the fonts.
    let mut chain = font::Chain::new();
    // The command line and polkit's wording are the only text here we did not
    // write; either can be in any script.
    chain.cover(subject.command.as_deref().unwrap_or_default());
    chain.cover(subject.purpose.as_deref().unwrap_or_default());
    chain.cover(&subject.message);
    let width = fitted_width(&chain, &subject);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(APP_ID)
            .with_inner_size([width, WINDOW_HEIGHT])
            .with_decorations(false)
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| {
            chain.install(&cc.egui_ctx);
            if let Some(theme) = theme::load() {
                cc.egui_ctx.set_theme(if theme.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                cc.egui_ctx.set_visuals(theme.visuals());
            }
            Ok(Box::new(Window::new(subject, chain, to_ui, from_ui)))
        }),
    )
    .map_err(|e| format!("cannot open the password window: {e}"))
}

struct Window {
    subject: Subject,
    chain: font::Chain,
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
    fn new(
        subject: Subject,
        chain: font::Chain,
        to_ui: Receiver<ToUi>,
        from_ui: Sender<FromUi>,
    ) -> Self {
        Self {
            subject,
            chain,
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
    fn drain(&mut self, ctx: &egui::Context) -> bool {
        loop {
            match self.to_ui.try_recv() {
                Ok(ToUi::Prompt { text, echo }) => {
                    self.cover(ctx, &text);
                    self.prompt = text;
                    self.echo = echo;
                    self.waiting = false;
                    self.focus_set = false;
                    // A new question deserves a fresh deadline.
                    self.deadline = Instant::now() + TIMEOUT;
                }
                Ok(ToUi::Info(text)) => {
                    self.cover(ctx, &text);
                    self.notice = Some((text, false));
                }
                Ok(ToUi::Error(text)) => {
                    self.cover(ctx, &text);
                    self.notice = Some((text, true));
                    self.waiting = false;
                }
                Ok(ToUi::Done) | Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    /// PAM speaks after the window is up, so text can arrive in a script the
    /// chain has no face for. A new chain takes effect on the next frame.
    fn cover(&mut self, ctx: &egui::Context, text: &str) {
        if self.chain.cover(text) {
            self.chain.install(ctx);
            ctx.request_repaint();
        }
    }

    /// Whole seconds until the caller gives up, or `None` where nothing is
    /// counting. Rounded up so the last second reads `1s` rather than `0s`
    /// for its whole length.
    fn seconds_left(&self) -> Option<u64> {
        let deadline = self.subject.deadline?;
        countdown(deadline.saturating_duration_since(Instant::now()))
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

        if !self.drain(&ctx) {
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

        // The countdown goes in the top-right margin band, put rather than
        // laid out: it must not move the composition below it, and a centred
        // headline must not be able to collide with it.
        let panel = ui.max_rect();

        egui::Frame::central_panel(ui.style())
            .inner_margin(PANEL_MARGIN as i8)
            .show(ui, |ui| {
                ui.vertical_centered_justified(|ui| {
                    // The command leads: the one cue that something unexpected is
                    // asking. polkit's own message says nothing useful for run0, so
                    // it is the fallback rather than the headline.
                    let headline = self.subject.headline();
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(headline)
                                .size(HEADLINE_SIZE)
                                .family(egui::FontFamily::Monospace)
                                .color(ui.visuals().hyperlink_color),
                        )
                        .truncate(),
                    );

                    // What it will do, when the command line does not already
                    // say. A desktop app's line names the binary and nothing
                    // else; this is where "mount the filesystem" appears.
                    if let Some(detail) = self.subject.detail() {
                        ui.add_space(3.0);
                        ui.add(
                            egui::Label::new(egui::RichText::new(detail).size(DETAIL_SIZE))
                                .truncate(),
                        );
                    }

                    // Whose password this is. The helper's prompt is a bare
                    // "Password:" and never says, so the window does.
                    if let Some(user) = &self.subject.user {
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new(format!("for {user}"))
                                .size(11.0)
                                .color(ui.visuals().text_color().gamma_multiply(0.5)),
                        );
                    }
                    ui.add_space(18.0);

                    // A prompt that is not a plain password (an OTP, a question)
                    // still needs its words; a password is spoken by the lock alone.
                    if self.echo && !self.prompt.is_empty() {
                        ui.label(
                            egui::RichText::new(&self.prompt)
                                .size(11.5)
                                .color(ui.visuals().text_color().gamma_multiply(0.5)),
                        );
                        ui.add_space(8.0);
                    }

                    // A lock glyph in front of the field, like the system dialog --
                    // no "Password:" label.
                    let entered = ui
                        .horizontal(|ui| {
                            // Hold the row at its narrow-window width, centred.
                            let slack = ui.available_width() - FIELD_ROW_WIDTH;
                            ui.add_space((slack / 2.0).max(0.0));
                            // Centre the glyph in a box the height of the field,
                            // so it lines up with the input rather than riding high.
                            let field_h =
                                ui.text_style_height(&egui::TextStyle::Monospace) + 16.0;
                            ui.add_sized(
                                [LOCK_WIDTH, field_h],
                                egui::Label::new(
                                    egui::RichText::new(LOCK_GLYPH)
                                        .size(18.0)
                                        .family(egui::FontFamily::Monospace)
                                        .color(ui.visuals().hyperlink_color),
                                ),
                            );
                            ui.add_space(LOCK_GAP);
                            let field = ui.add_enabled(
                                !self.waiting,
                                egui::TextEdit::singleline(self.password.buffer_mut())
                                    .password(!self.echo)
                                    .char_limit(crate::secret::MAX_CHARS)
                                    .font(egui::TextStyle::Monospace)
                                    .margin(egui::Margin::symmetric(10, 8))
                                    .desired_width(FIELD_ROW_WIDTH - LOCK_WIDTH - LOCK_GAP),
                            );
                            if !self.focus_set && !self.waiting {
                                field.request_focus();
                                self.focus_set = true;
                            }
                            let entered = field.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            // An empty line reads as a wrong password and costs an
                            // attempt, so keep the window open instead of submitting.
                            if entered && !self.waiting && self.password.is_empty() {
                                field.request_focus();
                            }
                            entered
                        })
                        .inner;

                    // Two lines under the field, and they say different kinds
                    // of thing. The first is what just happened -- a wrong
                    // password, something PAM wants shown -- and comes and
                    // goes. The second is how much of the shared faillock
                    // budget is left, which is true for as long as the window
                    // is open and so is never taken away to make room.
                    ui.add_space(14.0);
                    let transient = match (&self.notice, self.waiting) {
                        (Some((text, error)), _) => Some((text.as_str(), *error)),
                        (None, true) => Some(("Checking...", false)),
                        // Keeps the line's height so the budget below it does
                        // not jump when a notice arrives.
                        (None, false) => None,
                    };
                    match transient {
                        Some((text, true)) => ui.label(
                            egui::RichText::new(text)
                                .size(11.0)
                                .color(ui.visuals().error_fg_color),
                        ),
                        Some((text, false)) => {
                            ui.label(egui::RichText::new(text).size(11.0).weak())
                        }
                        None => ui.label(egui::RichText::new(" ").size(11.0)),
                    };

                    if let Some((text, low)) = &self.subject.attempts {
                        ui.add_space(2.0);
                        let text = egui::RichText::new(text).size(11.0);
                        ui.label(if *low {
                            text.color(ui.visuals().error_fg_color)
                        } else {
                            text.weak()
                        });
                    }

                    if entered && !self.waiting && !self.password.is_empty() {
                        self.submit();
                    }
                });
            });

        if let Some(left) = self.seconds_left() {
            let text = egui::RichText::new(format!("{left}s")).size(11.0);
            let text = if left <= HURRY_AT_OR_BELOW {
                text.color(ui.visuals().error_fg_color)
            } else {
                text.weak()
            };
            let badge = egui::Rect::from_min_max(
                egui::pos2(panel.right() - 56.0, panel.top() + 6.0),
                egui::pos2(panel.right() - 8.0, panel.top() + 22.0),
            );
            ui.put(badge, egui::Label::new(text).halign(egui::Align::RIGHT));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window is not unit-testable -- it owns an event loop -- but the
    /// countdown's arithmetic is, and getting it wrong is visible every second.
    #[test]
    fn the_countdown_rounds_up_so_the_last_second_is_shown() {
        assert_eq!(ceil_secs(Duration::ZERO), 0);
        assert_eq!(ceil_secs(Duration::from_millis(1)), 1);
        assert_eq!(ceil_secs(Duration::from_millis(999)), 1);
        assert_eq!(ceil_secs(Duration::from_millis(1000)), 1);
        assert_eq!(ceil_secs(Duration::from_millis(1001)), 2);
        assert_eq!(ceil_secs(Duration::from_secs(25)), 25);
    }

    #[test]
    fn short_lines_leave_the_window_at_its_usual_size() {
        assert_eq!(clamp_width(0.0), MIN_WIDTH);
        assert_eq!(clamp_width(MIN_WIDTH - CHROME_WIDTH - 1.0), MIN_WIDTH);
    }

    #[test]
    fn a_long_line_widens_the_window_but_only_so_far() {
        let grown = clamp_width(MIN_WIDTH);
        assert_eq!(grown, MIN_WIDTH + CHROME_WIDTH);
        assert!(grown > MIN_WIDTH && grown < MAX_WIDTH);
        assert_eq!(clamp_width(10_000.0), MAX_WIDTH, "a runaway argv cannot fill the screen");
    }

    #[test]
    fn the_password_row_does_not_grow_with_the_window() {
        // It is sized to the narrow window and centred in anything wider, so
        // the box a password goes into looks the same whatever is asking.
        assert_eq!(FIELD_ROW_WIDTH, MIN_WIDTH - PANEL_MARGIN * 2.0);
        assert!(FIELD_ROW_WIDTH < MIN_WIDTH);
        assert!(LOCK_WIDTH + LOCK_GAP < FIELD_ROW_WIDTH);
    }

    #[test]
    fn a_spent_countdown_is_not_drawn_at_all() {
        assert_eq!(countdown(Duration::ZERO), None);
        assert_eq!(countdown(Duration::from_millis(1)), Some(1));
        assert_eq!(countdown(Duration::from_secs(25)), Some(25));
    }

    #[test]
    fn the_last_five_seconds_are_the_ones_that_alarm() {
        assert!(ceil_secs(Duration::from_millis(4500)) <= HURRY_AT_OR_BELOW);
        assert!(ceil_secs(Duration::from_millis(5000)) <= HURRY_AT_OR_BELOW);
        assert!(ceil_secs(Duration::from_millis(5001)) > HURRY_AT_OR_BELOW);
    }
}
