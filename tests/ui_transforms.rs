#![cfg(feature = "portable")]
mod support;
use lagui::{
    Rect,
    gpu::{Damage, Shared, Transform2D, Transforms, VisualRenderer},
    text::{Cull, Font, TextRenderer, TextSection},
    visual::{Visual, VisualList},
    windfoil::Pipeline,
};

#[test]
fn pan_uploads_only_the_table_and_matches_screen_placement() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let shared = Shared::new(&device);
    let mut transforms = Transforms::new(&device, &shared);
    let mut renderer =
        VisualRenderer::with_shared(&device, wgpu::TextureFormat::Rgba8Unorm, 1, &shared).unwrap();
    let mut direct = VisualRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
    let clip = Rect::new(0., 0., 64., 64.);
    for scale in [0.25, 1., 3.7, 8.] {
        for offset in [[4., 4.], [12., 8.]] {
            transforms.begin();
            let transform = Transform2D { offset, scale };
            let id = transforms.push(transform).unwrap();
            transforms.commit(&queue);
            let mut storage = [Visual::default()];
            let mut list = VisualList::new(&mut storage);
            list.with_transform(id, |list| {
                list.fill(Rect::new(1., 1., 4., 4.), [1.; 4], 0., clip)
            })
            .unwrap();
            renderer
                .prepare_transformed(&queue, list.commands(), [64.; 2], 1., &transforms)
                .unwrap();
            let pixels = support::render(&device, &queue, [64; 2], |p| renderer.draw(p));
            if offset[0] == 12. {
                assert_eq!(renderer.upload_stats().bytes, 0);
                assert_eq!(transforms.uploaded_bytes, 32);
                assert!(matches!(renderer.damage(), Damage::Region(_)));
            }
            let mut storage = [Visual::default()];
            let mut list = VisualList::new(&mut storage);
            list.fill(transform.rect(Rect::new(1., 1., 4., 4.)), [1.; 4], 0., clip)
                .unwrap();
            direct
                .prepare(&queue, list.commands(), [64.; 2], 1.)
                .unwrap();
            assert_eq!(
                pixels,
                support::render(&device, &queue, [64; 2], |p| direct.draw(p))
            );
        }
    }
}

#[test]
fn culling_returns_section_ranges_and_guard_keeps_pan_uploads_zero() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let shared = Shared::new(&device);
    let pipeline = Pipeline::with_shared(&device, wgpu::TextureFormat::Rgba8Unorm, &shared);
    let mut renderer = TextRenderer::with_pipeline(&device, &pipeline, 1).unwrap();
    let mut font = Font::new(include_bytes!("assets/test-outline.ttf").to_vec(), 0).unwrap();
    let run = font.shape(&"O".repeat(10000)).unwrap();
    renderer.sync_font(&device, &font).unwrap();
    let mut transforms = Transforms::new(&device, &shared);
    for offset in [0., 1.] {
        transforms.begin();
        let id = transforms
            .push(Transform2D {
                offset: [offset, 0.],
                scale: 1.,
            })
            .unwrap();
        transforms.commit(&queue);
        let ranges = renderer
            .prepare_transformed(
                &queue,
                &[
                    TextSection {
                        transform: id,
                        cull: Cull::Guard { margin: 64. },
                        run: &run,
                        baseline: [0., 20.],
                        size: 12.,
                        color: [1.; 4],
                        clip: Some(Rect::new(0., 0., 64., 32.)),
                    },
                    TextSection {
                        transform: id,
                        cull: Cull::Clip,
                        run: &run,
                        baseline: [0., 200.],
                        size: 12.,
                        color: [1.; 4],
                        clip: Some(Rect::new(0., 0., 64., 32.)),
                    },
                ],
                [64.; 2],
                1.,
                &transforms,
            )
            .unwrap()
            .to_vec();
        assert!(ranges[0].end < 30);
        assert_eq!(ranges[1].start, ranges[1].end);
        assert!(renderer.culled() > 19970);
        if offset == 1. {
            assert_eq!(renderer.upload_stats().bytes, 0);
        }
        support::render(&device, &queue, [64; 2], |p| renderer.draw(p));
    }
}
