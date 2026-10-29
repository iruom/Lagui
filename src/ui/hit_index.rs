//! Derived bounds over contiguous painter-order ranges. Leaves refer to the
//! region array; no widget identity or interaction state is owned here.
use super::Region;
use alloc::vec::Vec;

#[derive(Clone, Copy)]
struct Bounds {
    low: [f32; 2],
    high: [f32; 2],
}

impl Bounds {
    const EMPTY: Self = Self {
        low: [f32::INFINITY; 2],
        high: [f32::NEG_INFINITY; 2],
    };
    fn region(region: &Region) -> Self {
        let a = region.bounds;
        let b = region.clip;
        // Preserve contains semantics, including empty, NaN and infinite bounds.
        if !(a.x < a.x + a.width
            && a.y < a.y + a.height
            && b.x < b.x + b.width
            && b.y < b.y + b.height)
        {
            return Self::EMPTY;
        }
        let low = [a.x.max(b.x), a.y.max(b.y)];
        let high = [
            (a.x + a.width).min(b.x + b.width),
            (a.y + a.height).min(b.y + b.height),
        ];
        if low[0] < high[0] && low[1] < high[1] {
            Self { low, high }
        } else {
            Self::EMPTY
        }
    }
    fn union(self, other: Self) -> Self {
        Self {
            low: [self.low[0].min(other.low[0]), self.low[1].min(other.low[1])],
            high: [
                self.high[0].max(other.high[0]),
                self.high[1].max(other.high[1]),
            ],
        }
    }
    fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.low[0]
            && point[1] >= self.low[1]
            && point[0] < self.high[0]
            && point[1] < self.high[1]
    }
}

#[derive(Default)]
pub(super) struct HitIndex {
    nodes: Vec<Bounds>,
    base: usize,
    pub len: usize,
}
impl HitIndex {
    pub fn is_unbuilt(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn rebuild(&mut self, regions: &[Region]) {
        self.len = regions.len();
        self.base = regions.len().max(1).next_power_of_two();
        self.nodes.resize(self.base * 2, Bounds::EMPTY);
        for (i, region) in regions.iter().enumerate() {
            self.nodes[self.base + i] = Bounds::region(region);
        }
        self.nodes[self.base + regions.len()..].fill(Bounds::EMPTY);
        for node in (1..self.base).rev() {
            self.nodes[node] = self.nodes[node * 2].union(self.nodes[node * 2 + 1]);
        }
    }
    pub fn hit(&self, point: [f32; 2], minimum: usize) -> Option<usize> {
        let mut node: usize = 1;
        loop {
            if self.nodes[node].contains(point) {
                if node >= self.base {
                    let index = node - self.base;
                    // Every remaining leaf is behind this one, hence also
                    // outside the modal scope when this leaf is outside it.
                    return (index >= minimum).then_some(index);
                }
                node = node * 2 + 1;
                continue;
            }
            // Painter order is a right-first traversal of this complete tree.
            // Skip finished left-child ancestors, then visit the left sibling
            // of the nearest right child. No query stack or allocations.
            node >>= node.trailing_zeros();
            if node == 1 {
                return None;
            }
            node -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Rect,
        ui::{UiRuntimeStorage, WidgetId},
    };

    #[test]
    fn indexed_queries_match_reverse_scan_for_overlap_clip_modal_and_numeric_edges() {
        let clip = Rect::new(-900., -900., 1800., 1800.);
        let mut regions = Vec::new();
        for i in 0..1537 {
            regions.push(Region {
                bounds: Rect::new(
                    ((i * 7919) % 1600) as f32 - 800.,
                    ((i * 607) % 1600) as f32 - 800.,
                    (i % 91) as f32,
                    (i % 67) as f32,
                ),
                clip: if i % 3 == 0 {
                    Rect::new(-300., -300., 600., 600.)
                } else {
                    clip
                },
                ..Region::default()
            });
        }
        for rect in [
            Rect::new(f32::NAN, 0., 10., 10.),
            Rect::new(0., 0., f32::INFINITY, 1.),
            Rect::new(0., 0., -1., 10.),
            Rect::new(-20., -20., 40., 40.),
        ] {
            regions.push(Region {
                bounds: rect,
                clip,
                ..Region::default()
            });
        }
        let mut index = HitIndex::default();
        index.rebuild(&regions);
        for minimum in [0, 17, 513, regions.len()] {
            for i in 0..4096 {
                let point = [
                    ((i * 997) % 1801) as f32 - 900.,
                    ((i * 127) % 1801) as f32 - 900.,
                ];
                let expected = (minimum..regions.len()).rev().find(|&r| {
                    regions[r].bounds.contains(point[0], point[1])
                        && regions[r].clip.contains(point[0], point[1])
                });
                assert_eq!(
                    index.hit(point, minimum),
                    expected,
                    "{point:?}, minimum={minimum}"
                );
            }
        }
        for point in [[f32::NAN, 0.], [f32::INFINITY, 0.], [0., f32::NEG_INFINITY]] {
            assert_eq!(index.hit(point, 0), None);
        }
        index.rebuild(&[]);
        assert_eq!(index.hit([0., 0.], 0), None);
    }

    #[test]
    fn frame_reuse_identity_changes_partial_registration_and_rollback_keep_queries_current() {
        let mut ui = UiRuntimeStorage::<0>::default();
        let clip = Rect::new(0., 0., 100., 100.);
        for frame in 0..4 {
            ui.begin_frame();
            for i in 0..33 {
                ui.register(
                    WidgetId(i + 100 * frame),
                    Rect::new(i as f32, 0., 1., 10.),
                    clip,
                    true,
                )
                .unwrap();
            }
            ui.end_frame();
            ui.pointer_move([0.5, 1.]);
            assert_eq!(ui.hovered_id(), Some(WidgetId(100 * frame)));
        }
        let storage = ui.hits.nodes.as_ptr();
        ui.begin_frame();
        ui.register(WidgetId(900), clip, clip, true).unwrap();
        let checkpoint = ui.checkpoint();
        ui.register(WidgetId(901), clip, clip, true).unwrap();
        ui.end_frame();
        ui.pointer_move([4., 4.]);
        assert_eq!(ui.hovered_id(), Some(WidgetId(901)));
        ui.rollback(checkpoint);
        ui.pointer_move([4., 4.]);
        assert_eq!(ui.hovered_id(), Some(WidgetId(900)));
        ui.end_frame();
        assert_eq!(ui.hovered_id(), Some(WidgetId(900)));
        assert_eq!(ui.hits.nodes.as_ptr(), storage);
        ui.begin_frame();
        ui.end_frame();
        ui.pointer_move([4., 4.]);
        assert_eq!(ui.hovered_id(), None);
    }
}
