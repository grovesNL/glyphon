use pollster::block_on;

pub struct State {
    pub device: egui_wgpu::wgpu::Device,
    pub queue: egui_wgpu::wgpu::Queue,
}

impl State {
    pub fn new() -> Self {
        let instance = egui_wgpu::wgpu::Instance::new(egui_wgpu::wgpu::InstanceDescriptor {
            backends: egui_wgpu::wgpu::Backends::all(),
            flags: egui_wgpu::wgpu::InstanceFlags::empty(),
            dx12_shader_compiler: egui_wgpu::wgpu::Dx12Compiler::Fxc,
            gles_minor_version: egui_wgpu::wgpu::Gles3MinorVersion::Automatic,
        });

        let adapter = block_on(
            egui_wgpu::wgpu::util::initialize_adapter_from_env_or_default(&instance, None),
        )
        .unwrap();

        let (device, queue) = block_on(adapter.request_device(
            &egui_wgpu::wgpu::DeviceDescriptor {
                label: Some("Benchmark Device"),
                required_features: adapter.features(),
                required_limits: adapter.limits(),
                memory_hints: egui_wgpu::wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .unwrap();

        Self { device, queue }
    }
}
