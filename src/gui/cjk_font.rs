//! CJK fallback font. egui's bundled fonts have no Chinese/Japanese/Korean glyphs, so mod, group
//! and profile names in those scripts render as boxes. A system font is appended as a fallback,
//! after egui's own fonts, so Latin text looks unchanged.

use std::path::{Path, PathBuf};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use tracing::info;

/// Environment variable to force a specific font file (face 0 is used).
const OVERRIDE_VAR: &str = "MINT_CJK_FONT";

/// Candidate font files and the face to use inside them (font collections hold several faces;
/// Noto Sans CJK's face 3 is Traditional Chinese).
const CANDIDATES: &[(&str, u32)] = &[
    // Linux
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 3),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 3),
    ("/usr/share/fonts/noto-cjk/NotoSansCJKtc-Regular.otf", 0),
    (
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        3,
    ),
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-VF.otf.ttc", 3),
    (
        "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        0,
    ),
    // Windows
    ("C:\\Windows\\Fonts\\msjh.ttc", 0),
    ("C:\\Windows\\Fonts\\msyh.ttc", 0),
    ("C:\\Windows\\Fonts\\meiryo.ttc", 0),
    ("C:\\Windows\\Fonts\\malgun.ttf", 0),
    // macOS
    ("/System/Library/Fonts/PingFang.ttc", 0),
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
];

fn candidates() -> impl Iterator<Item = (PathBuf, u32)> {
    let env = std::env::var_os(OVERRIDE_VAR).map(|p| (PathBuf::from(p), 0));
    env.into_iter()
        .chain(CANDIDATES.iter().map(|(p, i)| (PathBuf::from(p), *i)))
}

fn load(path: &Path, index: u32) -> Option<FontData> {
    let bytes = std::fs::read(path).ok()?;
    // reject files ab_glyph can't parse instead of letting egui panic on them later
    ab_glyph::FontRef::try_from_slice_and_index(&bytes, index).ok()?;
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    Some(data)
}

/// Adds the first available CJK font as the last fallback of the proportional and monospace
/// families.
pub fn install(ctx: &egui::Context) {
    let Some((path, data)) = candidates().find_map(|(path, index)| {
        let data = load(&path, index)?;
        Some((path, data))
    }) else {
        info!("no CJK font found; set {OVERRIDE_VAR}=/path/to/font to show CJK text");
        return;
    };
    info!("using CJK fallback font {}", path.display());

    let mut fonts = FontDefinitions::default();
    let name = "cjk-fallback".to_string();
    fonts.font_data.insert(name.clone(), data.into());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push(name.clone());
    }
    ctx.set_fonts(fonts);
}
