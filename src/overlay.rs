//! Stack state for popups, menus, tooltips and modal surfaces.

use crate::{CapacityExceeded, Rect, ui::WidgetId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayKind {
    Tooltip,
    #[default]
    Popup,
    Menu,
    Modal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Overlay {
    pub id: WidgetId,
    pub bounds: Rect,
    pub kind: OverlayKind,
    pub dismiss_on_outside_click: bool,
}

pub struct OverlayStack<const N: usize> {
    items: [Overlay; N],
    len: usize,
}

impl<const N: usize> Default for OverlayStack<N> {
    fn default() -> Self {
        Self {
            items: [Overlay::default(); N],
            len: 0,
        }
    }
}

impl<const N: usize> OverlayStack<N> {
    pub fn open(&mut self, overlay: Overlay) -> Result<(), CapacityExceeded> {
        if let Some(index) = self.items[..self.len]
            .iter()
            .position(|item| item.id == overlay.id)
        {
            self.items[index] = overlay;
            return Ok(());
        }
        let slot = self.items.get_mut(self.len).ok_or(CapacityExceeded)?;
        *slot = overlay;
        self.len += 1;
        Ok(())
    }
    pub fn close(&mut self, id: WidgetId) -> bool {
        let Some(index) = self.items[..self.len].iter().position(|item| item.id == id) else {
            return false;
        };
        self.items.copy_within(index + 1..self.len, index);
        self.len -= 1;
        true
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn items(&self) -> &[Overlay] {
        &self.items[..self.len]
    }
    pub fn top(&self) -> Option<Overlay> {
        self.items().last().copied()
    }
    pub fn modal(&self) -> Option<Overlay> {
        self.items()
            .iter()
            .rev()
            .find(|item| item.kind == OverlayKind::Modal)
            .copied()
    }
    pub fn pointer_down(&mut self, point: [f32; 2]) -> Option<WidgetId> {
        let top = self.top()?;
        if !top.bounds.contains(point[0], point[1]) && top.dismiss_on_outside_click {
            self.close(top.id);
            return Some(top.id);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outside_click_dismisses_topmost() {
        let mut s = OverlayStack::<2>::default();
        s.open(Overlay {
            id: WidgetId(1),
            bounds: Rect::new(0., 0., 10., 10.),
            kind: OverlayKind::Menu,
            dismiss_on_outside_click: true,
        })
        .unwrap();
        assert_eq!(s.pointer_down([20., 20.]), Some(WidgetId(1)));
        assert!(s.top().is_none());
    }
}
