//! Optional, outline-based text. One font and one directional shaping run at a time.
//! Font data is supplied by the caller; OS font files are never redistributed.
mod cache;
mod outline;
/// Paragraph layout, wrapping, bidi and caret hit testing through the shared
/// portable font system. Analytic directional runs remain available separately.
#[cfg(feature = "portable")]
pub mod paragraph;
mod renderer;
use crate::Rect;
#[cfg(feature = "portable")]
mod system_font;
pub use cache::TextCache;
use outline::{Curve, Outline};
pub use renderer::{Cull, TextRenderer, TextSection};
use std::{
    collections::HashMap,
    fmt,
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
    vec::Vec,
};
#[cfg(feature = "portable")]
pub use system_font::load_system_font;
use unicode_segmentation::UnicodeSegmentation;

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
    fn carets_cover_grapheme_boundaries_and_split_ligatures() {
        let mut font = font();
        let run = font.shape("O O").unwrap();
        let bytes: Vec<_> = run.carets().iter().map(|caret| caret.byte).collect();
        assert_eq!(bytes, [0, 1, 2, 3]);
        assert_eq!(run.carets()[0].x, 0.);
        assert!((run.carets()[3].x - run.advance[0]).abs() < 1e-5);
        assert!(run.carets().windows(2).all(|pair| pair[0].x < pair[1].x));
        assert_eq!(run.hit_test(-1.), 0);
        assert_eq!(run.hit_test(10.), 3);
        assert_eq!(run.caret_x(2), run.carets()[2].x);

        // "ffi" shapes to fewer glyphs than graphemes; every grapheme still gets a stop.
        let ligature = font.shape("ffi").unwrap();
        let stops = ligature.carets();
        assert_eq!(stops.len(), 4);
        assert!(stops.windows(2).all(|pair| pair[0].x < pair[1].x));
        assert!((stops[3].x - ligature.advance[0]).abs() < 1e-5);

        // A combining mark stays inside its grapheme.
        let accent = font.shape("e\u{301}").unwrap();
        let bytes: Vec<_> = accent.carets().iter().map(|caret| caret.byte).collect();
        assert_eq!(bytes, [0, 3]);
        assert_eq!(
            font.shape("").unwrap().carets(),
            &[Caret { byte: 0, x: 0. }]
        );
    }

    #[test]
    fn metrics_are_em_relative_and_center_capitals() {
        let metrics = font().metrics();
        assert!(metrics.ascender > 0. && metrics.descender <= 0.);
        assert!(metrics.cap_height > 0. && metrics.cap_height <= metrics.ascender);
        assert!(metrics.x_height > 0. && metrics.x_height <= metrics.cap_height);
        assert!(metrics.line_height() >= metrics.ascender - metrics.descender);
        let baseline = metrics.centered_baseline(10., 20., 10.);
        let cap_top = baseline - metrics.cap_height * 10.;
        assert!(((cap_top - 10.) - (30. - baseline)).abs() < 1e-4);
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
    carets: Vec<Caret>,
    ink_bounds: Option<Rect>,
    pub advance: [f32; 2],
}
impl TextRun {
    pub fn ink_bounds(&self) -> Option<Rect> {
        self.ink_bounds
    }

    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Caret stops at every grapheme boundary of the shaped text, including
    /// `0` and the text length, ordered by byte offset. `x` is in em units from
    /// the run origin; multiply by the font size for logical pixels.
    pub fn carets(&self) -> &[Caret] {
        &self.carets
    }

    /// Horizontal caret position (em) for a byte offset. Offsets inside a
    /// grapheme use the boundary before them.
    pub fn caret_x(&self, byte: usize) -> f32 {
        let index = self.carets.partition_point(|caret| caret.byte <= byte);
        self.carets[index.saturating_sub(1)].x
    }

    /// Byte offset of the caret stop nearest to `x` (em from the run origin).
    pub fn hit_test(&self, x: f32) -> usize {
        self.carets
            .iter()
            .min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()))
            .map_or(0, |caret| caret.byte)
    }
}

/// A position where a text caret can stand: a UTF-8 byte offset into the
/// shaped text and its x position in em units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub byte: usize,
    pub x: f32,
}

/// Vertical font metrics in em units, y up from the baseline. `descender` is
/// negative. Multiply by the font size for logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    pub ascender: f32,
    pub descender: f32,
    pub line_gap: f32,
    pub cap_height: f32,
    pub x_height: f32,
}

impl FontMetrics {
    /// Recommended distance between consecutive baselines, in em.
    pub fn line_height(&self) -> f32 {
        self.ascender - self.descender + self.line_gap
    }

    /// Baseline y (logical pixels, y down) that centers capital letters of
    /// `size` vertically in the band `top..top + height`.
    pub fn centered_baseline(&self, top: f32, height: f32, size: f32) -> f32 {
        top + (height + self.cap_height * size) * 0.5
    }
}

pub struct Font {
    bytes: Vec<u8>,
    index: u32,
    id: u64,
    units: f32,
    shaper: harfrust::ShaperData,
    metrics: FontMetrics,
    glyphs: HashMap<u16, Option<Glyph>>,
    curves: Vec<Curve>,
    bands: Vec<[u32; 5]>,
}
impl Font {
    pub fn new(bytes: Vec<u8>, face_index: u32) -> Result<Self, TextError> {
        let face =
            ttf_parser::Face::parse(&bytes, face_index).map_err(|_| TextError::InvalidFont)?;
        let units = face.units_per_em() as f32;
        let metrics = metrics(&face, units);
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
            metrics,
            glyphs: HashMap::new(),
            curves: Vec::new(),
            bands: Vec::new(),
        })
    }
    pub fn metrics(&self) -> FontMetrics {
        self.metrics
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
        let mut clusters = Vec::with_capacity(shaped.len());
        let mut pen = [0., 0.];
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            if info.glyph_id == 0 {
                return Err(TextError::MissingGlyph(info.cluster));
            }
            clusters.push((
                info.cluster as usize,
                pen[0],
                pen[0] + pos.x_advance as f32 / self.units,
            ));
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
        let ink_bounds = glyphs
            .iter()
            .map(|p| {
                Rect::new(
                    p.position[0] + p.glyph.bounds[0],
                    -p.position[1] - p.glyph.bounds[3],
                    p.glyph.bounds[2] - p.glyph.bounds[0],
                    p.glyph.bounds[3] - p.glyph.bounds[1],
                )
            })
            .reduce(|a, b| {
                let x = a.x.min(b.x);
                let y = a.y.min(b.y);
                Rect::new(
                    x,
                    y,
                    (a.x + a.width).max(b.x + b.width) - x,
                    (a.y + a.height).max(b.y + b.height) - y,
                )
            });
        Ok(TextRun {
            ink_bounds,
            font_id: self.id,
            glyphs,
            carets: carets(text, &clusters),
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
        let curve_end = self
            .curves
            .len()
            .checked_add(curves.len())
            .ok_or(TextError::Capacity)?;
        let band_end = self
            .bands
            .len()
            .checked_add(bands.len())
            .ok_or(TextError::Capacity)?;
        // Curve references are u32. Instance band addresses are exact f32 integers.
        if curve_end > u32::MAX as usize || band_end > (1usize << 24) {
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

fn metrics(face: &ttf_parser::Face<'_>, units: f32) -> FontMetrics {
    let ascender = face.ascender() as f32 / units;
    // Fonts without OS/2 heights: measure the glyphs that define them.
    let glyph_top = |character| {
        face.glyph_index(character)
            .and_then(|glyph| face.glyph_bounding_box(glyph))
            .map(|bounds| bounds.y_max as f32 / units)
    };
    FontMetrics {
        ascender,
        descender: face.descender() as f32 / units,
        line_gap: face.line_gap() as f32 / units,
        cap_height: face
            .capital_height()
            .filter(|height| *height > 0)
            .map(|height| height as f32 / units)
            .or_else(|| glyph_top('H'))
            .unwrap_or(ascender * 0.7),
        x_height: face
            .x_height()
            .filter(|height| *height > 0)
            .map(|height| height as f32 / units)
            .or_else(|| glyph_top('x'))
            .unwrap_or(ascender * 0.5),
    }
}

/// Caret stops for one shaped directional run. `glyphs` holds each glyph's
/// cluster byte offset and its horizontal pen span (em) in output order.
/// Graphemes inside one cluster (a ligature) share its width evenly.
fn carets(text: &str, glyphs: &[(usize, f32, f32)]) -> Vec<Caret> {
    if text.is_empty() || glyphs.is_empty() {
        return Vec::from([Caret { byte: 0, x: 0. }]);
    }
    // Clusters in byte order with their visual span.
    let mut clusters: Vec<(usize, f32, f32)> = Vec::new();
    for &(start, left, right) in glyphs {
        match clusters.iter_mut().find(|cluster| cluster.0 == start) {
            Some(cluster) => {
                cluster.1 = cluster.1.min(left);
                cluster.2 = cluster.2.max(right);
            }
            None => clusters.push((start, left, right)),
        }
    }
    clusters.sort_by_key(|cluster| cluster.0);
    // Shaping output is in visual order; decreasing byte offsets mean RTL.
    let rtl = glyphs[0].0 > glyphs[glyphs.len() - 1].0;
    let boundaries: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain([text.len()])
        .collect();
    let graphemes_between = |from: usize, to: usize| {
        boundaries
            .iter()
            .filter(|&&boundary| boundary > from && boundary <= to)
            .count()
    };
    boundaries
        .iter()
        .map(|&byte| {
            let index = clusters
                .partition_point(|cluster| cluster.0 <= byte)
                .saturating_sub(1);
            let (start, left, right) = clusters[index];
            let end = clusters.get(index + 1).map_or(text.len(), |next| next.0);
            let (lead, trail) = if rtl { (right, left) } else { (left, right) };
            let x = if byte >= end {
                trail
            } else {
                let total = graphemes_between(start, end).max(1);
                let done = graphemes_between(start, byte);
                lead + (trail - lead) * done as f32 / total as f32
            };
            Caret { byte, x }
        })
        .collect()
}

/// One contiguous shaping run selected from a [`FontFallback`] stack.
pub struct FallbackRun {
    pub font_index: usize,
    pub bytes: Range<usize>,
    pub run: TextRun,
}

/// Ordered font stack. Whole graphemes select a font; adjacent graphemes using
/// that font are shaped together. Paragraph bidi/layout use `paragraph`.
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
        for (offset, grapheme) in text.grapheme_indices(true) {
            let font = self
                .fonts
                .iter()
                .position(|font| grapheme.chars().all(|character| font.supports(character)))
                .or(current)
                .ok_or(TextError::MissingGlyph(offset as u32))?;
            match spans.last_mut() {
                Some((index, _, end)) if *index == font => *end = offset + grapheme.len(),
                _ => spans.push((font, offset, offset + grapheme.len())),
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
