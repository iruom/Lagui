//! Derived paragraph layout for ordinary application text. Hosts supply their
//! canonical text and share one FontSystem; this type cannot edit that text.
//! The immutable buffer can be rendered using `raster::TextArea` without a copy.
use super::TextError;
use crate::Rect;
use core::ops::Range;
pub use glyphon as raster;
pub use glyphon::{Affinity, Attrs, FontSystem, Metrics, Wrap};
use glyphon::{Buffer, Cursor, Shaping};
use std::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextHit {
    pub byte: usize,
    pub affinity: Affinity,
}

pub struct Paragraph {
    buffer: Buffer,
}
impl Paragraph {
    pub fn new(fonts: &mut FontSystem, metrics: Metrics) -> Result<Self, TextError> {
        valid_metrics(metrics)?;
        Ok(Self {
            buffer: Buffer::new(fonts, metrics),
        })
    }
    /// Returns false without reshaping or allocating when text and style match.
    /// Comparison uses the existing layout buffer; no second source string exists.
    pub fn set_text(&mut self, text: &str, attrs: &Attrs<'_>) -> bool {
        let unchanged = self
            .buffer
            .lines
            .iter()
            .flat_map(|line| line.text().bytes().chain(line.ending().as_str().bytes()))
            .eq(text.bytes())
            && self
                .buffer
                .lines
                .iter()
                .all(|line| line.attrs_list().defaults() == *attrs);
        if unchanged {
            return false;
        }
        self.buffer.set_text(text, attrs, Shaping::Advanced, None);
        true
    }
    pub fn set_metrics(&mut self, metrics: Metrics) -> Result<(), TextError> {
        valid_metrics(metrics)?;
        self.buffer.set_metrics(metrics);
        Ok(())
    }
    pub fn set_size(&mut self, width: Option<f32>, height: Option<f32>) -> Result<(), TextError> {
        if width
            .into_iter()
            .chain(height)
            .any(|v| !v.is_finite() || v < 0.)
        {
            return Err(TextError::InvalidGeometry);
        }
        self.buffer.set_size(width, height);
        Ok(())
    }
    pub fn set_wrap(&mut self, wrap: Wrap) {
        self.buffer.set_wrap(wrap);
    }
    pub fn layout(&mut self, fonts: &mut FontSystem) {
        self.buffer.shape_until_scroll(fonts, false);
    }
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }
    /// Coordinates are local logical pixels, independent of window position/DPI.
    pub fn hit_test(&self, point: [f32; 2]) -> Option<TextHit> {
        if point.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let cursor = self.buffer.hit(point[0], point[1])?;
        let offset: usize = self.buffer.lines[..cursor.line]
            .iter()
            .map(|line| line.text().len() + line.ending().as_str().len())
            .sum();
        Some(TextHit {
            byte: offset + cursor.index,
            affinity: cursor.affinity,
        })
    }
    /// A mixed-direction selection can produce several disjoint rectangles.
    pub fn selection_rects(&self, bytes: Range<usize>) -> Vec<Rect> {
        let start = self.cursor(bytes.start);
        let end = self.cursor(bytes.end);
        self.buffer
            .layout_runs()
            .flat_map(|run| {
                run.highlight(start, end)
                    .map(move |(x, width)| Rect::new(x, run.line_top, width, run.line_height))
            })
            .collect()
    }
    fn cursor(&self, byte: usize) -> Cursor {
        let mut remaining = byte;
        for (index, line) in self.buffer.lines.iter().enumerate() {
            if remaining <= line.text().len() {
                return Cursor::new(
                    index,
                    crate::text_edit::floor_boundary(line.text(), remaining),
                );
            }
            remaining = remaining.saturating_sub(line.text().len() + line.ending().as_str().len());
        }
        let line = self.buffer.lines.len().saturating_sub(1);
        Cursor::new(
            line,
            self.buffer
                .lines
                .get(line)
                .map_or(0, |line| line.text().len()),
        )
    }
}
fn valid_metrics(metrics: Metrics) -> Result<(), TextError> {
    if [metrics.font_size, metrics.line_height]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.)
    {
        Err(TextError::InvalidGeometry)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_text_reuses_layout_and_wrap_hit_testing_uses_byte_positions() {
        let mut fonts = FontSystem::new_with_fonts([glyphon::fontdb::Source::Binary(
            std::sync::Arc::new(include_bytes!("../../tests/assets/test-outline.ttf").to_vec()),
        )]);
        let mut paragraph = Paragraph::new(&mut fonts, Metrics::new(16., 20.)).unwrap();
        paragraph.set_size(Some(40.), None).unwrap();
        paragraph.set_wrap(Wrap::WordOrGlyph);
        assert!(paragraph.set_text("O O O O\n口", &Attrs::new()));
        paragraph.layout(&mut fonts);
        assert!(paragraph.buffer().layout_runs().count() >= 3);
        let first = paragraph
            .buffer()
            .layout_runs()
            .next()
            .unwrap()
            .glyphs
            .as_ptr();
        assert!(!paragraph.set_text("O O O O\n口", &Attrs::new()));
        paragraph.layout(&mut fonts);
        assert_eq!(
            first,
            paragraph
                .buffer()
                .layout_runs()
                .next()
                .unwrap()
                .glyphs
                .as_ptr()
        );
        let hit = paragraph.hit_test([0., 1.]).unwrap();
        assert_eq!(hit.byte, 0);
        assert!(!paragraph.selection_rects(0..3).is_empty());
        assert!(paragraph.hit_test([f32::NAN, 0.]).is_none());
        assert!(paragraph.set_metrics(Metrics::new(0., 20.)).is_err());
        assert!(paragraph.set_size(Some(f32::INFINITY), None).is_err());
    }
}
