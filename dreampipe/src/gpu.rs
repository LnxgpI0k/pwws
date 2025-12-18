use crate::context::Card;
use crate::error::CompositorResult;
use image::GenericImageView;
use wgpu::AddressMode;
use wgpu::BindGroup;
use wgpu::BindGroupDescriptor;
use wgpu::BindGroupEntry;
use wgpu::BindGroupLayoutDescriptor;
use wgpu::BindGroupLayoutEntry;
use wgpu::BindingResource;
use wgpu::BindingType;
use wgpu::Extent3d;
use wgpu::FilterMode;
use wgpu::Origin3d;
use wgpu::SamplerBindingType;
use wgpu::SamplerDescriptor;
use wgpu::ShaderStages;
use wgpu::TexelCopyBufferLayout;
use wgpu::TexelCopyTextureInfo;
use wgpu::TextureAspect;
use wgpu::TextureDescriptor;
use wgpu::TextureDimension;
use wgpu::TextureFormat;
use wgpu::TextureSampleType;
use wgpu::TextureUsages;
use wgpu::TextureViewDescriptor;
use wgpu::TextureViewDimension;

const TEXTURE_FORMAT: TextureFormat = TextureFormat::Rgba8Uint;
const BLIT_SHADER: &str = include_str!["blit.wgsl"];
#[cfg(not(feature = "expanding"))]
const BG_BYTES: &'static [u8] = include_bytes!["../mambutt.png"];
#[cfg(feature = "expanding")]
const BG_BYTES: &'static [u8] = &[];

pub fn load_default_bg(
  card: &Card,
  gpu: &wgpu::Device,
  queue: &wgpu::Queue,
) -> BindGroup {
  // This cfg is for display purposes when concatenating the entire project together.
  let bg =
    image::load_from_memory_with_format(BG_BYTES, image::ImageFormat::Png).unwrap();
  let rgba = bg.to_rgba8();
  let (width, height) = bg.dimensions();
  let size = Extent3d {
    width,
    height,
    depth_or_array_layers: 1,
  };
  let pci_ids = card.pci_ids();
  let texture = gpu.create_texture(&TextureDescriptor {
    label: Some(&format!["Default BG {pci_ids:x?}"]),
    size,
    mip_level_count: 1,
    sample_count: 1,
    dimension: TextureDimension::D2,
    format: TextureFormat::Bgra8UnormSrgb,
    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    view_formats: &[],
  });
  queue.write_texture(TexelCopyTextureInfo {
    texture: &texture,
    mip_level: 0,
    origin: Origin3d::ZERO,
    aspect: TextureAspect::All,
  }, &rgba, TexelCopyBufferLayout {
    offset: 0,
    bytes_per_row: Some(4 * width),
    rows_per_image: Some(height),
  }, size);
  let texture_view = texture.create_view(&TextureViewDescriptor::default());
  let sampler = gpu.create_sampler(&SamplerDescriptor {
    label: Some(&format!["Default BG Sampler {pci_ids:x?}"]),
    address_mode_u: AddressMode::ClampToEdge,
    address_mode_v: AddressMode::ClampToEdge,
    address_mode_w: AddressMode::ClampToEdge,
    mag_filter: FilterMode::Linear,
    min_filter: FilterMode::Nearest,
    mipmap_filter: FilterMode::Nearest,
    ..Default::default()
  });
  let bindgroup_layout = gpu.create_bind_group_layout(&BindGroupLayoutDescriptor {
    label: Some(&format!["Default Bg Bindgroup Layout {pci_ids:x?}"]),
    entries: &[BindGroupLayoutEntry {
      binding: 0,
      visibility: ShaderStages::FRAGMENT,
      ty: BindingType::Texture {
        sample_type: TextureSampleType::Float { filterable: true },
        view_dimension: TextureViewDimension::D2,
        multisampled: false,
      },
      count: None,
    }, BindGroupLayoutEntry {
      binding: 1,
      visibility: ShaderStages::FRAGMENT,
      ty: BindingType::Sampler(SamplerBindingType::Filtering),
      count: None,
    }],
  });
  let bind_group = gpu.create_bind_group(&BindGroupDescriptor {
    label: Some(&format!["Default BG Bindgroup {pci_ids:x?}"]),
    layout: &bindgroup_layout,
    entries: &[BindGroupEntry {
      binding: 0,
      resource: BindingResource::TextureView(&texture_view),
    }, BindGroupEntry {
      binding: 1,
      resource: BindingResource::Sampler(&sampler),
    }],
  });
  bind_group
}

pub async fn init_gpu(
  card: &Card,
) -> CompositorResult<(wgpu::Device, wgpu::Adapter, wgpu::Queue)> {
  if let Some((vendor_id, device_id)) = card.pci_ids() {
    let card_num = card.num();
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
  Ok((device, adapter, queue))
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
