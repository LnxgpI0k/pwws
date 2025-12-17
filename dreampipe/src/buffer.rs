use ash::vk::ImportMemoryFdInfoKHR;
use ash::vk::SubresourceLayout;
use ash::vk;
use crate::card::Card;
use crate::error::CompositorError;
use crate::error::CompositorResult;
use crate::fourcc::FourCc;
use drm::buffer::DrmFourcc;
use drm::control::AtomicCommitFlags;
use drm::control::Device as ControlDevice;
use drm::control::PlaneType;
use drm::control::ResourceHandles;
use drm::control::atomic;
use drm::control::crtc;
use drm::control::framebuffer;
use drm::control::plane;
use drm::control::property;
use gbm::BufferObjectFlags;
use std::collections::HashMap;
use std::collections::HashSet;
use std::os::fd::IntoRawFd;
use wgpu::Extent3d;
use wgpu::TextureDescriptor;
use wgpu::TextureDimension;
use wgpu::TextureFormat;
use wgpu::TextureUses;
use wgpu::wgc::api;
use wgpu_hal::MemoryFlags;

pub const CURSOR_DIM: u32 = 128;
pub const DRM_FORMAT: DrmFourcc = DrmFourcc::Xrgb8888;
pub const VK_FORMAT: ash::vk::Format = ash::vk::Format::B8G8R8A8_UNORM;

fn is_plane_compatible_with_crtc(
  card: &Card,
  resources: &ResourceHandles,
  plane: plane::Handle,
  crtc: crtc::Handle,
) -> bool {
  let plane_info = card.get_plane(plane).unwrap();
  resources.filter_crtcs(plane_info.possible_crtcs()).contains(&crtc)
}

fn find_compatible_plane(
  card: &Card,
  resources: &ResourceHandles,
  crtc: crtc::Handle,
  planes: &mut HashSet<plane::Handle>,
  planetype: PlaneType,
) -> Option<plane::Handle> {
  let mut compatible = Vec::new();
  for &plane in planes.iter() {
    if is_plane_compatible_with_crtc(card, &resources, plane, crtc) {
      compatible.push(plane);
    }
  }
  let mut plane = None;
  'outer: for p in compatible.into_iter() {
    if let Ok(plane_props) = card.get_properties(p) {
      for (&id, &val) in plane_props.iter() {
        if let Ok(info) = card.get_property(id) {
          if info.name().to_str().map(|x| x == "type").unwrap_or(false) {
            if val == planetype as u64 {
              plane = Some(p);
              break 'outer;
            }
          }
        }
      }
    }
  }
  plane
}

#[allow(unused)]
fn make_buffer(
  card: &Card,
  gbm: &gbm::Device<&Card>,
  planetype: PlaneType,
  size: (u32, u32),
) -> Result<gbm::BufferObject<()>, CompositorError> {
  let planeflag = match planetype {
    PlaneType::Overlay | PlaneType::Primary => BufferObjectFlags::SCANOUT,
    PlaneType::Cursor => BufferObjectFlags::CURSOR,
  };
  let buffer =
    gbm
      .create_buffer_object::<()>(
        size.0,
        size.1,
        DRM_FORMAT,
        planeflag | BufferObjectFlags::RENDERING,
      )
      .map_err(|err| {
        CompositorError::GbmCreation(err)
      })?;
  Ok(buffer)
}

pub fn find_memory_type_index(
  type_filter: u32,
  properties: vk::MemoryPropertyFlags,
  mem_properties: &vk::PhysicalDeviceMemoryProperties,
) -> Option<u32> {
  (0 .. mem_properties.memory_type_count).find(
    |&i| {
      (type_filter & (1 << i)) != 0 &&
        mem_properties.memory_types[i as usize].property_flags.contains(properties)
    },
  )
}

fn create_vulkan_image_from_dmabuf<
  'a,
>(
  hal_device: &'a <api::Vulkan as wgpu::hal::Api>::Device,
  bo: &gbm::BufferObject<()>,
  (width, height): (u32, u32),
) -> CompositorResult<(vk::Image, vk::DeviceMemory)> {
  // Validate dimensions
  if width == 0 || height == 0 {
    return Err(CompositorError::VulkanImageDim);
  }

  // Get raw Vulkan handles
  let device = hal_device.raw_device();
  let _instance = hal_device.shared_instance().raw_instance();

  // Get the modifier
  let modifier = bo.modifier();

  // Get plane info
  let plane_count = bo.plane_count();
  let mut plane_layouts: Vec<SubresourceLayout> =
    Vec::with_capacity(plane_count as usize);
  for plane in 0 .. plane_count {
    plane_layouts.push(SubresourceLayout {
      offset: bo.offset(plane as i32) as u64,
      size: 0,
      row_pitch: bo.stride_for_plane(plane as i32) as u64,
      array_pitch: 0,
      depth_pitch: 0,
    });
  }
  let mut drm_format_modifier =
    vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
      .drm_format_modifier(modifier.into())
      .plane_layouts(&plane_layouts);
  let mut external_memory_info =
    vk
    ::ExternalMemoryImageCreateInfo
    ::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

  // Create external memory image
  let image_create_info =
    vk::ImageCreateInfo::default()
      .image_type(vk::ImageType::TYPE_2D)
      .format(VK_FORMAT)
      .extent(vk::Extent3D {
        width,
        height,
        depth: 1,
      })
      .mip_levels(1)
      .array_layers(1)
      .samples(vk::SampleCountFlags::TYPE_1)
      .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
      .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT)
      .sharing_mode(vk::SharingMode::EXCLUSIVE)
      .push_next(&mut external_memory_info)
      .push_next(&mut drm_format_modifier);

  // Create the image
  let image =
    unsafe {
      device
        .create_image(&image_create_info, None)
        .map_err(|e| CompositorError::VulkanImageCreate(e))?
    };

  // Import memory from DMA-BUF
  let memory_requirements = unsafe {
    device.get_image_memory_requirements(image)
  };

  // Duplicate the file descriptor to avoid ownership issues
  let dup_fd = bo.fd().map_err(|e| CompositorError::GbmFd(e))?;
  let mut import_memory_fd =
    ImportMemoryFdInfoKHR::default()
      .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
      .fd(dup_fd.into_raw_fd());

  // Find a suitable memory type
  let memory_properties =
    unsafe {
      hal_device
        .shared_instance()
        .raw_instance()
        .get_physical_device_memory_properties(hal_device.raw_physical_device())
    };
  let memory_type_index =
    find_memory_type_index(
      memory_requirements.memory_type_bits,
      vk::MemoryPropertyFlags::DEVICE_LOCAL,
      &memory_properties,
    ).ok_or_else(|| CompositorError::VulkanMemoryTypeIndex)?;
  let allocate_info =
    vk::MemoryAllocateInfo::default()
      .allocation_size(memory_requirements.size)
      .memory_type_index(memory_type_index)
      .push_next(&mut import_memory_fd);
  let device_memory =
    unsafe {
      device
        .allocate_memory(&allocate_info, None)
        .map_err(|e| CompositorError::VulkanMemoryAlloc(e))?
    };

  // Bind memory to image
  unsafe {
    device
      .bind_image_memory(image, device_memory, 0)
      .map_err(|e| CompositorError::VulkanBindMemory(e))?;
  }
  Ok((image, device_memory))
}

#[derive(Debug)]
pub struct TripleBuffer {
  pub draw: usize,
  pub scan: usize,
  pub vk_images: [vk::Image; 3],
  pub vk_memories: [vk::DeviceMemory; 3],
  pub wgpu_textures: [wgpu::Texture; 3],
  pub bos: [gbm::BufferObject<()>; 3],
  pub fbs: [framebuffer::Handle; 3],
}

impl TripleBuffer {
  pub fn new(
    card: &Card,
    gbm: &gbm::Device<&Card>,
    gpu: &wgpu::Device,
    planetype: PlaneType,
    size @ (width, height): (u32, u32),
  ) -> CompositorResult<Self> {
    let [a, b, c] =
      std::array::from_fn(|_| make_buffer(card, gbm, planetype, size));
    let buffers = [a?, b?, c?];
    let [a, b, c] =
      std::array::from_fn(
        |i| card
          .add_framebuffer(&buffers[i], DRM_FORMAT.depth(), DRM_FORMAT.bpp())
          .map_err(|e| CompositorError::AddFrameBuffer(e)),
      );
    let framebuffers = [a?, b?, c?];
    let chain_id: u64 = rand::random::<u64>();
    let [a, b, c] =
      std::array::from_fn(
        |i| unsafe {
          let hal_device_guard = gpu.as_hal::<api::Vulkan>();
          let Some(hal_device) = hal_device_guard else {
            return Err(CompositorError::VulkanApi);
          };
          let (vk_image, vk_memory) =
            create_vulkan_image_from_dmabuf(&hal_device, &buffers[i], size)?;
          let hal_texture =
            <api::Vulkan as wgpu::hal::Api>::Device::texture_from_raw(
              &hal_device,
              vk_image,
              &wgpu::hal::TextureDescriptor {
                label: Some(&format!["DMA-BUF Texture {chain_id}-{i}"]),
                size: Extent3d {
                  width,
                  height,
                  depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Bgra8Unorm,
                // unknown if correct for a compositor using dma buf
                usage: TextureUses::COLOR_TARGET,
                // Don't know what to put here
                memory_flags: MemoryFlags::empty(),
                view_formats: vec![],
              },
              None,
            );
          let wgpu_texture =
            gpu.create_texture_from_hal::<api::Vulkan>(
              hal_texture,
              &TextureDescriptor {
                label: Some(&format!["DMA-BUF Texture {chain_id}-{i}"]),
                size: Extent3d {
                  width,
                  height,
                  depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT |
                  wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
              },
            );
          Ok((wgpu_texture, vk_image, vk_memory))
        },
      );
    let [(a0, a1, a2), (b0, b1, b2), (c0, c1, c2)] = [a?, b?, c?];
    let vk_memories = [a2, b2, c2];
    let vk_images = [a1, b1, c1];
    let wgpu_textures = [a0, b0, c0];
    Ok(Self {
      scan: 0,
      draw: 0,
      vk_memories,
      vk_images,
      wgpu_textures,
      bos: buffers,
      fbs: framebuffers,
    })
  }

  pub fn swap(&mut self) {
    self.scan = self.draw;
    self.draw = (self.draw + 1) % 3;
  }
}

#[derive(Debug)]
pub struct DrmCtx {
  pub plane: plane::Handle,
  pub plane_props: HashMap<String, property::Info>,
  pub size: (u32, u32),
  pub buffers: TripleBuffer,
}

impl DrmCtx {
  pub fn new(
    card: &Card,
    gbm: &gbm::Device<&'static Card>,
    gpu: &wgpu::Device,
    plane: plane::Handle,
    planetype: PlaneType,
    size: (u32, u32),
  ) -> CompositorResult<Self> {
    let plane_props =
      card
        .get_properties(plane)
        .map_err(|err| CompositorError::GetPlaneProperties(plane, err))?
        .as_hashmap(card)
        .map_err(|err| CompositorError::PropsToHashMap(err))?;
    let buffers = TripleBuffer::new(card, gbm, gpu, planetype, size)?;
    Ok(Self {
      plane,
      plane_props,
      size,
      buffers,
    })
  }

  pub fn from_connector(
    card: &Card,
    gbm: &gbm::Device<&'static Card>,
    gpu: &wgpu::Device,
    resources: &ResourceHandles,
    crtc: crtc::Handle,
    planes: &mut HashSet<plane::Handle>,
    planetype: PlaneType,
    size: (u32, u32),
  ) -> CompositorResult<Self> {
    let plane = find_compatible_plane(card, resources, crtc, planes, planetype);
    if let Some(plane) = plane {
      Self::new(card, gbm, gpu, plane, planetype, size)
    } else {
      Err(
        CompositorError::NoCompatiblePrimaryPlane(
          card.get_crtc(crtc).map_err(|e| CompositorError::GetCrtcInfo(crtc, e))?,
        ),
      )
    }
  }

  fn get_draw_fb(&self) -> framebuffer::Handle {
    self.buffers.fbs[self.buffers.draw]
  }

  pub fn init_req(
    &self,
    atomic_req: &mut atomic::AtomicModeReq,
    crtc: crtc::Handle,
  ) -> CompositorResult<()> {
    let plane = self.plane;
    let props = &self.plane_props;
    atomic_req.add_property(
      plane,
      props["FB_ID"].handle(),
      property::Value::Framebuffer(Some(self.buffers.fbs[0])),
    );
    atomic_req.add_property(
      plane,
      props["CRTC_ID"].handle(),
      property::Value::CRTC(Some(crtc)),
    );
    atomic_req.add_property(
      plane,
      props["SRC_X"].handle(),
      property::Value::UnsignedRange(0),
    );
    atomic_req.add_property(
      plane,
      props["SRC_Y"].handle(),
      property::Value::UnsignedRange(0),
    );
    atomic_req.add_property(
      plane,
      props["SRC_W"].handle(),
      property::Value::UnsignedRange((self.size.0 as u64) << 16),
    );
    atomic_req.add_property(
      plane,
      props["SRC_H"].handle(),
      property::Value::UnsignedRange((self.size.1 as u64) << 16),
    );
    atomic_req.add_property(
      plane,
      props["CRTC_X"].handle(),
      property::Value::SignedRange(0),
    );
    atomic_req.add_property(
      plane,
      props["CRTC_Y"].handle(),
      property::Value::SignedRange(0),
    );
    atomic_req.add_property(
      plane,
      props["CRTC_W"].handle(),
      property::Value::UnsignedRange(self.size.0 as u64),
    );
    atomic_req.add_property(
      plane,
      props["CRTC_H"].handle(),
      property::Value::UnsignedRange(self.size.1 as u64),
    );
    Ok(())
  }

  pub unsafe fn swap(
    &mut self,
    card: &Card,
    crtc: crtc::Handle,
  ) -> CompositorResult<()> {
    let plane = self.plane;

    // Queue a page flip
    let mut atomic_req = atomic::AtomicModeReq::new();
    atomic_req.add_property(
      plane,
      self.plane_props["FB_ID"].handle(),
      property::Value::Framebuffer(Some(self.get_draw_fb())),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["CRTC_ID"].handle(),
      property::Value::CRTC(Some(crtc)),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["SRC_X"].handle(),
      property::Value::UnsignedRange(0),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["SRC_Y"].handle(),
      property::Value::UnsignedRange(0),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["SRC_W"].handle(),
      property::Value::UnsignedRange((self.size.0 << 16) as u64),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["SRC_H"].handle(),
      property::Value::UnsignedRange((self.size.1 << 16) as u64),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["CRTC_X"].handle(),
      property::Value::SignedRange(0),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["CRTC_Y"].handle(),
      property::Value::SignedRange(0),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["CRTC_W"].handle(),
      property::Value::UnsignedRange(self.size.0 as u64),
    );
    atomic_req.add_property(
      plane,
      self.plane_props["CRTC_H"].handle(),
      property::Value::UnsignedRange(self.size.1 as u64),
    );
    card
      .atomic_commit(
        AtomicCommitFlags::NONBLOCK | AtomicCommitFlags::PAGE_FLIP_EVENT,
        atomic_req,
      )
      .map_err(|err| CompositorError::AtomicCommitFailed(err))?;
    self.buffers.swap();
    Ok(())
  }
}
