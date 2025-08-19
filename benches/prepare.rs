use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use criterion::{criterion_group, criterion_main, Criterion};
use glyphon::{
    Cache, ColorMode, Resolution, TextArea, TextAreaColorType, TextAtlas, TextBounds, TextRenderer, Viewport, Weight
};
use wgpu::{MultisampleState, TextureFormat};

mod state;

fn run_bench(ctx: &mut Criterion) {
    let mut group = ctx.benchmark_group("Prepare");
    group.noise_threshold(0.02);

    let state = state::State::new();

    // Set up text renderer
    let mut font_system = FontSystem::new();
    let mut swash_cache = SwashCache::new();
    let cache = Cache::new(&state.device);
    let mut viewport = Viewport::new(&state.device, &cache);
    let mut atlas = TextAtlas::with_color_mode(
        &state.device,
        &state.queue,
        &cache,
        TextureFormat::Bgra8Unorm,
        ColorMode::Web,
    );
    let mut text_renderer =
        TextRenderer::new(&mut atlas, &state.device, MultisampleState::default(), None);

    let attrs = Attrs::new()
        .family(Family::SansSerif)
        .weight(Weight::NORMAL);
    let shaping = Shaping::Advanced;
    viewport.update(
        &state.queue,
        Resolution {
            width: 1000,
            height: 1000,
        },
    );

    for (test_name, text_areas) in &[
        (
            "Latin - Single Text Area",
            vec![include_str!("../samples/latin.txt")],
        ),
        (
            "Arabic - Single Text Area",
            vec![include_str!("../samples/arabic.txt")],
        ),
        (
            "Latin - Many Text Areas",
            include_str!("../samples/latin.txt")
                .repeat(100)
                .split('\n')
                .collect(),
        ),
        (
            "Arabic - Many Text Areas",
            include_str!("../samples/arabic.txt")
                .repeat(20)
                .split('\n')
                .collect(),
        ),
    ] {
        let buffers: Vec<glyphon::Buffer> = text_areas
            .iter()
            .copied()
            .map(|s| {
                let mut text_buffer = Buffer::new(&mut font_system, Metrics::relative(1.0, 10.0));
                text_buffer.set_size(&mut font_system, Some(20.0), None);
                text_buffer.set_text(&mut font_system, s, &attrs, shaping);
                text_buffer.shape_until_scroll(&mut font_system, false);
                text_buffer
            })
            .collect();

        group.bench_function(*test_name, |b| {
            b.iter(|| {
                let text_areas: Vec<TextArea> = buffers
                    .iter()
                    .map(|b| TextArea {
                        buffer: b,
                        left: 0.0,
                        top: 0.0,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 1000,
                        },
                        default_color: Color::rgb(0, 0, 0),
                        color_type: TextAreaColorType::DarkOnLight,
                        custom_glyphs: &[],
                    })
                    .collect();

                criterion::black_box(
                    text_renderer
                        .prepare(
                            &state.device,
                            &state.queue,
                            &mut font_system,
                            &mut atlas,
                            &viewport,
                            text_areas,
                            &mut swash_cache,
                        )
                        .unwrap(),
                );

                atlas.trim();
            })
        });
    }
    group.finish();
}

criterion_group!(benches, run_bench);
criterion_main!(benches);
