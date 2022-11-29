struct VertexInput {
    @builtin(vertex_index) vertex_idx: u32,
    @location(0) pos: vec2<i32>,
    @location(1) dim: u32,
    @location(2) uv: u32,
    @location(3) color: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

struct Params {
    screen_resolution: vec2<u32>,
    _pad: vec2<u32>,
};

@group(0) @binding(0)
var<uniform> params: Params;

@group(0) @binding(1)
var atlas_texture: texture_2d<f32>;

@group(0) @binding(2)
var atlas_sampler: sampler;

@vertex
fn vs_main(in_vert: VertexInput) -> VertexOutput {
    var pos = in_vert.pos;
    let width = in_vert.dim & 0xffffu;
    let height = (in_vert.dim & 0xffff0000u) >> 16u;
    let color = in_vert.color;
    var uv = vec2<u32>(in_vert.uv & 0xffffu, (in_vert.uv & 0xffff0000u) >> 16u);
    let v = in_vert.vertex_idx % 4u;

    switch v {
        case 1u: {
            pos.x += i32(width);
            uv.x += width;
        }
        case 2u: {
            pos.x += i32(width);
            pos.y += i32(height);
            uv.x += width;
            uv.y += height;
        }
        case 3u: {
            pos.y += i32(height);
            uv.y += height;
        }
        default: {}
    }

    var vert_output: VertexOutput;

    vert_output.position = vec4<f32>(
        2.0 * vec2<f32>(pos) / vec2<f32>(params.screen_resolution) - 1.0,
        0.0,
        1.0,
    );

    vert_output.position.y *= -1.0;

    vert_output.color = vec4<f32>(
        f32((color & 0xffu)),
        f32((color & 0xff00u) >> 8u),
        f32((color & 0xff0000u) >> 16u),
        f32((color & 0xff000000u) >> 24u),
    ) / 255.0;

    vert_output.uv = vec2<f32>(uv) / vec2<f32>(textureDimensions(atlas_texture).xy);

    return vert_output;
}

@fragment
fn fs_main(in_frag: VertexOutput) -> @location(0) vec4<f32> {
    return in_frag.color * textureSample(atlas_texture, atlas_sampler, in_frag.uv).x;
}
