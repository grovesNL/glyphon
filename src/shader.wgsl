struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) tex_coords: vec2<f32>,
};

struct Params {
    screen_resolution: vec2<u32>,
};

@group(0) @binding(0)
var<uniform> params: Params;

@group(0) @binding(1)
var atlas_texture: texture_2d<f32>;

@group(0) @binding(2)
var atlas_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vertex_idx: u32, @location(0) in_vert: vec4<f32>, @location(1) tex_coords: vec2<f32>, @location(2) color: u32) -> VertexOutput {
    let width = in_vert.z;
    let height = in_vert.w;

    let v = vertex_idx % 4u;
    var pos = in_vert.xy;
    var uv = tex_coords;

    switch v {
        case 0u: {
        }
        case 1u: {
            pos.x += width;
            uv.x += width;
        }
        case 2u: {
            pos.x += width;
            pos.y += height;
            uv.x += width;
            uv.y += height;
        }
        case 3u: {
            pos.y += height;
            uv.y += height;
        }
        default: {}
    }

    pos = 2.0 * pos / vec2<f32>(params.screen_resolution) - 1.0;
    pos.y *= -1.0;

    var vert_output: VertexOutput;

    vert_output.position = vec4<f32>(pos.xy, 0.0, 1.0);

    vert_output.color = vec4<f32>(
        f32((color & 0xffu)),
        f32((color & 0xff00u) >> 8u),
        f32((color & 0xff0000u) >> 16u),
        f32((color & 0xff000000u) >> 24u),
    ) / 255.0;

    vert_output.tex_coords = uv / vec2<f32>(textureDimensions(atlas_texture).xy);

    return vert_output;
}

@fragment
fn fs_main(in_frag: VertexOutput) -> @location(0) vec4<f32> {
    return in_frag.color * textureSample(atlas_texture, atlas_sampler, in_frag.tex_coords).x;
}
