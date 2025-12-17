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

pub struct GpuContext {
  pub card: Box<Card>,
  pub gbm: gbm::Device<&'static Card>,
  // pub bg: BackgroundImage,
  pub displays: Vec<Display>,
}

impl GpuContext {
  pub fn update(&mut self) {
    // 1. get next buffer(s)
    // 
    // 2. render something
    // 
    // 3. atomic commit to display
    // 
    // 4. wait for vsync
    // NOTE: Each GPU corresponds to its own virtual desktop for now
    let events = match self.card.receive_events() {
      Ok(events) => {
        println!["Ready to receive events!"];
        events.peekable()
      },
      Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
        println!["Would block"];
        return;
      },
      Err(e) => panic!["{e}"],
    };
    events.for_each(
      |event| {
        match event {
          drm::control::Event::PageFlip(event) => {
            println!["Got page flip event"];
            let mut to_remove: HashSet<String> = HashSet::new();
            for display in self.displays.iter_mut() {
              if display.crtc != event.crtc {
                continue;
              }

              // Draw to the back buffer
              println!["Blitting to the framebuffer for {}", display.name];

              // Swap the buffers
              //? SAFETY: This is safe here because we are calling it right after a page flip
              //? event, indicating the hardware is no longer using it
              match unsafe {
                display.primary.swap(&self.card, display.crtc)
              } {
                Ok(()) => (),
                // Probably disconnected: remove the display from the list
                Err(e) => if let Ok(info) = self.card.get_connector(display.connector, false) {
                  println!["Got an error: {e}"];
                  if info.state() != connector::State::Connected {
                    to_remove.insert(display.name.to_owned());
                  }
                },
              }
            }
            self
              .displays
              .retain_mut(
                |display| if to_remove.contains(&display.name) {
                  for fb in display
                    .primary
                    .buffers
                    .fbs
                    .iter()
                    .chain(display.cursor.buffers.fbs.iter()) {
                    self.card.destroy_framebuffer(*fb).ok();
                  }
                  for overlay in display.overlays.iter() {
                    for fb in overlay.buffers.fbs.iter() {
                      self.card.destroy_framebuffer(*fb).ok();
                    }
                  }
                  true
                } else {
                  false
                },
              );
          },
          _ => (),
        }
      },
    );
  }

  pub fn displays_mut(&mut self) -> impl Iterator<Item = &mut Display> {
    self.displays.iter_mut()
  }

  /// Returns true if any new displays were acquired
  pub fn init_displays(&mut self, config: &Config) -> bool {
    // Don't re-initialize displays we are already using
    let ignore_list =
      HashSet::<String>::from_iter(
        self.displays.iter().map(|display| display.name.to_owned()),
      );
    let mut new_displays =
      Display::init_displays(ignore_list, &self.card, &self.gbm).unwrap_or_else(|e| {
        tracing::warn!["Failed to init displays: {e}"];
        Default::default()
      });
    if !new_displays.is_empty() {
      for display in new_displays.iter() {
        println!["Found display: {} {:?}", display.name, display.size];
      }
      // Modeset all newly connected displays.
    }
    for display in new_displays.iter_mut() {
      let mut atomic_req = atomic::AtomicModeReq::new();
      display
        .init_req(self.card.as_ref(), &mut atomic_req)
        .expect("Failed to init display");
      display
        .primary
        .init_req(&mut atomic_req, display.crtc)
        .expect("Failed to init primary surface");
      display
        .cursor
        .init_req(&mut atomic_req, display.crtc)
        .expect("Failed to init primary surface");
      for overlay in display.overlays.iter() {
        overlay
          .init_req(&mut atomic_req, display.crtc)
          .expect("Failed to init primary surface");
      }
      self
        .card
        .atomic_commit(
          AtomicCommitFlags::ALLOW_MODESET | AtomicCommitFlags::NONBLOCK |
            AtomicCommitFlags::PAGE_FLIP_EVENT,
          atomic_req,
        )
        .expect("Failed to set mode");
    }

    // If we have new displays, add them to the display tree
    if !new_displays.is_empty() {
      self.displays.extend(new_displays.into_iter());
      for display in self.displays.iter_mut() {
        let name = &display.name;
        let pos = config.get::<DisplayPosition>(&CompositorConfig::offset_key(name));
        if let Some(pos) = pos {
          display.pos = pos.into();
        }
      }
      true
    } else {
      false
    }
  }
}
