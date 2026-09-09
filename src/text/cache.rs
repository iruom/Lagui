use super::{Font, TextError, TextRun};
use std::{collections::HashMap, collections::VecDeque, string::String};

pub const DEFAULT_MAX_RUNS: usize = 1024;

/// Keeps size-independent shaping results and their shared outline atlas alive.
/// Applications can place the same run at any size without shaping it again.
pub struct TextCache {
    font: Font,
    runs: HashMap<String, TextRun>,
    insertion_order: VecDeque<String>,
    max_runs: usize,
}

impl TextCache {
    pub fn new(font: Font) -> Self {
        Self::with_run_capacity(font, DEFAULT_MAX_RUNS)
    }

    /// Creates a FIFO-bounded cache. The capacity is clamped to at least one run.
    pub fn with_run_capacity(font: Font, max_runs: usize) -> Self {
        Self {
            font,
            runs: HashMap::new(),
            insertion_order: VecDeque::new(),
            max_runs: max_runs.max(1),
        }
    }

    /// Shapes only the first time a string is encountered.
    pub fn cache(&mut self, text: &str) -> Result<(), TextError> {
        if !self.runs.contains_key(text) {
            let run = self.font.shape(text)?;
            self.insert(String::from(text), run);
        }
        Ok(())
    }

    /// Shapes `text`, replacing unsupported input with `fallback` while retaining
    /// the original string as the lookup key.
    pub fn cache_with_fallback(&mut self, text: &str, fallback: char) -> Result<(), TextError> {
        if self.runs.contains_key(text) {
            return Ok(());
        }
        let run = match self.font.shape(text) {
            Ok(run) => run,
            Err(TextError::MissingGlyph(_) | TextError::UnsupportedOutline(_)) => {
                let mut sanitized = String::with_capacity(text.len());
                let mut replaced = false;
                for character in text.chars() {
                    let mut encoded = [0; 4];
                    let character = character.encode_utf8(&mut encoded);
                    match self.font.shape(character) {
                        Ok(_) => sanitized.push_str(character),
                        Err(TextError::MissingGlyph(_) | TextError::UnsupportedOutline(_)) => {
                            sanitized.push(fallback);
                            replaced = true;
                        }
                        Err(error) => return Err(error),
                    }
                }
                if !replaced {
                    sanitized.clear();
                    sanitized.push(fallback);
                }
                self.font.shape(&sanitized)?
            }
            Err(error) => return Err(error),
        };
        self.insert(String::from(text), run);
        Ok(())
    }

    fn insert(&mut self, key: String, run: TextRun) {
        if self.runs.len() == self.max_runs
            && let Some(oldest) = self.insertion_order.pop_front()
        {
            self.runs.remove(&oldest);
        }
        self.insertion_order.push_back(key.clone());
        self.runs.insert(key, run);
    }

    pub fn get(&self, text: &str) -> Option<&TextRun> {
        self.runs.get(text)
    }

    /// Returns the shaped horizontal advance in logical pixels, caching the run.
    pub fn measure(&mut self, text: &str, size: f32) -> Result<f32, TextError> {
        self.cache(text)?;
        Ok(self.get(text).map_or(0., |run| run.advance[0] * size))
    }

    pub fn font(&self) -> &Font {
        &self.font
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.max_runs
    }

    pub fn clear(&mut self) {
        self.runs.clear();
        self.insertion_order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_strings_reuse_the_same_run_and_outline() {
        let font = Font::new(
            include_bytes!("../../tests/assets/test-outline.ttf").to_vec(),
            0,
        )
        .unwrap();
        let mut cache = TextCache::new(font);
        cache.cache("OO").unwrap();
        let atlas = cache.font().atlas_bytes();
        cache.cache("OO").unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.font().atlas_bytes(), atlas);
        assert_eq!(cache.get("OO").unwrap().glyph_count(), 2);
    }

    #[test]
    fn cache_evicts_old_runs_at_its_configured_bound() {
        let mut cache = TextCache::with_run_capacity(font(), 2);
        cache.cache("O").unwrap();
        cache.cache("OO").unwrap();
        cache.cache("OOO").unwrap();
        assert_eq!(cache.capacity(), 2);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("O").is_none());
        assert!(cache.get("OO").is_some());
        assert!(cache.get("OOO").is_some());
    }

    #[test]
    fn unsupported_input_can_be_cached_with_a_fallback() {
        let mut cache = TextCache::new(font());
        cache.cache_with_fallback("O☃O", 'O').unwrap();
        assert_eq!(cache.get("O☃O").unwrap().glyph_count(), 3);
    }

    fn font() -> Font {
        Font::new(
            include_bytes!("../../tests/assets/test-outline.ttf").to_vec(),
            0,
        )
        .unwrap()
    }

    #[test]
    fn failed_measure_does_not_cache_question_mark_under_original_key() {
        let mut cache = TextCache::new(font());
        assert!(matches!(
            cache.measure("☃", 12.),
            Err(TextError::MissingGlyph(_))
        ));
        assert!(cache.get("☃").is_none());
    }
}
