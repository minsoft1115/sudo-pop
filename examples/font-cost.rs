//! What the on-demand CJK fallback actually costs, measured rather than guessed.
//!
//!     cargo run --release --example font-cost
//!
//! Three things are timed separately, because they land in different places:
//! the fontconfig query and the file read happen wherever we choose to put
//! them, while the parse happens inside egui on the thread that owns the
//! window (`FontsImpl::new` builds every face in the chain eagerly).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui::epaint::text::{Fonts, TextOptions};

/// A Korean command line, the case this is all about.
const KOREAN: &str = "vim 계획서.md";
const ASCII: &str = "pacman -Syu";

fn main() {
    let primary = fc_match("monospace").expect("fc-match monospace");
    let wide = fc_match(":charset=ACC4 D68D C11C").expect("fc-match for Hangul");
    println!("primary : {} ({} bytes)", primary.0, size(&primary.1));
    println!("fallback: {} ({} bytes)\n", wide.0, size(&wide.1));

    println!("-- fontconfig --");
    time("fc-match monospace       ", 5, || {
        fc_match("monospace").unwrap();
    });
    time("fc-match :charset=<3 kr> ", 5, || {
        fc_match(":charset=ACC4 D68D C11C").unwrap();
    });

    println!("\n-- reading the file --");
    time("primary, warm cache      ", 5, || {
        std::fs::read(&primary.1).unwrap();
    });
    time("fallback, warm cache     ", 5, || {
        std::fs::read(&wide.1).unwrap();
    });
    time("fallback, cold cache     ", 3, || {
        evict(&wide.1);
        std::fs::read(&wide.1).unwrap();
    });

    println!("\n-- egui parsing the chain (this one is on the UI thread) --");
    let primary_bytes = std::fs::read(&primary.1).unwrap();
    let wide_bytes = std::fs::read(&wide.1).unwrap();

    let bundled = egui::FontDefinitions::default();
    let with_primary = chain(&[(&primary.0, &primary_bytes)]);
    let with_both = chain(&[(&primary.0, &primary_bytes), (&wide.0, &wide_bytes)]);

    time("egui defaults only       ", 5, || {
        Fonts::new(TextOptions::default(), bundled.clone());
    });
    time("+ primary  (today)       ", 5, || {
        Fonts::new(TextOptions::default(), with_primary.clone());
    });
    time("+ primary + CJK fallback ", 5, || {
        Fonts::new(TextOptions::default(), with_both.clone());
    });

    println!("\n-- laying out one line --");
    time("ascii,  primary chain    ", 5, || layout(&with_primary, ASCII));
    time("korean, primary chain    ", 5, || layout(&with_primary, KOREAN));
    time("korean, chain + fallback ", 5, || layout(&with_both, KOREAN));

    println!("\n-- what the two launches add up to --");
    time("ascii  launch (fc+read+parse+layout)", 5, || {
        let (name, path) = fc_match("monospace").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let defs = chain(&[(&name, &bytes)]);
        layout(&defs, ASCII);
    });
    time("korean launch (both faces, warm)   ", 5, || {
        let (name, path) = fc_match("monospace").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let (wname, wpath) = fc_match(":charset=ACC4 D68D C11C").unwrap();
        let wbytes = std::fs::read(wpath).unwrap();
        let defs = chain(&[(&name, &bytes), (&wname, &wbytes)]);
        layout(&defs, KOREAN);
    });
}

/// Build a chain: the listed faces after egui's defaults are seeded, in order.
fn chain(faces: &[(&str, &Vec<u8>)]) -> egui::FontDefinitions {
    let mut defs = egui::FontDefinitions::default();
    for (i, (name, bytes)) in faces.iter().enumerate() {
        defs.font_data.insert(
            (*name).to_owned(),
            Arc::new(egui::FontData::from_owned((*bytes).clone())),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let list = defs.families.entry(family).or_default();
            // First face leads (the Omarchy font), the rest trail.
            if i == 0 {
                list.insert(0, (*name).to_owned());
            } else {
                list.push((*name).to_owned());
            }
        }
    }
    defs
}

/// Build the fonts and lay one line out, as a frame would.
fn layout(defs: &egui::FontDefinitions, text: &str) {
    let mut fonts = Fonts::new(TextOptions::default(), defs.clone());
    fonts.begin_pass(TextOptions::default());
    let galley = fonts.with_pixels_per_point(1.0).layout_no_wrap(
        text.to_owned(),
        egui::FontId::monospace(11.5),
        egui::Color32::WHITE,
    );
    std::hint::black_box(galley.rect.width());
}

fn time(label: &str, runs: u32, mut f: impl FnMut()) {
    let mut best = Duration::MAX;
    let mut total = Duration::ZERO;
    for _ in 0..runs {
        let t = Instant::now();
        f();
        let d = t.elapsed();
        best = best.min(d);
        total += d;
    }
    println!(
        "{label}  min {:>8.2?}   mean {:>8.2?}",
        best,
        total / runs
    );
}

fn fc_match(pattern: &str) -> Option<(String, PathBuf)> {
    let out = Command::new("fc-match")
        .args([pattern, "-f", "%{family}\n%{file}"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let family = lines.next()?.split(',').next()?.trim().to_owned();
    let path = PathBuf::from(lines.next()?.trim());
    Some((family, path))
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Drop this file's pages so the next read is a real one.
fn evict(path: &Path) {
    use std::os::unix::io::AsRawFd;
    let Ok(f) = std::fs::File::open(path) else {
        return;
    };
    // SAFETY: a live fd, and DONTNEED on clean pages cannot lose data.
    unsafe {
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}
