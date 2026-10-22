#![cfg(feature = "portable")]
//! Retained instance uploads: a partially changed frame must render exactly
//! like a freshly prepared one, and damage must cover only what changed.
mod support;
use lagui::{
    Rect,
    gpu::{Damage, VisualRenderer},
    text::{Font, TextCache, TextRenderer, TextSection},
    visual::{DropShadow, Visual, VisualList},
    windfoil::Pipeline,
};

const SIZE: [u32; 2] = [64, 32];
const VIEWPORT: [f32; 2] = [64., 32.];
const CLIP: Rect = Rect::new(0., 0., 64., 32.);

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
) -> Vec<u8> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("retained upload regression"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let stride = SIZE[0] * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * SIZE[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        draw(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(SIZE[1]),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
    readback.unmap();
    pixels
}

fn scene(middle: [f32; 4], count: usize) -> Vec<Visual> {
    let mut storage = vec![Visual::default(); 4];
    let mut list = VisualList::new(&mut storage);
    let colors = [[1., 0., 0., 1.], middle, [0., 0., 1., 1.]];
    for (index, color) in colors.into_iter().enumerate().take(count) {
        list.fill(
            Rect::new(4. + index as f32 * 20., 8., 16., 16.),
            color,
            4.,
            CLIP,
        )
        .unwrap();
    }
    let len = list.commands().len();
    storage.truncate(len);
    storage
}

fn contains(damage: Damage, rect: Rect) -> bool {
    match damage {
        Damage::Full => true,
        Damage::None => false,
        Damage::Region(region) => {
            region.x <= rect.x
                && region.y <= rect.y
                && region.x + region.width >= rect.x + rect.width
                && region.y + region.height >= rect.y + rect.height
        }
    }
}

#[test]
fn partial_visual_uploads_match_fresh_preparation_and_report_damage() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut retained = VisualRenderer::new(&device, format, 1).unwrap();
    retained
        .prepare(&queue, &scene([0., 1., 0., 1.], 3), VIEWPORT, 1.)
        .unwrap();
    assert_eq!(retained.damage(), Damage::Full, "first frame redraws all");
    retained
        .prepare(&queue, &scene([0., 1., 0., 1.], 3), VIEWPORT, 1.)
        .unwrap();
    assert_eq!(retained.damage(), Damage::None, "identical frame");

    let changed = scene([1., 1., 0., 1.], 3);
    retained.prepare(&queue, &changed, VIEWPORT, 1.).unwrap();
    let middle = Rect::new(24., 8., 16., 16.);
    let Damage::Region(region) = retained.damage() else {
        panic!("one changed visual must report a region");
    };
    assert!(contains(Damage::Region(region), middle));
    assert!(
        region.x > 20. && region.x + region.width < 44.,
        "{region:?}"
    );

    let mut fresh = VisualRenderer::new(&device, format, 1).unwrap();
    fresh.prepare(&queue, &changed, VIEWPORT, 1.).unwrap();
    let a = render(&device, &queue, |pass| retained.draw(pass));
    let b = render(&device, &queue, |pass| fresh.draw(pass));
    assert_eq!(a, b, "retained and fresh uploads must render identically");
    let yellow = ((16 * SIZE[0] + 32) * 4) as usize;
    assert_eq!(&a[yellow..yellow + 4], &[255, 255, 0, 255]);

    // Removing the last visual damages where it used to be.
    retained
        .prepare(&queue, &scene([1., 1., 0., 1.], 2), VIEWPORT, 1.)
        .unwrap();
    assert!(contains(retained.damage(), Rect::new(44., 8., 16., 16.)));
    let a = render(&device, &queue, |pass| retained.draw(pass));
    let blue = ((16 * SIZE[0] + 52) * 4) as usize;
    assert_eq!(&a[blue..blue + 4], &[0, 0, 0, 0]);

    // Viewport or scale changes move every instance.
    retained
        .prepare(&queue, &scene([1., 1., 0., 1.], 2), VIEWPORT, 2.)
        .unwrap();
    assert_eq!(retained.damage(), Damage::Full);
}

#[test]
fn shadow_reach_is_derived_on_the_gpu_and_clipped_quads_match_discard_semantics() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let mut storage = [Visual::default(); 2];
    let mut list = VisualList::new(&mut storage);
    list.fill_styled(
        Rect::new(16., 8., 16., 16.),
        [1., 1., 1., 1.],
        Default::default(),
        Some(DropShadow {
            color: [0., 0., 0.5, 0.5],
            offset: [4., 0.],
            blur: 2.,
            spread: 0.,
        }),
        CLIP,
    )
    .unwrap();
    // Clip edges keep pixel-center inclusion: left/top inclusive, right/bottom exclusive.
    list.fill(
        Rect::new(40., 0., 24., 32.),
        [1., 0., 0., 1.],
        0.,
        Rect::new(44.5, 10., 8., 12.),
    )
    .unwrap();
    let mut renderer = VisualRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
    renderer.prepare(&queue, &storage, VIEWPORT, 1.).unwrap();
    let pixels = render(&device, &queue, |pass| renderer.draw(pass));
    let at = |x: u32, y: u32| {
        let i = ((y * SIZE[0] + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    assert!(at(34, 16)[2] > 0, "shadow outside the shape is rasterized");
    assert_eq!(at(24, 16), [255, 255, 255, 255]);
    assert_eq!(at(43, 16), [0, 0, 0, 0]);
    // A pixel center on the left clip edge is inside, as with the former discard test.
    assert_eq!(at(44, 16), [255, 0, 0, 255]);
    assert_eq!(at(51, 16), [255, 0, 0, 255]);
    assert_eq!(at(52, 16), [0, 0, 0, 0], "clip right edge is exclusive");
    assert_eq!(at(48, 9), [0, 0, 0, 0]);
    assert_eq!(at(48, 10), [255, 0, 0, 255]);
}

#[test]
fn shared_pipeline_text_matches_private_pipeline_and_retains_unchanged_glyphs() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let font = Font::new(include_bytes!("assets/test-outline.ttf").to_vec(), 0).unwrap();
    let mut cache = TextCache::new(font);
    cache.cache("O").unwrap();
    let pipeline = Pipeline::new(&device, format);
    let mut shared = TextRenderer::with_pipeline(&device, &pipeline, 1).unwrap();
    let mut private = TextRenderer::new(&device, format, 1).unwrap();
    let sections = |x: f32| {
        [
            TextSection {
                run: cache.get("O").unwrap(),
                baseline: [4., 24.],
                size: 20.,
                color: [1.; 4],
                clip: None,
            },
            TextSection {
                run: cache.get("O").unwrap(),
                baseline: [x, 24.],
                size: 20.,
                color: [1.; 4],
                clip: Some(Rect::new(0., 0., 60., 32.)),
            },
        ]
    };
    for renderer in [&mut shared, &mut private] {
        renderer.sync_font(&device, cache.font()).unwrap();
        renderer
            .prepare(&queue, &sections(30.), VIEWPORT, 1.)
            .unwrap();
    }
    shared
        .prepare(&queue, &sections(36.), VIEWPORT, 1.)
        .unwrap();
    let Damage::Region(region) = shared.damage() else {
        panic!("moving one glyph must report a region");
    };
    assert!(
        region.x >= 29.,
        "the unchanged first glyph is not damaged: {region:?}"
    );
    private
        .prepare(&queue, &sections(36.), VIEWPORT, 1.)
        .unwrap();
    let a = render(&device, &queue, |pass| shared.draw(pass));
    let b = render(&device, &queue, |pass| private.draw(pass));
    assert_eq!(a, b);
    assert!(a.chunks_exact(4).any(|pixel| pixel[3] == 255));
    let outside_clip = ((16 * SIZE[0] + 62) * 4) as usize;
    assert_eq!(a[outside_clip + 3], 0);
}
