#![cfg(feature = "portable")]
//! Instance buffers can grow without discarding the already uploaded outlines.
mod support;
use lagui::{
    Rect,
    text::{Font, TextCache, TextError, TextRenderer, TextSection},
    windfoil::{Atlas, Instance, Path, Renderer, WindfoilError},
};

#[test]
fn analytic_renderers_grow_past_the_old_limit_and_keep_their_atlases() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let font = Font::new(include_bytes!("assets/test-outline.ttf").to_vec(), 0).unwrap();
    let mut cache = TextCache::new(font);
    cache.cache("O").unwrap();
    let mut text = TextRenderer::new(&device, format, 1).unwrap();
    text.sync_font(&device, cache.font()).unwrap();
    // Deliberately cross the old 16,384 limit; only the device limit applies.
    let sections: Vec<_> = (0..17_000)
        .map(|_| TextSection {
            run: cache.get("O").unwrap(),
            baseline: [8., 20.],
            size: 12.,
            color: [1.; 4],
            clip: None,
        })
        .collect();
    text.prepare(&queue, &sections[..1], [64., 32.], 1.)
        .unwrap();
    // No sync_font call after growing: the uploaded outline atlas must survive.
    text.prepare(&queue, &sections, [64., 32.], 1.).unwrap();
    assert_eq!(text.prepared_len(), 17_000);
    text.reserve(&device, 1).unwrap();
    assert_eq!(
        text.prepared_len(),
        17_000,
        "reserving an existing capacity must retain the prepared batch"
    );

    let mut atlas = Atlas::new();
    let mut path = Path::new();
    path.move_to([4., 16.]).line_to([28., 16.]);
    let shape = atlas.add_stroke(&path, 2.).unwrap();
    let mut vectors = Renderer::new(&device, format, 1).unwrap();
    vectors.sync_atlas(&device, &atlas).unwrap();
    let instances: Vec<_> = (0..17_000)
        .map(|_| Instance {
            shape,
            origin: [0.; 2],
            scale: [1.; 2],
            color: [1.; 4],
            clip: Some(Rect::new(0., 0., 64., 32.)),
        })
        .collect();
    vectors
        .prepare(&queue, &instances[..1], [64., 32.], 1.)
        .unwrap();
    vectors.prepare(&queue, &instances, [64., 32.], 1.).unwrap();
    assert_eq!(vectors.prepared_len(), 17_000);
    let allocated = vectors.capacity();
    vectors.reserve(&device, 1).unwrap();
    assert_eq!(vectors.capacity(), allocated);
    assert_eq!(vectors.prepared_len(), 17_000);

    let beyond_device =
        (device.limits().max_buffer_size / 96).min(u64::from(u32::MAX)) as usize + 1;
    assert!(matches!(
        text.reserve(&device, beyond_device),
        Err(TextError::Capacity)
    ));
    assert!(matches!(
        vectors.reserve(&device, beyond_device),
        Err(WindfoilError::Capacity)
    ));
}
