//! Colors borrowed from the current Omarchy theme.
//!
//! Omarchy keeps a resolved palette at
//! `$XDG_STATE_HOME/omarchy/current/theme/colors.toml`, with semantic names
//! rather than raw ANSI slots. Reading it directly is a few hundred bytes and a
//! line scan — cheaper than shelling out to `omarchy-theme-color`, and startup
//! time is the one thing this window cannot spend.
//!
//! Because askpass is a fresh process per prompt, the palette is re-read every
//! time the window opens. Switching themes therefore takes effect on the next
//! prompt with no reload machinery of any kind.
//!
//! Every step degrades to egui's own defaults: a missing file, an unknown key,
//! or a malformed color leaves that part of the styling alone rather than
//! guessing.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui;

/// Palette file relative to the state directory.
const PALETTE: &str = "omarchy/current/theme/colors.toml";

/// The shell config, alongside colors.toml. Its `[polkit]` section is the
/// exact palette the system polkit dialog uses.
const SHELL: &str = "omarchy/current/theme/shell.toml";

/// The subset of the palette this window uses.
pub struct Theme {
    pub dark: bool,
    background: egui::Color32,
    surface: egui::Color32,
    text: egui::Color32,
    text_strong: egui::Color32,
    accent: egui::Color32,
    warning: egui::Color32,
}

/// Read and map the current theme, or `None` if there is nothing usable.
pub fn load() -> Option<Theme> {
    let keys = parse(&std::fs::read_to_string(palette_path()?).ok()?);

    // A palette without a usable background is not worth half-applying.
    let background = color(&keys, "background")?;

    let mut theme = Theme {
        dark: keys.get("mode").map(|m| m != "light").unwrap_or(true),
        background,
        surface: pick(
            &keys,
            &["lighter_background", "selection", "dark_background"],
        )
        .unwrap_or(background),
        text: pick(&keys, &["foreground", "light_foreground"]).unwrap_or(egui::Color32::GRAY),
        text_strong: pick(&keys, &["bright_foreground", "foreground"])
            .unwrap_or(egui::Color32::WHITE),
        accent: pick(&keys, &["accent", "blue"]).unwrap_or(egui::Color32::LIGHT_BLUE),
        warning: pick(&keys, &["red", "orange", "yellow"]).unwrap_or(egui::Color32::LIGHT_RED),
    };

    // The shell's own polkit palette wins where it exists, so our window matches
    // the system dialog -- most of all the failure color (`text-error`), which
    // we then don't have to choose ourselves.
    if let Some(polkit) = load_polkit() {
        if let Some(c) = polkit.background {
            theme.background = c;
        }
        if let Some(c) = polkit.text {
            theme.text = c;
        }
        if let Some(c) = polkit.text_error {
            theme.warning = c;
        }
        if let Some(c) = polkit.accent {
            theme.accent = c;
        }
    }

    Some(theme)
}

/// The subset of `[polkit]` we map onto our window. Any field may be absent,
/// in which case the colors.toml value stays.
struct Polkit {
    background: Option<egui::Color32>,
    text: Option<egui::Color32>,
    text_error: Option<egui::Color32>,
    accent: Option<egui::Color32>,
}

/// Read shell.toml and pull its `[polkit]` colors, or `None` if the file or the
/// section is missing.
fn load_polkit() -> Option<Polkit> {
    let text = std::fs::read_to_string(shell_path()?).ok()?;
    polkit_overlay(&parse_sections(&text))
}

/// Map a parsed shell.toml to the polkit colors, resolving `section.key`
/// references (e.g. `border = "hyprland.active-border"`) to their hex.
fn polkit_overlay(
    sections: &HashMap<String, HashMap<String, String>>,
) -> Option<Polkit> {
    let p = sections.get("polkit")?;
    let color = |key: &str| p.get(key).and_then(|raw| resolve(sections, raw, 3));
    Some(Polkit {
        background: color("background"),
        text: color("text"),
        text_error: color("text-error"),
        // The border is usually the accent; either one gives the field outline.
        accent: color("accent").or_else(|| color("border")),
    })
}

/// Resolve a shell.toml color: a `#hex`, or a `section.key` pointer into another
/// part of the file. One hop covers the polkit section; `depth` bounds any loop.
fn resolve(
    sections: &HashMap<String, HashMap<String, String>>,
    raw: &str,
    depth: u8,
) -> Option<egui::Color32> {
    if raw.starts_with('#') {
        return parse_hex(raw);
    }
    if depth == 0 {
        return None;
    }
    let (section, key) = raw.split_once('.')?;
    let next = sections.get(section)?.get(key)?;
    resolve(sections, next, depth - 1)
}

fn state_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_STATE_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/state")),
    }
}

fn palette_path() -> Option<PathBuf> {
    Some(state_dir()?.join(PALETTE))
}

fn shell_path() -> Option<PathBuf> {
    Some(state_dir()?.join(SHELL))
}

/// Group `key = "value"` lines by their `[section]` header. Comments, blank
/// lines, and keys before any section are ignored.
fn parse_sections(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim().to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                out.entry(section.clone())
                    .or_default()
                    .insert(key.trim().to_string(), value.to_string());
            }
        }
    }
    out
}

/// Collect `key = "value"` pairs. Anything else on a line is ignored.
fn parse(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim().trim_matches('"');
            if value.is_empty() {
                return None;
            }
            Some((key.trim().to_string(), value.to_string()))
        })
        .collect()
}

/// First key that resolves to a color.
fn pick(keys: &HashMap<String, String>, names: &[&str]) -> Option<egui::Color32> {
    names.iter().find_map(|name| color(keys, name))
}

fn color(keys: &HashMap<String, String>, name: &str) -> Option<egui::Color32> {
    parse_hex(keys.get(name)?)
}

/// `#rrggbb` or `#rrggbbaa`.
fn parse_hex(value: &str) -> Option<egui::Color32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    let (r, g, b) = (byte(0)?, byte(2)?, byte(4)?);
    match hex.len() {
        8 => Some(egui::Color32::from_rgba_unmultiplied(r, g, b, byte(6)?)),
        _ => Some(egui::Color32::from_rgb(r, g, b)),
    }
}

impl Theme {
    /// Build the egui visuals for this palette.
    pub fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        v.panel_fill = self.background;
        v.window_fill = self.background;
        // The password field, which egui draws with the "extreme" background.
        v.extreme_bg_color = self.surface;

        // Plain labels and the hint line.
        v.widgets.noninteractive.fg_stroke.color = self.text;
        v.widgets.inactive.fg_stroke.color = self.text;
        // `RichText::strong`, used for the prompt.
        v.widgets.active.fg_stroke.color = self.text_strong;
        v.widgets.hovered.fg_stroke.color = self.text_strong;

        // The field only reads as focused if its outline picks up the accent.
        v.selection.bg_fill = self.accent.gamma_multiply(0.4);
        v.selection.stroke.color = self.text_strong;
        v.text_cursor.stroke.color = self.accent;
        v.widgets.active.bg_stroke.color = self.accent;
        v.widgets.hovered.bg_stroke.color = self.accent;

        // egui's accent slot. The window reads it back for the command line,
        // which is the one element that should carry the theme's own color.
        v.hyperlink_color = self.accent;

        v.warn_fg_color = self.warning;
        v.error_fg_color = self.warning;

        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
mode = "dark"

accent = "#7aa2f7"
background = "#1a1b26"
foreground = "#a9b1d6"
# a comment
lighter_background = "#24283b"
bright_foreground = "#c0caf5"
red = "#f7768e"
"##;

    #[test]
    fn reads_semantic_keys() {
        let keys = parse(SAMPLE);
        assert_eq!(keys.get("mode").map(String::as_str), Some("dark"));
        assert_eq!(keys.get("accent").map(String::as_str), Some("#7aa2f7"));
        assert_eq!(keys.len(), 7);
    }

    #[test]
    fn parses_both_hex_lengths() {
        assert_eq!(
            parse_hex("#1a1b26"),
            Some(egui::Color32::from_rgb(26, 27, 38))
        );
        assert_eq!(
            parse_hex("#1a1b2680"),
            Some(egui::Color32::from_rgba_unmultiplied(26, 27, 38, 128))
        );
    }

    #[test]
    fn rejects_malformed_colors() {
        for bad in ["1a1b26", "#xyzxyz", "#1a1b2", "#", ""] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn falls_back_along_the_key_list() {
        let keys = parse(SAMPLE);
        // "selection" is absent, so the next name wins.
        assert_eq!(
            pick(&keys, &["selection", "lighter_background"]),
            parse_hex("#24283b")
        );
        assert_eq!(pick(&keys, &["nope", "still_nope"]), None);
    }

    #[test]
    fn light_mode_is_honoured() {
        let keys = parse("mode = \"light\"\nbackground = \"#ffffff\"\n");
        assert_eq!(keys.get("mode").map(String::as_str), Some("light"));
    }

    const SHELL_SAMPLE: &str = r##"
[hyprland]
active-border = "#7aa2f7"

[polkit]
# the system polkit palette
background       = "#1a1b26"
background-alpha = 1.0
text             = "#a9b1d6"
text-error       = "#f7768e"
border           = "hyprland.active-border"
accent           = "#7aa2f7"
"##;

    #[test]
    fn sections_are_grouped_by_header() {
        let s = parse_sections(SHELL_SAMPLE);
        assert_eq!(s["hyprland"]["active-border"], "#7aa2f7");
        assert_eq!(s["polkit"]["text-error"], "#f7768e");
        // comments and blank lines do not become keys
        assert!(!s["polkit"].contains_key("#"));
    }

    #[test]
    fn the_polkit_overlay_maps_colors() {
        let p = polkit_overlay(&parse_sections(SHELL_SAMPLE)).unwrap();
        assert_eq!(p.background, parse_hex("#1a1b26"));
        assert_eq!(p.text, parse_hex("#a9b1d6"));
        assert_eq!(p.text_error, parse_hex("#f7768e"), "the failure color comes from the theme");
        assert_eq!(p.accent, parse_hex("#7aa2f7"));
    }

    #[test]
    fn a_border_reference_resolves_to_another_section() {
        // accent absent, so the border reference is what fills the accent slot.
        let text = "[hyprland]\nactive-border = \"#010203\"\n[polkit]\nborder = \"hyprland.active-border\"\n";
        let p = polkit_overlay(&parse_sections(text)).unwrap();
        assert_eq!(p.accent, parse_hex("#010203"));
    }

    #[test]
    fn no_polkit_section_means_no_overlay() {
        assert!(polkit_overlay(&parse_sections("[bar]\ntext = \"#fff\"\n")).is_none());
    }

    /// A no-op off Omarchy; where the real shell.toml exists it proves the file
    /// on this machine actually maps to usable colors -- comments, references,
    /// alpha companions and all.
    #[test]
    fn a_present_shell_toml_polkit_section_resolves() {
        let Some(path) = shell_path() else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let sections = parse_sections(&text);
        if sections.contains_key("polkit") {
            let p = polkit_overlay(&sections).expect("a [polkit] section must map");
            assert!(
                p.background.is_some() && p.text_error.is_some(),
                "[polkit] is present but background/text-error did not resolve"
            );
        }
    }
}
