//! Reusable category navigation and transactional context-menu widgets.
//! Entries are read-only caller projections. State contains only navigation.
use super::{Theme, WidgetContext, WidgetError, WidgetSpec, WidgetState};
use crate::{
    ui::{Role, UiRuntimeStorage, WidgetId},
    visual::{StrokeAlignment, VisualList},
};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

pub struct Entry<'a, K> {
    pub key: K,
    pub label: &'a str,
    pub category: &'a [String],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target<K> {
    Action(K),
    Navigate(Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row<K> {
    pub label: String,
    pub target: Target<K>,
}
#[derive(Default)]
pub struct ContextMenu {
    path: Vec<String>,
    navigation: Vec<(WidgetId, Vec<String>)>,
}
impl ContextMenu {
    pub fn reset(&mut self) {
        self.path.clear();
        self.navigation.clear();
    }
    pub fn begin_frame(&mut self) {
        self.navigation.clear();
    }
    pub fn path(&self) -> &[String] {
        &self.path
    }
    pub fn navigate_to(&mut self, path: Vec<String>) {
        self.path = path;
        self.navigation.clear();
    }
    pub fn activate(&mut self, id: WidgetId) -> bool {
        let Some((_, path)) = self.navigation.iter().find(|(widget, _)| *widget == id) else {
            return false;
        };
        self.navigate_to(path.clone());
        true
    }
    /// Flat mode is used for global search results; browsing respects any depth.
    pub fn rows<'a, K: Clone + 'a>(
        &self,
        entries: impl IntoIterator<Item = Entry<'a, K>>,
        flat: bool,
    ) -> Vec<Row<K>> {
        let mut categories = BTreeMap::new();
        let mut actions = Vec::new();
        for entry in entries {
            if flat || entry.category == self.path.as_slice() {
                actions.push(Row {
                    label: entry.label.into(),
                    target: Target::Action(entry.key),
                });
            } else if entry.category.starts_with(&self.path) {
                let name = &entry.category[self.path.len()];
                categories.entry(name.clone()).or_insert_with(|| {
                    let mut path = self.path.clone();
                    path.push(name.clone());
                    path
                });
            }
        }
        let mut rows = Vec::new();
        if !flat && !self.path.is_empty() {
            rows.push(Row {
                label: "‹ Back".into(),
                target: Target::Navigate(self.path[..self.path.len() - 1].to_vec()),
            });
        }
        rows.extend(categories.into_iter().map(|(name, path)| Row {
            label: alloc::format!("{name} ›"),
            target: Target::Navigate(path),
        }));
        rows.extend(actions);
        rows
    }
    /// Labels are painted by the caller's text renderer, as with other Lagui widgets.
    pub fn paint_row<K, const N: usize>(
        &mut self,
        ui: &mut UiRuntimeStorage<N>,
        visuals: &mut VisualList<'_>,
        mut spec: WidgetSpec,
        row: &Row<K>,
        theme: Theme,
    ) -> Result<WidgetState, WidgetError> {
        spec.semantics.role = Role::MenuItem;
        spec.semantics.label = row.label.clone().into();
        let mut context = WidgetContext::new(ui, visuals);
        let state = context.transaction(|context| {
            let state = context.register(spec.clone(), true)?;
            if state.hovered || state.focused {
                context
                    .fill(spec.bounds, theme.surface_hovered, theme.radius, spec.clip)
                    .map_err(|_| WidgetError::VisualCapacityExceeded)?;
            }
            Ok::<_, WidgetError>(state)
        })?;
        if let Target::Navigate(path) = &row.target {
            self.navigation.push((spec.id, path.clone()));
        }
        Ok(state)
    }
}
/// Registers the popup as a menu and paints its frame atomically.
pub fn panel<const N: usize>(
    ui: &mut UiRuntimeStorage<N>,
    visuals: &mut VisualList<'_>,
    mut spec: WidgetSpec,
    theme: Theme,
) -> Result<(), WidgetError> {
    spec.semantics.role = Role::Menu;
    spec.semantics.label = "Context menu".into();
    let mut context = WidgetContext::new(ui, visuals);
    context.transaction(|context| {
        context.register(spec.clone(), false)?;
        context
            .outline(
                spec.bounds,
                theme.border,
                theme.surface,
                theme.radius,
                1.,
                StrokeAlignment::Inside,
                spec.clip,
            )
            .map_err(|_| WidgetError::VisualCapacityExceeded)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_support_arbitrary_depth_and_search_returns_actions_only() {
        let category = alloc::vec!["Shader".into(), "Math".into()];
        let entries = || {
            alloc::vec![Entry {
                key: "sine",
                label: "Sine",
                category: &category
            }]
        };
        let mut menu = ContextMenu::default();
        let rows = menu.rows(entries(), false);
        assert_eq!(
            rows[0].target,
            Target::Navigate(alloc::vec!["Shader".into()])
        );
        menu.navigate_to(alloc::vec!["Shader".into()]);
        assert_eq!(menu.rows(entries(), false)[1].label, "Math ›");
        menu.navigate_to(category.clone());
        assert_eq!(
            menu.rows(entries(), false)[1].target,
            Target::Action("sine")
        );
        assert_eq!(menu.rows(entries(), true).len(), 1);
        assert_eq!(menu.rows(entries(), true)[0].target, Target::Action("sine"));
        menu.reset();
        assert!(menu.path().is_empty());
    }
}
