//! Stable-identity split panel layout; alloc storage grows with the workspace.
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::{CapacityExceeded, Rect, layout::Axis};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PanelId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitId(u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLayout {
    pub id: PanelId,
    pub bounds: Rect,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            id: PanelId(0),
            bounds: Rect::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitLayout {
    pub id: SplitId,
    pub bounds: Rect,
    pub divider: Rect,
    pub axis: Axis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockZone {
    Left,
    Right,
    Top,
    Bottom,
}

impl Default for SplitLayout {
    fn default() -> Self {
        Self {
            id: SplitId(0),
            bounds: Rect::default(),
            divider: Rect::default(),
            axis: Axis::Horizontal,
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelError {
    CapacityExceeded,
    PanelNotFound,
    DuplicatePanel,
    SplitNotFound,
    IdentityExhausted,
}

impl core::fmt::Display for PanelError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::CapacityExceeded => "panel tree capacity was exceeded",
            Self::PanelNotFound => "panel was not found",
            Self::DuplicatePanel => "panel ID is already present",
            Self::SplitNotFound => "split was not found",
            Self::IdentityExhausted => "split identity space was exhausted",
        })
    }
}

impl core::error::Error for PanelError {}

/// Active-page state for a tabbed panel. Labels and page data remain owned by
/// the application so the core can stay allocation-free.
pub struct PanelTabs<const N: usize> {
    #[cfg(feature = "alloc")]
    tabs: Vec<PanelId>,
    #[cfg(not(feature = "alloc"))]
    tabs: [PanelId; N],
    len: usize,
    active: PanelId,
}

impl<const N: usize> PanelTabs<N> {
    pub fn new(first: PanelId) -> Result<Self, CapacityExceeded> {
        #[cfg(not(feature = "alloc"))]
        if N == 0 {
            return Err(CapacityExceeded);
        }
        #[cfg(not(feature = "alloc"))]
        let mut tabs = [PanelId(0); N];
        #[cfg(not(feature = "alloc"))]
        {
            tabs[0] = first;
        }
        #[cfg(feature = "alloc")]
        let tabs = {
            let mut tabs = Vec::with_capacity(N.max(1));
            tabs.push(first);
            tabs
        };
        Ok(Self {
            tabs,
            len: 1,
            active: first,
        })
    }

    pub fn add(&mut self, id: PanelId) -> Result<(), PanelError> {
        if self.tabs().contains(&id) {
            return Err(PanelError::DuplicatePanel);
        }
        #[cfg(feature = "alloc")]
        self.tabs.push(id);
        #[cfg(not(feature = "alloc"))]
        {
            let slot = self
                .tabs
                .get_mut(self.len)
                .ok_or(PanelError::CapacityExceeded)?;
            *slot = id;
        }
        self.len += 1;
        Ok(())
    }

    pub fn tabs(&self) -> &[PanelId] {
        &self.tabs[..self.len]
    }

    pub fn active(&self) -> PanelId {
        self.active
    }

    pub fn activate(&mut self, id: PanelId) -> Result<bool, PanelError> {
        if !self.tabs().contains(&id) {
            return Err(PanelError::PanelNotFound);
        }
        let changed = self.active != id;
        self.active = id;
        Ok(changed)
    }
}

#[derive(Clone, Copy, Debug)]
enum Node {
    Empty,
    Panel(PanelId),
    Split {
        id: SplitId,
        axis: Axis,
        ratio: f32,
        first: usize,
        second: usize,
    },
}

/// Binary panel tree with stable split identities independent of storage slots.
///
/// `N` reserves nodes with alloc and bounds them without alloc. Splitting one
/// panel needs two free nodes. Layout results use caller-owned slices.
pub struct PanelTree<const N: usize> {
    #[cfg(feature = "alloc")]
    nodes: Vec<Node>,
    #[cfg(not(feature = "alloc"))]
    nodes: [Node; N],
    next_split: u64,
    divider_width: f32,
    minimum_extent: f32,
}

impl<const N: usize> PanelTree<N> {
    pub fn new(root: PanelId) -> Result<Self, CapacityExceeded> {
        #[cfg(not(feature = "alloc"))]
        if N == 0 {
            return Err(CapacityExceeded);
        }
        #[cfg(not(feature = "alloc"))]
        let mut nodes = [Node::Empty; N];
        #[cfg(feature = "alloc")]
        let mut nodes = alloc::vec![Node::Empty; N.max(1)];
        nodes[0] = Node::Panel(root);
        Ok(Self {
            nodes,
            next_split: 1,
            divider_width: 6.,
            minimum_extent: 48.,
        })
    }

    pub fn root(&self) -> PanelId {
        match self.nodes[0] {
            Node::Panel(id) => id,
            Node::Split { .. } | Node::Empty => self.first_panel(0).unwrap_or(PanelId(0)),
        }
    }

    pub fn set_metrics(&mut self, divider_width: f32, minimum_extent: f32) {
        if divider_width.is_finite() && divider_width >= 0. {
            self.divider_width = divider_width;
        }
        if minimum_extent.is_finite() && minimum_extent >= 0. {
            self.minimum_extent = minimum_extent;
        }
    }

    pub fn split(
        &mut self,
        panel: PanelId,
        new_panel: PanelId,
        axis: Axis,
        ratio: f32,
    ) -> Result<SplitId, PanelError> {
        if self.find_panel(new_panel).is_some() {
            return Err(PanelError::DuplicatePanel);
        }
        let target = self.find_panel(panel).ok_or(PanelError::PanelNotFound)?;
        let next = self
            .next_split
            .checked_add(1)
            .ok_or(PanelError::IdentityExhausted)?;
        let id = SplitId(self.next_split);
        let (first, second) = self.free_pair()?;
        self.nodes[first] = Node::Panel(panel);
        self.nodes[second] = Node::Panel(new_panel);
        self.nodes[target] = Node::Split {
            id,
            axis,
            ratio: sanitize_ratio(ratio),
            first,
            second,
        };
        self.next_split = next;
        Ok(id)
    }

    pub fn set_ratio(&mut self, split: SplitId, ratio: f32) -> Result<(), PanelError> {
        let Node::Split { ratio: value, .. } = self
            .nodes
            .iter_mut()
            .find(|node| matches!(node, Node::Split { id, .. } if *id == split))
            .ok_or(PanelError::SplitNotFound)?
        else {
            return Err(PanelError::SplitNotFound);
        };
        *value = sanitize_ratio(ratio);
        Ok(())
    }

    /// Moves an existing panel beside another panel and compacts its old split.
    pub fn dock(
        &mut self,
        source: PanelId,
        target: PanelId,
        zone: DockZone,
        ratio: f32,
    ) -> Result<SplitId, PanelError> {
        if source == target {
            return Err(PanelError::DuplicatePanel);
        }
        self.find_panel(source).ok_or(PanelError::PanelNotFound)?;
        self.find_panel(target).ok_or(PanelError::PanelNotFound)?;
        let next = self
            .next_split
            .checked_add(1)
            .ok_or(PanelError::IdentityExhausted)?;
        let id = SplitId(self.next_split);
        self.remove_panel(source)?;
        let target_index = self.find_panel(target).ok_or(PanelError::PanelNotFound)?;
        let (first, second) = self.free_pair()?;
        let (axis, source_first) = match zone {
            DockZone::Left => (Axis::Horizontal, true),
            DockZone::Right => (Axis::Horizontal, false),
            DockZone::Top => (Axis::Vertical, true),
            DockZone::Bottom => (Axis::Vertical, false),
        };
        self.nodes[first] = Node::Panel(if source_first { source } else { target });
        self.nodes[second] = Node::Panel(if source_first { target } else { source });
        self.nodes[target_index] = Node::Split {
            id,
            axis,
            ratio: sanitize_ratio(ratio),
            first,
            second,
        };
        self.next_split = next;
        Ok(id)
    }

    /// Updates a split from a pointer coordinate, applying the configured
    /// minimum panel extent. Returns whether the ratio changed.
    pub fn resize(
        &mut self,
        split: SplitId,
        container: Rect,
        pointer: [f32; 2],
    ) -> Result<bool, PanelError> {
        let Node::Split { axis, ratio, .. } = self
            .nodes
            .iter_mut()
            .find(|node| matches!(node, Node::Split { id, .. } if *id == split))
            .ok_or(PanelError::SplitNotFound)?
        else {
            return Err(PanelError::SplitNotFound);
        };
        let (origin, extent, position) = match axis {
            Axis::Horizontal => (container.x, container.width, pointer[0]),
            Axis::Vertical => (container.y, container.height, pointer[1]),
        };
        let usable = (extent - self.divider_width).max(0.);
        if !usable.is_finite() || usable <= 0. || !position.is_finite() {
            return Ok(false);
        }
        let minimum_ratio = (self.minimum_extent / usable).min(0.5);
        let next = ((position - origin - self.divider_width * 0.5) / usable)
            .clamp(minimum_ratio, 1. - minimum_ratio);
        let changed = *ratio != next;
        *ratio = next;
        Ok(changed)
    }

    pub fn layout(
        &self,
        bounds: Rect,
        panels: &mut [PanelLayout],
        splits: &mut [SplitLayout],
    ) -> Result<(usize, usize), CapacityExceeded> {
        let (panel_count, split_count) = self.count(0);
        if panels.len() < panel_count || splits.len() < split_count {
            return Err(CapacityExceeded);
        }
        let mut panel_len = 0;
        let mut split_len = 0;
        self.layout_node(0, bounds, panels, splits, &mut panel_len, &mut split_len);
        Ok((panel_len, split_len))
    }

    fn layout_node(
        &self,
        index: usize,
        bounds: Rect,
        panels: &mut [PanelLayout],
        splits: &mut [SplitLayout],
        panel_len: &mut usize,
        split_len: &mut usize,
    ) {
        match self.nodes[index] {
            Node::Empty => {}
            Node::Panel(id) => {
                panels[*panel_len] = PanelLayout { id, bounds };
                *panel_len += 1;
            }
            Node::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, divider, b) = split_rects(bounds, axis, ratio, self.divider_width);
                splits[*split_len] = SplitLayout {
                    id,
                    bounds,
                    divider,
                    axis,
                };
                *split_len += 1;
                self.layout_node(first, a, panels, splits, panel_len, split_len);
                self.layout_node(second, b, panels, splits, panel_len, split_len);
            }
        }
    }

    fn free_pair(&mut self) -> Result<(usize, usize), PanelError> {
        #[cfg(feature = "alloc")]
        {
            let free = self
                .nodes
                .iter()
                .filter(|node| matches!(node, Node::Empty))
                .count();
            if free < 2 {
                self.nodes.resize(self.nodes.len() + 2 - free, Node::Empty);
            }
        }
        let mut free = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| matches!(node, Node::Empty).then_some(index));
        Ok((
            free.next().ok_or(PanelError::CapacityExceeded)?,
            free.next().ok_or(PanelError::CapacityExceeded)?,
        ))
    }

    fn find_panel(&self, id: PanelId) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| matches!(node, Node::Panel(value) if *value == id))
    }

    fn remove_panel(&mut self, id: PanelId) -> Result<(), PanelError> {
        let target = self.find_panel(id).ok_or(PanelError::PanelNotFound)?;
        if target == 0 {
            return Err(PanelError::PanelNotFound);
        }
        let parent = self
            .nodes
            .iter()
            .enumerate()
            .find_map(|(index, node)| match *node {
                Node::Split { first, second, .. } if first == target || second == target => {
                    Some((index, if first == target { second } else { first }))
                }
                _ => None,
            });
        let (parent, sibling) = parent.ok_or(PanelError::PanelNotFound)?;
        self.nodes[parent] = self.nodes[sibling];
        self.nodes[target] = Node::Empty;
        self.nodes[sibling] = Node::Empty;
        Ok(())
    }

    fn first_panel(&self, index: usize) -> Option<PanelId> {
        match *self.nodes.get(index)? {
            Node::Empty => None,
            Node::Panel(id) => Some(id),
            Node::Split { first, second, .. } => {
                self.first_panel(first).or_else(|| self.first_panel(second))
            }
        }
    }

    fn count(&self, index: usize) -> (usize, usize) {
        match self.nodes[index] {
            Node::Empty => (0, 0),
            Node::Panel(_) => (1, 0),
            Node::Split { first, second, .. } => {
                let a = self.count(first);
                let b = self.count(second);
                (a.0 + b.0, a.1 + b.1 + 1)
            }
        }
    }
}

fn sanitize_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(0., 1.)
    } else {
        0.5
    }
}

fn split_rects(bounds: Rect, axis: Axis, ratio: f32, divider: f32) -> (Rect, Rect, Rect) {
    match axis {
        Axis::Horizontal => {
            let usable = (bounds.width - divider).max(0.);
            let first = usable * ratio;
            (
                Rect::new(bounds.x, bounds.y, first, bounds.height),
                Rect::new(bounds.x + first, bounds.y, divider, bounds.height),
                Rect::new(
                    bounds.x + first + divider,
                    bounds.y,
                    usable - first,
                    bounds.height,
                ),
            )
        }
        Axis::Vertical => {
            let usable = (bounds.height - divider).max(0.);
            let first = usable * ratio;
            (
                Rect::new(bounds.x, bounds.y, bounds.width, first),
                Rect::new(bounds.x, bounds.y + first, bounds.width, divider),
                Rect::new(
                    bounds.x,
                    bounds.y + first + divider,
                    bounds.width,
                    usable - first,
                ),
            )
        }
    }
}

/// Chooses the nearest edge docking zone and returns its preview rectangle.
pub fn dock_preview(bounds: Rect, point: [f32; 2]) -> Option<(DockZone, Rect)> {
    if !bounds.contains(point[0], point[1]) {
        return None;
    }
    let x = (point[0] - bounds.x) / bounds.width.max(1.);
    let y = (point[1] - bounds.y) / bounds.height.max(1.);
    let left = x;
    let right = 1. - x;
    let top = y;
    let bottom = 1. - y;
    let (zone, distance) = [
        (DockZone::Left, left),
        (DockZone::Right, right),
        (DockZone::Top, top),
        (DockZone::Bottom, bottom),
    ]
    .into_iter()
    .min_by(|a, b| a.1.total_cmp(&b.1))?;
    if distance > 0.4 {
        return None;
    }
    let preview = match zone {
        DockZone::Left => Rect::new(bounds.x, bounds.y, bounds.width * 0.5, bounds.height),
        DockZone::Right => Rect::new(
            bounds.x + bounds.width * 0.5,
            bounds.y,
            bounds.width * 0.5,
            bounds.height,
        ),
        DockZone::Top => Rect::new(bounds.x, bounds.y, bounds.width, bounds.height * 0.5),
        DockZone::Bottom => Rect::new(
            bounds.x,
            bounds.y + bounds.height * 0.5,
            bounds.width,
            bounds.height * 0.5,
        ),
    };
    Some((zone, preview))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_identity_survives_relocation_and_removed_ids_cannot_alias() {
        let mut tree = PanelTree::<7>::new(PanelId(1)).unwrap();
        let removed = tree
            .split(PanelId(1), PanelId(2), Axis::Horizontal, 0.5)
            .unwrap();
        let retained = tree
            .split(PanelId(2), PanelId(3), Axis::Vertical, 0.5)
            .unwrap();
        let new = tree
            .dock(PanelId(1), PanelId(3), DockZone::Bottom, 0.5)
            .unwrap();
        assert_ne!(removed, new);
        assert_eq!(tree.set_ratio(removed, 0.9), Err(PanelError::SplitNotFound));
        tree.set_ratio(retained, 0.25).unwrap();
        let mut panels = [PanelLayout::default(); 3];
        let mut splits = [SplitLayout::default(); 2];
        tree.layout(Rect::new(0., 0., 300., 300.), &mut panels, &mut splits)
            .unwrap();
        assert!(splits.iter().any(|split| split.id == retained));
        assert!(splits.iter().any(|split| split.id == new));
    }
    #[test]
    #[cfg(feature = "alloc")]
    fn panel_and_tab_storage_grow_beyond_the_reservation() {
        let mut tree = PanelTree::<0>::new(PanelId(1)).unwrap();
        let mut tabs = PanelTabs::<0>::new(PanelId(1)).unwrap();
        for id in 2..=100 {
            tree.split(PanelId(1), PanelId(id), Axis::Horizontal, 0.5)
                .unwrap();
            tabs.add(PanelId(id)).unwrap();
        }
        let mut panels = alloc::vec![PanelLayout::default(); 100];
        let mut splits = alloc::vec![SplitLayout::default(); 99];
        assert_eq!(
            tree.layout(Rect::new(0., 0., 1000., 1000.), &mut panels, &mut splits),
            Ok((100, 99))
        );
        tabs.activate(PanelId(100)).unwrap();
        assert_eq!(tabs.active(), PanelId(100));
    }

    #[test]
    fn nested_splits_produce_panels_and_dividers() {
        let mut tree = PanelTree::<7>::new(PanelId(1)).unwrap();
        tree.set_metrics(4., 40.);
        tree.split(PanelId(1), PanelId(2), Axis::Horizontal, 0.25)
            .unwrap();
        tree.split(PanelId(2), PanelId(3), Axis::Vertical, 0.5)
            .unwrap();
        let mut panels = [PanelLayout {
            id: PanelId(0),
            bounds: Rect::default(),
        }; 3];
        let mut splits = [SplitLayout {
            id: SplitId(0),
            bounds: Rect::default(),
            divider: Rect::default(),
            axis: Axis::Horizontal,
        }; 2];
        assert_eq!(
            tree.layout(Rect::new(0., 0., 404., 204.), &mut panels, &mut splits),
            Ok((3, 2))
        );
        assert_eq!(
            panels[0],
            PanelLayout {
                id: PanelId(1),
                bounds: Rect::new(0., 0., 100., 204.)
            }
        );
        assert_eq!(panels[1].bounds, Rect::new(104., 0., 300., 100.));
        assert_eq!(panels[2].bounds, Rect::new(104., 104., 300., 100.));
    }

    #[test]
    fn resize_obeys_minimum_extent() {
        let mut tree = PanelTree::<3>::new(PanelId(1)).unwrap();
        tree.set_metrics(10., 80.);
        let split = tree
            .split(PanelId(1), PanelId(2), Axis::Horizontal, 0.5)
            .unwrap();
        tree.resize(split, Rect::new(20., 0., 310., 100.), [21., 0.])
            .unwrap();
        let mut panels = [PanelLayout {
            id: PanelId(0),
            bounds: Rect::default(),
        }; 2];
        let mut splits = [SplitLayout {
            id: SplitId(0),
            bounds: Rect::default(),
            divider: Rect::default(),
            axis: Axis::Horizontal,
        }];
        tree.layout(Rect::new(20., 0., 310., 100.), &mut panels, &mut splits)
            .unwrap();
        assert!((panels[0].bounds.width - 80.).abs() < 0.001);
        assert!((splits[0].divider.x - 100.).abs() < 0.001);
    }

    #[test]
    #[cfg(not(feature = "alloc"))]
    fn split_is_atomic_when_capacity_is_exhausted() {
        let mut tree = PanelTree::<2>::new(PanelId(1)).unwrap();
        assert_eq!(
            tree.split(PanelId(1), PanelId(2), Axis::Horizontal, 0.5),
            Err(PanelError::CapacityExceeded)
        );
        let mut panels = [PanelLayout {
            id: PanelId(0),
            bounds: Rect::default(),
        }];
        assert_eq!(
            tree.layout(Rect::new(0., 0., 10., 10.), &mut panels, &mut []),
            Ok((1, 0))
        );
        assert_eq!(panels[0].id, PanelId(1));
    }

    #[test]
    fn tabs_switch_active_panel_and_reject_duplicates() {
        let mut tabs = PanelTabs::<2>::new(PanelId(10)).unwrap();
        tabs.add(PanelId(11)).unwrap();
        assert_eq!(tabs.active(), PanelId(10));
        assert_eq!(tabs.activate(PanelId(11)), Ok(true));
        assert_eq!(tabs.activate(PanelId(11)), Ok(false));
        assert_eq!(tabs.add(PanelId(10)), Err(PanelError::DuplicatePanel));
    }

    #[test]
    fn docking_moves_a_panel_and_compacts_its_old_split() {
        let mut tree = PanelTree::<7>::new(PanelId(1)).unwrap();
        tree.split(PanelId(1), PanelId(2), Axis::Horizontal, 0.5)
            .unwrap();
        tree.split(PanelId(2), PanelId(3), Axis::Vertical, 0.5)
            .unwrap();
        tree.dock(PanelId(1), PanelId(3), DockZone::Bottom, 0.5)
            .unwrap();
        let mut panels = [PanelLayout {
            id: PanelId(0),
            bounds: Rect::default(),
        }; 3];
        let mut splits = [SplitLayout::default(); 2];
        assert_eq!(
            tree.layout(Rect::new(0., 0., 300., 300.), &mut panels, &mut splits),
            Ok((3, 2))
        );
        assert_eq!(panels[2].id, PanelId(1));
        assert!(panels[2].bounds.y > panels[1].bounds.y);
    }
}
