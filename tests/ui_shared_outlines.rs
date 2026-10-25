#![cfg(feature = "portable")]
mod support;
use lagui::{
    Rect,
    gpu::{RetainedTarget, outlines::OutlineBatch},
    text::{Cull, Font, TextRenderer, TextSection},
    transform::TransformId,
    windfoil::{Atlas, Instance, Path, Pipeline, Renderer},
};

#[test]
fn mixed_fonts_and_vectors_share_one_stream_without_changing_pixels() {
    let Some((device, queue)) = support::device() else {
        return;
    };
    let pipeline = Pipeline::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let mut font = Font::new(include_bytes!("assets/test-outline.ttf").to_vec(), 0).unwrap();
    let run = font.shape("O").unwrap();
    let mut text = TextRenderer::with_pipeline(&device, &pipeline, 1).unwrap();
    let mut reference = TextRenderer::with_pipeline(&device, &pipeline, 1).unwrap();
    let sections = [TextSection {
        run: &run,
        baseline: [3., 25.],
        size: 20.,
        color: [1.; 4],
        clip: None,
        cull: Cull::Clip,
        transform: TransformId::IDENTITY,
    }];
    reference.sync_font(&device, &font).unwrap();
    reference.prepare(&queue, &sections, [64.; 2], 1.).unwrap();
    let mut atlas = Atlas::new();
    let mut path = Path::new();
    path.move_to([0., 0.])
        .line_to([20., 0.])
        .line_to([20., 20.])
        .close();
    let items = [Instance {
        shape: atlas.add_fill(&path).unwrap(),
        origin: [30., 5.],
        scale: [1.; 2],
        color: [0., 0.5, 0., 0.5],
        clip: None,
        transform: TransformId::IDENTITY,
    }];
    let mut vectors = Renderer::with_pipeline(&device, &pipeline, 1).unwrap();
    let mut vector_reference = Renderer::with_pipeline(&device, &pipeline, 1).unwrap();
    vector_reference.sync_atlas(&device, &atlas).unwrap();
    vector_reference
        .prepare(&queue, &items, [64.; 2], 1.)
        .unwrap();
    let expected = support::render(&device, &queue, [64; 2], |p| {
        reference.draw(p);
        vector_reference.draw(p);
    });
    let mut batch = OutlineBatch::new(&device, &pipeline).unwrap();
    for frame in 0..2 {
        text.prepare_cpu(&font, &sections, [64.; 2], 1., None)
            .unwrap();
        vectors.prepare_cpu(&atlas, &items, [64.; 2], 1.).unwrap();
        let sources = [
            text.prepared_source(&font).unwrap(),
            vectors.prepared_source(&atlas).unwrap(),
        ];
        let ranges = batch
            .prepare(
                &queue,
                &sources,
                &[(0, 0..1), (1, 0..1)],
                [64.; 2],
                1.,
                None,
            )
            .unwrap();
        assert_eq!(ranges, &[0..1, 1..2]);
        let actual = support::render(&device, &queue, [64; 2], |p| {
            batch.draw_range(p, 0..batch.prepared_len())
        });
        assert_eq!(expected, actual);
        let mut emoji = lagui::gpu::emoji::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        emoji.prepare(&queue, &[], [64.; 2]).unwrap();
        let mut visuals =
            lagui::gpu::VisualRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
        visuals.prepare(&queue, &[], [64.; 2], 1.).unwrap();
        let mut unified = lagui::gpu::unified::UnifiedRenderer::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            &pipeline,
            &emoji,
        )
        .unwrap();
        unified
            .prepare(
                &queue,
                &visuals,
                &batch,
                &emoji,
                &[lagui::gpu::unified::Item::Outline(0..2)],
                None,
            )
            .unwrap();
        let pulled = support::render(&device, &queue, [64; 2], |p| {
            unified.draw_range(p, 0..unified.prepared_len())
        });
        assert_eq!(expected, pulled);
        if frame == 1 {
            assert_eq!(batch.upload_stats().bytes, 0);
        }
    }
}

#[test]
fn partial_replay_clears_transparency_and_matches_full_frames() {
    use lagui::{
        gpu::VisualRenderer,
        visual::{Visual, VisualList},
    };
    let Some((device, queue)) = support::device() else {
        return;
    };
    let mut renderer = VisualRenderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
    let mut retained = RetainedTarget::new(&device, [64; 2], wgpu::TextureFormat::Rgba8Unorm);
    for x in [4., 8., 12., 4.] {
        let mut storage = [Visual::default()];
        let mut list = VisualList::new(&mut storage);
        list.fill(
            Rect::new(x, 5., 12., 12.),
            [0.5, 0., 0., 0.5],
            2.,
            Rect::new(0., 0., 64., 64.),
        )
        .unwrap();
        renderer
            .prepare(&queue, list.commands(), [64.; 2], 1.)
            .unwrap();
        let expected = support::render(&device, &queue, [64; 2], |p| renderer.draw(p));
        let scissor = retained.scissor(renderer.damage(), 1.).unwrap();
        let actual = support::render_encoded(&device, &queue, [64; 2], |encoder, target| {
            retained.clear_region(encoder, scissor, false);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: retained.view(),
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                renderer.draw(&mut pass);
            }
            retained.blit(encoder, target);
        });
        retained.mark_rendered();
        assert_eq!(expected, actual);
    }
}
