use crate::buffer::DrmCtx;
use crate::buffer::CURSOR_DIM;
use crate::config::CompositorConfig;
use crate::config::Config;
use crate::util::DisplayPosition;
use crate::DRM_FORMAT;
use drm::control::plane;
use drm::control::PlaneType;
use drm::ClientCapability;
use drm::Device;
use crate::card::Card;
use crate::display::Display;
use crate::error::CompositorError;
use crate::error::CompositorResult;
use crate::fourcc::FourCc;
use crate::util::BackgroundImage;
use drm::control::AtomicCommitFlags;
use drm::control::Device as ControlDevice;
use drm::control::atomic;
use drm::control::connector;
use gbm::BufferObject;
use khregl::Dynamic;
use khregl::EGL1_5;
use khregl::Instance;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use taffy::NodeId;
use taffy::TaffyTree;

// use wgpu::TextureFormat;
// use crate::card::Card;
// use crate::error::CompositorError;
// use crate::error::CompositorResult;
// use crate::TEXTURE_FORMAT;
// use std::collections::HashMap;
// const BLIT_SHADER: &str =
//   r#"struct VertexOutput {
//    @builtin(position) position: vec4<f32>,
//    @location(0) tex_coords: vec2<f32>,
// }
// @vertex
// fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
//    var out: VertexOutput;
//    let x = f32((vertex_index & 1u) << 2u) - 1.0;
//    let x = f32((vertex_index & 2u) << 1u) - 1.0;
//    out.position = vec4<f32>(x, y, 0.0, 1.0);
//    out.tex_coords = vec2<f32>(x + 1.0, 1.0 - y) * 0.5;
//    return out;
// }
// @group(0) @binding(0) var tex: texture_2d<f32>;
// @group(0) @binding(1) var tex_sampler: sampler;
// @fragment
// fn fs_main(in: VertexOutput) -> @location(0) vec4<32> {
//    return textureSample(tex, tex_sampler, in.tex_coords);
// }"#;
// fn get_pci_ids_from_card(card_num: u32) -> Option<(u32, u32)> {
//   let sys_path = format!("/sys/class/drm/card{}/device", card_num);
//   let vendor = std::fs::read_to_string(format!("{}/vendor", sys_path)).ok()?;
//   let device = std::fs::read_to_string(format!("{}/device", sys_path)).ok()?;
//   let vendor_id =
//     u32::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok()?;
//   let device_id =
//     u32::from_str_radix(device.trim().trim_start_matches("0x"), 16).ok()?;
//   Some((vendor_id, device_id))
// }
// pub async fn init_card_and_gpu() -> CompositorResult<
//   (Card, wgpu::Adapter, wgpu::Device, wgpu::Queue),
// > {
//   let mut cards: HashMap<(u32, u32), (Card, usize)> = HashMap::new();
//   for card_num in 0 .. 16 {
//     let card_path = format!["/dev/dri/card{card_num}"];
//     if let Ok(card) = Card::open(&card_path) {
//       if let Some((vendor_id, device_id)) = get_pci_ids_from_card(card_num) {
//         println![
//           "Opend card{card_num}: vendor=0x{vendor_id:x}, device=0x{device_id:x}"
//         ];
//         cards.insert((vendor_id, device_id), (card, card_num as usize));
//       }
//     }
//   }
//   let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
//     backends: wgpu::Backends::VULKAN,
//     ..Default::default()
//   });
//   let adapters = instance.enumerate_adapters(wgpu::Backends::all());
//   println!["Available GPUs:"];
//   for (i, adapter) in adapters.iter().enumerate() {
//     let info = adapter.get_info();
//     println![
//       "  [{i}] {} - {:?} (Backend: {:?})",
//       info.name,
//       info.device_type,
//       info.backend
//     ];
//   }
//   let adapter =
//     adapters
//       .clone()
//       .into_iter()
//       .find(|a| a.get_info().device_type == wgpu::DeviceType::DiscreteGpu)
//       .or_else(|| adapters.into_iter().next())
//       .expect("No suitable adapter found");
//   let info = adapter.get_info();
//   println!["Selected GPU {} ({:?})", info.name, info.backend];
//   let (drm_card, card_num) =
//     cards.remove(&(info.vendor, info.device)).ok_or(CompositorError::GpuCard)?;
//   println!["Using card{card_num} for modesetting"];
//   drop(cards);
//   let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
//     label: Some("DMA-BUF Device"),
//     required_features: wgpu::Features::empty(),
//     required_limits: wgpu::Limits::defaults(),
//     memory_hints: wgpu::MemoryHints::default(),
//     experimental_features: wgpu::ExperimentalFeatures::disabled(),
//     trace: wgpu::Trace::Off,
//   }).await.expect("Failed to create device");
//   println!["Card and GPU successfully initialized in tandem."];
//   Ok((drm_card, adapter, device, queue))
// }
// pub fn create_pipeline(device: &wgpu::Device) {
//   let mut encoder =
//     device.create_command_encoder(
//       &wgpu::CommandEncoderDescriptor { label: Some("Compositor Render Encoder") },
//     );
//   let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
//     label: Some("Blit Shader"),
//     source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
//   });
//   let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
//     label: Some("Blit Pipeline"),
//     layout: None,
//     vertex: wgpu::VertexState {
//       module: &shader,
//       entry_point: Some("vs_main"),
//       buffers: &[],
//       compilation_options: wgpu::PipelineCompilationOptions::default(),
//     },
//     fragment: Some(wgpu::FragmentState {
//       module: &shader,
//       entry_point: Some("fs_main"),
//       targets: &[Some(wgpu::ColorTargetState {
//         format: TEXTURE_FORMAT,
//         blend: None,
//         write_mask: wgpu::ColorWrites::ALL,
//       })],
//       compilation_options: wgpu::PipelineCompilationOptions::default(),
//     }),
//     primitive: wgpu::PrimitiveState::default(),
//     depth_stencil: None,
//     multisample: wgpu::MultisampleState::default(),
//     multiview: None,
//     cache: None,
//   });
//   // let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
//   //    label: Some("Composite Pass"),
//   //    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
//   //       view: &output_view,
//   //       depth_slice: None,
//   //       resolve_target: None,
//   //       ops: wgpu::Operations {
//   //          load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
//   //          store: wgpu::StoreOp::Store,
//   //       },
//   //    })],
//   //    depth_stencil_attachment: None,
//   //    timestamp_writes: None,
//   //    occlusion_query_set: None,
//   // });
// }
pub struct GpuContext {
  pub card: Box<Card>,
  pub gbm: gbm::Device<&'static Card>,
  pub egl: Arc<Instance<Dynamic<libloading::Library, EGL1_5>>>,
  pub egldisplay: khregl::Display,
  pub eglctx: khregl::Context,
  pub eglconfig: khregl::Config,
  pub bg: BackgroundImage,
  pub displays: HashMap<String, Display>,
  pub layout: TaffyTree<String>,
  pub leaf_ids: Vec<NodeId>,
}

impl GpuContext {
  pub fn layout_displays(&mut self) {
    use taffy::Dimension;
    use taffy::Display as NodeDisplay;
    use taffy::FlexDirection;
    use taffy::Size;
    use taffy::Style;

    let mut displays = BTreeMap::new();
    for (_, display) in self.displays.iter() {
      let pos: (i32, i32) = display.pos.into();
      let key = (pos.1, pos.0);
      displays.insert(key, (display.name().to_owned(), Size {
        width: Dimension::length(display.size.0 as f32),
        height: Dimension::length(display.size.1 as f32),
      }));
    }
    let mut tree = TaffyTree::<String>::new();
    let mut leafs = Vec::new();
    let mut hnodes = Vec::new();
    let mut vnodes = Vec::new();
    let mut prev_y = 0;
    for ((y, _), (name, size)) in displays {
      if y > prev_y {
        let node = tree.new_with_children(Style {
          flex_direction: FlexDirection::Row,
          flex_grow: 0.0,
          flex_wrap: taffy::FlexWrap::NoWrap,
          flex_shrink: 0.0,
          ..Default::default()
        }, &hnodes).unwrap();
        vnodes.push(node);
        leafs.extend(hnodes.drain(..));
      }
      prev_y = y;
      hnodes.push(tree.new_leaf_with_context(Style {
        display: NodeDisplay::Block,
        size,
        min_size: size,
        max_size: size,
        flex_grow: 0.0,
        flex_shrink: 0.0,
        ..Default::default()
      }, name).unwrap());
    }
    {
      let node = tree.new_with_children(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 0.0,
        flex_wrap: taffy::FlexWrap::NoWrap,
        flex_shrink: 0.0,
        ..Default::default()
      }, &hnodes).unwrap();
      vnodes.push(node);
      leafs.extend(hnodes.drain(..));
    }
    let root_node = tree.new_with_children(Style {
      flex_direction: FlexDirection::Column,
      ..Default::default()
    }, &vnodes).unwrap();
    tree.compute_layout(root_node, Size::max_content()).unwrap();
    for leaf in leafs {
      let nym = tree.get_node_context(leaf).unwrap();
      let display = self.displays.get_mut(nym).unwrap();
      let pos = tree.layout(leaf).unwrap().location;
      display.pos = (pos.x as i32, pos.y as i32);
    }
    self.layout = tree;
  }

  fn init_displays(
    &mut self,
    ignore_list: impl Into<Option<HashSet::<String>>>,
  ) -> CompositorResult<Vec<(Display, BufferObject<()>, BufferObject<()>)>> {
    let ignore_list = ignore_list.into();
    for (
      cap,
      enable,
    ) in [
      (ClientCapability::UniversalPlanes, true),
      (ClientCapability::Atomic, true),
    ].into_iter() {
      (&self.card).set_client_capability(cap, enable).map_err(|err| {
        CompositorError::ClientCapability(cap, err)
      })?;
    }
    let resources = (&self.card).resource_handles().map_err(|err| {
      CompositorError::ResourcesError(err)
    })?;
    println!["Getting all connected connectors"];
    let connected: Vec<connector::Info> =
      resources
        .connectors()
        .iter()
        .flat_map(|con| (&self.card).get_connector(*con, true))
        .filter(|i| i.state() == connector::State::Connected && !i.modes().is_empty())
        .collect();
    if connected.is_empty() {
      Err(CompositorError::NoQualifiedConnectors)?;
    }
    let mut planes: HashSet<plane::Handle> =
      (&self.card)
        .plane_handles()
        .map_err(|err| CompositorError::GetPlanes(err))?
        .into_iter()
        .collect();
    println!["Organizing the display objects."];
    let max_displays = resources.crtcs().len().min(connected.len());
    let mut displays = Vec::new();
    for (
      connector,
      &crtc,
    ) in connected.into_iter().take(max_displays).zip(resources.crtcs()) {
      let name =
        format![
          "card{}-{}-{}",
          self.card.num(),
          connector.interface().as_str(),
          connector.interface_id()
        ];
      if let Some(ref ignore_list) = ignore_list {
        if ignore_list.contains(&name) {
          continue;
        }
      }
      let mode = *connector.modes().first().unwrap();
      let size = match mode.size() {
        (width, height) => (width as u32, height as u32),
      };
      let (primary, initial_primary_bo) =
        DrmCtx::from_connector(
          &self.card,
          &self.gbm,
          &self.egl,
          &self.eglconfig,
          &self.egldisplay,
          &resources,
          crtc,
          &mut planes,
          PlaneType::Primary,
          (size.0, size.1),
        )?;
      let (cursor, initial_cursor_bo) =
        DrmCtx::from_connector(
          &self.card,
          &self.gbm,
          &self.egl,
          &self.eglconfig,
          &self.egldisplay,
          &resources,
          crtc,
          &mut planes,
          PlaneType::Cursor,
          (CURSOR_DIM, CURSOR_DIM),
        )?;
      let connector_props =
        (&self.card)
          .get_properties(connector.handle())
          .map_err(
            |err| CompositorError::GetConnectorProperties(connector.handle(), err),
          )?
          .as_hashmap(self.card.as_ref())
          .map_err(|err| CompositorError::PropsToHashMap(err))?;
      let crtc_props =
        (&self.card)
          .get_properties(crtc)
          .map_err(|err| CompositorError::GetCrtcProperties(crtc, err))?
          .as_hashmap(self.card.as_ref())
          .map_err(|err| CompositorError::PropsToHashMap(err))?;
      let size = size.into();
      displays.push((Display {
        name,
        size,
        pos: Default::default(),
        connector: connector.handle(),
        crtc,
        connector_props,
        crtc_props,
        mode,
        primary,
        cursor,
        overlays: vec![],
      }, initial_primary_bo, initial_cursor_bo));
    }
    Ok(displays)
  }

  pub fn upkeep(&mut self, config: &Config) {
    // Don't re-initialize displays we are already using
    let ignore_list =
      HashSet::<String>::from_iter(
        self.displays.iter().map(|(name, _)| name.to_owned()),
      );
    let mut new_displays = self.init_displays(ignore_list).unwrap_or_else(|e| {
      tracing::warn!["Failed to init displays: {e}"];
      Default::default()
    });

    // Modeset all newly connected displays.
    for (display, initial_primary_bo, initial_cursor_bo) in new_displays.iter_mut() {
      println!["Found display {}", display.name()];
      self
        .egl
        .make_current(
          self.egldisplay,
          Some(display.primary.eglsurface),
          Some(display.primary.eglsurface),
          Some(self.eglctx),
        )
        .expect("Failed to make surface current");
      let mut atomic_req = atomic::AtomicModeReq::new();
      let initial_primary_fb =
        self
          .card
          .add_framebuffer(initial_primary_bo, DRM_FORMAT.depth(), DRM_FORMAT.bpp())
          .expect("Failed to get initial framebuffer");
      let initial_cursor_fb =
        self
          .card
          .add_framebuffer(initial_cursor_bo, DRM_FORMAT.depth(), DRM_FORMAT.bpp())
          .expect("Failed to get initial framebuffer");
      display
        .init_req(self.card.as_ref(), initial_primary_fb, &mut atomic_req)
        .expect("Failed to init display");
      display
        .primary
        .init_req(self.card.as_ref(), initial_primary_fb, &mut atomic_req, display.crtc)
        .expect("Failed to init primary surface");
      display
        .cursor
        .init_req(self.card.as_ref(), initial_cursor_fb, &mut atomic_req, display.crtc)
        .expect("Failed to init primary surface");
      for overlay in display.overlays.iter() {
        overlay
          .init_req(
            self.card.as_ref(),
            initial_primary_fb,
            &mut atomic_req,
            display.crtc,
          )
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
      self
        .card
        .destroy_framebuffer(initial_primary_fb)
        .expect("Failed to destroy initial framebuffer");
    }

    // If we have new displays, add them to the display tree
    if !new_displays.is_empty() {
      self
        .displays
        .extend(
          new_displays
            .into_iter()
            .map(|(display, _, _)| (display.name().to_owned(), display)),
        );
      for (name, display) in self.displays.iter_mut() {
        let pos = config.get::<DisplayPosition>(&CompositorConfig::offset_key(name));
        if let Some(pos) = pos {
          display.pos = pos.into();
        }
      }
      self.layout_displays();
    }

    // 1. get next buffer(s)
    // 
    // 2. render something
    // 
    // 3. atomic commit to display
    // 
    // 4. wait for vsync
    // NOTE: Each GPU corresponds to its own virtual desktop for now
    let events = match self.card.receive_events() {
      Ok(events) => events.peekable(),
      Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
        return;
      },
      Err(e) => panic!["{e}"],
    };
    events.for_each(
      |event| {
        match event {
          drm::control::Event::PageFlip(event) => {
            let mut to_remove = Vec::new();
            for id in self.leaf_ids.iter().map(|id| {
              let k = self.layout.get_node_context(*id).unwrap();
              k.to_owned()
            }) {
              let display = self.displays.get_mut(&id).unwrap();
              if display.crtc != event.crtc {
                continue;
              }

              // Draw to the back buffer
              self
                .egl
                .make_current(
                  self.egldisplay,
                  Some(display.primary.eglsurface),
                  None,
                  Some(self.eglctx),
                )
                .expect("Failed to make surface current");
              unsafe {
                gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, 0);
                gl::BindFramebuffer(gl::READ_FRAMEBUFFER, self.bg.fb_id);
                gl::BlitFramebuffer(
                  0,
                  0,
                  self.bg.width,
                  self.bg.height,
                  0,
                  0,
                  self.bg.width,
                  self.bg.height,
                  gl::COLOR_BUFFER_BIT,
                  gl::LINEAR,
                );
                // Can blit more after this as well, just change the read buffer!
              }

              // Swap the buffers
              //? SAFETY: This is safe here because we are calling it right after a page flip
              //? event, indicating the hardware is no longer using it
              match unsafe {
                display.primary.swap(&self.card, self.egl.as_ref(), &self.egldisplay)
              } {
                Ok(()) => (),
                // Probably disconnected: remove the display from the list
                Err(_) => if let Ok(info) = self.card.get_connector(display.connector, false) {
                  if info.state() != connector::State::Connected {
                    to_remove.push(display.name().to_owned());
                  }
                },
              }
            }
            for id in to_remove {
              // Quadtree.remove only overwrites location with null, does not rearrange displays
              if let Some(display) = self.displays.remove(&id) {
                for fb in display
                  .primary
                  .fbs
                  .into_inner()
                  .values()
                  .chain(display.cursor.fbs.into_inner().values()) {
                  self.card.destroy_framebuffer(*fb).ok();
                }
                for overlay in display.overlays.into_iter() {
                  for fb in overlay.fbs.into_inner().values() {
                    self.card.destroy_framebuffer(*fb).ok();
                  }
                }
              }
            }
          },
          _ => (),
        }
      },
    );
  }
}
