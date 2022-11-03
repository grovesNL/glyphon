use cosmic_text::{Attrs, Buffer, Color, FontSystem, Metrics, SwashCache};
use glyphon::{Resolution, TextAtlas, TextRenderer};
use wgpu::{
    Backends, CommandEncoderDescriptor, CompositeAlphaMode, DeviceDescriptor, Features, Instance,
    Limits, LoadOp, Operations, PresentMode, RenderPassColorAttachment, RenderPassDescriptor,
    RequestAdapterOptions, SurfaceConfiguration, TextureFormat, TextureUsages,
    TextureViewDescriptor,
};
use winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

fn main() {
    pollster::block_on(run());
}

static mut FONT_SYSTEM: Option<FontSystem> = None;

async fn run() {
    // Set up window
    let (width, height) = (800, 600);
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_inner_size(LogicalSize::new(width as f64, height as f64))
        .with_title("glyphon hello world")
        .build(&event_loop)
        .unwrap();
    let size = window.inner_size();
    let scale_factor = window.scale_factor();

    // Set up surface
    let instance = Instance::new(Backends::all());
    let adapter = instance
        .request_adapter(&RequestAdapterOptions::default())
        .await
        .unwrap();
    let (device, queue) = adapter
        .request_device(
            &DeviceDescriptor {
                label: None,
                features: Features::empty(),
                limits: Limits::downlevel_defaults(),
            },
            None,
        )
        .await
        .unwrap();
    let surface = unsafe { instance.create_surface(&window) };
    // TODO: handle srgb
    let swapchain_format = TextureFormat::Bgra8Unorm;
    let mut config = SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format: swapchain_format,
        width: size.width,
        height: size.height,
        present_mode: PresentMode::Mailbox,
        alpha_mode: CompositeAlphaMode::Opaque,
    };
    surface.configure(&device, &config);

    // Set up text renderer
    let mut text_renderer = TextRenderer::new(&device, &queue);
    unsafe {
        FONT_SYSTEM = Some(FontSystem::new());
    }
    let mut cache = SwashCache::new(unsafe { FONT_SYSTEM.as_ref().unwrap() });
    let mut atlas = TextAtlas::new(&device, &queue, swapchain_format);
    let mut buffer = Buffer::new(
        unsafe { FONT_SYSTEM.as_ref().unwrap() },
        Metrics::new(32, 44),
    );
    buffer.set_size((width as f64 * scale_factor) as i32, (height as f64) as i32);
    buffer.set_text(include_str!("./ligature.txt"), Attrs::new());
    buffer.shape_until_scroll();

    event_loop.run(move |event, _, control_flow| {
        let _ = (&instance, &adapter);

        *control_flow = ControlFlow::Poll;
        match event {
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                config.width = size.width;
                config.height = size.height;
                surface.configure(&device, &config);
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                text_renderer
                    .prepare(
                        &device,
                        &queue,
                        &mut atlas,
                        Resolution {
                            width: config.width,
                            height: config.height,
                        },
                        &mut buffer,
                        Color::rgb(255, 255, 255),
                        &mut cache,
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
                                load: LoadOp::Clear(wgpu::Color::BLACK),
                                store: true,
                            },
                        })],
                        depth_stencil_attachment: None,
                    });

                    text_renderer.render(&atlas, &mut pass).unwrap();
                }

                queue.submit(Some(encoder.finish()));
                frame.present();
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::MainEventsCleared => {
                window.request_redraw();
            }
            _ => {}
        }
    });
}
