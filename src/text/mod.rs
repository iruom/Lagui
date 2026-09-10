//! Optional, outline-based text. One font and one directional shaping run at a time.
//! Font data is supplied by the caller; OS font files are never redistributed.
mod cache;
mod outline;
mod renderer;
#[cfg(feature = "portable")]
mod system_font;
pub use cache::TextCache;
use outline::{Curve, Outline};
pub use renderer::{TextRenderer, TextSection};
use std::{
    collections::HashMap,
    fmt,
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
    vec::Vec,
};
#[cfg(feature = "portable")]
pub use system_font::load_system_font;

#[derive(Debug)]
pub enum TextError {
    InvalidFont,
    MissingGlyph(u32),
    UnsupportedOutline(u16),
    TooComplex,
    Capacity,
    WrongFont,
    AtlasOutOfDate,
    InvalidGeometry,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn font() -> Font {
        Font::new(
            include_bytes!("../../tests/assets/test-outline.ttf").to_vec(),
            0,
        )
        .unwrap()
    }
    #[test]
    fn shaping_handles_ligatures_combining_marks_and_whitespace() {
        let mut font = font();
        assert_eq!(font.shape("ffi").unwrap().glyph_count(), 2);
        assert_eq!(font.shape("e\u{301}").unwrap().glyph_count(), 1);
        assert_eq!(font.shape("O O").unwrap().glyph_count(), 2);
        assert!((font.shape("O O").unwrap().advance[0] - 2.3).abs() < 0.001);
        assert_eq!(font.shape("口").unwrap().glyph_count(), 1);
        assert_eq!(font.shape("").unwrap().glyph_count(), 0);
    }
    #[test]
    fn outlines_are_cached_and_missing_glyphs_are_reported() {
        let mut font = font();
        font.shape("O").unwrap();
        let before = (font.cached_glyphs(), font.curve_count(), font.atlas_bytes());
        font.shape("OOOO").unwrap();
        assert_eq!(
            before,
            (font.cached_glyphs(), font.curve_count(), font.atlas_bytes())
        );
        assert!(matches!(font.shape("☃"), Err(TextError::MissingGlyph(0))));
        assert!(Font::new(std::vec![0; 8], 0).is_err());
    }
    #[test]
    fn fallback_stack_returns_source_font_and_byte_range() {
        let mut fonts = FontFallback::new(font());
        let runs = fonts.shape("O口").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].font_index, 0);
        assert_eq!(runs[0].bytes, 0..4);
        assert_eq!(runs[0].run.glyph_count(), 2);
    }
}
impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "text: {self:?}")
    }
}
impl std::error::Error for TextError {}

#[derive(Clone, Copy)]
struct Glyph {
    bounds: [f32; 4],
    band_start: u32,
    bands: u32,
    band_h: f32,
    inv_h: f32,
}
struct PositionedGlyph {
    glyph: Glyph,
    position: [f32; 2],
}

/// Reusable, size-independent shaping result. Origin in `TextSection` is the baseline.
pub struct TextRun {
    font_id: u64,
    glyphs: Vec<PositionedGlyph>,
    pub advance: [f32; 2],
}
impl TextRun {
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }
}

pub struct Font {
    bytes: Vec<u8>,
    index: u32,
    id: u64,
    units: f32,
    shaper: harfrust::ShaperData,
    glyphs: HashMap<u16, Option<Glyph>>,
    curves: Vec<Curve>,
    bands: Vec<[u32; 5]>,
}
impl Font {
    pub fn new(bytes: Vec<u8>, face_index: u32) -> Result<Self, TextError> {
        let face =
            ttf_parser::Face::parse(&bytes, face_index).map_err(|_| TextError::InvalidFont)?;
        let units = face.units_per_em() as f32;
        let reference = harfrust::FontRef::from_index(&bytes, face_index)
            .map_err(|_| TextError::InvalidFont)?;
        let shaper = harfrust::ShaperData::new(&reference);
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            bytes,
            index: face_index,
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            units,
            shaper,
            glyphs: HashMap::new(),
            curves: Vec::new(),
            bands: Vec::new(),
        })
    }
    pub fn cached_glyphs(&self) -> usize {
        self.glyphs.len()
    }
    pub fn curve_count(&self) -> usize {
        self.curves.len()
    }
    pub fn atlas_bytes(&self) -> usize {
        self.curves.len() * 32 + self.bands.len() * 20
    }

    /// Returns whether the font maps `character` to a non-notdef glyph.
    pub fn supports(&self, character: char) -> bool {
        ttf_parser::Face::parse(&self.bytes, self.index)
            .ok()
            .and_then(|face| face.glyph_index(character))
            .is_some_and(|glyph| glyph.0 != 0)
    }

    /// Shape a single directional run, in em units. Break paragraphs and mixed
    /// bidi/script runs in the caller. Cache the returned run instead of shaping per frame.
    pub fn shape(&mut self, text: &str) -> Result<TextRun, TextError> {
        let reference = harfrust::FontRef::from_index(&self.bytes, self.index)
            .map_err(|_| TextError::InvalidFont)?;
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let shaped = self
            .shaper
            .shaper(&reference)
            .build()
            .shape(buffer, Default::default());
        let mut glyphs = Vec::with_capacity(shaped.len());
        let mut pen = [0., 0.];
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            if info.glyph_id == 0 {
                return Err(TextError::MissingGlyph(info.cluster));
            }
            let glyph =
                self.glyph(u16::try_from(info.glyph_id).map_err(|_| TextError::TooComplex)?)?;
            if let Some(glyph) = glyph {
                glyphs.push(PositionedGlyph {
                    glyph,
                    position: [
                        pen[0] + pos.x_offset as f32 / self.units,
                        pen[1] + pos.y_offset as f32 / self.units,
                    ],
                });
            }
            pen[0] += pos.x_advance as f32 / self.units;
            pen[1] += pos.y_advance as f32 / self.units;
        }
        Ok(TextRun {
            font_id: self.id,
            glyphs,
            advance: pen,
        })
    }
    fn glyph(&mut self, id: u16) -> Result<Option<Glyph>, TextError> {
        if let Some(g) = self.glyphs.get(&id) {
            return Ok(*g);
        }
        let face =
            ttf_parser::Face::parse(&self.bytes, self.index).map_err(|_| TextError::InvalidFont)?;
        let mut outline = Outline::new(face.units_per_em());
        let bounded = face.outline_glyph(ttf_parser::GlyphId(id), &mut outline);
        if outline.failed {
            return Err(TextError::TooComplex);
        }
        if outline.curves.is_empty() {
            // An absent outline is valid for whitespace, but bitmap/color-only glyphs
            // must not be silently treated as successfully rendered text.
            if face
                .glyph_raster_image(ttf_parser::GlyphId(id), u16::MAX)
                .is_some()
                || face.is_color_glyph(ttf_parser::GlyphId(id))
                || face.glyph_svg_image(ttf_parser::GlyphId(id)).is_some()
            {
                return Err(TextError::UnsupportedOutline(id));
            }
            self.glyphs.insert(id, None);
            return Ok(None);
        }
        if bounded.is_none() {
            return Err(TextError::UnsupportedOutline(id));
        }
        let bounds = outline::bounds(&outline.curves);
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            self.glyphs.insert(id, None);
            return Ok(None);
        }
        let pieces = outline::monotone(&outline.curves);
        let (mut curves, mut bands, band_h, inv_h) = outline::windfoil_bands(&pieces, bounds);
        if self.curves.len() + curves.len() > 2_097_152 || self.bands.len() + bands.len() > 262_144
        {
            return Err(TextError::Capacity);
        }
        let glyph = Glyph {
            bounds,
            band_start: self.bands.len() as u32,
            bands: bands.len() as u32,
            band_h,
            inv_h,
        };
        let base = self.curves.len() as u32;
        for band in &mut bands {
            band[0] += base;
        }
        self.curves.append(&mut curves);
        self.bands.append(&mut bands);
        self.glyphs.insert(id, Some(glyph));
        Ok(Some(glyph))
    }
}

/// One contiguous shaping run selected from a [`FontFallback`] stack.
pub struct FallbackRun {
    pub font_index: usize,
    pub bytes: Range<usize>,
    pub run: TextRun,
}

/// Ordered font stack. Contiguous characters using the same font are shaped together.
pub struct FontFallback {
    fonts: Vec<Font>,
}

impl FontFallback {
    pub fn new(primary: Font) -> Self {
        Self {
            fonts: Vec::from([primary]),
        }
    }

    pub fn push(&mut self, font: Font) {
        self.fonts.push(font);
    }

    pub fn fonts(&self) -> &[Font] {
        &self.fonts
    }

    pub fn shape(&mut self, text: &str) -> Result<Vec<FallbackRun>, TextError> {
        let mut spans: Vec<(usize, usize, usize)> = Vec::new();
        let mut current = None;
        for (offset, character) in text.char_indices() {
            let font = self
                .fonts
                .iter()
                .position(|font| font.supports(character))
                .or(current)
                .ok_or(TextError::MissingGlyph(offset as u32))?;
            match spans.last_mut() {
                Some((index, _, end)) if *index == font => *end = offset + character.len_utf8(),
                _ => spans.push((font, offset, offset + character.len_utf8())),
            }
            current = Some(font);
        }
        let mut runs = Vec::with_capacity(spans.len());
        for (font_index, start, end) in spans {
            runs.push(FallbackRun {
                font_index,
                bytes: start..end,
                run: self.fonts[font_index].shape(&text[start..end])?,
            });
        }
        Ok(runs)
    }
}
