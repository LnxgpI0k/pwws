use drm::control::atomic;
use drm::control::crtc;
use drm::control::framebuffer;
use drm::control::plane;
use drm::control::property;
use drm::control::AtomicCommitFlags;
pub use drm::control::Device as ControlDevice;
use drm::control::PlaneType;
use drm::control::ResourceHandles;
use gbm::BufferObjectFlags;
use std::collections::HashMap;
use std::collections::HashSet;
use crate::error::CompositorError;
use crate::error::CompositorResult;
use crate::card::Card;
use crate::fourcc::FourCc;
use crate::DRM_FORMAT;

pub const CURSOR_DIM: u32 = 128;

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

#[derive(Debug)]
pub struct TripleBuffer {
  pub draw: usize,
  pub scan: usize,
  pub bos: [gbm::BufferObject<()>; 3],
  pub fbs: [framebuffer::Handle; 3],
}

impl TripleBuffer {
  pub fn new(
    card: &Card,
    gbm: &gbm::Device<&Card>,
    planetype: PlaneType,
    size: (u32, u32),
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
    Ok(Self {
      scan: 0,
      draw: 0,
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
    let buffers = TripleBuffer::new(card, gbm, planetype, size)?;
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
    resources: &ResourceHandles,
    crtc: crtc::Handle,
    planes: &mut HashSet<plane::Handle>,
    planetype: PlaneType,
    size: (u32, u32),
  ) -> CompositorResult<Self> {
    let plane = find_compatible_plane(card, resources, crtc, planes, planetype);
    if let Some(plane) = plane {
      Self::new(card, gbm, plane, planetype, size)
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
