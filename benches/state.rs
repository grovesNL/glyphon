#[cfg(feature = "egui")]
use egui_wgpu::wgpu as WPGU;
#[cfg(not(feature = "egui"))]
use wgpu as WPGU;

use WPGU::{
    util, Backends, Device, DeviceDescriptor, Dx12Compiler, Gles3MinorVersion, Instance,
    InstanceDescriptor, InstanceFlags, MemoryHints, Queue,
};

use pollster::block_on;

pub struct State {
    pub device: Device,
    pub queue: Queue,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::all(),
            flags: InstanceFlags::empty(),
            dx12_shader_compiler: Dx12Compiler::Fxc,
            gles_minor_version: Gles3MinorVersion::Automatic,
        });

        let adapter = block_on(util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .unwrap();

        let (device, queue) = block_on(adapter.request_device(
            &DeviceDescriptor {
                label: Some("Benchmark Device"),
                required_features: adapter.features(),
                required_limits: adapter.limits(),
                memory_hints: MemoryHints::Performance,
            },
            None,
        ))
        .unwrap();

        Self { device, queue }
    }
}
