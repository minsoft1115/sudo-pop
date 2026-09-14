//! The authentication window.
//!
//! One window per authentication request, not per attempt. PAM asks through
//! the helper as many times as it likes -- a wrong password, an extra prompt,
//! a message to show -- and the window stays put while the text on it changes.
//!
//! Fingerprint and password are separate *phases* of that one window:
//! different clocks, different layout, and nothing counted in one is shown in
//! the other. winit allows exactly one event loop per process
//! (`EventLoopError::RecreationAttempt`), so they cannot be two OS windows.
//! The first Prompt ends the fingerprint phase for good; PAM may try the
//! sensor again on a wrong-password retry, but this process never goes back.
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

/// Fingerprint wait has no field. Shorter than the password layout, but tall
/// enough for the glyph and the sensor hint.
const FINGERPRINT_HEIGHT: f32 = 168.0;

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

/// Nerd Font `nf-md-fingerprint` (`U+F0237`). omarchy.polkit writes this as
/// the JS surrogate pair `\udb80\ude37`. `U+F0597` is `weather-rainy`.
const FINGERPRINT_GLYPH: &str = "\u{f0237}";

/// Short ASCII hint. pam_fprintd's own sentence is a paragraph; this is the
/// same kind of label as `for {user}` / `Wrong`.
const FINGERPRINT_HINT: &str = "Touch the sensor";

fn window_height(fingerprint: bool) -> f32 {
    if fingerprint {
        FINGERPRINT_HEIGHT
    } else {
        WINDOW_HEIGHT
    }
}

/// Fingerprint and password do not share counters, clocks, or messages.
enum Phase {
    Fingerprint {
        /// The last thing PAM said went wrong at the sensor, drawn in place
        /// of the hint. We do not count these: pam_fprintd sends the same
        /// kind of message for a bad swipe it does not charge a try for.
        notice: Option<String>,
    },
    Password {
        /// Our own 30s backstop, fresh with every prompt. `None` only while
        /// nothing has been asked yet, which is the fingerprint case.
        backstop: Option<Instant>,
        /// An answer is with the helper; the field is inert until it comes back.
        waiting: bool,
        focus_set: bool,
        notice: Option<(String, bool)>,
        prompt: String,
        echo: bool,
    },
}

/// Give up a little after the caller does.
///
/// polkit callers stop waiting at 25 seconds (sd-bus method timeout) and
/// polkitd then cancels the request, which closes this window on its own. This
/// is only a backstop for a cancel that never arrives.
const TIMEOUT: Duration = Duration::from_secs(30);

/// What the helper thread tells the window.
pub enum ToUi {
    Prompt {
        text: String,
        echo: bool,
    },
    Info(String),
    Error(String),
    Attempts(Option<(String, bool)>),
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
    /// `attempts::Budget::status`. Read once for the window and re-read after
    /// every wrong answer (`ToUi::Attempts`), since the shared faillock tally
    /// is what moved.
    pub attempts: Option<(String, bool)>,
    /// When the caller stops waiting, on the paths where one does.
    ///
    /// `None` on the sudo path: sudo waits for askpass however long it takes,
    /// so a countdown there would be inventing a deadline. On the polkit path
    /// the caller really does leave, and the window is the only place that can
    /// say so before it happens.
    pub deadline: Option<Instant>,
    /// The polkit PAM stack will try a fingerprint before asking for a
    /// password, and the lid is open. The field stays hidden until `Prompt`.
    pub fingerprint_wait: bool,
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
    let height = window_height(subject.fingerprint_wait);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(APP_ID)
            .with_inner_size([width, height])
            .with_min_inner_size([width, height])
            .with_max_inner_size([width, height])
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
            Ok(Box::new(Window::new(subject, chain, to_ui, from_ui, width)))
        }),
    )
    .map_err(|e| format!("cannot open the password window: {e}"))
}

struct Window {
    subject: Subject,
    chain: font::Chain,
    to_ui: Receiver<ToUi>,
    from_ui: Sender<FromUi>,
    password: Secret,
    width: f32,
    phase: Phase,
    came_from_fingerprint: bool,
}

impl Window {
    fn new(
        subject: Subject,
        chain: font::Chain,
        to_ui: Receiver<ToUi>,
        from_ui: Sender<FromUi>,
        width: f32,
    ) -> Self {
        let came_from_fingerprint = subject.fingerprint_wait;
        let phase = if subject.fingerprint_wait {
            Phase::Fingerprint { notice: None }
        } else {
            // A password window is armed from the moment it opens: a helper
            // that never gets round to asking must not leave it up forever.
            Phase::Password {
                backstop: Some(Instant::now() + TIMEOUT),
                waiting: false,
                focus_set: false,
                notice: None,
                prompt: "Password:".into(),
                echo: false,
            }
        };
        Self {
            subject,
            chain,
            to_ui,
            from_ui,
            password: Secret::new(),
            width,
            phase,
            came_from_fingerprint,
        }
    }

    fn enter_password(&mut self, ctx: &egui::Context, text: String, echo: bool) {
        self.phase = Phase::Password {
            backstop: Some(Instant::now() + TIMEOUT),
            waiting: false,
            focus_set: false,
            notice: None,
            prompt: text,
            echo,
        };
        if self.came_from_fingerprint {
            self.resize(ctx, WINDOW_HEIGHT);
        }
    }

    /// Take everything the helper thread has queued. Returns false when the
    /// conversation is over and the window should close.
    fn drain(&mut self, ctx: &egui::Context) -> bool {
        loop {
            match self.to_ui.try_recv() {
                Ok(ToUi::Prompt { text, echo }) => {
                    self.cover(ctx, &text);
                    match &mut self.phase {
                        Phase::Fingerprint { .. } => self.enter_password(ctx, text, echo),
                        // The notice stays: a re-prompt follows `Wrong
                        // password` within milliseconds, and clearing it here
                        // would wipe the line before a frame draws it. It
                        // goes when the next answer is submitted.
                        Phase::Password {
                            waiting,
                            focus_set,
                            prompt,
                            echo: echo_slot,
                            backstop,
                            ..
                        } => {
                            *prompt = text;
                            *echo_slot = echo;
                            *waiting = false;
                            *focus_set = false;
                            // A new question deserves a fresh deadline.
                            *backstop = Some(Instant::now() + TIMEOUT);
                        }
                    }
                }
                Ok(ToUi::Info(text)) => {
                    self.cover(ctx, &text);
                    if !self.came_from_fingerprint
                        && let Phase::Password { notice, .. } = &mut self.phase
                    {
                        *notice = Some((text, false));
                    }
                }
                Ok(ToUi::Error(text)) => {
                    self.cover(ctx, &text);
                    let wrong = text == crate::attempts::WRONG_PASSWORD;
                    match &mut self.phase {
                        Phase::Fingerprint { notice } => *notice = Some(text),
                        Phase::Password {
                            waiting,
                            focus_set,
                            notice,
                            ..
                        } => {
                            // Every error is shown: PAM's own words, a helper
                            // that went away, a locked account. Only a rejected
                            // password frees the field, though -- any other
                            // error can arrive while an answer is still on
                            // its way to a helper that is busy at the sensor.
                            *notice = Some((text, true));
                            if wrong {
                                *waiting = false;
                                *focus_set = false;
                            }
                        }
                    }
                    if wrong {
                        self.password.wipe();
                    }
                }
                Ok(ToUi::Attempts(attempts)) => {
                    self.subject.attempts = attempts;
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
    ///
    /// This is the caller's clock, not ours, so it runs through both phases:
    /// the seconds a fingerprint wait uses up are seconds the password no
    /// longer has, and the window is the only place that can say so.
    fn seconds_left(&self) -> Option<u64> {
        let deadline = self.subject.deadline?;
        countdown(deadline.saturating_duration_since(Instant::now()))
    }

    fn submit(&mut self) {
        let password = std::mem::take(&mut self.password);
        let _ = self.from_ui.send(FromUi::Answer(password));
        self.password = Secret::new();
        if let Phase::Password {
            waiting, notice, ..
        } = &mut self.phase
        {
            *waiting = true;
            *notice = None;
        }
    }

    fn cancel(&mut self, ctx: &egui::Context) {
        self.password.wipe();
        let _ = self.from_ui.send(FromUi::Cancel);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn backstop_hit(&self) -> bool {
        match self.phase {
            Phase::Password {
                backstop: Some(t), ..
            } => Instant::now() >= t,
            _ => false,
        }
    }

    /// The sensor wait: a glyph and a short hint, or PAM's last complaint in
    /// the hint's place. No field, no count -- pam_fprintd keeps its own tally
    /// and does not publish it, and the faillock line does not apply here.
    fn draw_fingerprint_phase(&self, ui: &mut egui::Ui) {
        let Phase::Fingerprint { notice } = &self.phase else {
            return;
        };
        let color = if notice.is_some() {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().hyperlink_color
        };
        ui.add(egui::Label::new(
            egui::RichText::new(FINGERPRINT_GLYPH)
                .size(28.0)
                .family(egui::FontFamily::Monospace)
                .color(color),
        ));
        ui.add_space(6.0);
        match notice {
            Some(text) => {
                ui.label(egui::RichText::new(text).size(11.0).color(color));
            }
            None => {
                ui.label(egui::RichText::new(FINGERPRINT_HINT).size(11.0).weak());
            }
        }
    }

    /// Returns true when the user submitted a non-empty password.
    fn draw_password_phase(&mut self, ui: &mut egui::Ui) -> bool {
        let (waiting, echo, prompt, notice) = match &self.phase {
            Phase::Password {
                waiting,
                echo,
                prompt,
                notice,
                ..
            } => (*waiting, *echo, prompt.clone(), notice.clone()),
            Phase::Fingerprint { .. } => return false,
        };

        if echo && !prompt.is_empty() {
            ui.label(
                egui::RichText::new(&prompt)
                    .size(11.5)
                    .color(ui.visuals().text_color().gamma_multiply(0.5)),
            );
            ui.add_space(8.0);
        }

        let entered = ui
            .horizontal(|ui| {
                let slack = ui.available_width() - FIELD_ROW_WIDTH;
                ui.add_space((slack / 2.0).max(0.0));
                let field_h = ui.text_style_height(&egui::TextStyle::Monospace) + 16.0;
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
                    !waiting,
                    egui::TextEdit::singleline(self.password.buffer_mut())
                        .password(!echo)
                        .char_limit(crate::secret::MAX_CHARS)
                        .font(egui::TextStyle::Monospace)
                        .margin(egui::Margin::symmetric(10, 8))
                        .desired_width(FIELD_ROW_WIDTH - LOCK_WIDTH - LOCK_GAP),
                );
                if let Phase::Password {
                    focus_set, waiting, ..
                } = &mut self.phase
                    && !*waiting
                    && !*focus_set
                {
                    field.request_focus();
                    *focus_set = true;
                }
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if entered && !waiting && self.password.is_empty() {
                    field.request_focus();
                }
                entered
            })
            .inner;

        ui.add_space(14.0);
        let transient = match (&notice, waiting) {
            (Some((text, error)), _) => Some((text.as_str(), *error)),
            (None, true) => Some(("Checking...", false)),
            (None, false) => None,
        };
        match transient {
            Some((text, true)) => {
                ui.label(
                    egui::RichText::new(text)
                        .size(11.0)
                        .color(ui.visuals().error_fg_color),
                );
            }
            Some((text, false)) => {
                ui.label(egui::RichText::new(text).size(11.0).weak());
            }
            None => {
                ui.label(egui::RichText::new(" ").size(11.0));
            }
        }

        // The standing line: how much of the shared faillock budget is left,
        // true for as long as the window is open and re-read after every
        // wrong answer. It is never taken away to make room for a notice.
        if let Some((text, low)) = &self.subject.attempts {
            ui.add_space(2.0);
            let text = egui::RichText::new(text).size(11.0);
            ui.label(if *low {
                text.color(ui.visuals().error_fg_color)
            } else {
                text.weak()
            });
        }

        entered && !waiting && !self.password.is_empty()
    }

    fn resize(&self, ctx: &egui::Context, height: f32) {
        let size = egui::vec2(self.width, height);
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
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
        // The switch from the sensor layout to the field asked for 200 once;
        // ask again only while the compositor still reports the old height,
        // not every frame for the rest of the window's life.
        if matches!(self.phase, Phase::Password { .. }) && self.came_from_fingerprint {
            let shown = ctx.input(|i| i.viewport().inner_rect.map(|r| r.height()));
            if shown.is_some_and(|h| (h - WINDOW_HEIGHT).abs() > 1.0) {
                self.resize(&ctx, WINDOW_HEIGHT);
            }
        }
        // Nothing wakes this loop when the helper speaks, so look often.
        ctx.request_repaint_after(Duration::from_millis(50));

        // SIGTERM from the agent (polkitd cancelled) takes the same road as
        // Esc, so the helper thread drops its channel before this process
        // ends -- see `prompt::TERMINATED`.
        if self.backstop_hit()
            || crate::prompt::terminated()
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
                    let fingerprint = matches!(self.phase, Phase::Fingerprint { .. });
                    ui.add_space(if fingerprint { 10.0 } else { 18.0 });
                    if fingerprint {
                        self.draw_fingerprint_phase(ui);
                    } else if self.draw_password_phase(ui) {
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
        assert_eq!(
            clamp_width(10_000.0),
            MAX_WIDTH,
            "a runaway argv cannot fill the screen"
        );
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

    #[test]
    fn the_fingerprint_glyph_is_md_fingerprint_not_weather() {
        assert_eq!(FINGERPRINT_GLYPH, "\u{f0237}");
        assert_ne!(FINGERPRINT_GLYPH, "\u{f0597}");
    }

    #[test]
    fn fingerprint_wait_uses_a_shorter_window() {
        assert_eq!(window_height(true), FINGERPRINT_HEIGHT);
        assert_eq!(window_height(false), WINDOW_HEIGHT);
        assert_eq!(WINDOW_HEIGHT, 200.0);
    }
}
