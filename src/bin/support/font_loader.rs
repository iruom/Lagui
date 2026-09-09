use best_gui::text::Font;
use std::{error::Error, path::PathBuf};

pub fn load_font() -> Result<Font, Box<dyn Error>> {
    let index = std::env::var("BEST_GUI_FONT_INDEX")
        .unwrap_or_else(|_| "0".into())
        .parse()?;
    let path = if let Some(path) = std::env::var_os("BEST_GUI_FONT") {
        PathBuf::from(path)
    } else {
        let candidates = if cfg!(target_os = "macos") {
            vec![
                PathBuf::from("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"),
                PathBuf::from("/System/Library/Fonts/Hiragino Sans GB.ttc"),
            ]
        } else if cfg!(target_os = "windows") {
            let root =
                PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()));
            vec![
                root.join("Fonts/YuGothM.ttc"),
                root.join("Fonts/meiryo.ttc"),
            ]
        } else {
            vec![
                PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
                PathBuf::from("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
            ]
        };
        candidates
            .into_iter()
            .find(|path| path.is_file())
            .ok_or("Set BEST_GUI_FONT to a Japanese-capable TTF/OTF/TTC font path")?
    };
    println!("Font: {} (face {index})", path.display());
    Ok(Font::new(std::fs::read(path)?, index)?)
}
