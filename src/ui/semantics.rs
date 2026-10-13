#[cfg(feature = "alloc")]
use alloc::{string::String, sync::Arc};
use core::{fmt, ops::Deref};

/// Accessible text owns its lifetime. Static labels allocate nothing; dynamic
/// labels are shared across frame projections without leaks or full-string clones.
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "alloc"), derive(Copy))]
pub enum SemanticText {
    Static(&'static str),
    #[cfg(feature = "alloc")]
    Shared(Arc<str>),
}
impl Default for SemanticText {
    fn default() -> Self {
        Self::Static("")
    }
}
impl From<&'static str> for SemanticText {
    fn from(text: &'static str) -> Self {
        Self::Static(text)
    }
}
#[cfg(feature = "alloc")]
impl From<String> for SemanticText {
    fn from(text: String) -> Self {
        Self::Shared(text.into())
    }
}
#[cfg(feature = "alloc")]
impl From<Arc<str>> for SemanticText {
    fn from(text: Arc<str>) -> Self {
        Self::Shared(text)
    }
}
impl Deref for SemanticText {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Self::Static(text) => text,
            #[cfg(feature = "alloc")]
            Self::Shared(text) => text,
        }
    }
}
impl AsRef<str> for SemanticText {
    fn as_ref(&self) -> &str {
        self
    }
}
impl PartialEq for SemanticText {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}
impl Eq for SemanticText {}
impl PartialEq<str> for SemanticText {
    fn eq(&self, other: &str) -> bool {
        self.as_ref() == other
    }
}
impl PartialEq<&str> for SemanticText {
    fn eq(&self, other: &&str) -> bool {
        self.as_ref() == *other
    }
}
impl PartialEq<SemanticText> for str {
    fn eq(&self, other: &SemanticText) -> bool {
        self == other.as_ref()
    }
}
impl PartialEq<SemanticText> for &str {
    fn eq(&self, other: &SemanticText) -> bool {
        *self == other.as_ref()
    }
}
impl fmt::Display for SemanticText {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    Generic,
    Button,
    Checkbox,
    Slider,
    TextInput,
    List,
    ListItem,
    Menu,
    MenuItem,
    Dialog,
}

/// Read-only accessibility projection, never committed application data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(feature = "alloc"), derive(Copy))]
pub struct Semantics {
    pub role: Role,
    pub label: SemanticText,
    pub value: SemanticText,
    pub disabled: bool,
    pub selected: bool,
    pub expanded: bool,
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    #[test]
    fn equality_compares_text_independently_of_storage() {
        let owned: SemanticText = String::from("名前").into();
        let static_text: SemanticText = "名前".into();
        assert_eq!(owned, static_text);
        assert_eq!(owned, "名前");
        assert_eq!("名前", owned);
        assert_ne!(owned, "別名");
    }
}
