use glyphon::{Attrs, Buffer as EmojiBuffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use lagui::{
    gpu::VisualRenderer,
    text::{TextCache, TextRenderer, TextSection},
    visual::{Visual, VisualList},
    windfoil::{
        Atlas as WindfoilAtlas, Instance as WindfoilInstance, Path as WindfoilPath,
        Renderer as WindfoilRenderer,
    },
};
use std::{
    error::Error,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;
#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize, PhysicalSize},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowId},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct EditMetrics {
    boundaries: Option<Vec<(usize, f32)>>,
    ime_cursor_x: f32,
    preedit_end_x: f32,
}

#[path = "support/gpu_preview.rs"]
mod gpu_preview;
#[path = "support/studio_scene.rs"]
mod studio_scene;
#[path = "support/window_chrome.rs"]
mod window_chrome;
use lagui::gpu::emoji as fixed_emoji;

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    visuals: VisualRenderer,
    text: TextRenderer,
    connections: WindfoilRenderer,
    preview: gpu_preview::Renderer,
    cache: TextCache,
    emoji_font_system: FontSystem,
    emoji_swash_cache: SwashCache,
    emoji_renderer: fixed_emoji::Renderer,
    emoji_quads: Vec<fixed_emoji::EmojiQuad>,
    emoji_visual_indices: Vec<usize>,
    label_glyph_ranges: Vec<Range<u32>>,
    label_visual_indices: Vec<usize>,
    connection_visual_index: Option<usize>,
    transparent: bool,
    edit_metric_text: String,
    edit_metric_points: Vec<(usize, f32)>,
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: if cfg!(target_os = "macos") {
                wgpu::Backends::METAL
            } else if cfg!(target_os = "windows") {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::VULKAN
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Graphy Studio device"),
                ..Default::default()
            })
            .await?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .ok_or("No sRGB surface format")?;
        let size = window.inner_size();
        let transparent = cfg!(target_os = "macos");
        let alpha_mode = if transparent {
            caps.alpha_modes
                .iter()
                .copied()
                .find(|mode| *mode == wgpu::CompositeAlphaMode::PostMultiplied)
                .ok_or("Metal surface does not support transparency")?
        } else {
            caps.alpha_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            // A native GUI should follow the compositor clock by default. In
            // particular, macOS live-resize is driven by Core Animation; an
            // immediate swapchain produces visibly uneven resize frames even
            // when presentsWithTransaction is enabled.
            present_mode: if std::env::var("LAGUI_VSYNC").is_ok_and(|value| value == "0") {
                wgpu::PresentMode::AutoNoVsync
            } else {
                wgpu::PresentMode::Fifo
            },
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let visuals =
            VisualRenderer::new(&device, format, 4096).map_err(|_| "visual renderer capacity")?;
        // Text and connections share one compiled Windfoil pipeline.
        let windfoil = lagui::windfoil::Pipeline::new(&device, format);
        let text = TextRenderer::with_pipeline(&device, &windfoil, 4096)?;
        let connections = WindfoilRenderer::with_pipeline(&device, &windfoil, 64)?;
        let preview = gpu_preview::Renderer::new(&device, format, [size.width, size.height]);
        let cache = TextCache::new(lagui::text::load_system_font()?);
        let mut emoji_font_system = FontSystem::new();
        // Force system font discovery before the first interactive input.
        emoji_font_system.db_mut().load_system_fonts();
        let emoji_swash_cache = SwashCache::new();
        let emoji_renderer = fixed_emoji::Renderer::new(&device, format);
        println!(
            "Graphy Studio: {:?} / {} / {:?} / {:?}",
            adapter.get_info().backend,
            adapter.get_info().name,
            config.present_mode,
            config.alpha_mode
        );
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            visuals,
            text,
            connections,
            preview,
            cache,
            emoji_font_system,
            emoji_swash_cache,
            emoji_renderer,
            emoji_quads: Vec::with_capacity(16),
            emoji_visual_indices: Vec::with_capacity(16),
            label_glyph_ranges: Vec::with_capacity(256),
            label_visual_indices: Vec::with_capacity(256),
            connection_visual_index: None,
            transparent,
            edit_metric_text: String::new(),
            edit_metric_points: vec![(0, 0.)],
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        if self.config.width == size.width && self.config.height == size.height {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.preview.resize(&self.device, [size.width, size.height]);
    }

    fn prepare_text(
        &mut self,
        labels: &[studio_scene::Label<'_>],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<()> {
        struct Segment {
            label: usize,
            text: String,
            windfoil: bool,
            offset: f32,
            width: f32,
        }

        self.label_glyph_ranges.clear();
        self.label_visual_indices.clear();
        self.emoji_quads.clear();
        self.emoji_visual_indices.clear();
        let mut segments = Vec::new();
        let mut label_widths = vec![0.; labels.len()];
        for (label_index, label) in labels.iter().enumerate() {
            let mut grouped: Vec<(bool, String)> = Vec::new();
            for grapheme in label.text.graphemes(true) {
                let windfoil = self.cache.cache(grapheme).is_ok();
                match grouped.last_mut() {
                    // Vector text benefits from shaping a whole run. Raster
                    // fallback stays one grapheme per fixed atlas layer.
                    Some((true, text)) if windfoil => text.push_str(grapheme),
                    _ => grouped.push((windfoil, grapheme.to_owned())),
                }
            }
            let mut offset = 0.;
            for (windfoil, text) in grouped {
                let width = if windfoil {
                    self.cache.measure(&text, label.size.max(1.))?
                } else {
                    self.measure_text(&text, label.size.max(1.))?
                };
                segments.push(Segment {
                    label: label_index,
                    text,
                    windfoil,
                    offset,
                    width,
                });
                offset += width;
            }
            label_widths[label_index] = offset;
        }

        let origin_x = |index: usize| {
            labels[index]
                .center_x
                .map_or(labels[index].baseline[0], |center| {
                    center - label_widths[index] * 0.5
                })
        };

        for segment in segments.iter().filter(|segment| !segment.windfoil) {
            let label = &labels[segment.label];
            let left = origin_x(segment.label) + segment.offset;
            let logical_bounds = lagui::Rect::new(
                left,
                label.baseline[1] - label.size,
                segment.width.max(1.),
                label.size.max(1.) * 1.5,
            );
            if !logical_bounds.intersects(label.clip)
                || !logical_bounds.intersects(lagui::Rect::new(0., 0., viewport[0], viewport[1]))
            {
                continue;
            }
            if let Some(cached) = self.emoji_renderer.cache(
                &self.queue,
                &mut self.emoji_font_system,
                &mut self.emoji_swash_cache,
                &segment.text,
            )? {
                let size = label.size.max(1.) * scale_factor;
                self.emoji_quads.push(fixed_emoji::EmojiQuad {
                    bounds: [
                        left * scale_factor,
                        label.baseline[1] * scale_factor - size,
                        size * cached.advance,
                        size,
                    ],
                    clip: [
                        label.clip.x * scale_factor,
                        label.clip.y * scale_factor,
                        label.clip.width * scale_factor,
                        label.clip.height * scale_factor,
                    ],
                    layer: cached.layer,
                });
                self.emoji_visual_indices.push(label.visual_index);
            }
        }

        self.text.sync_font(&self.device, self.cache.font())?;
        let mut glyph_end = 0u32;
        let sections: Vec<_> = segments
            .iter()
            .filter(|segment| segment.windfoil)
            .filter_map(|segment| {
                let label = &labels[segment.label];
                let run = self.cache.get(&segment.text)?;
                let glyph_start = glyph_end;
                glyph_end += run.glyph_count() as u32;
                self.label_glyph_ranges.push(glyph_start..glyph_end);
                self.label_visual_indices.push(label.visual_index);
                Some(TextSection {
                    run,
                    baseline: [origin_x(segment.label) + segment.offset, label.baseline[1]],
                    size: label.size.max(1.),
                    color: label.color,
                    clip: Some(label.clip),
                })
            })
            .collect();
        self.text
            .prepare(&self.queue, &sections, viewport, scale_factor)?;
        self.emoji_renderer.prepare(
            &self.queue,
            &self.emoji_quads,
            [viewport[0] * scale_factor, viewport[1] * scale_factor],
        )?;
        debug_assert_eq!(glyph_end, self.text.prepared_len());
        Ok(())
    }

    fn edit_metrics(
        &mut self,
        text: &str,
        cursor: usize,
        preedit: &str,
        preedit_cursor: usize,
        size: f32,
    ) -> Result<EditMetrics> {
        let boundaries = if self.edit_metric_text != text {
            self.edit_metric_text.clear();
            self.edit_metric_text.push_str(text);
            self.edit_metric_points.clear();
            self.edit_metric_points.push((0, 0.));
            for (byte, grapheme) in text.grapheme_indices(true) {
                let end = byte + grapheme.len();
                let width = self.measure_text(&text[..end], size)?;
                self.edit_metric_points.push((end, width));
            }
            Some(self.edit_metric_points.clone())
        } else {
            None
        };
        let committed_x = self
            .edit_metric_points
            .iter()
            .find(|(byte, _)| *byte == cursor)
            .map_or(0., |(_, x)| *x);
        if preedit.is_empty() {
            return Ok(EditMetrics {
                boundaries,
                ime_cursor_x: committed_x,
                preedit_end_x: committed_x,
            });
        }
        let preedit_end = preedit_cursor.min(preedit.len());
        let composed_full = format!("{}{}", &text[..cursor], preedit);
        let preedit_end_x = self.measure_text(&composed_full, size)?;
        let ime_x = if preedit_end == preedit.len() {
            preedit_end_x
        } else {
            let preedit_prefix = preedit.get(..preedit_end).unwrap_or(preedit);
            let composed = format!("{}{}", &text[..cursor], preedit_prefix);
            self.measure_text(&composed, size)?
        };
        Ok(EditMetrics {
            boundaries,
            ime_cursor_x: ime_x,
            preedit_end_x,
        })
    }

    fn measure_text(&mut self, text: &str, size: f32) -> Result<f32> {
        if let Ok(width) = self.cache.measure(text, size) {
            return Ok(width);
        }
        let mut buffer =
            EmojiBuffer::new(&mut self.emoji_font_system, Metrics::new(size, size * 1.25));
        buffer.set_size(None, Some(size * 1.5));
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.emoji_font_system, false);
        Ok(buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0., f32::max))
    }

    fn prepare_connections(
        &mut self,
        connections: &[studio_scene::Connection],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<()> {
        self.connection_visual_index = connections
            .first()
            .map(|connection| connection.visual_index);
        if connections.is_empty() {
            self.connections
                .prepare(&self.queue, &[], viewport, scale_factor)?;
            return Ok(());
        }
        debug_assert!(
            connections
                .iter()
                .all(|connection| Some(connection.visual_index) == self.connection_visual_index)
        );
        let mut atlas = WindfoilAtlas::new();
        let mut accepted = Vec::with_capacity(connections.len());
        for connection in connections {
            let values = connection
                .start
                .into_iter()
                .chain(connection.control_a)
                .chain(connection.control_b)
                .chain(connection.end)
                .chain([connection.thickness]);
            if values.into_iter().any(|value| !value.is_finite())
                || connection.thickness < 0.0001
                || ((connection.end[0] - connection.start[0]).abs()
                    + (connection.end[1] - connection.start[1]).abs())
                    < 0.0001
            {
                continue;
            }
            let mut path = WindfoilPath::new();
            path.move_to(connection.start).cubic_to(
                connection.control_a,
                connection.control_b,
                connection.end,
            );
            match atlas.add_stroke(&path, connection.thickness) {
                Ok(shape) => accepted.push((connection, shape)),
                Err(lagui::windfoil::WindfoilError::EmptyPath)
                | Err(lagui::windfoil::WindfoilError::InvalidGeometry) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        self.connections.sync_atlas(&self.device, &atlas)?;
        let instances: Vec<_> = accepted
            .into_iter()
            .map(|(connection, shape)| WindfoilInstance {
                shape,
                origin: [0., 0.],
                scale: [1., 1.],
                color: connection.color,
                clip: Some(connection.clip),
            })
            .collect();
        self.connections
            .prepare(&self.queue, &instances, viewport, scale_factor)?;
        Ok(())
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) -> Result<()> {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Graphy Studio pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(if self.transparent {
                        wgpu::Color::TRANSPARENT
                    } else {
                        wgpu::Color::BLACK
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let mut visual_start = 0usize;
        let mut label_start = 0usize;
        let mut emoji_start = 0usize;
        let connection_index = self.connection_visual_index.unwrap_or(usize::MAX);
        let mut connections_drawn = self.connection_visual_index.is_none();
        while label_start < self.label_visual_indices.len()
            || emoji_start < self.emoji_visual_indices.len()
            || !connections_drawn
        {
            let label_index = self
                .label_visual_indices
                .get(label_start)
                .copied()
                .unwrap_or(usize::MAX);
            let emoji_index = self
                .emoji_visual_indices
                .get(emoji_start)
                .copied()
                .unwrap_or(usize::MAX);
            let next = label_index.min(emoji_index).min(if connections_drawn {
                usize::MAX
            } else {
                connection_index
            });
            let visual_end = next.max(visual_start).min(self.visuals.prepared_len());
            self.visuals.draw_range(&mut pass, visual_start..visual_end);
            visual_start = visual_end;

            if !connections_drawn && connection_index == next {
                self.connections.draw(&mut pass);
                connections_drawn = true;
            }
            if label_index == next {
                let mut label_end = label_start + 1;
                while label_end < self.label_visual_indices.len()
                    && self.label_visual_indices[label_end] == next
                {
                    label_end += 1;
                }
                let glyph_start = self.label_glyph_ranges[label_start].start;
                let glyph_end = self.label_glyph_ranges[label_end - 1].end;
                self.text.draw_range(&mut pass, glyph_start..glyph_end);
                label_start = label_end;
            }
            if emoji_index == next {
                let mut emoji_end = emoji_start + 1;
                while emoji_end < self.emoji_visual_indices.len()
                    && self.emoji_visual_indices[emoji_end] == next
                {
                    emoji_end += 1;
                }
                self.emoji_renderer
                    .draw_range(&mut pass, emoji_start as u32..emoji_end as u32);
                emoji_start = emoji_end;
            }
        }
        self.visuals
            .draw_range(&mut pass, visual_start..self.visuals.prepared_len());
        Ok(())
    }
}

struct App {
    gpu: Option<Gpu>,
    preview_window: Option<PreviewWindow>,
    studio: studio_scene::Studio,
    visual_storage: Vec<Visual>,
    labels: Vec<studio_scene::Label<'static>>,
    connections: Vec<studio_scene::Connection>,
    pointer: [f32; 2],
    modifiers: ModifiersState,
    last_frame: Instant,
    build_total_us: f64,
    build_max_us: f64,
    frames: u64,
    smoke: bool,
    failed: bool,
    occluded: bool,
    clipboard: Option<arboard::Clipboard>,
}

struct PreviewWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: gpu_preview::Renderer,
    started: Instant,
}

impl PreviewWindow {
    async fn new(window: Arc<Window>) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: if cfg!(target_os = "macos") {
                wgpu::Backends::METAL
            } else if cfg!(target_os = "windows") {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::VULKAN
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Lagui preview window device"),
                ..Default::default()
            })
            .await?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .ok_or("No sRGB preview format")?;
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let renderer = gpu_preview::Renderer::new(&device, format, [config.width, config.height]);
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
            started: Instant::now(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.renderer
            .resize(&self.device, [size.width, size.height]);
    }

    fn render(&mut self) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.resize(self.window.inner_size());
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("preview surface validation error".into());
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview window clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.012,
                            g: 0.012,
                            b: 0.018,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        let logical = self
            .window
            .inner_size()
            .to_logical::<f32>(self.window.scale_factor());
        self.renderer.encode(
            &mut encoder,
            &view,
            &self.queue,
            lagui::gpu::GpuCanvas::new(
                lagui::Rect::new(0., 0., logical.width, logical.height),
                lagui::Rect::new(0., 0., logical.width, logical.height),
            ),
            self.window.scale_factor() as f32,
            self.started.elapsed().as_secs_f32(),
        );
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        Ok(())
    }
}

impl App {
    fn open_preview_window(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if let Some(preview) = &self.preview_window {
            preview.window.focus_window();
            preview.window.request_redraw();
            return Ok(());
        }
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Graphy · Live 3D Preview")
                    .with_inner_size(LogicalSize::new(720., 540.)),
            )?,
        );
        let preview = pollster::block_on(PreviewWindow::new(window))?;
        preview.window.request_redraw();
        self.preview_window = Some(preview);
        Ok(())
    }

    fn redraw(&self) {
        if let Some(gpu) = &self.gpu {
            gpu.window.request_redraw();
        }
    }

    fn build(
        studio: &mut studio_scene::Studio,
        visual_storage: &mut [Visual],
        labels: &mut Vec<studio_scene::Label<'static>>,
        connections: &mut Vec<studio_scene::Connection>,
        width: f32,
        height: f32,
    ) -> Result<usize> {
        let mut list = VisualList::new(visual_storage);
        studio
            .paint(&mut list, labels, connections, width, height)
            .map_err(|_| "studio visual capacity")?;
        Ok(list.commands().len())
    }

    fn render(&mut self) -> Result<()> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.studio.tick(elapsed);
        let Some(gpu) = &mut self.gpu else {
            return Ok(());
        };
        let window_size = gpu.window.inner_size();
        if self.occluded || window_size.width == 0 || window_size.height == 0 {
            return Ok(());
        }
        let dpi = gpu.window.scale_factor() as f32;
        let logical = window_size.to_logical::<f32>(dpi as f64);
        let build_start = Instant::now();
        let selection = self.studio.search.selection();
        let insertion = selection.ordered().start;
        let preedit_selection = self.studio.search.preedit_selection();
        let search_text = self.studio.search.text().to_owned();
        let preedit = self.studio.search.preedit().to_owned();
        let metrics = gpu.edit_metrics(
            &search_text,
            insertion,
            &preedit,
            preedit_selection.cursor,
            11.,
        )?;
        if let Some(boundaries) = metrics.boundaries {
            self.studio.set_search_boundaries(boundaries);
        }
        self.studio
            .set_preedit_metrics(metrics.ime_cursor_x, metrics.preedit_end_x);
        let visual_count = Self::build(
            &mut self.studio,
            &mut self.visual_storage,
            &mut self.labels,
            &mut self.connections,
            logical.width,
            logical.height,
        )?;
        if let Some(cursor) = self.studio.ime_cursor_rect() {
            gpu.window.set_ime_cursor_area(
                LogicalPosition::new(cursor.x, cursor.y),
                LogicalSize::new(cursor.width, cursor.height),
            );
        }
        let build_us = build_start.elapsed().as_secs_f64() * 1e6;
        self.build_total_us += build_us;
        self.build_max_us = self.build_max_us.max(build_us);
        gpu.visuals
            .prepare(
                &gpu.queue,
                &self.visual_storage[..visual_count],
                [logical.width, logical.height],
                dpi,
            )
            .map_err(|_| "studio visual prepare")?;
        gpu.prepare_text(&self.labels, [logical.width, logical.height], dpi)?;
        gpu.prepare_connections(&self.connections, [logical.width, logical.height], dpi)?;
        let gpu_canvas = self.studio.gpu_canvas();
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.resize(window_size);
                gpu.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("wgpu surface validation error".into());
            }
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.encode(
            &mut encoder,
            &frame.texture.create_view(&Default::default()),
        )?;
        if let Some(canvas) = gpu_canvas {
            gpu.preview.encode(
                &mut encoder,
                &frame.texture.create_view(&Default::default()),
                &gpu.queue,
                canvas,
                dpi,
                self.studio.current_time,
            );
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.window.pre_present_notify();
        gpu.queue.present(frame);
        self.frames += 1;

        Ok(())
    }

    fn run_smoke(&mut self) -> Result<()> {
        const WIDTH: u32 = 1280;
        const HEIGHT: u32 = 768;
        // Populate the ordinary interaction projection before driving inputs.
        Self::build(
            &mut self.studio,
            &mut self.visual_storage,
            &mut self.labels,
            &mut self.connections,
            WIDTH as f32,
            HEIGHT as f32,
        )?;
        // Exercise selection, node dragging, pane resizing, slider capture and timeline.
        self.studio.pointer_move([64., 244.]);
        self.studio.pointer_down([64., 244.]);
        self.studio.pointer_up([64., 244.]);
        self.studio.pointer_move([538., 152.]);
        self.studio.pointer_down([538., 152.]);
        self.studio.pointer_move([566., 174.]);
        self.studio.pointer_up([566., 174.]);
        self.studio.pointer_move([1030., 273.]);
        self.studio.pointer_down([1030., 273.]);
        self.studio.pointer_move([1160., 273.]);
        self.studio.pointer_up([1160., 273.]);
        self.studio.ui.set_focus(Some(lagui::ui::WidgetId(10)));
        self.studio.commit_text("A😀B");

        let Some(gpu) = &mut self.gpu else {
            return Err("GPU missing".into());
        };
        // Match the interactive path: caret measurement happens before paint.
        let emoji_text = self.studio.search.text().to_owned();
        gpu.edit_metrics(&emoji_text, emoji_text.len(), "", 0, 11.)?;
        let visual_count = Self::build(
            &mut self.studio,
            &mut self.visual_storage,
            &mut self.labels,
            &mut self.connections,
            WIDTH as f32,
            HEIGHT as f32,
        )?;
        gpu.visuals
            .prepare(
                &gpu.queue,
                &self.visual_storage[..visual_count],
                [WIDTH as f32, HEIGHT as f32],
                1.,
            )
            .map_err(|_| "smoke visual prepare")?;
        gpu.prepare_text(&self.labels, [WIDTH as f32, HEIGHT as f32], 1.)?;
        if gpu.emoji_quads.is_empty() {
            return Err("studio emoji fallback was not prepared".into());
        }
        gpu.prepare_connections(&self.connections, [WIDTH as f32, HEIGHT as f32], 1.)?;
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("studio smoke target"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("studio smoke readback"),
            size: (WIDTH * HEIGHT * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.encode(&mut encoder, &texture.create_view(&Default::default()))?;
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WIDTH * 4),
                    rows_per_image: Some(HEIGHT),
                },
            },
            texture.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let bytes = readback.slice(..).get_mapped_range()?;
        let bgra = gpu.config.format == wgpu::TextureFormat::Bgra8UnormSrgb;
        let offset = ((25 * WIDTH + 600) * 4) as usize;
        let solid_panel = if bgra {
            [bytes[offset + 2], bytes[offset + 1], bytes[offset]]
        } else {
            [bytes[offset], bytes[offset + 1], bytes[offset + 2]]
        };
        #[cfg(target_os = "macos")]
        // The offscreen target has no system backdrop. PANEL therefore blends
        // over the renderer clear color, unlike the translucent live window.
        let expected_panel = [50u8, 50, 50];
        #[cfg(not(target_os = "macos"))]
        let expected_panel = [49u8, 56, 66];
        if solid_panel
            .iter()
            .zip(expected_panel)
            .any(|(actual, expected)| actual.abs_diff(expected) > 3)
        {
            return Err(format!(
                "panel color smoke failed: expected {expected_panel:?}, got {solid_panel:?}"
            )
            .into());
        }
        let mut bright_title_pixels = 0usize;
        for y in 10..48 {
            for x in 42..190 {
                let offset = ((y * WIDTH + x) * 4) as usize;
                let (red, green, blue) = if bgra {
                    (bytes[offset + 2], bytes[offset + 1], bytes[offset])
                } else {
                    (bytes[offset], bytes[offset + 1], bytes[offset + 2])
                };
                if red.max(green).max(blue) > 120 {
                    bright_title_pixels += 1;
                }
            }
        }
        if bright_title_pixels < 120 {
            return Err(format!("studio text smoke failed: {bright_title_pixels} pixels").into());
        }
        std::fs::create_dir_all("build")?;
        use std::io::Write;
        let mut output = std::io::BufWriter::new(std::fs::File::create("build/studio-smoke.ppm")?);
        output.write_all(b"P6\n1280 768\n255\n")?;
        for pixel in bytes.chunks_exact(4) {
            output.write_all(&if bgra {
                [pixel[2], pixel[1], pixel[0]]
            } else {
                [pixel[0], pixel[1], pixel[2]]
            })?;
        }
        output.flush()?;
        drop(bytes);
        readback.unmap();
        let mut build_samples = [0f64; 2048];
        for sample in &mut build_samples {
            let start = Instant::now();
            let _ = Self::build(
                &mut self.studio,
                &mut self.visual_storage,
                &mut self.labels,
                &mut self.connections,
                WIDTH as f32,
                HEIGHT as f32,
            )?;
            *sample = start.elapsed().as_secs_f64() * 1e6;
        }
        build_samples.sort_by(f64::total_cmp);
        println!(
            "studio smoke: {visual_count} visuals, {} labels, {} cached runs, {bright_title_pixels} title pixels; build/studio-smoke.ppm",
            self.labels.len(),
            gpu.cache.len()
        );
        println!(
            "studio CPU build (excludes GPU/display): p50={:.1} us p95={:.1} us p99={:.1} us max={:.1} us / 2048 samples",
            build_samples[1024], build_samples[1945], build_samples[2027], build_samples[2047]
        );
        Ok(())
    }

    fn report(&self) {
        if self.frames > 0 {
            println!(
                "studio frames={} UI build mean={:.1} us max={:.1} us",
                self.frames,
                self.build_total_us / self.frames as f64,
                self.build_max_us
            );
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let result = (|| -> Result<Gpu> {
            let attributes = Window::default_attributes()
                .with_title("Graphy Studio · Lagui advanced native demo")
                .with_inner_size(LogicalSize::new(1280., 768.));
            #[cfg(target_os = "macos")]
            let attributes = attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true);
            let window = Arc::new(event_loop.create_window(attributes)?);
            window_chrome::position_traffic_lights(&window);
            window.set_ime_allowed(true);
            pollster::block_on(Gpu::new(window))
        })();
        match result {
            Ok(gpu) => {
                gpu.window.request_redraw();
                self.gpu = Some(gpu);
                self.last_frame = Instant::now();
                // Smoke verifies offscreen GPU readback. An occluded window
                // may never acquire a drawable, so it must not gate this test.
                if self.smoke {
                    if let Err(error) = self.run_smoke() {
                        eprintln!("studio smoke: {error}");
                        self.failed = true;
                    }
                    event_loop.exit();
                }
            }
            Err(error) => {
                eprintln!("studio startup: {error}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .preview_window
            .as_ref()
            .is_some_and(|preview| preview.window.id() == window_id)
        {
            let result = match event {
                WindowEvent::CloseRequested => {
                    self.preview_window = None;
                    Ok(())
                }
                WindowEvent::Resized(size) => {
                    if let Some(preview) = &mut self.preview_window {
                        preview.resize(size);
                        preview.window.request_redraw();
                    }
                    Ok(())
                }
                WindowEvent::RedrawRequested => {
                    if let Some(preview) = &mut self.preview_window {
                        let rendered = preview.render();
                        if rendered.is_ok() {
                            preview.window.request_redraw();
                        }
                        rendered
                    } else {
                        Ok(())
                    }
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                eprintln!("preview window: {error}");
                self.preview_window = None;
            }
            return;
        }
        let result = match event {
            WindowEvent::CloseRequested => {
                self.report();
                event_loop.exit();
                Ok(())
            }
            WindowEvent::Resized(size) => {
                if size.width != 0
                    && size.height != 0
                    && let Some(gpu) = &mut self.gpu
                {
                    gpu.resize(size);
                    window_chrome::position_traffic_lights(&gpu.window);
                    gpu.window.request_redraw();
                }
                Ok(())
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(gpu.window.inner_size());
                    window_chrome::position_traffic_lights(&gpu.window);
                }
                self.redraw();
                Ok(())
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded {
                    self.redraw();
                }
                Ok(())
            }
            WindowEvent::Focused(false) => {
                self.studio.cancel_pointer();
                self.studio.clear_preedit();
                Ok(())
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                Ok(())
            }
            WindowEvent::CursorMoved { position, .. } if !self.smoke => {
                if let Some(gpu) = &self.gpu {
                    let logical = position.to_logical::<f32>(gpu.window.scale_factor());
                    self.pointer = [logical.x, logical.y];
                    if self.studio.pointer_move(self.pointer) {
                        self.redraw();
                    }
                }
                Ok(())
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } if !self.smoke => {
                if state == ElementState::Pressed {
                    self.studio.pointer_down(self.pointer);
                } else {
                    self.studio.pointer_up(self.pointer);
                }
                self.redraw();
                Ok(())
            }
            WindowEvent::MouseWheel { delta, .. } if !self.smoke => {
                match delta {
                    // Discrete mouse wheels are reported in lines. Use them
                    // for cursor-anchored zoom, matching editor applications.
                    MouseScrollDelta::LineDelta(_, y) => {
                        self.studio.pinch((y * 0.12) as f64);
                    }
                    // Trackpads report high-resolution pixel deltas and retain
                    // both axes for natural two-finger panning.
                    MouseScrollDelta::PixelDelta(position) => {
                        let scale_factor = self
                            .gpu
                            .as_ref()
                            .map_or(1., |gpu| gpu.window.scale_factor())
                            as f32;
                        self.studio.scroll([
                            position.x as f32 / scale_factor,
                            position.y as f32 / scale_factor,
                        ]);
                    }
                }
                self.redraw();
                Ok(())
            }
            WindowEvent::PinchGesture { delta, .. } if !self.smoke => {
                self.studio.pinch(delta);
                self.redraw();
                Ok(())
            }
            WindowEvent::PanGesture { delta, .. } if !self.smoke => {
                let scale_factor =
                    self.gpu
                        .as_ref()
                        .map_or(1., |gpu| gpu.window.scale_factor()) as f32;
                self.studio
                    .scroll([delta.x / scale_factor, delta.y / scale_factor]);
                self.redraw();
                Ok(())
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) if !self.smoke => {
                self.studio.set_preedit(text, cursor);
                self.redraw();
                Ok(())
            }
            WindowEvent::Ime(Ime::Commit(text)) if !self.smoke => {
                self.studio.commit_text(&text);
                self.redraw();
                Ok(())
            }
            WindowEvent::Ime(Ime::Disabled) if !self.smoke => {
                self.studio.clear_preedit();
                self.redraw();
                Ok(())
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !self.smoke =>
            {
                match event.logical_key {
                    Key::Named(NamedKey::Tab) => self.studio.focus_next(self.modifiers.shift_key()),
                    Key::Named(NamedKey::Enter) => self.studio.activate_focused(),
                    Key::Named(NamedKey::Space) => self.studio.keyboard(" "),
                    Key::Named(NamedKey::Backspace) => self.studio.keyboard("Backspace"),
                    Key::Named(NamedKey::Delete) => self.studio.keyboard("Delete"),
                    Key::Named(NamedKey::Escape) => self.studio.keyboard("Escape"),
                    Key::Named(NamedKey::ArrowLeft) => self
                        .studio
                        .move_search_cursor(false, self.modifiers.shift_key()),
                    Key::Named(NamedKey::ArrowRight) => self
                        .studio
                        .move_search_cursor(true, self.modifiers.shift_key()),
                    Key::Character(ref text)
                        if (self.modifiers.control_key() || self.modifiers.super_key())
                            && text.eq_ignore_ascii_case("a") =>
                    {
                        self.studio.keyboard("SelectAll")
                    }
                    Key::Character(ref text)
                        if (self.modifiers.control_key() || self.modifiers.super_key())
                            && text.eq_ignore_ascii_case("z")
                            && self.modifiers.shift_key() =>
                    {
                        self.studio.keyboard("Redo")
                    }
                    Key::Character(ref text)
                        if (self.modifiers.control_key() || self.modifiers.super_key())
                            && text.eq_ignore_ascii_case("z") =>
                    {
                        self.studio.keyboard("Undo")
                    }
                    Key::Character(ref text)
                        if (self.modifiers.control_key() || self.modifiers.super_key())
                            && text.eq_ignore_ascii_case("c") =>
                    {
                        if let (Some(clipboard), Some(selected)) =
                            (&mut self.clipboard, self.studio.selected_search_text())
                        {
                            let _ = clipboard.set_text(selected);
                        }
                    }
                    Key::Character(ref text)
                        if (self.modifiers.control_key() || self.modifiers.super_key())
                            && text.eq_ignore_ascii_case("x") =>
                    {
                        if let Some(selected) =
                            self.studio.selected_search_text().map(str::to_owned)
                            && let Some(clipboard) = &mut self.clipboard
                            && clipboard.set_text(selected).is_ok()
                        {
                            self.studio.cut_search_selection();
                        }
                    }
                    Key::Character(ref text)
                        if (self.modifiers.control_key() || self.modifiers.super_key())
                            && text.eq_ignore_ascii_case("v") =>
                    {
                        if let Some(clipboard) = &mut self.clipboard
                            && let Ok(text) = clipboard.get_text()
                        {
                            self.studio.paste_search(&text);
                        }
                    }
                    Key::Character(text)
                        if !self.modifiers.control_key() && !self.modifiers.super_key() =>
                    {
                        self.studio.keyboard(&text)
                    }
                    _ => {}
                }
                self.redraw();
                Ok(())
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => Ok(()),
        };
        if let Err(error) = result {
            eprintln!("studio: {error}");
            self.failed = true;
            event_loop.exit();
        }
        if self.studio.take_preview_window_request()
            && let Err(error) = self.open_preview_window(event_loop)
        {
            eprintln!("open preview window: {error}");
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.studio.playing && !self.occluded {
            self.redraw();
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(8),
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn main() -> Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = App {
        gpu: None,
        preview_window: None,
        studio: studio_scene::Studio::default(),
        visual_storage: vec![Visual::default(); 4096],
        labels: Vec::with_capacity(256),
        connections: Vec::with_capacity(8),
        pointer: [-100.; 2],
        modifiers: ModifiersState::default(),
        last_frame: Instant::now(),
        build_total_us: 0.,
        build_max_us: 0.,
        frames: 0,
        smoke: std::env::args().any(|argument| argument == "--smoke"),
        failed: false,
        occluded: false,
        clipboard: arboard::Clipboard::new().ok(),
    };
    event_loop.run_app(&mut app)?;
    if app.failed {
        Err("studio demo failed".into())
    } else {
        Ok(())
    }
}
