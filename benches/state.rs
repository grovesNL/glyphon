use pollster::block_on;

pub struct State {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl State {
    pub fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::empty(),
            dx12_shader_compiler: wgpu::Dx12Compiler::Fxc,
            gles_minor_version: wgpu::Gles3MinorVersion::Automatic,
        });

        let adapter = block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .unwrap();

        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Benchmark Device"),
                required_features: adapter.features(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .unwrap();

        Self { device, queue }
    }
}
