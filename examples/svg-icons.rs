use glyphon::{
    icon::{IconDesc, IconRenderer, IconSourceID, IconSystem},
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
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
            .with_title("glyphon svg icons")
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
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));

    let physical_width = (width as f64 * scale_factor) as f32;
    let physical_height = (height as f64 * scale_factor) as f32;

    buffer.set_size(&mut font_system, physical_width, physical_height);
    buffer.set_text(
        &mut font_system,
        "SVG icons!     --->\n\nThe icons below should be partially clipped.",
        Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut font_system, false);

    // Set up icon renderer
    let mut icon_system = IconSystem::new();
    let mut icon_renderer =
        IconRenderer::new(&mut atlas, &device, MultisampleState::default(), None);

    // Add SVG sources to the icon system.
    icon_system.add_svg(
        IconSourceID(0),
        resvg::usvg::Tree::from_data(LION_SVG, &Default::default()).unwrap(),
        true,
    );
    icon_system.add_svg(
        IconSourceID(1),
        resvg::usvg::Tree::from_data(EAGLE_SVG, &Default::default()).unwrap(),
        false,
    );

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

                        let bounds = TextBounds {
                            left: 0,
                            top: 0,
                            right: 650,
                            bottom: 180,
                        };

                        text_renderer
                            .prepare(
                                &device,
                                &queue,
                                &mut font_system,
                                &mut atlas,
                                &viewport,
                                [TextArea {
                                    buffer: &buffer,
                                    left: 10.0,
                                    top: 10.0,
                                    scale: 1.0,
                                    bounds,
                                    default_color: Color::rgb(255, 255, 255),
                                }],
                                &mut swash_cache,
                            )
                            .unwrap();

                        icon_renderer
                            .prepare(
                                &device,
                                &queue,
                                &mut icon_system,
                                &mut font_system,
                                &mut atlas,
                                &viewport,
                                [
                                    IconDesc {
                                        id: IconSourceID(0),
                                        size: 64.0,
                                        left: 300,
                                        top: 15,
                                        color: Color::rgb(200, 200, 255),
                                        bounds,
                                        metadata: 0,
                                    },
                                    IconDesc {
                                        id: IconSourceID(1),
                                        size: 64.0,
                                        left: 400,
                                        top: 15,
                                        color: Color::rgb(255, 255, 255),
                                        bounds,
                                        metadata: 0,
                                    },
                                    IconDesc {
                                        id: IconSourceID(0),
                                        size: 64.0,
                                        left: 300,
                                        top: 140,
                                        color: Color::rgb(200, 255, 200),
                                        bounds,
                                        metadata: 0,
                                    },
                                    IconDesc {
                                        id: IconSourceID(1),
                                        size: 64.0,
                                        left: 400,
                                        top: 140,
                                        color: Color::rgb(255, 255, 255),
                                        bounds,
                                        metadata: 0,
                                    },
                                ],
                                &mut swash_cache,
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
                            icon_renderer.render(&atlas, &viewport, &mut pass).unwrap();
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
