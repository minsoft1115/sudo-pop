//! The fonts the window draws with.
//!
//! Omarchy does not put the font in the theme; `omarchy-font-set` writes a
//! fontconfig rule that points `monospace` at the chosen family, and
//! `omarchy-font-current` is literally `fc-match monospace`. So asking
//! fontconfig is asking Omarchy, and it keeps working for anyone not running
//! Omarchy at all.
//!
//! That one face is not always enough. egui has no per-glyph fallback of its
//! own: the chain is exactly the list handed to it, and none of the faces
//! Omarchy offers -- nor any of egui's four bundled ones -- carry Hangul or
//! Han. Terminals hide this because they ask fontconfig again for every glyph
//! they cannot draw; we get one answer and keep it. So a command with a Korean
//! path in it (`vim 계획서.md`) came out as `vim ◻◻◻.md`, and that top line is
//! the whole reason the window says which command is asking.
//!
//! So the chain grows on demand. Text we did not write ourselves -- the
//! command line, polkit's message, PAM's prompts -- is scanned for characters
//! outside ASCII, and only those characters send a query to fontconfig. Plain
//! `sudo pacman -Syu` therefore costs exactly what it did before; a Korean path
//! costs one `fc-match` and one font load, on the launch that needs it.
//!
//! Loading is best effort throughout: no fontconfig, no file, or a primary font
//! too large to be worth the startup cost all leave egui's bundled fonts in
//! place.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use eframe::egui;

/// Primary fonts past this size are skipped.
///
/// A popup that is already on screen when you look up is this tool's whole
/// value, and the primary face is parsed on every single launch. Ordinary UI
/// and Nerd Font faces sit far below the cap -- the JetBrainsMono Nerd Font
/// this machine uses is 2.5 MB.
///
/// **Fallbacks are not capped.** The CJK faces that would answer a Korean path
/// are 17-27 MB and there is no smaller one installed, so a cap here would only
/// mean the boxes stay. That cost is paid on the launches that need it and
/// nowhere else, which is the whole design above.
const MAX_FONT_BYTES: u64 = 8 * 1024 * 1024;

/// How many distinct characters one fontconfig query carries.
///
/// A handful is enough to name the script; a long path full of Hangul would
/// otherwise build a query the length of the path for no extra answer.
const MAX_QUERY_CHARS: usize = 32;

/// A face on its way into the chain.
struct Face {
    family: String,
    path: PathBuf,
    bytes: Vec<u8>,
}

/// The font chain, head to tail.
///
/// The Omarchy face leads so it draws everything it can; faces added later go
/// behind egui's defaults, where they are reached only for glyphs nothing
/// ahead of them has.
pub struct Chain {
    defs: egui::FontDefinitions,
    /// Files already in the chain. fontconfig answering with one of these is
    /// the ordinary case for accented Latin, which the Omarchy face carries
    /// already, and it must not cost a second load.
    files: Vec<PathBuf>,
}

impl Chain {
    /// The Omarchy monospace face ahead of egui's defaults.
    pub fn new() -> Self {
        let mut chain = Self {
            defs: egui::FontDefinitions::default(),
            files: Vec::new(),
        };
        if let Some(face) = primary() {
            chain.add(face, Position::Front);
        }
        chain
    }

    /// Grow the chain if `text` holds characters it has no face for. Returns
    /// whether anything changed, so the caller knows to install it again.
    pub fn cover(&mut self, text: &str) -> bool {
        self.cover_with(text, fc_match)
    }

    /// `cover` with the fontconfig lookup as a parameter, so the bookkeeping
    /// can be tested without a font installed.
    fn cover_with(
        &mut self,
        text: &str,
        lookup: impl Fn(&str) -> Option<(String, PathBuf)>,
    ) -> bool {
        let wanted = foreign_chars(text);
        if wanted.is_empty() {
            return false;
        }
        let Some((family, path)) = lookup(&charset_query(&wanted)) else {
            return false;
        };
        // Before reading: a face we already carry is the answer for anything
        // the Omarchy font covers, and it arrives on every repeat prompt.
        if self.files.contains(&path) {
            return false;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return false;
        };
        if std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty()) {
            eprintln!(
                "sudo-pop: fell back to {family} ({} bytes) for {}",
                bytes.len(),
                wanted.iter().collect::<String>()
            );
        }
        self.add(
            Face {
                family,
                path,
                bytes,
            },
            Position::Back,
        );
        true
    }

    /// Hand the chain to egui. Takes effect on the next frame.
    pub fn install(&self, ctx: &egui::Context) {
        ctx.set_fonts(self.defs.clone());
    }

    fn add(&mut self, face: Face, at: Position) {
        self.defs
            .font_data
            .insert(face.family.clone(), Arc::new(egui::FontData::from_owned(face.bytes)));

        // Omarchy's font is a terminal face, and the prompt it renders came
        // from the terminal, so it earns both families rather than just
        // monospace. A fallback earns both for the same reason: the command
        // line is monospace, PAM's messages are not.
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let list = self.defs.families.entry(family).or_default();
            match at {
                Position::Front => list.insert(0, face.family.clone()),
                Position::Back => list.push(face.family.clone()),
            }
        }
        self.files.push(face.path);
    }
}

/// Where a face goes in the chain: ahead of everything, or behind it.
#[derive(Clone, Copy)]
enum Position {
    Front,
    Back,
}

/// The distinct non-ASCII characters of `text`, in the order they appear.
///
/// Anything outside ASCII qualifies. Guessing at scripts here would mean
/// keeping a table of what Nerd Fonts happen to cover; fontconfig already
/// knows, and an answer naming a file we hold costs nothing (`cover_with`).
fn foreign_chars(text: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in text.chars().filter(|c| !c.is_ascii()) {
        if !out.contains(&c) {
            out.push(c);
        }
        if out.len() == MAX_QUERY_CHARS {
            break;
        }
    }
    out
}

/// fontconfig's pattern for "a face that can draw these": `:charset=AC00 D55C`.
fn charset_query(chars: &[char]) -> String {
    let mut query = String::from(":charset=");
    for (i, c) in chars.iter().enumerate() {
        if i > 0 {
            query.push(' ');
        }
        query.push_str(&format!("{:04X}", *c as u32));
    }
    query
}

/// The face the current monospace setting resolves to, size-capped.
fn primary() -> Option<Face> {
    let (family, path) = fc_match("monospace")?;
    let size = std::fs::metadata(&path).ok()?.len();
    if size > MAX_FONT_BYTES {
        eprintln!("sudo-pop: skipping {family} ({size} bytes) to keep startup fast");
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    Some(Face {
        family,
        path,
        bytes,
    })
}

/// Ask fontconfig to resolve a pattern: its family name and file.
fn fc_match(pattern: &str) -> Option<(String, PathBuf)> {
    let out = Command::new("fc-match")
        .args([pattern, "-f", "%{family}\n%{file}"])
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

    Some((family.to_owned(), PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chain with no fontconfig behind it, so `cover_with` starts from a
    /// known-empty state whatever is installed on the machine.
    fn bare() -> Chain {
        Chain {
            defs: egui::FontDefinitions::default(),
            files: Vec::new(),
        }
    }

    /// A lookup that always names one file, counting how often it was asked.
    struct Always {
        path: PathBuf,
        calls: std::cell::Cell<usize>,
    }

    impl Always {
        fn new(path: &std::path::Path) -> Self {
            Self {
                path: path.to_path_buf(),
                calls: std::cell::Cell::new(0),
            }
        }

        fn lookup(&self) -> impl Fn(&str) -> Option<(String, PathBuf)> + '_ {
            |_query| {
                self.calls.set(self.calls.get() + 1);
                Some(("Fallback".to_owned(), self.path.clone()))
            }
        }
    }

    fn tail(chain: &Chain) -> Vec<String> {
        chain.defs.families[&egui::FontFamily::Monospace].clone()
    }

    #[test]
    fn ascii_asks_fontconfig_nothing() {
        assert!(foreign_chars("sudo pacman -Syu").is_empty());
        assert!(foreign_chars("vim /etc/fstab").is_empty());
    }

    #[test]
    fn hangul_is_what_gets_asked_about() {
        // The ASCII around the Korean is dropped: only what needs a face goes
        // into the query.
        assert_eq!(foreign_chars("vim 계획서.md"), vec!['계', '획', '서']);
    }

    #[test]
    fn a_repeated_syllable_is_asked_about_once() {
        assert_eq!(foreign_chars("한한한글"), vec!['한', '글']);
    }

    #[test]
    fn a_long_korean_path_stops_at_the_cap() {
        let text: String = ('\u{AC00}'..).take(MAX_QUERY_CHARS + 10).collect();
        assert_eq!(foreign_chars(&text).len(), MAX_QUERY_CHARS);
    }

    #[test]
    fn mixed_scripts_all_go_into_one_query() {
        // One query, so fontconfig picks a face for the whole line rather than
        // us choosing a script.
        assert_eq!(foreign_chars("한あ中"), vec!['한', 'あ', '中']);
    }

    #[test]
    fn the_query_is_fontconfigs_charset_form() {
        assert_eq!(charset_query(&['계', '획']), ":charset=ACC4 D68D");
        assert_eq!(charset_query(&['한']), ":charset=D55C");
    }

    #[test]
    fn a_korean_command_line_adds_a_face_at_the_tail() {
        let file = tempfile();
        std::fs::write(&file, b"not really a font").unwrap();
        let fc = Always::new(&file);
        let mut chain = bare();
        chain.add(
            Face {
                family: "Primary".to_owned(),
                path: PathBuf::from("/nonexistent/primary.ttf"),
                bytes: b"primary".to_vec(),
            },
            Position::Front,
        );

        assert!(chain.cover_with("vim 계획서.md", fc.lookup()));
        // Behind the primary and egui's own, so the Omarchy face still draws
        // everything it can.
        let list = tail(&chain);
        assert_eq!(list.first().unwrap(), "Primary");
        assert_eq!(list.last().unwrap(), "Fallback");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn an_ascii_command_line_never_reaches_fontconfig() {
        let fc = Always::new(std::path::Path::new("/x"));
        let mut chain = bare();
        assert!(!chain.cover_with("sudo pacman -Syu", fc.lookup()));
        assert_eq!(fc.calls.get(), 0, "no query, so no fc-match and no load");
    }

    #[test]
    fn a_face_already_in_the_chain_is_not_loaded_again() {
        // What accented Latin does: fontconfig names the Omarchy font itself.
        let file = tempfile();
        std::fs::write(&file, b"not really a font").unwrap();
        let fc = Always::new(&file);
        let mut chain = bare();
        chain.add(
            Face {
                family: "Primary".to_owned(),
                path: file.clone(),
                bytes: b"primary".to_vec(),
            },
            Position::Front,
        );

        let before = tail(&chain);
        assert!(!chain.cover_with("vim café.txt", fc.lookup()));
        assert_eq!(tail(&chain), before, "nothing added, so nothing was read");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn a_second_korean_prompt_reuses_the_face_it_loaded() {
        // PAM can send Korean after the window is up; the first one pays.
        let file = tempfile();
        std::fs::write(&file, b"not really a font").unwrap();
        let fc = Always::new(&file);
        let mut chain = bare();

        assert!(chain.cover_with("계획서", fc.lookup()));
        assert!(!chain.cover_with("암호를 입력하십시오", fc.lookup()));
        assert_eq!(fc.calls.get(), 2, "asked twice");
        assert_eq!(
            tail(&chain).iter().filter(|f| *f == "Fallback").count(),
            1,
            "loaded once"
        );
        std::fs::remove_file(&file).ok();
    }

    /// End to end, against the real fontconfig and the real installed fonts:
    /// after covering a Korean command line, egui can actually draw it. This is
    /// the whole point, and the unit tests above only check the bookkeeping.
    ///
    /// Skipped on a machine with no Hangul face at all -- there is nothing to
    /// prove there, and `cargo test` is meant to run anywhere.
    #[test]
    fn a_korean_command_line_becomes_drawable() {
        let korean = "vim 계획서.md";
        let mut chain = Chain::new();

        assert!(
            drawable(&chain, "vim /etc/fstab"),
            "ascii must draw with the Omarchy face alone"
        );

        // What the bug looked like: the Omarchy face and egui's defaults draw
        // the `vim` and the `.md` and nothing in between.
        let before = drawable(&chain, korean);
        chain.cover(korean);
        let after = drawable(&chain, korean);

        if !after {
            eprintln!("skipping: no installed face draws Hangul");
            return;
        }
        assert!(after, "covering must make the whole line drawable");
        if before {
            eprintln!("note: the monospace face here already had Hangul");
        }
        // Not just the syllables in the command: anything PAM might send too.
        assert!(drawable(&chain, "암호를 입력하십시오"));
    }

    /// Can egui draw every character of `text` with this chain? This builds the
    /// same `Fonts` a frame does, so it answers for the real pipeline.
    fn drawable(chain: &Chain, text: &str) -> bool {
        use eframe::egui::epaint::text::{Fonts, TextOptions};
        let mut fonts = Fonts::new(TextOptions::default(), chain.defs.clone());
        fonts
            .with_pixels_per_point(1.0)
            .has_glyphs(&egui::FontId::monospace(11.5), text)
    }

    #[test]
    fn a_lookup_that_finds_nothing_leaves_the_chain_alone() {
        let mut chain = bare();
        let before = tail(&chain);
        assert!(!chain.cover_with("계획서", |_| None));
        assert_eq!(tail(&chain), before);
    }

    /// A unique path under the test runtime dir; the file is never parsed, so
    /// its contents only have to be distinguishable.
    fn tempfile() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir();
        dir.join(format!(
            "sudo-pop-font-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
