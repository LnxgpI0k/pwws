use crate::config::CompositorConfig;
use crate::config::Config;
use crate::util::DisplayPosition;
use crate::card::Card;
use crate::display::Display;
use crate::error::CompositorResult;
use drm::control::AtomicCommitFlags;
use drm::control::Device as ControlDevice;
use drm::control::atomic;
use drm::control::connector;
use std::collections::HashSet;
use taffy::NodeId;
use taffy::TaffyTree;
use wgpu::TextureFormat;

const TEXTURE_FORMAT: TextureFormat = TextureFormat::Rgba8Uint;
const BLIT_SHADER: &str = include_str!["blit.wgsl"];

fn get_pci_ids_from_card(card_num: u32) -> Option<(u32, u32)> {
  let sys_path = format!("/sys/class/drm/card{}/device", card_num);
  let vendor = std::fs::read_to_string(format!("{sys_path}/vendor")).ok()?;
  let device = std::fs::read_to_string(format!("{sys_path}/device")).ok()?;
  let vendor_id =
    u32::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok()?;
  let device_id =
    u32::from_str_radix(device.trim().trim_start_matches("0x"), 16).ok()?;
  Some((vendor_id, device_id))
}

pub async fn init_gpu(
  card: &Card,
) -> CompositorResult<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
  let card_num = card.num();
  if let Some((vendor_id, device_id)) = get_pci_ids_from_card(card_num) {
    println!["Opend card{card_num}: vendor=0x{vendor_id:x}, device=0x{device_id:x}"];
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
  let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
    label: Some("DMA-BUF Device"),
    required_features: wgpu::Features::empty(),
    required_limits: wgpu::Limits::defaults(),
    memory_hints: wgpu::MemoryHints::default(),
    experimental_features: wgpu::ExperimentalFeatures::disabled(),
    trace: wgpu::Trace::Off,
  }).await.expect("Failed to create device");
  println!["Card and GPU successfully initialized in tandem."];
  Ok((adapter, device, queue))
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
