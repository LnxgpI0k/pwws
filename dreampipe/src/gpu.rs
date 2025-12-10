use wgpu::TextureFormat;
use crate::card::Card;
use crate::error::CompositorError;
use crate::error::CompositorResult;
use crate::TEXTURE_FORMAT;
use std::collections::HashMap;

const BLIT_SHADER: &str =
   r#"struct VertexOutput {
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
}"#;

fn get_pci_ids_from_card(card_num: u32) -> Option<(u32, u32)> {
   let sys_path = format!("/sys/class/drm/card{}/device", card_num);
   let vendor = std::fs::read_to_string(format!("{}/vendor", sys_path)).ok()?;
   let device = std::fs::read_to_string(format!("{}/device", sys_path)).ok()?;
   let vendor_id =
      u32::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok()?;
   let device_id =
      u32::from_str_radix(device.trim().trim_start_matches("0x"), 16).ok()?;
   Some((vendor_id, device_id))
}

pub async fn init_card_and_gpu() -> CompositorResult<
   (Card, wgpu::Adapter, wgpu::Device, wgpu::Queue),
> {
   let mut cards: HashMap<(u32, u32), (Card, usize)> = HashMap::new();
   for card_num in 0 .. 16 {
      let card_path = format!["/dev/dri/card{card_num}"];
      if let Ok(card) = Card::open(&card_path) {
         if let Some((vendor_id, device_id)) = get_pci_ids_from_card(card_num) {
            println![
               "Opend card{card_num}: vendor=0x{vendor_id:x}, device=0x{device_id:x}"
            ];
            cards.insert((vendor_id, device_id), (card, card_num as usize));
         }
      }
   }
   let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
      backends: wgpu::Backends::VULKAN,
      ..Default::default()
   });
   let adapters = instance.enumerate_adapters(wgpu::Backends::all());
   println!["Available GPUs:"];
   for (i, adapter) in adapters.iter().enumerate() {
      let info = adapter.get_info();
      println![
         "  [{i}] {} - {:?} (Backend: {:?})",
         info.name,
         info.device_type,
         info.backend
      ];
   }
   let adapter =
      adapters
         .clone()
         .into_iter()
         .find(|a| a.get_info().device_type == wgpu::DeviceType::DiscreteGpu)
         .or_else(|| adapters.into_iter().next())
         .expect("No suitable adapter found");
   let info = adapter.get_info();
   println!["Selected GPU {} ({:?})", info.name, info.backend];
   let (drm_card, card_num) =
      cards.remove(&(info.vendor, info.device)).ok_or(CompositorError::GpuCard)?;
   println!["Using card{card_num} for modesetting"];
   drop(cards);
   let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
      label: Some("DMA-BUF Device"),
      required_features: wgpu::Features::empty(),
      required_limits: wgpu::Limits::defaults(),
      memory_hints: wgpu::MemoryHints::default(),
      experimental_features: wgpu::ExperimentalFeatures::disabled(),
      trace: wgpu::Trace::Off,
   }).await.expect("Failed to create device");
   println!["Card and GPU successfully initialized in tandem."];
   Ok((drm_card, adapter, device, queue))
}

pub fn create_pipeline(device: &wgpu::Device) {
   let mut encoder =
      device.create_command_encoder(
         &wgpu::CommandEncoderDescriptor { label: Some("Compositor Render Encoder") },
      );
   let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
      label: Some("Blit Shader"),
      source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
   });
   let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
      label: Some("Blit Pipeline"),
      layout: None,
      vertex: wgpu::VertexState {
         module: &shader,
         entry_point: Some("vs_main"),
         buffers: &[],
         compilation_options: wgpu::PipelineCompilationOptions::default(),
      },
      fragment: Some(wgpu::FragmentState {
         module: &shader,
         entry_point: Some("fs_main"),
         targets: &[Some(wgpu::ColorTargetState {
            format: TEXTURE_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
         })],
         compilation_options: wgpu::PipelineCompilationOptions::default(),
      }),
      primitive: wgpu::PrimitiveState::default(),
      depth_stencil: None,
      multisample: wgpu::MultisampleState::default(),
      multiview: None,
      cache: None,
   });
   // let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
   //    label: Some("Composite Pass"),
   //    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
   //       view: &output_view,
   //       depth_slice: None,
   //       resolve_target: None,
   //       ops: wgpu::Operations {
   //          load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
   //          store: wgpu::StoreOp::Store,
   //       },
   //    })],
   //    depth_stencil_attachment: None,
   //    timestamp_writes: None,
   //    occlusion_query_set: None,
   // });
}
