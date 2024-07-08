use crate::{Cache, Params, Resolution, PARAMS_LEN, PARAMS_LEN_NONZERO};
use std::{mem, slice};
use wgpu::{BindGroup, Buffer, BufferDescriptor, BufferUsages, Device, Queue};

#[derive(Debug)]
pub struct Viewport {
    params: Params,
    params_buffer: Buffer,
    pub(crate) bind_group: BindGroup,
}

impl Viewport {
    pub fn new(device: &Device, cache: &Cache) -> Self {
        let params = Params {
            screen_resolution: Resolution {
                width: 0,
                height: 0,
            },
            _pad: [0, 0],
        };

        let params_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("glyphon params"),
            size: mem::size_of::<Params>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = cache.create_uniforms_bind_group(device, &params_buffer);

        Self {
            params,
            params_buffer,
            bind_group,
        }
    }

    pub fn update(&mut self, queue: &Queue, resolution: Resolution) {
        if self.params.screen_resolution == resolution {
            return;
        }

        self.params.screen_resolution = resolution;

        if let Some(mut view) = queue.write_buffer_with(&self.params_buffer, 0, PARAMS_LEN_NONZERO)
        {
            let params_raw = unsafe {
                slice::from_raw_parts(&self.params as *const Params as *const u8, PARAMS_LEN)
            };

            view.copy_from_slice(params_raw);
        }
    }

    pub fn resolution(&self) -> Resolution {
        self.params.screen_resolution
    }
}
