//! The font Omarchy is currently set to.
//!
//! Omarchy does not put the font in the theme; `omarchy-font-set` writes a
//! fontconfig rule that points `monospace` at the chosen family, and
//! `omarchy-font-current` is literally `fc-match monospace`. So asking
//! fontconfig is asking Omarchy, and it keeps working for anyone not running
//! Omarchy at all.
//!
//! Loading is best effort throughout: no fontconfig, no file, or a font too
//! large to be worth the startup cost all leave egui's bundled fonts in place.

use std::process::Command;
use std::sync::Arc;

use eframe::egui;

/// Fonts past this size are skipped.
///
/// A popup that is already on screen when you look up is this tool's whole
/// value, and a full CJK face is tens of megabytes to parse. Ordinary UI and
/// Nerd Font faces sit far below the cap — the JetBrainsMono Nerd Font this
/// machine uses is 2.5 MB.
const MAX_FONT_BYTES: u64 = 8 * 1024 * 1024;

/// Install the current font ahead of egui's defaults.
///
/// The defaults stay in the fallback chain, so glyphs the chosen face lacks
/// still render instead of turning into blanks.
pub fn apply(ctx: &egui::Context) {
    let Some((name, bytes)) = load() else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));

    // Omarchy's font is a terminal face, and the prompt it renders came from
    // the terminal, so it earns both families rather than just monospace.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, name.clone());
    }

    ctx.set_fonts(fonts);
}

/// Ask fontconfig for the current monospace face: its family name and bytes.
fn load() -> Option<(String, Vec<u8>)> {
    let out = Command::new("fc-match")
        .args(["monospace", "-f", "%{family}\n%{file}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    // fontconfig returns comma-separated aliases; the first is the real name.
    let family = lines.next()?.split(',').next()?.trim();
    let path = lines.next()?.trim();
    if family.is_empty() || path.is_empty() {
        return None;
    }

    let size = std::fs::metadata(path).ok()?.len();
    if size > MAX_FONT_BYTES {
        eprintln!("sudo-pop: skipping {family} ({size} bytes) to keep startup fast");
        return None;
    }

    Some((family.to_owned(), std::fs::read(path).ok()?))
}
