//! Keep plaintext out of egui's raw input, undo history and layout snapshots.
//!
//! Before Context::run_ui, replace each input payload with an opaque per-event
//! marker. TextEdit only sees markers and a masked TextBuffer. Its insertions
//! resolve those markers into locked Secret allocations, without returning
//! plaintext to egui. Unknown/replayed markers cannot insert anything.
//! Explicit PAM echo-on prompts may expose their non-secret text to the renderer.
//! OS/IME/clipboard/backend copies made before this hook are outside this boundary.

use std::collections::HashMap;
use std::ops::Range;

use eframe::egui::{self, Event, ImeEvent, Key, TextBuffer, text::CharIndex};
use zeroize::Zeroize;

use crate::secret::{MAX_CHARS, Secret};

pub(crate) const FIELD_ID: &str = "sudo-pop-password";

#[derive(Default)]
pub(crate) struct SecureInput {
    pending: HashMap<char, (Secret, usize)>,
}

impl SecureInput {
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Called before egui clones RawInput. Even discarded input is wiped.
    pub fn protect(&mut self, raw: &mut egui::RawInput, accept: bool) {
        if !accept {
            self.clear();
        }
        let mut next = self
            .pending
            .keys()
            .map(|c| *c as u32)
            .max()
            .map_or(0xe000, |c| c + 1);
        raw.events.retain_mut(|event| {
            let paste = matches!(event, Event::Paste(_));
            let text = match event {
                Event::Text(text)
                | Event::Paste(text)
                | Event::Ime(ImeEvent::Commit(text))
                | Event::Ime(ImeEvent::Preedit { text, .. }) => text,
                // Undo cannot reconstruct a secret from mask-only history.
                Event::Key {
                    key: Key::Z | Key::Y,
                    modifiers,
                    ..
                } if modifiers.command || modifiers.ctrl => return false,
                _ => return true,
            };
            // eframe can return the same RawInput while a window is occluded.
            // Preserve its pending payload, rather than interpreting our marker
            // as a new password. The allocation identity distinguishes new input.
            if accept
                && text.chars().next().is_some_and(|marker| {
                    self.pending
                        .get(&marker)
                        .is_some_and(|(_, ptr)| *ptr == text.as_ptr() as usize)
                        && text.chars().all(|c| c == marker)
                })
            {
                return true;
            }
            if !accept || next > 0xf8ff || (!paste && (text == "\n" || text == "\r")) {
                text.zeroize();
                return false;
            }
            // Empty IME events must survive: they cancel a composition.
            if text.is_empty() {
                return true;
            }
            let mut secret = Secret::new();
            for c in text.chars().take(MAX_CHARS) {
                let c = if paste && matches!(c, '\r' | '\n') {
                    ' '
                } else {
                    c
                };
                secret.buffer_mut().push(c);
            }
            text.zeroize();
            let marker = char::from_u32(next).unwrap();
            next += 1;
            text.extend(std::iter::repeat_n(marker, secret.text().chars().count()));
            self.pending
                .insert(marker, (secret, text.as_ptr() as usize));
            true
        });
    }

    pub fn buffer<'a>(&mut self, secret: &'a mut Secret, echo: bool) -> MaskedBuffer<'a> {
        MaskedBuffer {
            mask: "•".repeat(secret.text().chars().count()),
            secret,
            echo,
            pending: std::mem::take(&mut self.pending),
        }
    }
}

pub(crate) struct MaskedBuffer<'a> {
    secret: &'a mut Secret,
    echo: bool,
    mask: String,
    pending: HashMap<char, (Secret, usize)>,
}

impl TextBuffer for MaskedBuffer<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MaskedBuffer<'static>>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        if self.echo {
            self.secret.text()
        } else {
            &self.mask
        }
    }

    fn insert_text(&mut self, text: &str, at: CharIndex) -> usize {
        let Some(marker) = text.chars().next() else {
            return 0;
        };
        if !text.chars().all(|c| c == marker) {
            return 0;
        }
        let Some((payload, _)) = self.pending.remove(&marker) else {
            return 0;
        };
        let inserted = self
            .secret
            .insert(at.0, payload.text(), text.chars().count());
        self.mask = "•".repeat(self.secret.text().chars().count());
        inserted
    }

    fn delete_char_range(&mut self, range: Range<CharIndex>) {
        self.secret.delete(range.start.0..range.end.0);
        self.mask = "•".repeat(self.secret.text().chars().count());
    }

    // TextEdit's undo/redo uses replace_with. Never treat mask history as input.
    fn replace_with(&mut self, _text: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        input: &mut SecureInput,
        secret: &mut Secret,
        events: Vec<Event>,
        time: f64,
    ) {
        let mut raw = egui::RawInput {
            events,
            time: Some(time),
            ..Default::default()
        };
        input.protect(&mut raw, true);
        for event in &raw.events {
            match event {
                Event::Text(s)
                | Event::Paste(s)
                | Event::Ime(ImeEvent::Commit(s))
                | Event::Ime(ImeEvent::Preedit { text: s, .. }) => {
                    assert!(s.chars().all(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)));
                }
                _ => {}
            }
        }
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(FIELD_ID)));
        let mut output = ctx.run_ui(raw, |ui| {
            ui.scope(|ui| {
                let mut buffer = input.buffer(secret, false);
                ui.add(
                    egui::TextEdit::singleline(&mut buffer)
                        .id(egui::Id::new(FIELD_ID))
                        .password(true)
                        .char_limit(MAX_CHARS),
                );
                assert!(buffer.as_str().chars().all(|c| c == '•'));
            });
        });
        output.textures_delta.clear();
        // A password field must never put its contents on the clipboard.
        assert!(output.platform_output.commands.is_empty());
        let state = egui::text_edit::TextEditState::load(ctx, egui::Id::new(FIELD_ID)).unwrap();
        let mut history = state.undoer();
        let mut current = (
            egui::text::CCursorRange::default(),
            "•".repeat(secret.text().chars().count()),
        );
        while let Some(previous) = history.undo(&current) {
            assert!(previous.1.chars().all(|c| c == '•'));
            current = previous.clone();
        }
        input.clear();
    }

    fn key(key: Key, modifiers: egui::Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn egui_retains_only_masks_while_unicode_editing_still_works() {
        let ctx = egui::Context::default();
        let mut input = SecureInput::default();
        let mut secret = Secret::new();
        frame(&ctx, &mut input, &mut secret, vec![], 0.0);
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![Event::Text("a한🔐z".into())],
            1.0,
        );
        assert_eq!(secret.text(), "a한🔐z");
        frame(&ctx, &mut input, &mut secret, vec![], 3.0);
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![
                key(Key::ArrowLeft, egui::Modifiers::NONE),
                key(Key::Backspace, egui::Modifiers::NONE),
            ],
            4.0,
        );
        assert_eq!(secret.text(), "a한z");
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![Event::Paste("XY".into())],
            5.0,
        );
        assert_eq!(secret.text(), "a한XYz");
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![key(Key::Z, egui::Modifiers::COMMAND)],
            6.0,
        );
        assert_eq!(secret.text(), "a한XYz");
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![
                key(Key::A, egui::Modifiers::COMMAND),
                Event::Copy,
                Event::Cut,
            ],
            7.0,
        );
        assert!(secret.is_empty());
    }

    #[test]
    fn ime_preedit_replacement_commit_and_cancel_do_not_expose_plaintext() {
        let ctx = egui::Context::default();
        let mut input = SecureInput::default();
        let mut secret = Secret::new();
        frame(&ctx, &mut input, &mut secret, vec![], 0.0);
        let preedit = |s: &str| {
            Event::Ime(ImeEvent::Preedit {
                text: s.into(),
                active_range_chars: None,
            })
        };
        frame(&ctx, &mut input, &mut secret, vec![preedit("ㅎ")], 1.0);
        frame(&ctx, &mut input, &mut secret, vec![preedit("하")], 2.0);
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![Event::Ime(ImeEvent::Commit("한".into()))],
            3.0,
        );
        assert_eq!(secret.text(), "한");
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![preedit("글"), preedit("")],
            4.0,
        );
        assert_eq!(secret.text(), "한");
    }

    #[test]
    fn rejected_input_is_removed_and_unknown_or_replayed_tokens_cannot_insert() {
        let mut input = SecureInput::default();
        let mut raw = egui::RawInput {
            events: vec![Event::Paste("private".into())],
            ..Default::default()
        };
        input.protect(&mut raw, false);
        assert!(raw.events.is_empty());
        assert!(input.pending.is_empty());
        raw.events.push(Event::Text("private".into()));
        input.protect(&mut raw, true);
        let Event::Text(token) = &raw.events[0] else {
            panic!()
        };
        let mut secret = Secret::new();
        let mut buffer = input.buffer(&mut secret, false);
        assert_eq!(buffer.insert_text("forged", CharIndex(0)), 0);
        assert_eq!(buffer.insert_text(token, CharIndex(0)), 7);
        assert_eq!(buffer.insert_text(token, CharIndex(0)), 0);
        buffer.replace_with("•");
        drop(buffer);
        assert_eq!(secret.text(), "private");
    }

    #[test]
    fn oversized_paste_is_bounded_and_does_not_reallocate_the_secret() {
        let ctx = egui::Context::default();
        let mut input = SecureInput::default();
        let mut secret = Secret::new();
        let ptr = secret.as_bytes().as_ptr();
        frame(&ctx, &mut input, &mut secret, vec![], 0.0);
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![Event::Paste("🔐".repeat(1000))],
            1.0,
        );
        assert_eq!(secret.text().chars().count(), MAX_CHARS);
        assert_eq!(ptr, secret.as_bytes().as_ptr());
    }

    #[test]
    fn deferred_raw_input_keeps_its_original_payload() {
        let mut input = SecureInput::default();
        let mut raw = egui::RawInput {
            events: vec![Event::Text("original".into())],
            ..Default::default()
        };
        input.protect(&mut raw, true);
        input.protect(&mut raw, true);
        let Event::Text(token) = &raw.events[0] else {
            panic!()
        };
        let mut secret = Secret::new();
        let mut buffer = input.buffer(&mut secret, false);
        assert_eq!(buffer.insert_text(token, CharIndex(0)), 8);
        drop(buffer);
        assert_eq!(secret.text(), "original");
    }

    #[test]
    fn multiple_events_at_capacity_keep_the_correct_payload_order() {
        let ctx = egui::Context::default();
        let mut input = SecureInput::default();
        let mut secret = Secret::new();
        frame(&ctx, &mut input, &mut secret, vec![], 0.0);
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![
                Event::Text("x".repeat(255)),
                Event::Paste("한글".into()),
                Event::Text("ignored".into()),
                key(Key::Backspace, egui::Modifiers::NONE),
                Event::Text("z".into()),
            ],
            1.0,
        );
        assert_eq!(secret.text(), format!("{}z", "x".repeat(255)));
    }

    #[test]
    fn oversized_ime_composition_can_be_replaced_by_its_commit() {
        let ctx = egui::Context::default();
        let mut input = SecureInput::default();
        let mut secret = Secret::new();
        frame(&ctx, &mut input, &mut secret, vec![], 0.0);
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![Event::Ime(ImeEvent::Preedit {
                text: "한".repeat(300),
                active_range_chars: Some(290..300),
            })],
            1.0,
        );
        frame(
            &ctx,
            &mut input,
            &mut secret,
            vec![Event::Ime(ImeEvent::Commit("확정".into()))],
            2.0,
        );
        assert_eq!(secret.text(), "확정");
    }
}
