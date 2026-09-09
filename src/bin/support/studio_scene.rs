use best_gui::{
    CapacityExceeded, Rect,
    layout::{Axis, virtual_range},
    panel::{DockZone, PanelId, PanelLayout, PanelTabs, PanelTree, SplitLayout, dock_preview},
    text_edit::TextEdit,
    ui::{Role, Semantics, UiRuntime, WidgetId},
    viewport::{Viewport2D, ViewportGesture},
    visual::VisualList,
};
use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

const TOP_PLAY: WidgetId = WidgetId(1);
const TOP_ADD: WidgetId = WidgetId(2);
const TOP_EXPORT: WidgetId = WidgetId(3);
const SEARCH: WidgetId = WidgetId(10);
const CANVAS: WidgetId = WidgetId(20);
const ZOOM_OUT: WidgetId = WidgetId(21);
const ZOOM_IN: WidgetId = WidgetId(22);
const SNAP: WidgetId = WidgetId(23);
const LEFT_SPLITTER: WidgetId = WidgetId(30);
const OPACITY: WidgetId = WidgetId(40);
const ENABLED: WidgetId = WidgetId(41);
const PLAYHEAD: WidgetId = WidgetId(50);
const PANEL_HEADER_BASE: u64 = 60;
const PANEL_ACTION_BASE: u64 = 80;
const PANEL_MENU_BASE: u64 = 1000;
const LAYER_BASE: u64 = 100;
const NODE_BASE: u64 = 200;
const MAX_SEARCH_GRAPHEMES: usize = 128;
const PANEL_HEADER_HEIGHT: f32 = 28.;
const LEFT_PANEL: PanelId = PanelId(1);
const CANVAS_PANEL: PanelId = PanelId(2);
const RIGHT_PANEL: PanelId = PanelId(3);
const TIMELINE_PANEL: PanelId = PanelId(4);
const INSPECTOR_PAGE: PanelId = PanelId(10);
const PROJECT_PAGE: PanelId = PanelId(11);
const LAYERS_PAGE: PanelId = PanelId(12);
const ASSETS_PAGE: PanelId = PanelId(13);
const GRAPH_PAGE: PanelId = PanelId(14);
const PREVIEW_PAGE: PanelId = PanelId(15);
const TIMELINE_PAGE: PanelId = PanelId(16);
const CONSOLE_PAGE: PanelId = PanelId(17);
const PANEL_PAGES: [(PanelId, &str); 8] = [
    (LAYERS_PAGE, "LAYERS"),
    (ASSETS_PAGE, "ASSETS"),
    (GRAPH_PAGE, "GRAPH"),
    (PREVIEW_PAGE, "PREVIEW"),
    (INSPECTOR_PAGE, "INSPECTOR"),
    (PROJECT_PAGE, "PROJECT"),
    (TIMELINE_PAGE, "TIMELINE"),
    (CONSOLE_PAGE, "CONSOLE"),
];
const SLOT_PANELS: [PanelId; 4] = [LEFT_PANEL, CANVAS_PANEL, RIGHT_PANEL, TIMELINE_PANEL];

// Colors are premultiplied. On macOS the darker surfaces remain translucent so
// the system-provided window blur can show through without washing out the UI.
#[cfg(target_os = "macos")]
const BG: [f32; 4] = [0.023, 0.023, 0.023, 1.];
#[cfg(not(target_os = "macos"))]
const BG: [f32; 4] = [0.023, 0.023, 0.023, 1.];
#[cfg(target_os = "macos")]
const PANEL: [f32; 4] = [0.0215, 0.0215, 0.0215, 0.55];
#[cfg(not(target_os = "macos"))]
const PANEL: [f32; 4] = [0.039, 0.039, 0.039, 1.];
#[cfg(target_os = "macos")]
const PANEL_2: [f32; 4] = [0.0399, 0.0399, 0.0399, 0.70];
#[cfg(not(target_os = "macos"))]
const PANEL_2: [f32; 4] = [0.057, 0.057, 0.057, 1.];
#[cfg(target_os = "macos")]
const CANVAS_BG: [f32; 4] = [0.0176, 0.0176, 0.0176, 0.65];
#[cfg(not(target_os = "macos"))]
const CANVAS_BG: [f32; 4] = [0.027, 0.027, 0.027, 1.];
const BORDER: [f32; 4] = [0.126, 0.126, 0.126, 1.];
const PANEL_HEADER: [f32; 4] = [0.032, 0.032, 0.032, 1.];
const PANEL_HEADER_HOVER: [f32; 4] = [0.052, 0.052, 0.052, 1.];
const TEXT: [f32; 4] = [0.81, 0.81, 0.81, 1.];
const MUTED: [f32; 4] = [0.39, 0.39, 0.39, 1.];
const ACCENT: [f32; 4] = [0.33, 0.22, 0.94, 1.];
const CYAN: [f32; 4] = [0.12, 0.66, 0.92, 1.];
const GREEN: [f32; 4] = [0.12, 0.72, 0.46, 1.];
const ORANGE: [f32; 4] = [0.95, 0.43, 0.12, 1.];

pub struct Label<'a> {
    pub text: Cow<'a, str>,
    pub baseline: [f32; 2],
    pub size: f32,
    pub color: [f32; 4],
    pub clip: Rect,
    /// Number of visual commands already emitted when this label was painted.
    pub visual_index: usize,
    /// Horizontal center to align the shaped run to, when this is centered text.
    pub center_x: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Connection {
    pub start: [f32; 2],
    pub control_a: [f32; 2],
    pub control_b: [f32; 2],
    pub end: [f32; 2],
    pub color: [f32; 4],
    pub thickness: f32,
    pub clip: Rect,
    pub visual_index: usize,
}

#[derive(Clone, Copy, Default)]
struct Layout {
    left: Rect,
    canvas: Rect,
    timeline: Rect,
    panels: [PanelLayout; 4],
    panel_count: usize,
    splits: [SplitLayout; 3],
    opacity: Rect,
    search: Rect,
    gpu_canvas: Option<Rect>,
}

pub struct Studio {
    pub ui: UiRuntime,
    pub selected: usize,
    pub playing: bool,
    pub snapping: bool,
    pub opacity: f32,
    pub view: Viewport2D,
    pub current_time: f32,
    pub search: TextEdit<512>,
    panels: PanelTree<7>,
    panel_choices: [PanelTabs<8>; 4],
    open_panel_selector: Option<usize>,
    dock_drag: Option<PanelId>,
    dock_target: Option<(PanelId, DockZone, Rect)>,
    nodes: [[f32; 2]; 4],
    enabled: bool,
    layers_scroll: f32,
    layout: Layout,
    previous_pointer: [f32; 2],
    search_metrics: Vec<(usize, f32)>,
    preedit_cursor_x: f32,
    preedit_end_x: f32,
    preview_window_requested: bool,
}

impl Default for Studio {
    fn default() -> Self {
        let mut panels = PanelTree::<7>::new(LEFT_PANEL).expect("panel tree has root capacity");
        panels
            .split(LEFT_PANEL, TIMELINE_PANEL, Axis::Vertical, 0.72)
            .expect("panel tree has split capacity");
        panels
            .split(LEFT_PANEL, CANVAS_PANEL, Axis::Horizontal, 0.19)
            .expect("panel tree has split capacity");
        panels
            .split(CANVAS_PANEL, RIGHT_PANEL, Axis::Horizontal, 0.72)
            .expect("panel tree has split capacity");
        panels.set_metrics(6., 120.);
        let panel_choices =
            [LAYERS_PAGE, GRAPH_PAGE, INSPECTOR_PAGE, TIMELINE_PAGE].map(panel_choices);
        Self {
            ui: UiRuntime::default(),
            selected: 1,
            playing: false,
            snapping: true,
            opacity: 0.82,
            view: Viewport2D::default(),
            current_time: 2.4,
            search: TextEdit::default(),
            panels,
            panel_choices,
            open_panel_selector: None,
            dock_drag: None,
            dock_target: None,
            // Canvas-space positions are centered at world (0, 0).
            nodes: [[-322., -191.], [-76., -195.], [-76., -49.], [170., -127.]],
            enabled: true,
            layers_scroll: 0.,
            layout: Layout::default(),
            previous_pointer: [0.; 2],
            search_metrics: vec![(0, 0.)],
            preedit_cursor_x: 0.,
            preedit_end_x: 0.,
            preview_window_requested: false,
        }
    }
}

impl Studio {
    pub fn pointer_move(&mut self, position: [f32; 2]) -> bool {
        let delta = [
            position[0] - self.previous_pointer[0],
            position[1] - self.previous_pointer[1],
        ];
        let response = self.ui.pointer_move(position);
        if let Some(source) = self.dock_drag.filter(|_| response.dragging) {
            self.dock_target = self.layout.panels[..self.layout.panel_count]
                .iter()
                .filter(|panel| panel.id != source)
                .find_map(|panel| {
                    dock_preview(panel.bounds, position)
                        .map(|(zone, preview)| (panel.id, zone, preview))
                });
        }
        if let Some(target) = response.target.filter(|_| response.dragging) {
            match target {
                SEARCH => self.place_search_cursor(position[0], true),
                WidgetId(id) if (LEFT_SPLITTER.0..LEFT_SPLITTER.0 + 3).contains(&id) => {
                    let split = self.layout.splits[(id - LEFT_SPLITTER.0) as usize];
                    let _ = self.panels.resize(split.id, split.bounds, position);
                }
                CANVAS => {
                    self.view
                        .apply(self.layout.canvas, ViewportGesture::Pan { delta });
                }
                OPACITY => {
                    self.opacity = ((position[0] - self.layout.opacity.x)
                        / self.layout.opacity.width)
                        .clamp(0., 1.);
                }
                PLAYHEAD => {
                    self.current_time =
                        ((position[0] - self.layout.timeline.x - 94.) / 72.).clamp(0., 9.99);
                }
                WidgetId(id) if (NODE_BASE..NODE_BASE + 4).contains(&id) => {
                    let index = (id - NODE_BASE) as usize;
                    self.nodes[index][0] += delta[0] / self.view.scale;
                    self.nodes[index][1] += delta[1] / self.view.scale;
                }
                _ => {}
            }
        }
        self.previous_pointer = position;
        response.changed
    }

    pub fn pointer_down(&mut self, position: [f32; 2]) -> bool {
        self.previous_pointer = position;
        let response = self.ui.pointer_down(position);
        if let Some(WidgetId(id)) = response.target
            && (PANEL_HEADER_BASE..PANEL_HEADER_BASE + 4).contains(&id)
        {
            self.dock_drag = Some(SLOT_PANELS[(id - PANEL_HEADER_BASE) as usize]);
            self.dock_target = None;
        }
        if response.target == Some(SEARCH) {
            self.place_search_cursor(position[0], false);
        }
        if let Some(WidgetId(id)) = response.target
            && (NODE_BASE..NODE_BASE + 4).contains(&id)
        {
            self.selected = (id - NODE_BASE) as usize;
        }
        if !self.ui.focused(SEARCH) {
            self.clear_preedit();
        }
        self.pointer_move(position)
    }

    pub fn pointer_up(&mut self, position: [f32; 2]) -> bool {
        let docking = self.dock_drag.zip(self.dock_target);
        let response = self.ui.pointer_up(position);
        if let Some((source, (target, zone, _))) = docking {
            let _ = self.panels.dock(source, target, zone, 0.5);
        }
        self.dock_drag = None;
        self.dock_target = None;
        if response.clicked
            && let Some(target) = response.target
        {
            self.activate(target);
        }
        self.previous_pointer = position;
        response.changed || response.clicked
    }

    pub fn cancel_pointer(&mut self) {
        self.ui.cancel_pointer();
    }

    /// Two-axis scrolling is a pan gesture for the canvas and remains ordinary
    /// vertical scrolling over the layers panel.
    pub fn scroll(&mut self, delta: [f32; 2]) {
        let pointer = self.ui.pointer();
        if self.layout.canvas.contains(pointer[0], pointer[1]) {
            self.view
                .apply(self.layout.canvas, ViewportGesture::Pan { delta });
        } else if self
            .layout
            .left
            .contains(self.ui.pointer()[0], self.ui.pointer()[1])
        {
            self.layers_scroll = (self.layers_scroll - delta[1]).clamp(0., 160.);
        }
    }

    /// Native trackpad pinch delta. Exponential mapping stays positive and
    /// gives symmetric in/out magnification with no arbitrary zoom bounds.
    pub fn pinch(&mut self, delta: f64) {
        let pointer = self.ui.pointer();
        if self.layout.canvas.contains(pointer[0], pointer[1]) && delta.is_finite() {
            self.view.apply(
                self.layout.canvas,
                ViewportGesture::Zoom {
                    anchor: pointer,
                    factor: (delta as f32).exp(),
                },
            );
        }
    }

    fn zoom_at_canvas_center(&mut self, factor: f32) {
        self.view.zoom_at(
            self.layout.canvas,
            Viewport2D::center(self.layout.canvas),
            factor,
        );
    }

    pub fn focus_next(&mut self, reverse: bool) {
        self.ui.focus_next(reverse);
        if !self.ui.focused(SEARCH) {
            self.clear_preedit();
        }
    }

    pub fn activate_focused(&mut self) {
        if let Some(id) = self.ui.activate_focused() {
            self.activate(id);
        }
    }

    pub fn move_search_cursor(&mut self, right: bool, extend: bool) {
        if self.ui.focused(SEARCH) {
            self.search.move_cursor(right, extend);
        }
    }

    pub fn selected_search_text(&self) -> Option<&str> {
        (self.ui.focused(SEARCH) && !self.search.selection().is_empty())
            .then(|| self.search.selected_text())
    }

    pub fn cut_search_selection(&mut self) {
        if self.ui.focused(SEARCH) {
            self.search.cut();
        }
    }
    pub fn paste_search(&mut self, text: &str) {
        if self.ui.focused(SEARCH) {
            self.insert_search_text(text);
        }
    }

    pub fn keyboard(&mut self, key: &str) {
        if self.ui.focused(SEARCH) {
            match key {
                "Backspace" => {
                    if self.search.selection().is_empty()
                        && let Some((index, _)) = self.search.text()
                            [..self.search.selection().cursor]
                            .grapheme_indices(true)
                            .next_back()
                    {
                        self.search
                            .set_selection(index, self.search.selection().cursor);
                    }
                    self.search.backspace();
                }
                "Delete" => {
                    self.search.delete_forward();
                }
                "ArrowLeft" => self.search.move_cursor(false, false),
                "ArrowRight" => self.search.move_cursor(true, false),
                "Undo" => {
                    self.search.undo();
                }
                "Redo" => {
                    self.search.redo();
                }
                "SelectAll" => self.search.select_all(),
                "Escape" => {
                    self.ui.set_focus(None);
                    self.clear_preedit();
                }
                _ => {
                    let _ = self.search.insert(key);
                }
            }
            return;
        }
        match key {
            " " => self.playing = !self.playing,
            "+" | "=" => self.zoom_at_canvas_center(1.1),
            "-" => self.zoom_at_canvas_center(1. / 1.1),
            "ArrowLeft" => self.current_time = (self.current_time - 0.1).max(0.),
            "ArrowRight" => self.current_time = (self.current_time + 0.1).min(9.99),
            _ => {}
        }
    }

    pub fn set_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if self.ui.focused(SEARCH) {
            let remaining =
                MAX_SEARCH_GRAPHEMES.saturating_sub(self.search.text().graphemes(true).count());
            let mut accepted = String::new();
            for grapheme in text.graphemes(true).take(remaining) {
                if !grapheme.chars().any(char::is_control) {
                    accepted.push_str(grapheme);
                }
            }
            let _ = self.search.set_preedit(&accepted, cursor);
        } else {
            self.clear_preedit();
        }
    }

    pub fn commit_text(&mut self, text: &str) {
        if self.ui.focused(SEARCH) {
            self.insert_search_text(text);
        }
        self.clear_preedit();
    }

    fn insert_search_text(&mut self, text: &str) {
        let mut count = self.search.text().graphemes(true).count();
        let mut accepted = String::new();
        for grapheme in text.graphemes(true) {
            if count == MAX_SEARCH_GRAPHEMES {
                break;
            }
            if !grapheme.chars().any(char::is_control) {
                accepted.push_str(grapheme);
                count += 1;
            }
        }
        let _ = self.search.insert(&accepted);
    }

    pub fn clear_preedit(&mut self) {
        self.search.clear_preedit();
    }

    pub fn set_search_boundaries(&mut self, metrics: Vec<(usize, f32)>) {
        self.search_metrics = metrics;
    }

    pub fn set_preedit_metrics(&mut self, preedit_cursor_x: f32, preedit_end_x: f32) {
        self.preedit_cursor_x = preedit_cursor_x;
        self.preedit_end_x = preedit_end_x;
    }

    fn place_search_cursor(&mut self, pointer_x: f32, extend: bool) {
        let local = (pointer_x - self.layout.search.x - 32.).max(0.);
        let mut best = (0usize, f32::MAX);
        for &(byte, x) in &self.search_metrics {
            let distance = (x - local).abs();
            if distance < best.1 {
                best = (byte, distance);
            }
        }
        let anchor = if extend {
            self.search.selection().anchor
        } else {
            best.0
        };
        self.search.set_selection(anchor, best.0);
    }

    pub fn ime_cursor_rect(&self) -> Option<Rect> {
        self.ui.focused(SEARCH).then(|| {
            let x = (self.layout.search.x + 32. + self.preedit_cursor_x)
                .min(self.layout.search.x + self.layout.search.width - 6.);
            Rect::new(
                x,
                self.layout.search.y + 7.,
                1.,
                (self.layout.search.height - 12.).max(1.),
            )
        })
    }

    pub fn tick(&mut self, seconds: f32) {
        if self.playing {
            self.current_time = (self.current_time + seconds) % 10.;
        }
    }

    /// Application-owned GPU content is rendered into this region after the
    /// retained GUI pass. It is present only while the Preview page is active.
    pub fn gpu_canvas(&self) -> Option<best_gui::gpu::GpuCanvas> {
        self.layout
            .gpu_canvas
            .map(|bounds| best_gui::gpu::GpuCanvas::new(bounds, bounds))
    }

    pub fn take_preview_window_request(&mut self) -> bool {
        std::mem::take(&mut self.preview_window_requested)
    }

    fn activate(&mut self, target: WidgetId) {
        match target {
            TOP_PLAY => self.playing = !self.playing,
            TOP_ADD => {
                self.selected = (self.selected + 1) % 4;
                self.nodes[self.selected][0] += 12.;
            }
            TOP_EXPORT => self.preview_window_requested = true,
            ZOOM_OUT => self.zoom_at_canvas_center(1. / 1.1),
            ZOOM_IN => self.zoom_at_canvas_center(1.1),
            SNAP => self.snapping = !self.snapping,
            ENABLED => self.enabled = !self.enabled,
            WidgetId(id) if (PANEL_HEADER_BASE..PANEL_HEADER_BASE + 4).contains(&id) => {
                let slot = (id - PANEL_HEADER_BASE) as usize;
                self.open_panel_selector = (self.open_panel_selector != Some(slot)).then_some(slot);
            }
            WidgetId(id) if (PANEL_MENU_BASE..PANEL_MENU_BASE + 32).contains(&id) => {
                let value = id - PANEL_MENU_BASE;
                let slot = (value / 8) as usize;
                let choice = (value % 8) as usize;
                let page = PANEL_PAGES[choice].0;
                let _ = self.panel_choices[slot].activate(page);
                self.open_panel_selector = None;
            }
            WidgetId(id) if (LAYER_BASE..LAYER_BASE + 4).contains(&id) => {
                self.selected = (id - LAYER_BASE) as usize;
            }
            _ => {}
        }
    }

    pub fn paint<'a>(
        &mut self,
        visuals: &mut VisualList<'_>,
        labels: &mut Vec<Label<'a>>,
        connections: &mut Vec<Connection>,
        width: f32,
        height: f32,
    ) -> Result<(), CapacityExceeded> {
        self.ui.begin_frame();
        labels.clear();
        connections.clear();
        let viewport = Rect::new(0., 0., width, height);
        let top_h = 54.;
        let status_h = 24.;
        let status = Rect::new(0., height - status_h, width, status_h);
        let content = Rect::new(0., top_h, width, (height - top_h - status_h).max(0.));
        let mut panel_layouts = [PanelLayout {
            id: PanelId(0),
            bounds: Rect::default(),
        }; 4];
        let mut split_layouts = [SplitLayout::default(); 3];
        let (panel_count, split_count) =
            self.panels
                .layout(content, &mut panel_layouts, &mut split_layouts)?;
        let panel_bounds = |id| {
            panel_layouts[..panel_count]
                .iter()
                .find(|panel| panel.id == id)
                .map(|panel| panel.bounds)
                .unwrap_or_default()
        };
        let slot_bounds = SLOT_PANELS.map(panel_bounds);
        let page_bounds = |page| {
            self.panel_choices
                .iter()
                .position(|choices| choices.active() == page)
                .map(|slot| slot_bounds[slot])
                .unwrap_or_default()
        };
        let left = page_bounds(LAYERS_PAGE);
        let canvas = page_bounds(GRAPH_PAGE);
        let timeline = page_bounds(TIMELINE_PAGE);
        self.layout = Layout {
            left,
            canvas,
            timeline,
            panels: panel_layouts,
            panel_count,
            splits: split_layouts,
            opacity: Rect::default(),
            search: Rect::default(),
            gpu_canvas: None,
        };

        visuals.fill(viewport, BG, 0., viewport)?;
        self.paint_top(visuals, labels, viewport, top_h)?;
        for (slot, panel) in slot_bounds.into_iter().enumerate() {
            let page = self.panel_choices[slot].active();
            let body = Rect::new(
                panel.x,
                panel.y + PANEL_HEADER_HEIGHT,
                panel.width,
                (panel.height - PANEL_HEADER_HEIGHT).max(0.),
            );
            match page {
                LAYERS_PAGE | ASSETS_PAGE => self.paint_left(visuals, labels, body, page)?,
                GRAPH_PAGE | PREVIEW_PAGE => {
                    self.paint_canvas(visuals, labels, connections, body, page)?
                }
                INSPECTOR_PAGE | PROJECT_PAGE => self.paint_right(visuals, labels, body, page)?,
                TIMELINE_PAGE | CONSOLE_PAGE => self.paint_timeline(visuals, labels, body, page)?,
                _ => {}
            }
            paint_panel_selector(
                &mut self.ui,
                visuals,
                labels,
                panel,
                slot,
                page,
                self.open_panel_selector,
            )?;
        }
        self.paint_status(visuals, labels, status)?;

        for (index, split) in split_layouts[..split_count].iter().enumerate() {
            let id = WidgetId(LEFT_SPLITTER.0 + index as u64);
            self.ui.register(id, split.divider, viewport, false)?;
            visuals.fill(
                centered_divider(split.divider, split.axis, 2.),
                if self.ui.hovered(id) || self.ui.active(id) {
                    ACCENT
                } else {
                    BORDER
                },
                0.,
                viewport,
            )?;
        }
        if let Some((_, _, preview)) = self.dock_target {
            visuals.fill(preview, [0.18, 0.12, 0.62, 0.48], 8., viewport)?;
            visuals.outline(preview, ACCENT, [0.; 4], 8., 2., viewport)?;
        }
        self.ui.end_frame();
        Ok(())
    }

    fn paint_top<'a>(
        &mut self,
        v: &mut VisualList<'_>,
        labels: &mut Vec<Label<'a>>,
        viewport: Rect,
        height: f32,
    ) -> Result<(), CapacityExceeded> {
        let bar = Rect::new(0., 0., viewport.width, height);
        v.fill(bar, PANEL, 0., viewport)?;
        v.fill(
            Rect::new(0., height - 1., viewport.width, 1.),
            BORDER,
            0.,
            viewport,
        )?;
        v.fill(Rect::new(88., 17., 20., 20.), ACCENT, 6., viewport)?;
        label(v, labels, "G", 94., 32., 13., [1.; 4], viewport);
        label(v, labels, "GRAPHY STUDIO", 118., 26., 13., TEXT, viewport);
        label(
            v,
            labels,
            "Scene / Product launch",
            118.,
            42.,
            10.,
            MUTED,
            viewport,
        );

        button(
            &mut self.ui,
            v,
            labels,
            TOP_ADD,
            Rect::new(300., 11., 74., 32.),
            ("Add", false),
            viewport,
        )?;
        button(
            &mut self.ui,
            v,
            labels,
            TOP_PLAY,
            Rect::new(384., 11., 82., 32.),
            (if self.playing { "Pause" } else { "Play" }, self.playing),
            viewport,
        )?;
        button(
            &mut self.ui,
            v,
            labels,
            TOP_EXPORT,
            Rect::new(viewport.width - 102., 11., 84., 32.),
            ("3D Window", true),
            viewport,
        )?;
        label(
            v,
            labels,
            "Saved",
            viewport.width - 158.,
            32.,
            11.,
            GREEN,
            viewport,
        );
        Ok(())
    }

    fn paint_left<'a>(
        &mut self,
        v: &mut VisualList<'_>,
        labels: &mut Vec<Label<'a>>,
        panel: Rect,
        active_page: PanelId,
    ) -> Result<(), CapacityExceeded> {
        v.fill(panel, PANEL, 0., panel)?;
        if active_page != LAYERS_PAGE {
            label(
                v,
                labels,
                "PROJECT ASSETS",
                16.,
                panel.y + 68.,
                10.,
                MUTED,
                panel,
            );
            for (i, name) in ["Images", "Videos", "Audio", "Fonts"]
                .into_iter()
                .enumerate()
            {
                let row = Rect::new(
                    panel.x + 10.,
                    panel.y + 82. + i as f32 * 40.,
                    panel.width - 20.,
                    32.,
                );
                v.fill(row, if i == 0 { PANEL_2 } else { PANEL }, 6., panel)?;
                label(v, labels, name, row.x + 12., row.y + 21., 10., TEXT, panel);
            }
            return Ok(());
        }
        let search = Rect::new(panel.x + 12., panel.y + 42., panel.width - 24., 32.);
        self.layout.search = search;
        self.ui.register_semantic(
            SEARCH,
            search,
            panel,
            true,
            Semantics {
                role: Role::TextInput,
                label: "Search layers",
                value: "",
                ..Semantics::default()
            },
        )?;
        let search_color = if self.ui.focused(SEARCH) {
            [0.075, 0.075, 0.075, 1.]
        } else {
            PANEL_2
        };
        v.outline(
            search,
            if self.ui.focused(SEARCH) {
                ACCENT
            } else {
                BORDER
            },
            search_color,
            7.,
            1.,
            panel,
        )?;
        label(
            v,
            labels,
            "Q",
            panel.x + 23.,
            search.y + 21.,
            12.,
            MUTED,
            search,
        );
        if self.search.text().is_empty() && self.search.preedit().is_empty() {
            label(
                v,
                labels,
                "Search layers",
                panel.x + 44.,
                search.y + 21.,
                11.,
                MUTED,
                search,
            );
        } else {
            let selection = self.search.selection().ordered();
            let composing = !self.search.preedit().is_empty();
            let mut displayed = self.search.text().to_owned();
            if composing {
                displayed.replace_range(selection.clone(), self.search.preedit());
            }
            if !selection.is_empty() && !composing {
                let start = self
                    .search_metrics
                    .iter()
                    .find(|(byte, _)| *byte == selection.start)
                    .map_or(0., |(_, x)| *x);
                let end = self
                    .search_metrics
                    .iter()
                    .find(|(byte, _)| *byte == selection.end)
                    .map_or(start, |(_, x)| *x);
                v.fill(
                    Rect::new(
                        panel.x + 44. + start,
                        search.y + 6.,
                        (end - start).max(1.),
                        20.,
                    ),
                    [0.18, 0.12, 0.52, 1.],
                    2.,
                    search,
                )?;
            }
            labels.push(Label {
                text: Cow::Owned(displayed),
                baseline: [panel.x + 44., search.y + 21.],
                size: 11.,
                color: TEXT,
                clip: search,
                visual_index: v.len(),
                center_x: None,
            });
            if composing {
                let start = self
                    .search_metrics
                    .iter()
                    .find(|(byte, _)| *byte == selection.start)
                    .map_or(0., |(_, x)| *x);
                v.fill(
                    Rect::new(
                        panel.x + 44. + start,
                        search.y + 25.,
                        (self.preedit_end_x - start).max(1.),
                        1.,
                    ),
                    TEXT,
                    0.,
                    search,
                )?;
            }
            if self.ui.focused(SEARCH) {
                let caret_offset = if composing {
                    self.preedit_cursor_x
                } else {
                    self.search_metrics
                        .iter()
                        .find(|(byte, _)| *byte == self.search.selection().cursor)
                        .map_or(0., |(_, x)| *x)
                };
                let caret_x = (panel.x + 44. + caret_offset).min(search.x + search.width - 5.);
                v.fill(Rect::new(caret_x, search.y + 7., 1., 18.), TEXT, 0., search)?;
            }
        }

        let names = ["Source image", "Color grade", "Soft shadow", "Final output"];
        let kinds = ["IMG", "FX", "FX", "OUT"];
        let list_clip = Rect::new(panel.x, panel.y + 82., panel.width, panel.height - 82.);
        let query = self.search.text().to_lowercase();
        let mut visible_row = 0usize;
        let visible_indices = virtual_range(4, 45., self.layers_scroll, list_clip.height, 1);
        for i in visible_indices {
            if !query.is_empty()
                && !names[i].to_lowercase().contains(&query)
                && !kinds[i].to_lowercase().contains(&query)
            {
                continue;
            }
            let y = panel.y + 90. + visible_row as f32 * 45. - self.layers_scroll;
            visible_row += 1;
            let row = Rect::new(panel.x + 8., y, panel.width - 16., 38.);
            let id = WidgetId(LAYER_BASE + i as u64);
            self.ui.register(id, row, list_clip, true)?;
            if i == self.selected {
                v.fill(row, [0.07, 0.06, 0.16, 1.], 7., list_clip)?;
                v.fill(
                    Rect::new(panel.x + 8., y + 7., 2., 24.),
                    ACCENT,
                    1.,
                    list_clip,
                )?;
            } else if self.ui.hovered(id) {
                v.fill(row, PANEL_2, 7., list_clip)?;
            }
            v.fill(
                Rect::new(panel.x + 17., y + 8., 30., 22.),
                [0.1, 0.1, 0.1, 1.],
                5.,
                list_clip,
            )?;
            label(
                v,
                labels,
                kinds[i],
                panel.x + 22.,
                y + 23.,
                8.,
                MUTED,
                list_clip,
            );
            label(
                v,
                labels,
                names[i],
                panel.x + 57.,
                y + 18.,
                11.,
                TEXT,
                list_clip,
            );
            label(
                v,
                labels,
                if i == 0 { "1920 x 1080" } else { "Compositing" },
                panel.x + 57.,
                y + 31.,
                8.,
                MUTED,
                list_clip,
            );
        }
        if visible_row == 0 {
            label(
                v,
                labels,
                "No matching layers",
                panel.x + 22.,
                panel.y + 113. - self.layers_scroll,
                10.,
                MUTED,
                list_clip,
            );
        }
        label(
            v,
            labels,
            "ASSETS",
            panel.x + 16.,
            panel.y + 293. - self.layers_scroll,
            10.,
            MUTED,
            list_clip,
        );
        for i in 0..3 {
            let x = panel.x + 14. + i as f32 * 66.;
            let y = panel.y + 307. - self.layers_scroll;
            v.fill(
                Rect::new(x, y, 56., 48.),
                [0.065 + i as f32 * 0.01, 0.075, 0.105, 1.],
                7.,
                list_clip,
            )?;
            v.fill(
                Rect::new(x + 8., y + 8., 40., 24.),
                [0.13, 0.11 + i as f32 * 0.04, 0.22, 1.],
                4.,
                list_clip,
            )?;
        }
        Ok(())
    }

    fn paint_canvas<'a>(
        &mut self,
        v: &mut VisualList<'_>,
        labels: &mut Vec<Label<'a>>,
        connections: &mut Vec<Connection>,
        canvas: Rect,
        active_page: PanelId,
    ) -> Result<(), CapacityExceeded> {
        v.fill(canvas, CANVAS_BG, 0., canvas)?;
        let content = canvas;
        self.ui.register(CANVAS, content, content, false)?;
        if active_page != GRAPH_PAGE {
            let preview = Rect::new(
                content.x + 48.,
                content.y + 30.,
                (content.width - 96.).max(0.),
                (content.height - 60.).max(0.),
            );
            self.layout.gpu_canvas = Some(preview);
            v.fill(preview, [0.01, 0.01, 0.01, 1.], 8., content)?;
            v.outline(preview, BORDER, [0.01, 0.01, 0.01, 1.], 8., 1., content)?;
            return Ok(());
        }
        // Keep the visible grid useful at arbitrarily small scales by moving
        // to coarser world subdivisions instead of imposing a zoom floor.
        let mut grid = 28. * self.view.scale;
        while grid.is_finite() && grid > 0. && grid < 12. {
            grid *= 10.;
        }
        let center = [
            content.x + content.width * 0.5,
            content.y + content.height * 0.5,
        ];
        let origin = [
            center[0] + self.view.offset[0],
            center[1] + self.view.offset[1],
        ];
        let ox = content.x + (origin[0] - content.x).rem_euclid(grid);
        let oy = content.y + (origin[1] - content.y).rem_euclid(grid);
        let mut x = ox;
        while x < content.x + content.width {
            v.fill(
                Rect::new(x, content.y, 1., content.height),
                [0.043, 0.043, 0.043, 1.],
                0.,
                content,
            )?;
            x += grid;
        }
        let mut y = oy;
        while y < content.y + content.height {
            v.fill(
                Rect::new(content.x, y, content.width, 1.),
                [0.043, 0.043, 0.043, 1.],
                0.,
                content,
            )?;
            y += grid;
        }

        let centers: [[f32; 2]; 4] = std::array::from_fn(|i| {
            [
                origin[0] + self.nodes[i][0] * self.view.scale,
                origin[1] + self.nodes[i][1] * self.view.scale,
            ]
        });
        for (a, b, color) in [(0, 1, CYAN), (1, 2, ACCENT), (1, 3, GREEN)] {
            let start = [
                centers[a][0] + 174. * self.view.scale,
                centers[a][1] + 55. * self.view.scale,
            ];
            let end = [centers[b][0], centers[b][1] + 55. * self.view.scale];
            let thickness = 2. * self.view.scale;
            let handle = ((end[0] - start[0]).abs() * 0.5).max(thickness * 20.);
            connections.push(Connection {
                start,
                control_a: [start[0] + handle, start[1]],
                control_b: [end[0] - handle, end[1]],
                end,
                color,
                thickness,
                clip: content,
                visual_index: v.len(),
            });
        }

        let titles = [
            "Image input 🖼️",
            "Color grade 🎨",
            "Soft shadow 🌘",
            "Composite ✨",
        ];
        let subtitles = ["product.png", "Contrast +12", "Blur 24 px", "Screen"];
        let colors = [CYAN, ACCENT, ORANGE, GREEN];
        for i in 0..4 {
            let bounds = Rect::new(
                centers[i][0],
                centers[i][1],
                174. * self.view.scale,
                104. * self.view.scale,
            );
            let id = WidgetId(NODE_BASE + i as u64);
            self.ui.register(id, bounds, content, true)?;
            v.fill(
                Rect::new(
                    bounds.x + 3. * self.view.scale,
                    bounds.y + 5. * self.view.scale,
                    bounds.width,
                    bounds.height,
                ),
                [0.009, 0.009, 0.009, 0.72],
                11. * self.view.scale,
                content,
            )?;
            let body = if i == self.selected {
                [0.06, 0.06, 0.06, 1.]
            } else {
                PANEL_2
            };
            v.outline(
                bounds,
                if i == self.selected { ACCENT } else { BORDER },
                body,
                10. * self.view.scale,
                if i == self.selected { 2. } else { 1. } * self.view.scale,
                content,
            )?;
            v.fill(
                Rect::new(
                    bounds.x + 1.,
                    bounds.y + 1.,
                    bounds.width - 2.,
                    31. * self.view.scale,
                ),
                [
                    colors[i][0] * 0.18,
                    colors[i][1] * 0.18,
                    colors[i][2] * 0.18,
                    1.,
                ],
                9. * self.view.scale,
                content,
            )?;
            v.fill(
                Rect::new(
                    bounds.x + 12. * self.view.scale,
                    bounds.y + 13. * self.view.scale,
                    7. * self.view.scale,
                    7. * self.view.scale,
                ),
                colors[i],
                4. * self.view.scale,
                content,
            )?;
            v.fill(
                Rect::new(
                    bounds.x - 5. * self.view.scale,
                    bounds.y + 50. * self.view.scale,
                    10. * self.view.scale,
                    10. * self.view.scale,
                ),
                colors[i],
                5. * self.view.scale,
                content,
            )?;
            v.fill(
                Rect::new(
                    bounds.x + bounds.width - 5. * self.view.scale,
                    bounds.y + 50. * self.view.scale,
                    10. * self.view.scale,
                    10. * self.view.scale,
                ),
                colors[i],
                5. * self.view.scale,
                content,
            )?;
            label(
                v,
                labels,
                titles[i],
                bounds.x + 27. * self.view.scale,
                bounds.y + 22. * self.view.scale,
                11. * self.view.scale,
                TEXT,
                content,
            );
            label(
                v,
                labels,
                subtitles[i],
                bounds.x + 13. * self.view.scale,
                bounds.y + 58. * self.view.scale,
                9. * self.view.scale,
                MUTED,
                content,
            );
            label(
                v,
                labels,
                if i < 3 { "Output" } else { "Preview" },
                bounds.x + 13. * self.view.scale,
                bounds.y + 84. * self.view.scale,
                8. * self.view.scale,
                colors[i],
                content,
            );
        }

        let controls = Rect::new(canvas.x + 12., canvas.y + canvas.height - 42., 190., 30.);
        v.fill(controls, [0.034, 0.034, 0.034, 0.96], 8., content)?;
        button(
            &mut self.ui,
            v,
            labels,
            ZOOM_OUT,
            Rect::new(controls.x + 5., controls.y + 4., 24., 22.),
            ("-", false),
            content,
        )?;
        button(
            &mut self.ui,
            v,
            labels,
            ZOOM_IN,
            Rect::new(controls.x + 64., controls.y + 4., 24., 22.),
            ("+", false),
            content,
        )?;
        labels.push(Label {
            text: Cow::Owned(format!("{:.0}%", self.view.scale * 100.)),
            baseline: [controls.x + 33., controls.y + 19.],
            size: 9.,
            color: TEXT,
            clip: content,
            visual_index: v.len(),
            center_x: None,
        });
        button(
            &mut self.ui,
            v,
            labels,
            SNAP,
            Rect::new(controls.x + 99., controls.y + 4., 84., 22.),
            ("Snap", self.snapping),
            content,
        )?;
        Ok(())
    }

    fn paint_right<'a>(
        &mut self,
        v: &mut VisualList<'_>,
        labels: &mut Vec<Label<'a>>,
        panel: Rect,
        active_page: PanelId,
    ) -> Result<(), CapacityExceeded> {
        v.fill(panel, PANEL, 0., panel)?;
        if active_page != INSPECTOR_PAGE {
            label(
                v,
                labels,
                "PROJECT FILES",
                panel.x + 16.,
                panel.y + 68.,
                11.,
                MUTED,
                panel,
            );
            for (index, name) in ["Assets", "Compositions", "Exports"]
                .into_iter()
                .enumerate()
            {
                let row = Rect::new(
                    panel.x + 10.,
                    panel.y + 82. + index as f32 * 42.,
                    panel.width - 20.,
                    34.,
                );
                v.fill(row, if index == 0 { PANEL_2 } else { PANEL }, 6., panel)?;
                label(v, labels, name, row.x + 14., row.y + 22., 11., TEXT, panel);
            }
            return Ok(());
        }
        label(
            v,
            labels,
            ["IMAGE INPUT", "COLOR GRADE", "SOFT SHADOW", "COMPOSITE"][self.selected],
            panel.x + 16.,
            panel.y + 56.,
            15.,
            TEXT,
            panel,
        );
        v.fill(
            Rect::new(panel.x + 16., panel.y + 69., panel.width - 32., 1.),
            BORDER,
            0.,
            panel,
        )?;
        label(
            v,
            labels,
            "TRANSFORM",
            panel.x + 16.,
            panel.y + 96.,
            10.,
            MUTED,
            panel,
        );
        field(
            v,
            labels,
            Rect::new(
                panel.x + 16.,
                panel.y + 108.,
                (panel.width - 42.) * 0.5,
                32.,
            ),
            "X",
            self.nodes[self.selected][0],
            panel,
        )?;
        field(
            v,
            labels,
            Rect::new(
                panel.x + 26. + (panel.width - 42.) * 0.5,
                panel.y + 108.,
                (panel.width - 42.) * 0.5,
                32.,
            ),
            "Y",
            self.nodes[self.selected][1],
            panel,
        )?;
        label(
            v,
            labels,
            "APPEARANCE",
            panel.x + 16.,
            panel.y + 169.,
            10.,
            MUTED,
            panel,
        );
        label(
            v,
            labels,
            "Opacity",
            panel.x + 16.,
            panel.y + 198.,
            11.,
            TEXT,
            panel,
        );
        labels.push(Label {
            text: Cow::Owned(format!("{:.0}%", self.opacity * 100.)),
            baseline: [panel.x + panel.width - 48., panel.y + 198.],
            size: 10.,
            color: TEXT,
            clip: panel,
            visual_index: v.len(),
            center_x: None,
        });
        let slider = Rect::new(panel.x + 16., panel.y + 211., panel.width - 32., 18.);
        self.layout.opacity = slider;
        self.ui.register(OPACITY, slider, panel, true)?;
        v.fill(
            Rect::new(slider.x, slider.y + 7., slider.width, 4.),
            BORDER,
            2.,
            panel,
        )?;
        v.fill(
            Rect::new(slider.x, slider.y + 7., slider.width * self.opacity, 4.),
            ACCENT,
            2.,
            panel,
        )?;
        v.fill(
            Rect::new(
                slider.x + slider.width * self.opacity - 6.,
                slider.y + 2.,
                12.,
                14.,
            ),
            [0.72, 0.66, 1., 1.],
            6.,
            panel,
        )?;
        label(
            v,
            labels,
            "Enabled",
            panel.x + 16.,
            panel.y + 260.,
            11.,
            TEXT,
            panel,
        );
        let toggle = Rect::new(panel.x + panel.width - 52., panel.y + 245., 36., 20.);
        self.ui.register(ENABLED, toggle, panel, true)?;
        v.fill(
            toggle,
            if self.enabled { ACCENT } else { BORDER },
            10.,
            panel,
        )?;
        v.fill(
            Rect::new(
                toggle.x + if self.enabled { 18. } else { 2. },
                toggle.y + 2.,
                16.,
                16.,
            ),
            [0.88, 0.88, 0.88, 1.],
            8.,
            panel,
        )?;
        v.fill(
            Rect::new(panel.x + 16., panel.y + 282., panel.width - 32., 1.),
            BORDER,
            0.,
            panel,
        )?;
        label(
            v,
            labels,
            "COLOR",
            panel.x + 16.,
            panel.y + 308.,
            10.,
            MUTED,
            panel,
        );
        for (i, color) in [ACCENT, CYAN, GREEN, ORANGE, [0.82, 0.15, 0.42, 1.]]
            .into_iter()
            .enumerate()
        {
            v.fill(
                Rect::new(panel.x + 16. + i as f32 * 39., panel.y + 323., 30., 30.),
                color,
                7.,
                panel,
            )?;
        }
        label(
            v,
            labels,
            "Blend mode",
            panel.x + 16.,
            panel.y + 388.,
            11.,
            TEXT,
            panel,
        );
        let field = Rect::new(panel.x + 16., panel.y + 400., panel.width - 32., 34.);
        v.outline(field, BORDER, PANEL_2, 7., 1., panel)?;
        label(
            v,
            labels,
            "Normal",
            field.x + 11.,
            field.y + 22.,
            10.,
            TEXT,
            field,
        );
        Ok(())
    }

    fn paint_timeline<'a>(
        &mut self,
        v: &mut VisualList<'_>,
        labels: &mut Vec<Label<'a>>,
        panel: Rect,
        active_page: PanelId,
    ) -> Result<(), CapacityExceeded> {
        v.fill(panel, PANEL, 0., panel)?;
        if active_page != TIMELINE_PAGE {
            for (i, line) in ["Best GUI Studio ready", "Renderer: wgpu", "No errors"]
                .into_iter()
                .enumerate()
            {
                label(
                    v,
                    labels,
                    line,
                    panel.x + 16.,
                    panel.y + 68. + i as f32 * 22.,
                    10.,
                    if i == 2 { GREEN } else { TEXT },
                    panel,
                );
            }
            return Ok(());
        }
        label(
            v,
            labels,
            if self.playing { "PAUSE" } else { "PLAY" },
            panel.x + 106.,
            panel.y + 47.,
            10.,
            if self.playing { GREEN } else { TEXT },
            panel,
        );
        labels.push(Label {
            text: Cow::Owned(format!(
                "00:{:02.0}:{:02.0}",
                self.current_time.floor(),
                (self.current_time.fract() * 60.).floor()
            )),
            baseline: [panel.x + panel.width - 88., panel.y + 47.],
            size: 10.,
            color: TEXT,
            clip: panel,
            visual_index: v.len(),
            center_x: None,
        });
        let ruler_y = panel.y + 57.;
        v.fill(
            Rect::new(panel.x, ruler_y, panel.width, 25.),
            [0.032, 0.032, 0.032, 1.],
            0.,
            panel,
        )?;
        v.fill(
            Rect::new(panel.x + 92., ruler_y, 1., panel.height - 57.),
            BORDER,
            0.,
            panel,
        )?;
        let usable = (panel.width - 100.).max(100.);
        let pixels_per_second = (usable / 10.).min(72.);
        for second in 0..11 {
            let x = panel.x + 94. + second as f32 * pixels_per_second;
            v.fill(Rect::new(x, ruler_y + 15., 1., 10.), BORDER, 0., panel)?;
            labels.push(Label {
                text: Cow::Owned(second.to_string()),
                baseline: [x + 4., ruler_y + 14.],
                size: 8.,
                color: MUTED,
                clip: panel,
                visual_index: v.len(),
                center_x: None,
            });
        }
        let tracks = ["Image", "Grade", "Shadow"];
        for (i, name) in tracks.into_iter().enumerate() {
            let y = ruler_y + 25. + i as f32 * 34.;
            if i == self.selected.min(2) {
                v.fill(
                    Rect::new(panel.x, y, panel.width, 34.),
                    [0.052, 0.052, 0.052, 1.],
                    0.,
                    panel,
                )?;
            }
            label(
                v,
                labels,
                name,
                panel.x + 14.,
                y + 21.,
                9.,
                if i == self.selected.min(2) {
                    TEXT
                } else {
                    MUTED
                },
                panel,
            );
            v.fill(
                Rect::new(panel.x + 93., y + 33., panel.width - 93., 1.),
                [0.065, 0.065, 0.065, 1.],
                0.,
                panel,
            )?;
            for time in [0.7 + i as f32 * 0.4, 2.4, 5.1 + i as f32 * 0.5, 8.2] {
                let x = panel.x + 94. + time * pixels_per_second;
                v.fill(
                    Rect::new(x - 4., y + 13., 8., 8.),
                    if i == 1 { ACCENT } else { CYAN },
                    2.,
                    panel,
                )?;
            }
        }
        let x = panel.x + 94. + self.current_time * pixels_per_second;
        let playhead = Rect::new(x - 6., ruler_y, 12., panel.height - 57.);
        self.ui.register(PLAYHEAD, playhead, panel, true)?;
        v.fill(
            Rect::new(x - 1., ruler_y, 2., panel.height - 57.),
            ORANGE,
            1.,
            panel,
        )?;
        v.fill(Rect::new(x - 5., ruler_y + 1., 10., 10.), ORANGE, 3., panel)?;
        Ok(())
    }

    fn paint_status<'a>(
        &mut self,
        v: &mut VisualList<'_>,
        labels: &mut Vec<Label<'a>>,
        status: Rect,
    ) -> Result<(), CapacityExceeded> {
        v.fill(status, [0.029, 0.029, 0.029, 1.], 0., status)?;
        v.fill(
            Rect::new(0., status.y, status.width, 1.),
            BORDER,
            0.,
            status,
        )?;
        v.fill(Rect::new(12., status.y + 8., 7., 7.), GREEN, 4., status)?;
        label(
            v,
            labels,
            "GPU ready",
            26.,
            status.y + 16.,
            9.,
            MUTED,
            status,
        );
        labels.push(Label {
            text: Cow::Owned(format!(
                "{}%  |  {} cached labels",
                (self.view.scale * 100.) as u32,
                31 + self.search.text().len()
            )),
            baseline: [status.width - 178., status.y + 16.],
            size: 9.,
            color: MUTED,
            clip: status,
            visual_index: v.len(),
            center_x: None,
        });
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn label<'a>(
    visuals: &VisualList<'_>,
    labels: &mut Vec<Label<'a>>,
    text: &'static str,
    x: f32,
    y: f32,
    size: f32,
    color: [f32; 4],
    clip: Rect,
) {
    labels.push(Label {
        text: Cow::Borrowed(text),
        baseline: [x, y],
        size,
        color,
        clip,
        visual_index: visuals.len(),
        center_x: None,
    });
}

fn button<'a>(
    ui: &mut UiRuntime,
    v: &mut VisualList<'_>,
    labels: &mut Vec<Label<'a>>,
    id: WidgetId,
    bounds: Rect,
    content: (&'static str, bool),
    clip: Rect,
) -> Result<(), CapacityExceeded> {
    let (text, selected) = content;
    ui.register_semantic(
        id,
        bounds,
        clip,
        true,
        Semantics {
            role: Role::Button,
            label: text,
            selected,
            ..Semantics::default()
        },
    )?;
    let color = if selected || ui.active(id) {
        ACCENT
    } else if ui.hovered(id) {
        [0.086, 0.086, 0.086, 1.]
    } else {
        PANEL_2
    };
    v.outline(
        bounds,
        if ui.focused(id) { ACCENT } else { BORDER },
        color,
        7.,
        1.,
        clip,
    )?;
    labels.push(Label {
        text: Cow::Borrowed(text),
        baseline: [
            bounds.x + bounds.width * 0.5,
            bounds.y + bounds.height * 0.66 - 1.,
        ],
        size: 10.,
        color: TEXT,
        clip,
        visual_index: v.len(),
        center_x: Some(bounds.x + bounds.width * 0.5),
    });
    Ok(())
}

fn field<'a>(
    v: &mut VisualList<'_>,
    labels: &mut Vec<Label<'a>>,
    bounds: Rect,
    name: &'static str,
    value: f32,
    clip: Rect,
) -> Result<(), CapacityExceeded> {
    v.outline(bounds, BORDER, PANEL_2, 6., 1., clip)?;
    label(
        v,
        labels,
        name,
        bounds.x + 9.,
        bounds.y + 20.,
        9.,
        MUTED,
        bounds,
    );
    labels.push(Label {
        text: Cow::Owned(format!("{value:.0}")),
        baseline: [bounds.x + 29., bounds.y + 21.],
        size: 10.,
        color: TEXT,
        clip: bounds,
        visual_index: v.len(),
        center_x: None,
    });
    Ok(())
}

fn panel_choices(initial: PanelId) -> PanelTabs<8> {
    let mut choices = PanelTabs::<8>::new(PANEL_PAGES[0].0).unwrap();
    for &(page, _) in &PANEL_PAGES[1..] {
        choices.add(page).unwrap();
    }
    choices.activate(initial).unwrap();
    choices
}

fn panel_name(page: PanelId) -> &'static str {
    PANEL_PAGES
        .iter()
        .find(|(id, _)| *id == page)
        .map(|(_, name)| *name)
        .unwrap_or("PANEL")
}

fn paint_panel_selector<'a>(
    ui: &mut UiRuntime,
    v: &mut VisualList<'_>,
    labels: &mut Vec<Label<'a>>,
    panel: Rect,
    slot: usize,
    active_page: PanelId,
    open: Option<usize>,
) -> Result<(), CapacityExceeded> {
    let header = Rect::new(panel.x, panel.y, panel.width, 28.);
    let selector_width = panel.width.min(96.);
    let selector = Rect::new(header.x, header.y, selector_width, header.height);
    let header_id = WidgetId(PANEL_HEADER_BASE + slot as u64);
    ui.register_semantic(
        header_id,
        selector,
        panel,
        true,
        Semantics {
            role: Role::Button,
            label: panel_name(active_page),
            expanded: open == Some(slot),
            ..Semantics::default()
        },
    )?;
    v.fill(header, PANEL_HEADER, 0., panel)?;
    if ui.hovered(header_id) || open == Some(slot) {
        v.fill(selector, PANEL_HEADER_HOVER, 0., panel)?;
    }
    v.fill(
        Rect::new(header.x, header.y, 2., header.height),
        [0.28, 0.28, 0.28, 1.],
        0.,
        panel,
    )?;
    v.fill(
        Rect::new(header.x, header.y + header.height - 1., header.width, 1.),
        BORDER,
        0.,
        panel,
    )?;
    v.fill(
        Rect::new(header.x + 10., header.y + 11., 6., 6.),
        [0.72, 0.72, 0.72, 1.],
        3.,
        panel,
    )?;
    label(
        v,
        labels,
        panel_name(active_page),
        header.x + 22.,
        header.y + 18.,
        9.,
        TEXT,
        panel,
    );
    label(
        v,
        labels,
        if open == Some(slot) { "^" } else { "v" },
        selector.x + selector.width - 15.,
        selector.y + 18.,
        9.,
        if open == Some(slot) { TEXT } else { MUTED },
        panel,
    );
    let mut menu_x = selector.x + selector.width + 5.;
    for (index, name) in ["View", "Select", "Add"].into_iter().enumerate() {
        let width = if name == "Select" { 39. } else { 31. };
        if menu_x + width > header.x + header.width - 3. {
            break;
        }
        let action = Rect::new(menu_x, header.y + 3., width, 22.);
        let action_id = WidgetId(PANEL_ACTION_BASE + slot as u64 * 3 + index as u64);
        ui.register_semantic(
            action_id,
            action,
            panel,
            true,
            Semantics {
                role: Role::Button,
                label: name,
                ..Semantics::default()
            },
        )?;
        if ui.hovered(action_id) || ui.active(action_id) {
            v.outline(
                action,
                [0.32, 0.32, 0.32, 1.],
                [0.14, 0.14, 0.14, 1.],
                4.,
                1.,
                panel,
            )?;
        }
        label(
            v,
            labels,
            name,
            action.x + 5.,
            action.y + 15.,
            9.,
            [0.96, 0.96, 0.96, 1.],
            panel,
        );
        menu_x += width + 2.;
    }

    if open == Some(slot) {
        let menu = Rect::new(
            selector.x + 4.,
            header.y + header.height + 2.,
            (panel.width - 8.).clamp(0., 180.),
            8. * 27. + 8.,
        );
        v.outline(menu, BORDER, [0.032, 0.032, 0.032, 1.], 7., 1., panel)?;
        for (index, &(page, name)) in PANEL_PAGES.iter().enumerate() {
            let id = WidgetId(PANEL_MENU_BASE + slot as u64 * 8 + index as u64);
            let row = Rect::new(
                menu.x + 4.,
                menu.y + 4. + index as f32 * 27.,
                menu.width - 8.,
                27.,
            );
            ui.register_semantic(
                id,
                row,
                panel,
                true,
                Semantics {
                    role: Role::Button,
                    label: name,
                    selected: page == active_page,
                    ..Semantics::default()
                },
            )?;
            if page == active_page || ui.hovered(id) {
                v.fill(
                    row,
                    if page == active_page {
                        [0.22, 0.22, 0.22, 1.]
                    } else {
                        PANEL_2
                    },
                    5.,
                    menu,
                )?;
            }
            label(v, labels, name, row.x + 10., row.y + 18., 10., TEXT, menu);
        }
    }
    Ok(())
}

fn centered_divider(bounds: Rect, axis: Axis, width: f32) -> Rect {
    match axis {
        Axis::Horizontal => Rect::new(
            bounds.x + (bounds.width - width) * 0.5,
            bounds.y,
            width,
            bounds.height,
        ),
        Axis::Vertical => Rect::new(
            bounds.x,
            bounds.y + (bounds.height - width) * 0.5,
            bounds.width,
            width,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use best_gui::visual::Visual;

    fn paint(studio: &mut Studio) -> (Vec<Visual>, Vec<Label<'static>>) {
        let mut storage = vec![Visual::default(); 4096];
        let mut labels = Vec::new();
        let mut connections = Vec::new();
        let visual_count = {
            let mut visuals = VisualList::new(&mut storage);
            studio
                .paint(&mut visuals, &mut labels, &mut connections, 1280., 768.)
                .unwrap();
            visuals.len()
        };
        storage.truncate(visual_count);
        (storage, labels)
    }

    #[test]
    fn labels_retain_their_position_in_painters_order() {
        let mut studio = Studio::default();
        let (visuals, labels) = paint(&mut studio);
        assert!(
            labels
                .windows(2)
                .all(|pair| pair[0].visual_index <= pair[1].visual_index)
        );
        assert!(
            labels
                .iter()
                .all(|label| label.visual_index <= visuals.len())
        );
    }

    #[test]
    fn graph_connections_are_emitted_as_cubic_windfoil_paths() {
        let mut studio = Studio::default();
        let mut storage = vec![Visual::default(); 4096];
        let mut labels = Vec::new();
        let mut connections = Vec::new();
        let mut visuals = VisualList::new(&mut storage);
        studio
            .paint(&mut visuals, &mut labels, &mut connections, 1280., 768.)
            .unwrap();
        assert_eq!(connections.len(), 3);
        assert!(connections.iter().all(|connection| {
            connection.thickness > 0.
                && connection.control_a[0] > connection.start[0]
                && connection.control_b[0] < connection.end[0]
                && connection.visual_index <= visuals.len()
        }));
    }

    #[test]
    fn search_accepts_preedit_and_deletes_a_whole_grapheme() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        studio.pointer_down([50., 138.]);
        assert!(studio.ui.focused(SEARCH));

        studio.set_preedit(String::from("入力"), Some((0, 6)));
        let (_, labels) = paint(&mut studio);
        assert!(labels.iter().any(|label| label.text == "入力"));

        studio.commit_text("e\u{301}");
        assert_eq!(studio.search.text(), "e\u{301}");
        studio.keyboard("Backspace");
        assert!(studio.search.text().is_empty());
    }

    #[test]
    fn committed_search_text_filters_layer_rows() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        studio.pointer_down([50., 138.]);
        studio.commit_text("shadow");
        let (_, labels) = paint(&mut studio);
        assert!(labels.iter().any(|label| label.text == "Soft shadow 🌘"));
        assert!(
            !labels
                .iter()
                .any(|label| label.text == "Source image" && label.baseline[0] < 238.)
        );
        assert!(
            !labels
                .iter()
                .any(|label| label.text == "Color grade 🎨" && label.baseline[0] < 238.)
        );
    }

    #[test]
    fn search_click_drag_and_shift_arrows_update_selection() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        studio.pointer_down([50., 138.]);
        studio.commit_text("abcd");
        studio.set_search_boundaries(vec![(0, 0.), (1, 7.), (2, 15.), (3, 22.), (4, 30.)]);
        studio.set_preedit_metrics(30., 30.);
        studio.pointer_up([50., 138.]);

        studio.pointer_down([51., 138.]);
        studio.pointer_move([72., 138.]);
        studio.pointer_up([72., 138.]);
        assert_eq!(studio.selected_search_text(), Some("bcd"));

        studio.move_search_cursor(false, false);
        studio.move_search_cursor(false, true);
        assert_eq!(studio.selected_search_text(), Some("a"));
    }

    #[test]
    fn ime_preedit_visually_replaces_selection_and_commit_matches() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        studio.pointer_down([50., 138.]);
        studio.commit_text("abc");
        studio.search.set_selection(1, 2);
        studio.set_search_boundaries(vec![(0, 0.), (1, 7.), (2, 14.), (3, 21.)]);
        studio.set_preedit_metrics(19., 31.);
        studio.set_preedit(String::from("入力"), Some((0, 3)));

        let (visuals, labels) = paint(&mut studio);
        assert!(labels.iter().any(|label| label.text == "a入力c"));
        assert!(!labels.iter().any(|label| label.text == "ab入力c"));
        assert!(visuals.iter().any(|visual| {
            visual.bounds == Rect::new(51., 149., 24., 1.) && visual.color == TEXT
        }));

        studio.commit_text("入力");
        assert_eq!(studio.search.text(), "a入力c");
    }

    #[test]
    fn canvas_zoom_keeps_the_world_point_under_the_pointer() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        let pointer = [820., 210.];
        studio.pointer_move(pointer);
        let before = studio.view.screen_to_world(studio.layout.canvas, pointer);

        studio.pinch(0.18);

        let after = studio.view.screen_to_world(studio.layout.canvas, pointer);
        assert!((before[0] - after[0]).abs() < 0.0001);
        assert!((before[1] - after[1]).abs() < 0.0001);
        assert!(studio.view.scale > 1.);
    }

    #[test]
    fn canvas_world_origin_tracks_its_center_and_pan() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        let center = Viewport2D::center(studio.layout.canvas);
        assert_eq!(studio.view.offset, [0., 0.]);
        assert_eq!(center, Viewport2D::center(studio.layout.canvas));

        let empty_canvas_point = [700., 450.];
        studio.pointer_down(empty_canvas_point);
        studio.pointer_move([empty_canvas_point[0] + 25., empty_canvas_point[1] - 10.]);
        studio.pointer_up([empty_canvas_point[0] + 25., empty_canvas_point[1] - 10.]);
        assert_eq!(studio.view.offset, [25., -10.]);
    }

    #[test]
    fn panel_tree_drives_studio_splitter_dragging() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        let before = studio.layout.left.width;
        let divider_x = studio.layout.left.x + studio.layout.left.width + 3.;

        studio.pointer_move([divider_x, 200.]);
        studio.pointer_down([divider_x, 200.]);
        studio.pointer_move([divider_x + 42., 200.]);
        studio.pointer_up([divider_x + 42., 200.]);
        let _ = paint(&mut studio);

        assert!(studio.layout.left.width > before + 40.);
        assert!(studio.layout.canvas.width < 750.);
    }

    #[test]
    fn panel_header_menu_switches_visible_content() {
        let mut studio = Studio::default();
        let (_, labels) = paint(&mut studio);
        assert!(labels.iter().any(|label| label.text == "TRANSFORM"));
        let inspector = labels
            .iter()
            .find(|label| label.text == "INSPECTOR")
            .expect("inspector header is visible");
        let header_point = [inspector.baseline[0] + 15., inspector.baseline[1] - 10.];
        studio.pointer_move(header_point);
        studio.pointer_down(header_point);
        studio.pointer_up(header_point);
        let (_, labels) = paint(&mut studio);
        let project = labels
            .iter()
            .find(|label| label.text == "PROJECT")
            .expect("project choice is visible");
        let point = [project.baseline[0] + 15., project.baseline[1] - 10.];

        studio.pointer_move(point);
        studio.pointer_down(point);
        studio.pointer_up(point);
        let (_, labels) = paint(&mut studio);

        assert!(labels.iter().any(|label| label.text == "PROJECT FILES"));
        assert!(!labels.iter().any(|label| label.text == "TRANSFORM"));
    }

    #[test]
    fn selecting_timeline_in_another_area_renders_real_timeline_content() {
        let mut studio = Studio::default();
        studio.panel_choices[0].activate(TIMELINE_PAGE).unwrap();
        let (_, labels) = paint(&mut studio);

        assert!(labels.iter().any(|label| label.text == "TIMELINE"));
        assert!(labels.iter().any(|label| label.text == "Image"));
        assert!(
            !labels
                .iter()
                .any(|label| label.text == "Panel moved to this area")
        );
    }

    #[test]
    fn dragging_a_header_docks_its_panel_at_a_target_edge() {
        let mut studio = Studio::default();
        let (_, labels) = paint(&mut studio);
        let layers = labels
            .iter()
            .find(|label| label.text == "LAYERS")
            .expect("layers header is visible");
        let start = [layers.baseline[0] + 12., layers.baseline[1] - 10.];
        let target = studio.layout.panels[..studio.layout.panel_count]
            .iter()
            .find(|panel| panel.id == RIGHT_PANEL)
            .unwrap()
            .bounds;
        let drop = [target.x + target.width - 2., target.y + target.height * 0.5];

        studio.pointer_move(start);
        studio.pointer_down(start);
        studio.pointer_move(drop);
        assert!(studio.dock_target.is_some());
        studio.pointer_up(drop);
        let (_, labels) = paint(&mut studio);

        let moved = studio.layout.panels[..studio.layout.panel_count]
            .iter()
            .find(|panel| panel.id == LEFT_PANEL)
            .unwrap()
            .bounds;
        assert!(moved.x > target.x);
        let layer_label = labels
            .iter()
            .find(|label| label.text == "Source image")
            .expect("docked layers content is visible");
        assert!(moved.contains(layer_label.baseline[0], layer_label.baseline[1]));
        assert!(
            studio
                .ui
                .semantics_for(WidgetId(PANEL_HEADER_BASE + 3))
                .is_some()
        );
    }

    #[test]
    fn preview_starts_below_the_shared_header_with_equal_vertical_margins() {
        let mut studio = Studio::default();
        studio.panel_choices[1].activate(PREVIEW_PAGE).unwrap();
        let (visuals, _) = paint(&mut studio);
        let panel = studio.layout.panels[..studio.layout.panel_count]
            .iter()
            .find(|panel| panel.id == CANVAS_PANEL)
            .unwrap()
            .bounds;
        let body_y = panel.y + PANEL_HEADER_HEIGHT;
        let body_bottom = panel.y + panel.height;
        let preview = visuals
            .iter()
            .find(|visual| visual.color == [0.01, 0.01, 0.01, 1.])
            .expect("preview surface is visible")
            .bounds;

        assert_eq!(preview.y - body_y, 30.);
        assert_eq!(body_bottom - (preview.y + preview.height), 30.);
        assert!(preview.y >= body_y);
    }

    #[test]
    fn trackpad_pan_is_two_axis_and_pinch_has_no_ui_zoom_limits() {
        let mut studio = Studio::default();
        let _ = paint(&mut studio);
        studio.pointer_move([700., 450.]);

        studio.scroll([31., -17.]);
        assert_eq!(studio.view.offset, [31., -17.]);

        studio.pinch(3.);
        assert!(studio.view.scale > 1.8);
        studio.pinch(-8.);
        assert!(studio.view.scale < 0.55);
    }
}
