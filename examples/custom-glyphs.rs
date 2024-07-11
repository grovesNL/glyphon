use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyphDesc, CustomGlyphInput, CustomGlyphOutput,
    Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds,
    TextRenderer, Viewport,
};
use std::sync::Arc;
use wgpu::{
    CommandEncoderDescriptor, CompositeAlphaMode, DeviceDescriptor, Instance, InstanceDescriptor,
    LoadOp, MultisampleState, Operations, PresentMode, RenderPassColorAttachment,
    RenderPassDescriptor, RequestAdapterOptions, SurfaceConfiguration, TextureFormat,
    TextureUsages, TextureViewDescriptor,
};
use winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

// Example SVG icons are from https://publicdomainvectors.org/
static LION_SVG: &[u8] = include_bytes!("./lion.svg");
static EAGLE_SVG: &[u8] = include_bytes!("./eagle.svg");

fn main() {
    pollster::block_on(run());
}

async fn run() {
    // Set up window
    let (width, height) = (800, 600);
    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_inner_size(LogicalSize::new(width as f64, height as f64))
            .with_title("glyphon custom glyphs")
            .build(&event_loop)
            .unwrap(),
    );
    let size = window.inner_size();
    let scale_factor = window.scale_factor();

    // Set up surface
    let instance = Instance::new(InstanceDescriptor::default());
    let adapter = instance
        .request_adapter(&RequestAdapterOptions::default())
        .await
        .unwrap();
    let (device, queue) = adapter
        .request_device(&DeviceDescriptor::default(), None)
        .await
        .unwrap();

    let surface = instance
        .create_surface(window.clone())
        .expect("Create surface");
    let swapchain_format = TextureFormat::Bgra8UnormSrgb;
    let mut config = SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format: swapchain_format,
        width: size.width,
        height: size.height,
        present_mode: PresentMode::Fifo,
        alpha_mode: CompositeAlphaMode::Opaque,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    // Set up text renderer
    let mut font_system = FontSystem::new();
    let mut swash_cache = SwashCache::new();
    let cache = Cache::new(&device);
    let mut viewport = Viewport::new(&device, &cache);
    let mut atlas = TextAtlas::new(&device, &queue, &cache, swapchain_format);
    let mut text_renderer =
        TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
    let mut text_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));

    let physical_width = (width as f64 * scale_factor) as f32;
    let physical_height = (height as f64 * scale_factor) as f32;

    text_buffer.set_size(
        &mut font_system,
        Some(physical_width),
        Some(physical_height),
    );
    text_buffer.set_text(
        &mut font_system,
        "SVG icons!     --->\n\nThe icons below should be partially clipped.",
        Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
    );
    text_buffer.shape_until_scroll(&mut font_system, false);

    // Set up custom svg renderer

    let svg_0 = resvg::usvg::Tree::from_data(LION_SVG, &Default::default()).unwrap();
    let svg_1 = resvg::usvg::Tree::from_data(EAGLE_SVG, &Default::default()).unwrap();

    let rasterize_svg = move |input: CustomGlyphInput| -> Option<CustomGlyphOutput> {
        // Select the svg data based on the custom glyph ID.
        let (svg, content_type) = match input.id {
            0 => (&svg_0, ContentType::Mask),
            1 => (&svg_1, ContentType::Color),
            _ => return None,
        };

        // Calculate the scale based on the "font size".
        let svg_size = svg.size();
        let max_side_len = svg_size.width().max(svg_size.height());
        let scale = input.size / max_side_len;

        // Create a buffer to write pixels to.
        let width = (svg_size.width() * scale).ceil() as u32;
        let height = (svg_size.height() * scale).ceil() as u32;
        let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(width, height) else {
            return None;
        };

        let mut transform = resvg::usvg::Transform::from_scale(scale, scale);

        // Offset the glyph by the subpixel amount.
        let offset_x = input.x_bin.as_float();
        let offset_y = input.y_bin.as_float();
        if offset_x != 0.0 || offset_y != 0.0 {
            transform = transform.post_translate(offset_x, offset_y);
        }

        resvg::render(svg, transform, &mut pixmap.as_mut());

        let data: Vec<u8> = if let ContentType::Mask = content_type {
            // Only use the alpha channel for symbolic icons.
            pixmap.data().iter().skip(3).step_by(4).copied().collect()
        } else {
            pixmap.data().to_vec()
        };

        Some(CustomGlyphOutput {
            data,
            width,
            height,
            content_type,
        })
    };

    event_loop
        .run(move |event, target| {
            if let Event::WindowEvent {
                window_id: _,
                event,
            } = event
            {
                match event {
                    WindowEvent::Resized(size) => {
                        config.width = size.width;
                        config.height = size.height;
                        surface.configure(&device, &config);
                        window.request_redraw();
                    }
                    WindowEvent::RedrawRequested => {
                        viewport.update(
                            &queue,
                            Resolution {
                                width: config.width,
                                height: config.height,
                            },
                        );

                        text_renderer
                            .prepare(
                                &device,
                                &queue,
                                &mut font_system,
                                &mut atlas,
                                &viewport,
                                [TextArea {
                                    buffer: &text_buffer,
                                    left: 10.0,
                                    top: 10.0,
                                    scale: 1.0,
                                    bounds: TextBounds {
                                        left: 0,
                                        top: 0,
                                        right: 650,
                                        bottom: 180,
                                    },
                                    default_color: Color::rgb(255, 255, 255),
                                    custom_glyphs: &[
                                        CustomGlyphDesc {
                                            id: 0,
                                            left: 300.0,
                                            top: 5.0,
                                            size: 64.0,
                                            color: Some(Color::rgb(200, 200, 255)),
                                            metadata: 0,
                                        },
                                        CustomGlyphDesc {
                                            id: 1,
                                            left: 400.0,
                                            top: 5.0,
                                            size: 64.0,
                                            color: None,
                                            metadata: 0,
                                        },
                                        CustomGlyphDesc {
                                            id: 0,
                                            left: 300.0,
                                            top: 130.0,
                                            size: 64.0,
                                            color: Some(Color::rgb(200, 255, 200)),
                                            metadata: 0,
                                        },
                                        CustomGlyphDesc {
                                            id: 1,
                                            left: 400.0,
                                            top: 130.0,
                                            size: 64.0,
                                            color: None,
                                            metadata: 0,
                                        },
                                    ],
                                }],
                                &mut swash_cache,
                                |input| rasterize_svg(input),
                            )
                            .unwrap();

                        let frame = surface.get_current_texture().unwrap();
                        let view = frame.texture.create_view(&TextureViewDescriptor::default());
                        let mut encoder = device
                            .create_command_encoder(&CommandEncoderDescriptor { label: None });
                        {
                            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                                label: None,
                                color_attachments: &[Some(RenderPassColorAttachment {
                                    view: &view,
                                    resolve_target: None,
                                    ops: Operations {
                                        load: LoadOp::Clear(wgpu::Color {
                                            r: 0.02,
                                            g: 0.02,
                                            b: 0.02,
                                            a: 1.0,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });

                            text_renderer.render(&atlas, &viewport, &mut pass).unwrap();
                        }

                        queue.submit(Some(encoder.finish()));
                        frame.present();

                        atlas.trim();
                    }
                    WindowEvent::CloseRequested => target.exit(),
                    _ => {}
                }
            }
        })
        .unwrap();
}
