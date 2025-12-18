use wgpu::{BackendOptions, Dx12BackendOptions};

use pollster::block_on;

pub struct State {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl State {
    pub fn new() -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            memory_budget_thresholds: Default::default(),
            flags: wgpu::InstanceFlags::empty(),
            backend_options: BackendOptions {
                gl: wgpu::GlBackendOptions {
                    gles_minor_version: wgpu::Gles3MinorVersion::Automatic,
                    ..Default::default()
                },
                dx12: Dx12BackendOptions {
                    shader_compiler: wgpu::Dx12Compiler::Fxc,
                    ..Default::default()
                },
                ..Default::default()
            },
        });

        let adapter = block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .unwrap();

        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Benchmark Device"),
            required_features: adapter.features(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        }))
        .unwrap();

        Self { device, queue }
    }
}
