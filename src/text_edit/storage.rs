//! Storage policy only; editing and Unicode behavior are shared by both policies.
use crate::CapacityExceeded;
use core::ops::Range;

#[doc(hidden)]
pub trait TextStorage: Default {
    fn as_str(&self) -> &str;
    fn from_str(text: &str) -> Result<Self, CapacityExceeded>;
    fn accepts(&self, len: usize) -> bool;
    fn replace(&mut self, range: Range<usize>, text: &str);
}

#[doc(hidden)]
pub struct FixedText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> Default for FixedText<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}
impl<const N: usize> TextStorage for FixedText<N> {
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).expect("editor preserves UTF-8")
    }
    fn from_str(text: &str) -> Result<Self, CapacityExceeded> {
        if text.len() > N {
            return Err(CapacityExceeded);
        }
        let mut storage = Self::default();
        storage.bytes[..text.len()].copy_from_slice(text.as_bytes());
        storage.len = text.len();
        Ok(storage)
    }
    fn accepts(&self, len: usize) -> bool {
        len <= N
    }
    fn replace(&mut self, range: Range<usize>, text: &str) {
        self.bytes
            .copy_within(range.end..self.len, range.start + text.len());
        self.bytes[range.start..range.start + text.len()].copy_from_slice(text.as_bytes());
        self.len = self.len - range.len() + text.len();
    }
}

#[cfg(feature = "alloc")]
impl TextStorage for alloc::string::String {
    fn as_str(&self) -> &str {
        self.as_str()
    }
    fn from_str(text: &str) -> Result<Self, CapacityExceeded> {
        Ok(text.into())
    }
    fn accepts(&self, _len: usize) -> bool {
        true
    }
    fn replace(&mut self, range: Range<usize>, text: &str) {
        self.replace_range(range, text);
    }
}

#[doc(hidden)]
pub trait History<E>: Default {
    fn push(&mut self, edit: E);
    fn pop(&mut self) -> Option<E>;
    fn clear(&mut self);
}

#[doc(hidden)]
pub struct FixedHistory<E, const N: usize> {
    edits: [Option<E>; N],
    len: usize,
}
impl<E, const N: usize> Default for FixedHistory<E, N> {
    fn default() -> Self {
        Self {
            edits: core::array::from_fn(|_| None),
            len: 0,
        }
    }
}
impl<E, const N: usize> History<E> for FixedHistory<E, N> {
    fn push(&mut self, edit: E) {
        if N == 0 {
            return;
        }
        if self.len == N {
            self.edits.rotate_left(1);
            self.len -= 1;
        }
        self.edits[self.len] = Some(edit);
        self.len += 1;
    }
    fn pop(&mut self) -> Option<E> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        self.edits[self.len].take()
    }
    fn clear(&mut self) {
        for edit in &mut self.edits[..self.len] {
            *edit = None;
        }
        self.len = 0;
    }
}

#[cfg(feature = "alloc")]
impl<E> History<E> for alloc::vec::Vec<E> {
    fn push(&mut self, edit: E) {
        self.push(edit);
    }
    fn pop(&mut self) -> Option<E> {
        self.pop()
    }
    fn clear(&mut self) {
        self.clear();
    }
}
