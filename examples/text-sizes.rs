use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};
use std::sync::Arc;
use wgpu::{
    CommandEncoderDescriptor, CompositeAlphaMode, DeviceDescriptor, Instance, InstanceDescriptor,
    LoadOp, MultisampleState, Operations, PresentMode, RenderPassColorAttachment,
    RenderPassDescriptor, RequestAdapterOptions, SurfaceConfiguration, TextureFormat,
    TextureUsages, TextureViewDescriptor,
};
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    event::WindowEvent,
    event_loop::EventLoop,
    window::Window,
};

const TEXT: &str = "The quick brown fox jumped over the lazy doggo. 🐕";
const WEIGHT: Weight = Weight::NORMAL;
const SIZES: [f32; 16] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 32.0, 48.0,
];
const LINE_HEIGHT: f32 = 1.15;
const BG_COLOR: wgpu::Color = wgpu::Color::WHITE;
const FONT_COLOR: Color = Color::rgb(0, 0, 0);
//const BG_COLOR: wgpu::Color = wgpu::Color::BLACK;
//const FONT_COLOR: Color = Color::rgb(255, 255, 255);
const USE_WEB_COLORS: bool = true;

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop
        .run_app(&mut Application { window_state: None })
        .unwrap();
}

struct WindowState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: SurfaceConfiguration,
    physical_size: PhysicalSize<i32>,
    scale_factor: f32,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: glyphon::Viewport,
    atlas: glyphon::TextAtlas,
    text_renderer: glyphon::TextRenderer,
    buffers: Vec<glyphon::Buffer>,

    // Make sure that the winit window is last in the struct so that
    // it is dropped after the wgpu surface is dropped, otherwise the
    // program may crash when closed. This is probably a bug in wgpu.
    window: Arc<Window>,
}

impl WindowState {
    async fn new(window: Arc<Window>) -> Self {
        let physical_size = window.inner_size();
        let scale_factor = window.scale_factor() as f32;

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

        let (color_mode, swapchain_format) = if USE_WEB_COLORS {
            (ColorMode::Web, TextureFormat::Bgra8Unorm)
        } else {
            (ColorMode::Accurate, TextureFormat::Bgra8UnormSrgb)
        };

        let surface = instance
            .create_surface(window.clone())
            .expect("Create surface");
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

        let logical_width = physical_size.width as f32 / scale_factor;

        // Set up text renderer
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas =
            TextAtlas::with_color_mode(&device, &queue, &cache, swapchain_format, color_mode);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);

        let attrs = Attrs::new().family(Family::SansSerif).weight(WEIGHT);
        let shaping = Shaping::Advanced;

        let buffers: Vec<glyphon::Buffer> = SIZES
            .iter()
            .copied()
            .map(|s| {
                let mut text_buffer =
                    Buffer::new(&mut font_system, Metrics::relative(s, LINE_HEIGHT));

                text_buffer.set_size(&mut font_system, Some(logical_width - 20.0), None);

                text_buffer.set_text(
                    &mut font_system,
                    &format!("size {s}: {TEXT}"),
                    attrs,
                    shaping,
                );

                text_buffer.shape_until_scroll(&mut font_system, false);

                text_buffer
            })
            .collect();

        Self {
            device,
            queue,
            surface,
            surface_config,
            physical_size: physical_size.cast(),
            scale_factor: scale_factor as f32,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            buffers,
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
            buffers,
            scale_factor,
            physical_size,
            ..
        } = state;

        match event {
            WindowEvent::Resized(size) => {
                surface_config.width = size.width;
                surface_config.height = size.height;
                surface.configure(&device, &surface_config);
                window.request_redraw();

                *scale_factor = window.scale_factor() as f32;
                *physical_size = size.cast();

                let logical_width = size.width as f32 / *scale_factor;

                for b in buffers.iter_mut() {
                    b.set_size(font_system, Some(logical_width - 20.0), None);
                    b.shape_until_scroll(font_system, false);
                }
            }
            WindowEvent::RedrawRequested => {
                viewport.update(
                    &queue,
                    Resolution {
                        width: surface_config.width,
                        height: surface_config.height,
                    },
                );

                let scale_factor = *scale_factor;

                let left = 10.0 * scale_factor;
                let mut top = 10.0 * scale_factor;

                let bounds_left = left.floor() as i32;
                let bounds_right = physical_size.width - 10;

                let text_areas: Vec<TextArea> = buffers
                    .iter()
                    .map(|b| {
                        let a = TextArea {
                            buffer: b,
                            left,
                            top,
                            scale: scale_factor,
                            bounds: TextBounds {
                                left: bounds_left,
                                top: top.floor() as i32,
                                right: bounds_right,
                                bottom: top.floor() as i32 + physical_size.height,
                            },
                            default_color: FONT_COLOR,
                        };

                        let total_lines = b
                            .layout_runs()
                            .fold(0usize, |total_lines, _| total_lines + 1);

                        top += (total_lines as f32 * b.metrics().line_height + 5.0) * scale_factor;

                        a
                    })
                    .collect();

                text_renderer
                    .prepare(
                        device,
                        queue,
                        font_system,
                        atlas,
                        viewport,
                        text_areas,
                        swash_cache,
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
                                load: LoadOp::Clear(BG_COLOR),
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
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }
}
