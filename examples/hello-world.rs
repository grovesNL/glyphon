use glyphon::{
    fontdue::{
        layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle},
        Font, FontSettings,
    },
    Resolution, TextRenderer,
};
use wgpu::{
    Color, CommandEncoderDescriptor, LoadOp, Operations, RenderPassColorAttachment,
    RenderPassDescriptor, TextureViewDescriptor,
};
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::Window,
};

fn main() {
    pollster::block_on(run());
}

async fn run() {
    let instance = wgpu::Instance::new(wgpu::Backends::all());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .unwrap();
    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::empty(),
                limits: wgpu::Limits::downlevel_defaults(),
            },
            None,
        )
        .await
        .unwrap();

    let event_loop = EventLoop::new();
    let window = Window::new(&event_loop).unwrap();
    let surface = unsafe { instance.create_surface(&window) };
    let size = window.inner_size();
    let swapchain_format = surface.get_preferred_format(&adapter).unwrap();
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: swapchain_format,
        width: size.width,
        height: size.height,
        present_mode: wgpu::PresentMode::Mailbox,
    };
    surface.configure(&device, &config);

    let mut text_renderer = TextRenderer::new(
        &device,
        &queue,
        swapchain_format,
        Resolution {
            width: size.width,
            height: size.height,
        },
    );

    let font = include_bytes!("./Inter-Bold.ttf") as &[u8];
    let font = Font::from_bytes(font, FontSettings::default()).unwrap();
    let fonts = vec![font];
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);

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
                layout.reset(&LayoutSettings {
                    x: 0.0,
                    y: 0.0,
                    ..LayoutSettings::default()
                });

                layout.append(
                    fonts.as_slice(),
                    &TextStyle::new("Hello world!\nI'm on a new line!", 50.0, 0),
                );

                text_renderer
                    .prepare(
                        &device,
                        &queue,
                        Resolution {
                            width: config.width,
                            height: config.height,
                        },
                        &fonts,
                        &[&layout],
                    )
                    .unwrap();

                let frame = surface.get_current_texture().unwrap();
                let view = frame.texture.create_view(&TextureViewDescriptor::default());
                let mut encoder =
                    device.create_command_encoder(&CommandEncoderDescriptor { label: None });
                {
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: None,
                        color_attachments: &[RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: Operations {
                                load: LoadOp::Clear(Color::BLACK),
                                store: true,
                            },
                        }],
                        depth_stencil_attachment: None,
                    });

                    text_renderer.render(&mut pass).unwrap();
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
