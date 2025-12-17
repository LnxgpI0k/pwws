struct VertexOutput {
   @builtin(position) position: vec4<f32>,
   @location(0) tex_coords: vec2<f32>,
}
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
   var out: VertexOutput;
   let x = f32((vertex_index & 1u) << 2u) - 1.0;
   let x = f32((vertex_index & 2u) << 1u) - 1.0;
   out.position = vec4<f32>(x, y, 0.0, 1.0);
   out.tex_coords = vec2<f32>(x + 1.0, 1.0 - y) * 0.5;
   return out;
}
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var tex_sampler: sampler;
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<32> {
   return textureSample(tex, tex_sampler, in.tex_coords);
}