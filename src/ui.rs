use crate::{CapacityExceeded, Rect};
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "alloc")]
mod registration_index;
mod semantics;
pub use semantics::{Role, SemanticText, Semantics};

pub const DEFAULT_REGION_RESERVE: usize = 512;

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

#[derive(Clone, Debug, Default)]
#[cfg_attr(not(feature = "alloc"), derive(Copy))]
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
    #[cfg(feature = "alloc")]
    regions: Vec<Region>,
    #[cfg(not(feature = "alloc"))]
    regions: [Region; N],
    #[cfg(feature = "alloc")]
    registrations: registration_index::RegistrationIndex,
    len: usize,
    pointer: [f32; 2],
    hovered: Option<WidgetId>,
    active: Option<WidgetId>,
    focused: Option<WidgetId>,
    modal_bounds: Option<Rect>,
    modal_start: usize,
}

/// With alloc, N is an initial allocation hint and regions grow with the frame.
/// Without an allocator, the portable core retains explicitly bounded storage.
pub type UiRuntime = UiRuntimeStorage<DEFAULT_REGION_RESERVE>;

impl<const N: usize> Default for UiRuntimeStorage<N> {
    fn default() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            regions: Vec::with_capacity(N),
            #[cfg(not(feature = "alloc"))]
            regions: [Region::default(); N],
            #[cfg(feature = "alloc")]
            registrations: registration_index::RegistrationIndex::default(),
            len: 0,
            pointer: [-1.0e6; 2],
            hovered: None,
            active: None,
            focused: None,
            modal_bounds: None,
            modal_start: 0,
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
        #[cfg(feature = "alloc")]
        for region in &self.regions[checkpoint.len..self.len] {
            self.registrations.remove(region.id.0);
        }
        self.len = checkpoint.len;
        #[cfg(feature = "alloc")]
        self.regions.truncate(self.len);
        self.hovered = checkpoint.hovered;
    }

    pub fn begin_frame(&mut self) {
        #[cfg(feature = "alloc")]
        self.registrations.clear();
        self.len = 0;
        self.hovered = None;
        self.modal_bounds = None;
        self.modal_start = 0;
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
        #[cfg(feature = "alloc")]
        if self.len == self.regions.len() {
            self.regions.push(Region::default());
        }
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
        #[cfg(feature = "alloc")]
        {
            self.registrations.insert(id.0);
        }
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
        #[cfg(feature = "alloc")]
        let duplicate = self.registrations.contains(id.0);
        #[cfg(not(feature = "alloc"))]
        let duplicate = self.regions[..self.len]
            .iter()
            .any(|region| region.id == id);
        if duplicate {
            Err(RegistrationError::DuplicateId(id))
        } else if !cfg!(feature = "alloc") && self.len == self.capacity() {
            Err(RegistrationError::CapacityExceeded)
        } else {
            Ok(())
        }
    }

    /// Call before registering the modal and its children. Restricts pointer and
    /// keyboard input to those subsequent regions for this frame.
    pub fn set_modal_bounds(&mut self, bounds: Option<Rect>) {
        self.modal_bounds = bounds;
        self.modal_start = if bounds.is_some() { self.len } else { 0 };
    }

    pub fn semantics(&self) -> impl Iterator<Item = (WidgetId, Rect, &Semantics)> + '_ {
        self.regions[..self.len].iter().filter_map(|r| {
            r.bounds
                .intersection(r.clip)
                .map(|bounds| (r.id, bounds, &r.semantics))
        })
    }

    pub fn semantics_for(&self, id: WidgetId) -> Option<&Semantics> {
        self.regions[..self.len]
            .iter()
            .find(|region| region.id == id)
            .map(|region| &region.semantics)
    }

    /// Reconciles persistent interaction state after all regions for a frame
    /// have been registered.
    pub fn end_frame(&mut self) {
        let in_scope = &self.regions[self.modal_start..self.len];
        if self.active.is_some_and(|id| {
            !in_scope
                .iter()
                .any(|r| r.id == id && r.is_visible() && !r.semantics.disabled && self.in_modal(r))
        }) {
            self.active = None;
        }
        if self.focused.is_some_and(|id| {
            !in_scope
                .iter()
                .any(|r| r.id == id && r.can_focus() && self.in_modal(r))
        }) {
            self.focused = None;
        }
        self.hovered = self.hit_test(self.pointer).map(|region| region.id);
        #[cfg(feature = "alloc")]
        self.regions.truncate(self.len);
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
        let hit = self.hit_test(position);
        let active = hit.filter(|r| !r.semantics.disabled).map(|r| r.id);
        let focused = hit.filter(|r| r.can_focus()).map(|r| r.id);
        let hovered = hit.map(|r| r.id);
        self.hovered = hovered;
        self.active = active;
        self.focused = focused;
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
        let previous_hovered = self.hovered;
        let region = self.hit_test(position);
        let hit = region.map(|region| region.id);
        let enabled = region.is_some_and(|r| !r.semantics.disabled);
        let active = self.active.take();
        self.hovered = hit;
        UiResponse {
            target: active,
            clicked: enabled && active.is_some() && active == hit,
            changed: active.is_some() || previous_hovered != hit,
            ..UiResponse::default()
        }
    }

    pub fn cancel_pointer(&mut self) {
        self.active = None;
        self.hovered = None;
    }

    pub fn focus_next(&mut self, reverse: bool) -> Option<WidgetId> {
        let regions = &self.regions[self.modal_start..self.len];
        let current = regions.iter().position(|r| Some(r.id) == self.focused);
        self.focused = if reverse {
            regions[..current.unwrap_or(regions.len())]
                .iter()
                .rev()
                .find(|r| r.can_focus() && self.in_modal(r))
                .or_else(|| {
                    regions
                        .iter()
                        .rev()
                        .find(|r| r.can_focus() && self.in_modal(r))
                })
        } else {
            regions[current.map_or(0, |i| i + 1)..]
                .iter()
                .find(|r| r.can_focus() && self.in_modal(r))
                .or_else(|| regions.iter().find(|r| r.can_focus() && self.in_modal(r)))
        }
        .map(|r| r.id);
        self.focused
    }
    pub fn activate_focused(&self) -> Option<WidgetId> {
        self.focused.filter(|id| {
            self.regions[self.modal_start..self.len]
                .iter()
                .any(|r| r.id == *id && r.can_focus() && self.in_modal(r))
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

    pub fn capacity(&self) -> usize {
        #[cfg(feature = "alloc")]
        {
            self.regions.capacity()
        }
        #[cfg(not(feature = "alloc"))]
        {
            N
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn remaining_capacity(&self) -> usize {
        self.capacity() - self.len
    }

    fn in_modal(&self, region: &Region) -> bool {
        self.modal_bounds.is_none_or(|modal| {
            region
                .bounds
                .intersection(region.clip)
                .is_some_and(|visible| visible.intersects(modal))
        })
    }

    fn hit_test(&self, point: [f32; 2]) -> Option<&Region> {
        if self
            .modal_bounds
            .is_some_and(|bounds| !bounds.contains(point[0], point[1]))
        {
            return None;
        }
        self.regions[self.modal_start..self.len]
            .iter()
            .rev()
            .find(|region| {
                region.bounds.contains(point[0], point[1])
                    && region.clip.contains(point[0], point[1])
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLIP: Rect = Rect::new(0., 0., 100., 100.);

    #[test]
    fn disabled_foreground_blocks_click_through_and_cancels_capture() {
        let mut ui = UiRuntime::default();
        ui.begin_frame();
        ui.register_unique(WidgetId(1), CLIP, CLIP, true).unwrap();
        ui.pointer_down([1., 1.]);
        ui.begin_frame();
        ui.register_semantic_unique(
            WidgetId(1),
            CLIP,
            CLIP,
            true,
            Semantics {
                disabled: true,
                ..Semantics::default()
            },
        )
        .unwrap();
        ui.end_frame();
        assert_eq!(ui.active_id(), None);
        assert_eq!(ui.focus_next(false), None);
        assert_eq!(ui.pointer_down([1., 1.]).target, None);
        assert!(!ui.pointer_up([1., 1.]).clicked);
    }
    #[test]
    fn modal_scope_blocks_background_pointer_and_keyboard_input() {
        let mut ui = UiRuntime::default();
        ui.begin_frame();
        ui.register_unique(WidgetId(1), CLIP, CLIP, true).unwrap();
        ui.set_focus(Some(WidgetId(1)));
        ui.set_modal_bounds(Some(CLIP));
        // A modal's blank space cannot click the background even inside its bounds.
        assert_eq!(ui.pointer_down([1., 1.]).target, None);
        ui.register_unique(WidgetId(2), CLIP, CLIP, true).unwrap();
        ui.register_unique(
            WidgetId(3),
            Rect::new(200., 200., 20., 20.),
            Rect::new(200., 200., 20., 20.),
            true,
        )
        .unwrap();
        ui.end_frame();
        assert_eq!(ui.focused_id(), None);
        assert_eq!(ui.focus_next(false), Some(WidgetId(2)));
        assert_eq!(ui.focus_next(true), Some(WidgetId(2)));
        assert_eq!(ui.activate_focused(), Some(WidgetId(2)));
        assert_eq!(ui.pointer_down([1., 1.]).target, Some(WidgetId(2)));
    }
    #[test]
    fn pointer_release_reports_a_hover_change_without_capture() {
        let mut ui = UiRuntime::default();
        ui.begin_frame();
        ui.register_unique(WidgetId(1), CLIP, CLIP, true).unwrap();
        ui.pointer_move([1., 1.]);
        assert!(ui.pointer_up([200., 200.]).changed);
        assert!(!ui.pointer_up([200., 200.]).changed);
    }
    #[test]
    #[cfg(feature = "alloc")]
    fn dynamic_semantics_own_text_and_release_removed_frames() {
        let mut ui = UiRuntime::default();
        let text: alloc::sync::Arc<str> = alloc::sync::Arc::from("動的な名前");
        ui.begin_frame();
        ui.register_semantic_unique(
            WidgetId(1),
            CLIP,
            CLIP,
            true,
            Semantics {
                label: text.clone().into(),
                value: alloc::string::String::from("42").into(),
                ..Semantics::default()
            },
        )
        .unwrap();
        ui.end_frame();
        assert_eq!(
            ui.semantics_for(WidgetId(1)).unwrap().label.as_ref(),
            "動的な名前"
        );
        assert_eq!(ui.semantics_for(WidgetId(1)).unwrap().value.as_ref(), "42");
        ui.begin_frame();
        ui.end_frame();
        assert_eq!(alloc::sync::Arc::strong_count(&text), 1);
    }

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
        assert_eq!(
            ui.semantics()
                .find(|(id, _, _)| *id == WidgetId(1))
                .unwrap()
                .1,
            Rect::new(0., 0., 50., 20.)
        );
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
    #[cfg(feature = "alloc")]
    fn region_storage_grows_and_retains_capture_across_frames() {
        let mut ui = UiRuntimeStorage::<1>::default();
        ui.begin_frame();
        assert_eq!(ui.capacity(), 1);
        assert_eq!(ui.register(WidgetId(1), CLIP, CLIP, true), Ok(()));
        assert_eq!(ui.remaining_capacity(), 0);
        ui.register(WidgetId(2), CLIP, CLIP, true).unwrap();
        assert_eq!(ui.len(), 2);
        assert!(ui.capacity() >= 2);
        let response = ui.pointer_down([1., 1.]);
        assert_eq!(response.target, Some(WidgetId(2)));
        ui.begin_frame();
        ui.register(WidgetId(2), CLIP, CLIP, true).unwrap();
        ui.end_frame();
        assert!(ui.active(WidgetId(2)));
    }

    #[test]
    #[cfg(feature = "alloc")]
    fn large_frames_keep_unique_ids_hit_order_and_focus() {
        let mut ui = UiRuntimeStorage::<0>::default();
        ui.begin_frame();
        for id in 1..=13_000 {
            ui.register_unique(WidgetId(id), CLIP, CLIP, true).unwrap();
        }
        assert_eq!(ui.len(), 13_000);
        assert_eq!(ui.pointer_down([1., 1.]).target, Some(WidgetId(13_000)));
        assert_eq!(ui.focused_id(), Some(WidgetId(13_000)));
        ui.begin_frame();
        ui.register_unique(WidgetId(13_000), CLIP, CLIP, true)
            .unwrap();
        ui.end_frame();
        assert!(ui.active(WidgetId(13_000)));
        assert_eq!(ui.focused_id(), Some(WidgetId(13_000)));
        ui.begin_frame();
        ui.end_frame();
        assert_eq!(ui.focused_id(), None);
    }

    #[test]
    fn registration_rollback_preserves_existing_duplicates_and_releases_new_ids() {
        let mut ui = UiRuntimeStorage::<4>::default();
        ui.begin_frame();
        ui.register(WidgetId(1), CLIP, CLIP, true).unwrap();
        let checkpoint = ui.checkpoint();
        ui.register(WidgetId(1), CLIP, CLIP, true).unwrap();
        ui.register_unique(WidgetId(2), CLIP, CLIP, true).unwrap();
        ui.rollback(checkpoint);
        assert_eq!(ui.len(), 1);
        assert_eq!(
            ui.check_unique_registration(WidgetId(1)),
            Err(RegistrationError::DuplicateId(WidgetId(1)))
        );
        ui.register_unique(WidgetId(2), CLIP, CLIP, true).unwrap();
        ui.begin_frame();
        ui.register_unique(WidgetId(1), CLIP, CLIP, true).unwrap();
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
