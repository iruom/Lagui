//! Allocation-free layout helpers for application-defined widgets.

use crate::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Insets {
    pub const fn all(value: f32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridTrack {
    Fixed(f32),
    Fraction(f32),
}

/// Resolves row/column tracks and returns the number of cells written in row-major order.
pub fn grid(
    container: Rect,
    columns: &[GridTrack],
    rows: &[GridTrack],
    gap: [f32; 2],
    out: &mut [Rect],
) -> usize {
    if columns.is_empty() || rows.is_empty() {
        return 0;
    }
    let count = columns.len().saturating_mul(rows.len()).min(out.len());
    let resolve = |tracks: &[GridTrack], available: f32, index: usize| {
        let fixed: f32 = tracks
            .iter()
            .map(|t| match t {
                GridTrack::Fixed(v) => v.max(0.),
                GridTrack::Fraction(_) => 0.,
            })
            .sum();
        let fractions: f32 = tracks
            .iter()
            .map(|t| match t {
                GridTrack::Fraction(v) => v.max(0.),
                GridTrack::Fixed(_) => 0.,
            })
            .sum();
        match tracks[index] {
            GridTrack::Fixed(v) => v.max(0.),
            GridTrack::Fraction(v) if fractions > 0. => {
                (available - fixed).max(0.) * v.max(0.) / fractions
            }
            GridTrack::Fraction(_) => 0.,
        }
    };
    let available_w =
        (container.width - gap[0].max(0.) * columns.len().saturating_sub(1) as f32).max(0.);
    let available_h =
        (container.height - gap[1].max(0.) * rows.len().saturating_sub(1) as f32).max(0.);
    let mut y = container.y;
    let mut written = 0;
    for row in 0..rows.len() {
        let height = resolve(rows, available_h, row);
        let mut x = container.x;
        for column in 0..columns.len() {
            if written == count {
                return written;
            }
            let width = resolve(columns, available_w, column);
            out[written] = Rect::new(x, y, width, height);
            written += 1;
            x += width + gap[0].max(0.);
        }
        y += height + gap[1].max(0.);
    }
    written
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlexItem {
    pub basis: f32,
    pub grow: f32,
    pub shrink: f32,
    pub min: f32,
    pub max: f32,
}

impl FlexItem {
    pub const fn fixed(size: f32) -> Self {
        Self {
            basis: size,
            grow: 0.,
            shrink: 0.,
            min: size,
            max: size,
        }
    }
    pub const fn flexible(basis: f32, grow: f32) -> Self {
        Self {
            basis,
            grow,
            shrink: 1.,
            min: 0.,
            max: f32::MAX,
        }
    }
}

/// Resolves a single flex row/column into caller-owned output storage.
pub fn flex(
    container: Rect,
    axis: Axis,
    gap: f32,
    padding: Insets,
    items: &[FlexItem],
    out: &mut [Rect],
) -> usize {
    let count = items.len().min(out.len());
    if count == 0 {
        return 0;
    }
    let main = match axis {
        Axis::Horizontal => container.width - padding.left - padding.right,
        Axis::Vertical => container.height - padding.top - padding.bottom,
    };
    let gaps = gap.max(0.) * count.saturating_sub(1) as f32;
    let available = (main - gaps).max(0.);
    for (slot, item) in out[..count].iter_mut().zip(&items[..count]) {
        let min = finite_nonnegative(item.min);
        let max = finite_nonnegative(item.max).max(min);
        slot.width = finite_nonnegative(item.basis).clamp(min, max);
    }
    distribute_flex_space(&items[..count], &mut out[..count], available);
    let mut cursor = match axis {
        Axis::Horizontal => container.x + padding.left,
        Axis::Vertical => container.y + padding.top,
    };
    for slot in &mut out[..count] {
        let size = slot.width;
        *slot = match axis {
            Axis::Horizontal => Rect::new(
                cursor,
                container.y + padding.top,
                size,
                (container.height - padding.top - padding.bottom).max(0.),
            ),
            Axis::Vertical => Rect::new(
                container.x + padding.left,
                cursor,
                (container.width - padding.left - padding.right).max(0.),
                size,
            ),
        };
        cursor += size + gap.max(0.);
    }
    count
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() { value.max(0.) } else { 0. }
}

fn distribute_flex_space(items: &[FlexItem], out: &mut [Rect], available: f32) {
    // Iteration redistributes space left behind when an item reaches min/max.
    // At least one item becomes constrained per pass, so N passes are enough.
    for _ in 0..items.len() {
        let used: f32 = out.iter().map(|slot| slot.width).sum();
        let remaining = available - used;
        if remaining.abs() <= f32::EPSILON {
            break;
        }
        let growing = remaining > 0.;
        let total_weight: f32 = items
            .iter()
            .zip(out.iter())
            .map(|(item, slot)| {
                let min = finite_nonnegative(item.min);
                let max = finite_nonnegative(item.max).max(min);
                if growing && slot.width < max {
                    finite_nonnegative(item.grow)
                } else if !growing && slot.width > min {
                    finite_nonnegative(item.shrink) * slot.width
                } else {
                    0.
                }
            })
            .sum();
        if total_weight <= 0. {
            break;
        }
        let mut changed = false;
        for (item, slot) in items.iter().zip(out.iter_mut()) {
            let min = finite_nonnegative(item.min);
            let max = finite_nonnegative(item.max).max(min);
            let weight = if growing {
                finite_nonnegative(item.grow)
            } else {
                finite_nonnegative(item.shrink) * slot.width
            };
            if weight == 0. {
                continue;
            }
            let next = (slot.width + remaining * weight / total_weight).clamp(min, max);
            changed |= next != slot.width;
            slot.width = next;
        }
        if !changed {
            break;
        }
    }
}

/// Returns the half-open item range intersecting a scroll viewport, with overscan.
pub fn virtual_range(
    item_count: usize,
    item_extent: f32,
    scroll: f32,
    viewport_extent: f32,
    overscan: usize,
) -> core::ops::Range<usize> {
    if item_count == 0 || item_extent <= 0. || viewport_extent <= 0. {
        return 0..0;
    }
    let start = scroll.max(0.) / item_extent;
    // The values are non-negative, so integer conversion is floor without
    // requiring libm in the no_std core.
    let first = (start as usize).saturating_sub(overscan);
    let end = (scroll.max(0.) + viewport_extent) / item_extent;
    let end_floor = end as usize;
    let visible_end = end_floor
        .saturating_add(usize::from(end > end_floor as f32))
        .min(item_count);
    first.min(item_count)..visible_end.saturating_add(overscan).min(item_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flex_distributes_remaining_space() {
        let mut out = [Rect::default(); 3];
        flex(
            Rect::new(0., 0., 300., 40.),
            Axis::Horizontal,
            10.,
            Insets::all(0.),
            &[
                FlexItem::fixed(50.),
                FlexItem::flexible(50., 1.),
                FlexItem::flexible(50., 1.),
            ],
            &mut out,
        );
        assert_eq!(out[0].width, 50.);
        assert_eq!(out[1].width, 115.);
        assert_eq!(out[2].x, 185.);
    }
    #[test]
    fn flex_shrinks_and_redistributes_after_constraints() {
        let mut out = [Rect::default(); 2];
        let items = [
            FlexItem {
                basis: 100.,
                grow: 1.,
                shrink: 1.,
                min: 80.,
                max: 110.,
            },
            FlexItem {
                basis: 100.,
                grow: 1.,
                shrink: 1.,
                min: 20.,
                max: 200.,
            },
        ];
        flex(
            Rect::new(0., 0., 120., 20.),
            Axis::Horizontal,
            0.,
            Insets::all(0.),
            &items,
            &mut out,
        );
        assert_eq!(out[0].width, 80.);
        assert_eq!(out[1].width, 40.);

        flex(
            Rect::new(0., 0., 260., 20.),
            Axis::Horizontal,
            0.,
            Insets::all(0.),
            &items,
            &mut out,
        );
        assert_eq!(out[0].width, 110.);
        assert_eq!(out[1].width, 150.);
    }
    #[test]
    fn virtualization_clamps_and_overscans() {
        assert_eq!(virtual_range(1000, 20., 200., 100., 2), 8..17);
    }
    #[test]
    fn grid_resolves_fixed_and_fraction_tracks() {
        let mut out = [Rect::default(); 4];
        assert_eq!(
            grid(
                Rect::new(0., 0., 210., 110.),
                &[GridTrack::Fixed(50.), GridTrack::Fraction(1.)],
                &[GridTrack::Fraction(1.), GridTrack::Fraction(1.)],
                [10., 10.],
                &mut out
            ),
            4
        );
        assert_eq!(out[1], Rect::new(60., 0., 150., 50.));
        assert_eq!(out[3].y, 60.);
    }
}
