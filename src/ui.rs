use crate::{CapacityExceeded, Rect};

pub const MAX_REGIONS: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WidgetId(pub u64);

/// Failure to register an interactive region for the current frame.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationError {
    CapacityExceeded,
    DuplicateId(WidgetId),
}

impl core::error::Error for RegistrationError {}

impl core::fmt::Display for RegistrationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CapacityExceeded => formatter.write_str("UI region capacity was exceeded"),
            Self::DuplicateId(id) => write!(formatter, "widget ID {} was registered twice", id.0),
        }
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

/// Semantic state consumed by a platform accessibility adapter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Semantics {
    pub role: Role,
    pub label: &'static str,
    pub value: &'static str,
    pub disabled: bool,
    pub selected: bool,
    pub expanded: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Region {
    id: WidgetId,
    bounds: Rect,
    clip: Rect,
    focusable: bool,
    semantics: Semantics,
}

#[derive(Clone, Copy)]
pub(crate) struct FrameCheckpoint {
    len: usize,
    hovered: Option<WidgetId>,
}

impl Region {
    fn is_visible(&self) -> bool {
        self.bounds.intersects(self.clip)
    }

    fn can_focus(&self) -> bool {
        self.focusable && self.is_visible()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiResponse {
    pub target: Option<WidgetId>,
    pub clicked: bool,
    pub drag_started: bool,
    pub dragging: bool,
    /// Whether the event changed visible interaction state.
    pub changed: bool,
}

/// Platform-independent focus, hit testing and pointer capture.
/// Regions are supplied in paint order; hit testing walks them front-to-back.
pub struct UiRuntimeStorage<const N: usize> {
    regions: [Region; N],
    len: usize,
    pointer: [f32; 2],
    hovered: Option<WidgetId>,
    active: Option<WidgetId>,
    focused: Option<WidgetId>,
    modal_bounds: Option<Rect>,
}

/// Default-capacity runtime retained for the common case.
pub type UiRuntime = UiRuntimeStorage<MAX_REGIONS>;

impl<const N: usize> Default for UiRuntimeStorage<N> {
    fn default() -> Self {
        Self {
            regions: [Region::default(); N],
            len: 0,
            pointer: [-1.0e6; 2],
            hovered: None,
            active: None,
            focused: None,
            modal_bounds: None,
        }
    }
}

impl<const N: usize> UiRuntimeStorage<N> {
    pub(crate) fn checkpoint(&self) -> FrameCheckpoint {
        FrameCheckpoint {
            len: self.len,
            hovered: self.hovered,
        }
    }

    pub(crate) fn rollback(&mut self, checkpoint: FrameCheckpoint) {
        self.len = checkpoint.len;
        self.hovered = checkpoint.hovered;
    }

    pub fn begin_frame(&mut self) {
        self.len = 0;
        self.hovered = None;
        self.modal_bounds = None;
    }

    pub fn register(
        &mut self,
        id: WidgetId,
        bounds: Rect,
        clip: Rect,
        focusable: bool,
    ) -> Result<(), CapacityExceeded> {
        self.register_semantic(id, bounds, clip, focusable, Semantics::default())
    }

    pub fn register_semantic(
        &mut self,
        id: WidgetId,
        bounds: Rect,
        clip: Rect,
        focusable: bool,
        semantics: Semantics,
    ) -> Result<(), CapacityExceeded> {
        let Some(slot) = self.regions.get_mut(self.len) else {
            return Err(CapacityExceeded);
        };
        *slot = Region {
            id,
            bounds,
            clip,
            focusable: focusable && !semantics.disabled,
            semantics,
        };
        self.len += 1;
        if bounds.contains(self.pointer[0], self.pointer[1])
            && clip.contains(self.pointer[0], self.pointer[1])
        {
            self.hovered = Some(id);
        }
        Ok(())
    }

    /// Registers a region while enforcing the stable-ID uniqueness invariant.
    ///
    /// Prefer this for reusable widgets and new integrations. The original
    /// [`Self::register`] method remains available for compatibility with code
    /// that deliberately registers more than one hit region for one logical ID.
    pub fn register_unique(
        &mut self,
        id: WidgetId,
        bounds: Rect,
        clip: Rect,
        focusable: bool,
    ) -> Result<(), RegistrationError> {
        self.register_semantic_unique(id, bounds, clip, focusable, Semantics::default())
    }

    /// Semantic variant of [`Self::register_unique`].
    pub fn register_semantic_unique(
        &mut self,
        id: WidgetId,
        bounds: Rect,
        clip: Rect,
        focusable: bool,
        semantics: Semantics,
    ) -> Result<(), RegistrationError> {
        self.check_unique_registration(id)?;
        self.register_semantic(id, bounds, clip, focusable, semantics)
            .map_err(|_| RegistrationError::CapacityExceeded)
    }

    /// Checks whether a unique registration would succeed without changing the frame.
    pub fn check_unique_registration(&self, id: WidgetId) -> Result<(), RegistrationError> {
        if self.regions[..self.len]
            .iter()
            .any(|region| region.id == id)
        {
            Err(RegistrationError::DuplicateId(id))
        } else if self.len == N {
            Err(RegistrationError::CapacityExceeded)
        } else {
            Ok(())
        }
    }

    /// Restricts subsequent hit testing to a modal surface for this frame.
    pub fn set_modal_bounds(&mut self, bounds: Option<Rect>) {
        self.modal_bounds = bounds;
    }

    pub fn semantics(&self) -> impl Iterator<Item = (WidgetId, Rect, Semantics)> + '_ {
        self.regions[..self.len]
            .iter()
            .filter(|r| r.is_visible())
            .map(|r| (r.id, r.bounds, r.semantics))
    }

    pub fn semantics_for(&self, id: WidgetId) -> Option<Semantics> {
        self.regions[..self.len]
            .iter()
            .find(|region| region.id == id)
            .map(|region| region.semantics)
    }

    /// Reconciles persistent interaction state after all regions for a frame
    /// have been registered.
    pub fn end_frame(&mut self) {
        if !self.regions.get(..self.len).is_some_and(|regions| {
            regions
                .iter()
                .any(|region| Some(region.id) == self.active && region.is_visible())
        }) {
            self.active = None;
        }
        if !self.regions.get(..self.len).is_some_and(|regions| {
            regions
                .iter()
                .any(|region| Some(region.id) == self.focused && region.can_focus())
        }) {
            self.focused = None;
        }
        self.hovered = self.hit_test(self.pointer).map(|region| region.id);
    }

    pub fn pointer_move(&mut self, position: [f32; 2]) -> UiResponse {
        let previous_hovered = self.hovered;
        self.pointer = position;
        self.hovered = self.hit_test(position).map(|region| region.id);
        UiResponse {
            target: self.active.or(self.hovered),
            dragging: self.active.is_some(),
            changed: self.active.is_some() || previous_hovered != self.hovered,
            ..UiResponse::default()
        }
    }

    pub fn pointer_down(&mut self, position: [f32; 2]) -> UiResponse {
        self.pointer = position;
        let hit = self.hit_test(position).copied();
        self.active = hit.map(|region| region.id);
        self.focused = hit
            .filter(|region| region.focusable)
            .map(|region| region.id);
        UiResponse {
            target: self.active,
            drag_started: self.active.is_some(),
            dragging: self.active.is_some(),
            changed: true,
            ..UiResponse::default()
        }
    }

    pub fn pointer_up(&mut self, position: [f32; 2]) -> UiResponse {
        self.pointer = position;
        let hit = self.hit_test(position).map(|region| region.id);
        let active = self.active.take();
        self.hovered = hit;
        UiResponse {
            target: active,
            clicked: active.is_some() && active == hit,
            changed: active.is_some() || self.hovered != hit,
            ..UiResponse::default()
        }
    }

    pub fn cancel_pointer(&mut self) {
        self.active = None;
        self.hovered = None;
    }

    pub fn focus_next(&mut self, reverse: bool) -> Option<WidgetId> {
        let focusable = |region: &&Region| region.can_focus();
        let current = self.regions[..self.len]
            .iter()
            .position(|region| Some(region.id) == self.focused);
        self.focused = if reverse {
            self.regions[..current.unwrap_or(self.len)]
                .iter()
                .rev()
                .find(focusable)
                .or_else(|| self.regions[..self.len].iter().rev().find(focusable))
                .map(|region| region.id)
        } else {
            self.regions[current.map_or(0, |index| index + 1)..self.len]
                .iter()
                .find(focusable)
                .or_else(|| self.regions[..self.len].iter().find(focusable))
                .map(|region| region.id)
        };
        self.focused
    }

    pub fn activate_focused(&self) -> Option<WidgetId> {
        self.focused.filter(|focused| {
            self.regions[..self.len]
                .iter()
                .any(|region| region.id == *focused && region.can_focus())
        })
    }

    pub fn hovered_id(&self) -> Option<WidgetId> {
        self.hovered
    }

    pub fn active_id(&self) -> Option<WidgetId> {
        self.active
    }

    pub fn focused_id(&self) -> Option<WidgetId> {
        self.focused
    }

    pub fn set_focus(&mut self, id: Option<WidgetId>) {
        self.focused = id;
    }

    pub fn hovered(&self, id: WidgetId) -> bool {
        self.hovered == Some(id)
    }

    pub fn active(&self, id: WidgetId) -> bool {
        self.active == Some(id)
    }

    pub fn focused(&self, id: WidgetId) -> bool {
        self.focused == Some(id)
    }

    pub fn pointer(&self) -> [f32; 2] {
        self.pointer
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn remaining_capacity(&self) -> usize {
        N - self.len
    }

    fn hit_test(&self, point: [f32; 2]) -> Option<&Region> {
        if self
            .modal_bounds
            .is_some_and(|bounds| !bounds.contains(point[0], point[1]))
        {
            return None;
        }
        self.regions[..self.len].iter().rev().find(|region| {
            region.bounds.contains(point[0], point[1]) && region.clip.contains(point[0], point[1])
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLIP: Rect = Rect::new(0., 0., 100., 100.);

    #[test]
    fn frontmost_region_wins_and_capture_survives_leaving() {
        let mut ui = UiRuntime::default();
        ui.begin_frame();
        ui.register(WidgetId(1), Rect::new(0., 0., 80., 80.), CLIP, true)
            .unwrap();
        ui.register(WidgetId(2), Rect::new(20., 20., 80., 80.), CLIP, true)
            .unwrap();
        assert_eq!(ui.pointer_down([30., 30.]).target, Some(WidgetId(2)));
        let drag = ui.pointer_move([200., 200.]);
        assert_eq!(drag.target, Some(WidgetId(2)));
        assert!(drag.dragging);
        assert!(!ui.pointer_up([200., 200.]).clicked);
    }

    #[test]
    fn clipping_and_focus_traversal_work() {
        let mut ui = UiRuntime::default();
        ui.begin_frame();
        ui.register(
            WidgetId(1),
            Rect::new(0., 0., 200., 20.),
            Rect::new(0., 0., 50., 50.),
            true,
        )
        .unwrap();
        ui.register(WidgetId(2), Rect::new(0., 30., 20., 20.), CLIP, false)
            .unwrap();
        ui.register(WidgetId(3), Rect::new(30., 30., 20., 20.), CLIP, true)
            .unwrap();
        assert_eq!(ui.pointer_down([80., 10.]).target, None);
        assert_eq!(ui.focus_next(false), Some(WidgetId(1)));
        assert_eq!(ui.focus_next(false), Some(WidgetId(3)));
        assert_eq!(ui.focus_next(false), Some(WidgetId(1)));
        assert_eq!(ui.focus_next(true), Some(WidgetId(3)));
        ui.pointer_down([35., 35.]);
        assert_eq!(ui.focused_id(), Some(WidgetId(3)));
        ui.pointer_up([35., 35.]);
        ui.pointer_down([10., 35.]);
        assert_eq!(ui.focused_id(), None);
    }

    #[test]
    fn end_frame_clears_removed_or_fully_clipped_state() {
        let mut ui = UiRuntimeStorage::<2>::default();
        ui.begin_frame();
        ui.register(WidgetId(1), Rect::new(0., 0., 20., 20.), CLIP, true)
            .unwrap();
        ui.pointer_down([10., 10.]);
        assert_eq!(ui.active_id(), Some(WidgetId(1)));
        assert_eq!(ui.focused_id(), Some(WidgetId(1)));

        ui.begin_frame();
        ui.register(WidgetId(1), Rect::new(200., 200., 20., 20.), CLIP, true)
            .unwrap();
        ui.end_frame();
        assert_eq!(ui.active_id(), None);
        assert_eq!(ui.focused_id(), None);
        assert_eq!(ui.activate_focused(), None);
    }

    #[test]
    fn region_capacity_is_configurable_and_overflow_is_an_error() {
        let mut ui = UiRuntimeStorage::<1>::default();
        ui.begin_frame();
        assert_eq!(ui.capacity(), 1);
        assert_eq!(ui.register(WidgetId(1), CLIP, CLIP, true), Ok(()));
        assert_eq!(ui.remaining_capacity(), 0);
        assert_eq!(
            ui.register(WidgetId(2), CLIP, CLIP, true),
            Err(CapacityExceeded)
        );
    }

    #[test]
    fn unique_registration_rejects_duplicate_ids_without_mutation() {
        let mut ui = UiRuntimeStorage::<2>::default();
        ui.begin_frame();
        ui.register_unique(WidgetId(7), CLIP, CLIP, true).unwrap();
        assert_eq!(
            ui.register_unique(WidgetId(7), CLIP, CLIP, true),
            Err(RegistrationError::DuplicateId(WidgetId(7)))
        );
        assert_eq!(ui.len(), 1);
        assert_eq!(ui.remaining_capacity(), 1);
    }
}
