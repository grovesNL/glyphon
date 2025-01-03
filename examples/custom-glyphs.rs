#[cfg(feature = "egui")]
use egui_wgpu::wgpu as WPGU;
#[cfg(not(feature = "egui"))]
use wgpu as WPGU;

use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};
use std::sync::Arc;
use winit::{dpi::LogicalSize, event::WindowEvent, event_loop::EventLoop, window::Window};
use WPGU::{
    CommandEncoderDescriptor, CompositeAlphaMode, Device, DeviceDescriptor, Instance,
    InstanceDescriptor, LoadOp, MultisampleState, Operations, PresentMode, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RequestAdapterOptions, StoreOp, Surface,
    SurfaceConfiguration, TextureFormat, TextureUsages, TextureViewDescriptor,
};

// Example SVG icons are from https://publicdomainvectors.org/
static LION_SVG: &[u8] = include_bytes!("./lion.svg");
static EAGLE_SVG: &[u8] = include_bytes!("./eagle.svg");

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop
        .run_app(&mut Application { window_state: None })
        .unwrap();
}

struct WindowState {
    device: Device,
    queue: Queue,
    surface: Surface<'static>,
    surface_config: SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: glyphon::Viewport,
    atlas: glyphon::TextAtlas,
    text_renderer: glyphon::TextRenderer,
    text_buffer: glyphon::Buffer,
    rasterize_svg: Box<dyn Fn(RasterizeCustomGlyphRequest) -> Option<RasterizedCustomGlyph>>,
    // Make sure that the winit window is last in the struct so that
    // it is dropped after the wgpu surface is dropped, otherwise the
    // program may crash when closed. This is probably a bug in wgpu.
    window: Arc<Window>,
}

impl WindowState {
    async fn new(window: Arc<Window>) -> Self {
        let physical_size = window.inner_size();
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
        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: swapchain_format,
            width: physical_size.width,
            height: physical_size.height,
            present_mode: PresentMode::Fifo,
            alpha_mode: CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // Set up text renderer
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, swapchain_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let mut text_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));

        let physical_width = (physical_size.width as f64 * scale_factor) as f32;
        let physical_height = (physical_size.height as f64 * scale_factor) as f32;

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

        let rasterize_svg =
            move |input: RasterizeCustomGlyphRequest| -> Option<RasterizedCustomGlyph> {
                // Select the svg data based on the custom glyph ID.
                let (svg, content_type) = match input.id {
                    0 => (&svg_0, ContentType::Mask),
                    1 => (&svg_1, ContentType::Color),
                    _ => return None,
                };

                // Calculate the scale based on the "glyph size".
                let svg_size = svg.size();
                let scale_x = input.width as f32 / svg_size.width();
                let scale_y = input.height as f32 / svg_size.height();

                let mut pixmap =
                    resvg::tiny_skia::Pixmap::new(input.width as u32, input.height as u32)?;

                let mut transform = resvg::usvg::Transform::from_scale(scale_x, scale_y);

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

                Some(RasterizedCustomGlyph { data, content_type })
            };

        Self {
            device,
            queue,
            surface,
            surface_config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_buffer,
            rasterize_svg: Box::new(rasterize_svg),
            window,
        }
    }
}

struct Application {
    window_state: Option<WindowState>,
}

impl winit::application::ApplicationHandler for Application {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window_state.is_some() {
            return;
        }

        // Set up window
        let (width, height) = (800, 600);
        let window_attributes = Window::default_attributes()
            .with_inner_size(LogicalSize::new(width as f64, height as f64))
            .with_title("glyphon hello world");
        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        self.window_state = Some(pollster::block_on(WindowState::new(window)));
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.window_state else {
            return;
        };

        let WindowState {
            window,
            device,
            queue,
            surface,
            surface_config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_buffer,
            rasterize_svg,
            ..
        } = state;

        match event {
            WindowEvent::Resized(size) => {
                surface_config.width = size.width;
                surface_config.height = size.height;
                surface.configure(device, surface_config);
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                viewport.update(
                    queue,
                    Resolution {
                        width: surface_config.width,
                        height: surface_config.height,
                    },
                );

                text_renderer
                    .prepare_with_custom(
                        device,
                        queue,
                        font_system,
                        atlas,
                        viewport,
                        [TextArea {
                            buffer: text_buffer,
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
                                CustomGlyph {
                                    id: 0,
                                    left: 300.0,
                                    top: 5.0,
                                    width: 64.0,
                                    height: 64.0,
                                    color: Some(Color::rgb(200, 200, 255)),
                                    snap_to_physical_pixel: true,
                                    metadata: 0,
                                },
                                CustomGlyph {
                                    id: 1,
                                    left: 400.0,
                                    top: 5.0,
                                    width: 64.0,
                                    height: 64.0,
                                    color: None,
                                    snap_to_physical_pixel: true,
                                    metadata: 0,
                                },
                                CustomGlyph {
                                    id: 0,
                                    left: 300.0,
                                    top: 130.0,
                                    width: 64.0,
                                    height: 64.0,
                                    color: Some(Color::rgb(200, 255, 200)),
                                    snap_to_physical_pixel: true,
                                    metadata: 0,
                                },
                                CustomGlyph {
                                    id: 1,
                                    left: 400.0,
                                    top: 130.0,
                                    width: 64.0,
                                    height: 64.0,
                                    color: None,
                                    snap_to_physical_pixel: true,
                                    metadata: 0,
                                },
                            ],
                        }],
                        swash_cache,
                        rasterize_svg,
                    )
                    .unwrap();

                let frame = surface.get_current_texture().unwrap();
                let view = frame.texture.create_view(&TextureViewDescriptor::default());
                let mut encoder =
                    device.create_command_encoder(&CommandEncoderDescriptor { label: None });
                {
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: Operations {
                                load: LoadOp::Clear(WPGU::Color {
                                    r: 0.02,
                                    g: 0.02,
                                    b: 0.02,
                                    a: 1.0,
                                }),
                                store: StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    text_renderer.render(atlas, viewport, &mut pass).unwrap();
                }

                queue.submit(Some(encoder.finish()));
                frame.present();

                atlas.trim();
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }
}
