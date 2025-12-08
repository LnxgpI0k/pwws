use drm::control::atomic;
use drm::control::crtc;
use drm::control::framebuffer;
use drm::control::plane;
use drm::control::property;
use drm::control::AtomicCommitFlags;
pub use drm::control::Device as ControlDevice;
use drm::control::PlaneType;
use drm::control::ResourceHandles;
use gbm::AsRaw;
use gbm::BufferObjectFlags;
use khregl::ATTRIB_NONE;
use khregl::EGL1_5;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::os::fd::AsRawFd;
use crate::error::CompositorError;
use crate::error::CompositorResult;
use crate::card::Card;
use crate::fourcc::FourCc;
use crate::DRM_FORMAT;
use std::os::raw::c_void;

pub const CURSOR_DIM: u32 = 64;

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

   // let framebuffer =
   //    card
   //       .add_framebuffer(&buffer, DRM_FORMAT.depth(), DRM_FORMAT.bpp())
   //       .map_err(|err| CompositorError::AddFrameBuffer(err))?;
   Ok(buffer)
}

fn make_surfaces(
   gbm: &gbm::Device<&'static Card>,
   egl: &khregl::DynamicInstance<EGL1_5>,
   config: &khregl::Config,
   egldisplay: &khregl::Display,
   planetype: PlaneType,
   (width, height): (u32, u32),
) -> CompositorResult<(gbm::Surface<&'static Card>, khregl::Surface)> {
   let planeflag = match planetype {
      PlaneType::Overlay | PlaneType::Primary => BufferObjectFlags::SCANOUT,
      PlaneType::Cursor => BufferObjectFlags::CURSOR,
   };
   let gbmsurface =
      gbm
         .create_surface::<&'static Card>(
            width,
            height,
            DRM_FORMAT,
            planeflag | BufferObjectFlags::RENDERING,
         )
         .map_err(|e| CompositorError::GbmSurfaceCreate(e))?;
   let eglsurface =
      unsafe {
         egl.create_platform_window_surface(
            *egldisplay,
            *config,
            gbmsurface.as_raw() as *mut c_void,
            &[ATTRIB_NONE],
         )
      }.map_err(|e| CompositorError::EglSurfaceCreate(e))?;
   Ok((gbmsurface, eglsurface))
}

#[derive(Debug)]
pub struct DrmCtx {
   pub plane: Option<plane::Handle>,
   pub plane_props: HashMap<String, property::Info>,
   pub size: (u32, u32),
   pub current_bo: Option<gbm::BufferObject<&'static Card>>,
   // NOTE: All framebuffers must be dropped prior to changing surface config
   pub fbs: UnsafeCell<HashMap<i32, framebuffer::Handle>>,
   pub gbmsurface: gbm::Surface<&'static Card>,
   pub eglsurface: khregl::Surface,
}

impl DrmCtx {
   pub fn new(
      card: &Card,
      gbm: &gbm::Device<&'static Card>,
      egl: &khregl::DynamicInstance<EGL1_5>,
      config: &khregl::Config,
      egldisplay: &khregl::Display,
      plane: Option<plane::Handle>,
      planetype: PlaneType,
      size: (u32, u32),
   ) -> CompositorResult<(Self, gbm::BufferObject<()>)> {
      let (gbmsurface, eglsurface) =
         make_surfaces(gbm, egl, config, egldisplay, planetype, size)?;
      let plane_props =
         if let Some(plane) = plane {
            card
               .get_properties(plane)
               .map_err(|err| CompositorError::GetPlaneProperties(plane, err))?
               .as_hashmap(card)
               .map_err(|err| CompositorError::PropsToHashMap(err))?
         } else {
            HashMap::new()
         };
      let initial_buffer = make_buffer(card, gbm, planetype, size)?;
      let fbs = HashMap::new();
      Ok((Self {
         plane,
         plane_props,
         size,
         current_bo: None,
         fbs: fbs.into(),
         gbmsurface,
         eglsurface,
      }, initial_buffer))
   }

   pub fn from_connector(
      card: &Card,
      gbm: &gbm::Device<&'static Card>,
      egl: &khregl::DynamicInstance<EGL1_5>,
      config: &khregl::Config,
      egldisplay: &khregl::Display,
      resources: &ResourceHandles,
      crtc: crtc::Handle,
      planes: &mut HashSet<plane::Handle>,
      planetype: PlaneType,
      size: (u32, u32),
   ) -> CompositorResult<(Self, gbm::BufferObject<()>)> {
      let plane = find_compatible_plane(card, resources, crtc, planes, planetype);
      if let Some(plane) = plane {
         let _ = planes.remove(&plane);
      }
      Self::new(card, gbm, egl, config, egldisplay, plane, planetype, size)
   }

   fn get_fb(
      &self,
      card: &Card,
      bo: &gbm::BufferObject<&'static Card>,
   ) -> CompositorResult<framebuffer::Handle> {
      let fd = bo.fd().map_err(|e| CompositorError::GbmFd(e))?.as_raw_fd();
      if let Some(fb) = unsafe {
         self.fbs.get().as_ref().unwrap().get(&fd)
      } {
         Ok(*fb)
      } else {
         let fb =
            card
               .add_framebuffer(bo, DRM_FORMAT.depth(), DRM_FORMAT.bpp())
               .map_err(|err| CompositorError::AddFrameBuffer(err))?;
         let fbs = (unsafe {
            &mut *self.fbs.get()
         }) as &mut HashMap<i32, framebuffer::Handle>;
         fbs.insert(fd, fb);
         Ok(fb)
      }
   }

   pub fn init_req(
      &self,
      card: &Card,
      initial_fb: framebuffer::Handle,
      atomic_req: &mut atomic::AtomicModeReq,
      crtc: crtc::Handle,
   ) -> CompositorResult<()> {
      if let Some(plane) = self.plane {
         let props = &self.plane_props;
         atomic_req.add_property(
            plane,
            props["FB_ID"].handle(),
            property::Value::Framebuffer(Some(initial_fb)),
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
      }
      Ok(())
   }

   /// SAFETY: This function must be called exactly once after a page flip event, not
   /// before
   pub unsafe fn swap(
      &mut self,
      card: &Card,
      egl: &khregl::DynamicInstance<EGL1_5>,
      display: &khregl::Display,
   ) -> CompositorResult<()> {
      if let Some(plane) = self.plane {
         // First, drop whatever buffer we were scanning out before.
         self.current_bo.take();

         // Put rendered buffer in scan position
         egl
            .swap_buffers(*display, self.eglsurface)
            .map_err(|e| CompositorError::BufferSwap(e))?;

         // Lock the scan buffer so we can play with it
         let bo =
            unsafe {
               self
                  .gbmsurface
                  .lock_front_buffer()
                  .map_err(|_| CompositorError::FrontBufferLock)?
            };

         // Get the framebuffer for this buffer (if it exists in our cache)
         let fb = self.get_fb(card, &bo)?;
         let props = &self.plane_props;
         let mut atomic_req = atomic::AtomicModeReq::new();
         atomic_req.add_property(
            plane,
            props["FB_ID"].handle(),
            property::Value::Framebuffer(Some(fb)),
         );
         card
            .atomic_commit(
               AtomicCommitFlags::NONBLOCK | AtomicCommitFlags::PAGE_FLIP_EVENT,
               atomic_req,
            )
            .map_err(|err| CompositorError::AtomicCommitFailed(err))?;
      }
      Ok(())
   }
}
