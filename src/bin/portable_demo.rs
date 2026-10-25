use lagui::{
    DrawList, Quad,
    demo::Demo,
    gpu::{CAPACITY, Renderer},
};
use std::{
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[path = "support/metal_resize.rs"]
mod metal_resize;
#[path = "support/text_scene.rs"]
mod text_scene;
#[path = "support/text_validation.rs"]
mod text_validation;

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    resizing: bool,
    renderer: Renderer,
    text: text_scene::TextScene,
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: if let Ok(backend) = std::env::var("LAGUI_BACKEND") {
                match backend.as_str() {
                    "dx12" => wgpu::Backends::DX12,
                    "vulkan" => wgpu::Backends::VULKAN,
                    "metal" => wgpu::Backends::METAL,
                    _ => return Err(format!("unsupported LAGUI_BACKEND: {backend}").into()),
                }
            } else if cfg!(target_os = "macos") {
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
                label: Some("Lagui device"),
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
        let vsync = std::env::var("LAGUI_VSYNC").is_ok_and(|v| v == "1");
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if vsync {
                wgpu::PresentMode::Fifo
            } else {
                wgpu::PresentMode::AutoNoVsync
            },
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        println!(
            "wgpu {:?}: {}; present={:?}; supported={:?}",
            adapter.get_info().backend,
            adapter.get_info().name,
            config.present_mode,
            caps.present_modes
        );
        surface.configure(&device, &config);
        let renderer = Renderer::new(&device, format);
        let text = text_scene::TextScene::new(&device, format)?;
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            resizing: false,
            renderer,
            text,
        })
    }
    fn set_window_resizing(&mut self, resizing: bool) {
        if self.resizing == resizing {
            return;
        }
        metal_resize::set_present_with_transaction(
            &self.surface,
            &self.device,
            &self.config,
            resizing,
        );
        self.resizing = resizing;
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
    }
    fn encode(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lagui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        self.renderer.draw(&mut pass);
        self.text.renderer.draw(&mut pass);
    }
    fn smoke(&mut self, demo: &mut Demo) -> Result<()> {
        demo.input(1, 80., 175.);
        demo.input(2, 80., 175.);
        demo.input(1, 260., 275.);
        demo.input(2, 260., 275.);
        let mut storage = [Quad::default(); CAPACITY];
        let mut list = DrawList::new(&mut storage);
        demo.paint(&mut list, 960., 600.)
            .map_err(|_| "draw capacity")?;
        self.renderer
            .prepare(&self.queue, list.commands(), [960., 600.])
            .map_err(|_| "GPU prepare")?;
        self.text
            .prepare(&self.queue, [960., 600.], 1., demo.value)?;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke target"),
            size: wgpu::Extent3d {
                width: 960,
                height: 600,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke readback"),
            size: 960 * 600 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.encode(&mut encoder, &texture.create_view(&Default::default()));
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(960 * 4),
                    rows_per_image: Some(600),
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let bytes = readback.slice(..).get_mapped_range()?;
        let offset = (42 * 960 + 45) * 4;
        let bgra = self.config.format == wgpu::TextureFormat::Bgra8UnormSrgb;
        let pixel = if bgra {
            [
                bytes[offset + 2],
                bytes[offset + 1],
                bytes[offset],
                bytes[offset + 3],
            ]
        } else {
            [
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]
        };
        if !(pixel[0] > 195
            && pixel[0] < 210
            && pixel[1] > 125
            && pixel[1] < 145
            && pixel[2] > 240
            && pixel[3] == 255)
        {
            return Err(format!("smoke pixel mismatch: {pixel:?}").into());
        }
        // These areas contained only flat backgrounds before the text pass.
        // Check independent Latin and Japanese regions, including partially
        // covered edge pixels, so a blank or solid glyph rectangle fails.
        for (name, x0, y0, x1, y1) in [("Latin", 40, 65, 210, 95), ("Japanese", 380, 105, 720, 135)]
        {
            let mut ink = 0;
            let mut partial = 0;
            let mut background = 0;
            for y in y0..y1 {
                for x in x0..x1 {
                    let off = (y * 960 + x) * 4;
                    let r = bytes[off + if bgra { 2 } else { 0 }];
                    if r > 120 {
                        ink += 1;
                    }
                    if r > 65 && r < 180 {
                        partial += 1;
                    }
                    if r < 65 {
                        background += 1;
                    }
                }
            }
            if ink < 80 || partial < 10 || background < 80 {
                return Err(format!(
                    "{name} text readback failed: ink={ink}, AA={partial}, background={background}"
                )
                .into());
            }
            println!("{name}: {ink} ink pixels, {partial} antialiased pixels");
        }
        std::fs::create_dir_all("build")?;
        use std::io::Write;
        let mut file = std::io::BufWriter::new(std::fs::File::create("build/wgpu-smoke.ppm")?);
        file.write_all(b"P6\n960 600\n255\n")?;
        for p in bytes.chunks_exact(4) {
            file.write_all(&if bgra {
                [p[2], p[1], p[0]]
            } else {
                [p[0], p[1], p[2]]
            })?;
        }
        file.flush()?;
        drop(bytes);
        readback.unmap();
        text_validation::run(&self.device, &self.queue)?;
        println!(
            "smoke: Rust state + wgpu GPU pixel verified; build/wgpu-smoke.ppm (synthetic, not input latency)"
        );
        Ok(())
    }
}

struct App {
    gpu: Option<Gpu>,
    demo: Demo,
    pointer: [f32; 2],
    pressed: bool,
    received: Option<Instant>,
    samples: [[f64; 4]; 2048],
    sample_count: usize,
    cursor: usize,
    frames: u64,
    smoke: bool,
    failed: bool,
    retry: Option<Instant>,
    occluded: bool,
}
impl App {
    fn input(&mut self, kind: u32) {
        self.received = Some(Instant::now());
        self.demo.input(kind, self.pointer[0], self.pointer[1]);
        if let Some(gpu) = &self.gpu {
            gpu.window.request_redraw();
        }
    }
    fn report(&self) {
        println!(
            "frames={} input_samples={} (receive -> submit; excludes GPU/display completion)",
            self.frames, self.sample_count
        );
        if self.sample_count == 0 {
            return;
        }
        for (metric, name) in [
            "rectangle list",
            "text prepare/upload",
            "surface wait",
            "input receive -> submit",
        ]
        .iter()
        .enumerate()
        {
            let mut values = [0.; 2048];
            for (i, s) in self.samples[..self.sample_count].iter().enumerate() {
                values[i] = s[metric];
            }
            let values = &mut values[..self.sample_count];
            values.sort_by(f64::total_cmp);
            let n = values.len() - 1;
            println!(
                "{name}: p50 {:.3} us | p95 {:.3} us | p99 {:.3} us | max {:.3} us",
                values[n / 2],
                values[n * 95 / 100],
                values[n * 99 / 100],
                values[n]
            );
        }
    }
    fn render(&mut self) -> Result<()> {
        let Some(gpu) = &mut self.gpu else {
            return Ok(());
        };
        let size = gpu.window.inner_size();
        if self.occluded || size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let logical = size.to_logical::<f32>(gpu.window.scale_factor());
        let mut storage = [Quad::default(); CAPACITY];
        let mut list = DrawList::new(&mut storage);
        let start = Instant::now();
        self.demo
            .paint(&mut list, logical.width, logical.height)
            .map_err(|_| "draw capacity")?;
        let built = start.elapsed().as_secs_f64() * 1e6;
        let acquire = Instant::now();
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.resize(size);
                self.retry = Some(Instant::now() + Duration::from_millis(16));
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.retry = Some(Instant::now() + Duration::from_millis(16));
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("wgpu surface validation error".into());
            }
        };
        let wait = acquire.elapsed().as_secs_f64() * 1e6;
        gpu.renderer
            .prepare(&gpu.queue, list.commands(), [logical.width, logical.height])
            .map_err(|_| "GPU prepare")?;
        let text_start = Instant::now();
        gpu.text.prepare(
            &gpu.queue,
            [logical.width, logical.height],
            gpu.window.scale_factor() as f32,
            self.demo.value,
        )?;
        let text_time = text_start.elapsed().as_secs_f64() * 1e6;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.encode(
            &mut encoder,
            &frame.texture.create_view(&Default::default()),
        );
        gpu.queue.submit([encoder.finish()]);
        if let Some(input) = self.received.take() {
            self.samples[self.cursor] =
                [built, text_time, wait, input.elapsed().as_secs_f64() * 1e6];
            self.cursor = (self.cursor + 1) % 2048;
            self.sample_count = (self.sample_count + 1).min(2048);
        }
        gpu.window.pre_present_notify();
        gpu.queue.present(frame);
        self.frames += 1;
        Ok(())
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let result = (|| -> Result<Gpu> {
            let window=Arc::new(el.create_window(Window::default_attributes()
                .with_title("Lagui · winit + wgpu | Click left button / Space: color | Drag slider / arrows: intensity | P: timings")
                .with_inner_size(LogicalSize::new(960.,600.)))?);
            pollster::block_on(Gpu::new(window))
        })();
        match result {
            Ok(mut gpu) => {
                // Readback validates rendering independently of window visibility.
                if self.smoke {
                    if let Err(error) = gpu.smoke(&mut self.demo) {
                        eprintln!("smoke: {error}");
                        self.failed = true;
                    }
                    el.exit();
                } else {
                    gpu.window.request_redraw();
                }
                self.gpu = Some(gpu);
            }
            Err(e) => {
                eprintln!("startup: {e}");
                self.failed = true;
                el.exit();
            }
        }
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // winit exposes the resize lifecycle as a stream of Resized events,
        // without explicit begin/end notifications. The first non-resize
        // event marks the end of that stream.
        if !matches!(&event, WindowEvent::Resized(_))
            && let Some(gpu) = &mut self.gpu
        {
            gpu.set_window_resizing(false);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.report();
                el.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gpu {
                    g.set_window_resizing(true);
                    g.resize(size);
                    g.window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(g) = &mut self.gpu {
                    g.resize(g.window.inner_size());
                    g.window.request_redraw();
                }
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded && let Some(g) = &self.gpu {
                    g.window.request_redraw();
                }
            }
            WindowEvent::Focused(false) => {
                self.pressed = false;
                self.input(3);
            }
            WindowEvent::CursorMoved { position, .. } if !self.smoke => {
                if let Some(g) = &self.gpu {
                    let p = position.to_logical::<f32>(g.window.scale_factor());
                    self.pointer = [p.x, p.y];
                    self.input(0);
                }
            }
            WindowEvent::CursorLeft { .. } if !self.pressed && !self.smoke => self.input(3),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } if !self.smoke => {
                self.pressed = state == ElementState::Pressed;
                self.input(if self.pressed { 1 } else { 2 });
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !self.smoke =>
            {
                match event.logical_key {
                    Key::Named(NamedKey::Space) => self.input(4),
                    Key::Named(NamedKey::ArrowLeft) => self.input(5),
                    Key::Named(NamedKey::ArrowRight) => self.input(6),
                    Key::Character(c) if c.eq_ignore_ascii_case("p") => self.report(),
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.render() {
                    eprintln!("render: {e}");
                    self.failed = true;
                    el.exit();
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(time) = self.retry {
            if Instant::now() >= time {
                self.retry = None;
                if let Some(gpu) = &self.gpu {
                    gpu.window.request_redraw();
                }
                el.set_control_flow(ControlFlow::Wait);
            } else {
                el.set_control_flow(ControlFlow::WaitUntil(time));
            }
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn main() -> Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = App {
        gpu: None,
        demo: Demo::default(),
        pointer: [-20.; 2],
        pressed: false,
        received: None,
        samples: [[0.; 4]; 2048],
        sample_count: 0,
        cursor: 0,
        frames: 0,
        smoke: std::env::args().any(|a| a == "--smoke"),
        failed: false,
        retry: None,
        occluded: false,
    };
    event_loop.run_app(&mut app)?;
    if app.failed {
        return Err("native demo failed".into());
    }
    Ok(())
}
