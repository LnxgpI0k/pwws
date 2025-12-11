mod buffer;
mod card;
mod config;
mod display;
mod error;
mod fourcc;
mod util;
mod gpu;

use config::Config;
use crate::card::Card;
use crate::config::CompositorConfig;
use crate::display::Display;
use crate::gpu::GpuContext;
use crate::util::BackgroundImage;
use crate::util::GlFns;
use crate::util::create_context;
use drm::buffer::DrmFourcc;
use drm::control::Device as ControlDevice;
use gbm::AsRaw;
use gbm::BufferObjectFlags;
use image::DynamicImage;
use image::GenericImage;
use khregl::ATTRIB_NONE;
use khregl::ClientBuffer;
use khregl::Context;
use nix::fcntl::FcntlArg;
use nix::fcntl::OFlag;
use nix::fcntl::fcntl;
use notify::Event as NotifyEvent;
use notify::RecursiveMode;
use notify::Watcher;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use taffy::TaffyTree;

const EGL_NO_CONTEXT: *mut c_void = std::ptr::null_mut();
const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_LINUX_DMA_BUF_EXT: u32 = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: usize = 12913;
const EGL_DMA_BUF_PLANE0_FD_EXT: usize = 12914;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: usize = 12915;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: usize = 12916;
const EGL_NATIVE_PIXMAP_KHR: u32 = 12464;

// 65536x65536
pub const VIRTUAL_SCREEN_EXTENTS: (i32, i32) = (0x1000, 0x1000);
pub const DRM_FORMAT: DrmFourcc = DrmFourcc::Xrgb8888;

fn load_default_bg() -> DynamicImage {
  // This cfg is for display purposes when concatenating the entire project together.
  #[cfg(not(feature = "expanding"))]
  let bg =
    image::load_from_memory_with_format(
      include_bytes!["../mambutt.png"],
      image::ImageFormat::Png,
    ).unwrap();
  #[cfg(feature = "expanding")]
  let bg = image::load_from_memory_with_format(&[], image::ImageFormat::Png).unwrap();
  bg
}

fn main() {
  // Initialize config watcher
  let config_path = CompositorConfig::config_path().unwrap_or("/dev/null".into());
  let (mut tx, mut rx) = crossbeam::channel::bounded(2);
  tx.send(Ok(NotifyEvent::default())).unwrap();
  let mut config_watcher = notify::recommended_watcher(tx).ok();
  config_watcher.map(
    |mut watcher| watcher.watch(&config_path, RecursiveMode::NonRecursive).ok(),
  );
  let mut config = Config::new(&config_path).unwrap_or_default();

  // Open all the cards! Why not?
  let cards = Card::open_all().into_iter().map(|card| {
    let flags = fcntl(&card, FcntlArg::F_GETFL).expect("Failed to get card FD flags");
    let new_flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
    fcntl(&card, FcntlArg::F_SETFL(new_flags)).expect("Failed to set new flags");
    card
  }).collect::<Vec<_>>();
  let cards =
    cards
      .into_iter()
      .filter_map(|card| if let Some(res) = card.resource_handles().ok() {
        for &conn in res.connectors() {
          let info = card.get_connector(conn, false).ok()?;
          if info.state() == drm::control::connector::State::Connected {
            return Some(card);
          }
        }
        None
      } else {
        None
      })
      .collect::<Vec<_>>();

  // Create gbm devices and default bg buffers
  let mut contexts = Vec::new();
  let bg = load_default_bg();

  // Alignment required for frame buffers
  let aligned_width = (bg.width() + 15) & !15;
  let aligned_height = (bg.height() + 15) & !15;
  let mut padded = DynamicImage::new_rgba8(aligned_width, aligned_height);
  padded.copy_from(&bg, 0, 0).unwrap();

  // Create gbm contexts and copy bg into gpu memory
  for card in cards.into_iter() {
    let card = Box::new(card);
    let card_ref = Box::leak(card);
    let card_ref_clone = card_ref as *const Card;
    let gbm = gbm::Device::new(unsafe {
      &*card_ref_clone
    }).expect("Failed to init GBM with device");
    let card = unsafe {
      Box::from_raw(card_ref)
    };
    let egl =
      unsafe {
        Arc::new(
          khregl
          ::DynamicInstance::<khregl::EGL1_5>
          ::load_required().expect("unable to load libEGL.so.1"),
        )
      };
    let egldisplay =
      unsafe {
        egl.get_platform_display(
          EGL_PLATFORM_GBM_KHR,
          gbm.as_raw() as *mut c_void,
          &[ATTRIB_NONE],
        )
      }.expect("Failed to get platform display");
    egl.initialize(egldisplay).expect("Failed to initialize display");
    let (eglctx, eglconfig) = create_context(egl.as_ref(), egldisplay);
    gl::load_with(
      |name| egl
        .get_proc_address(name)
        .map(|ptr| ptr as *const _)
        .unwrap_or(std::ptr::null()),
    );
    let gl_fns = GlFns::load(&egl);
    let mut bgbo =
      gbm
        .create_buffer_object::<()>(
          aligned_width,
          aligned_height,
          DRM_FORMAT,
          BufferObjectFlags::RENDERING,
        )
        .unwrap();
    bgbo.map_mut(0, 0, aligned_width, aligned_height, |map| {
      map.buffer_mut().copy_from_slice(padded.as_rgba8().unwrap());
    }).unwrap();
    let egl_image =
      unsafe {
        egl
          .create_image(
            egldisplay,
            Context::from_ptr(EGL_NO_CONTEXT),
            EGL_NATIVE_PIXMAP_KHR,
            ClientBuffer::from_ptr(bgbo.as_raw() as *mut c_void),
            &[ATTRIB_NONE],
          )
          .expect("Failed to create EGL image")
      };
    let bg =
      unsafe {
        let mut tex_id = 0;
        gl::GenTextures(1, &mut tex_id);
        gl::BindTexture(gl::TEXTURE_2D, tex_id);
        (gl_fns.EGLImageTargetTexture2DOES)(gl::TEXTURE_2D, egl_image.as_ptr());
        let mut fb_id = 0;
        gl::GenFramebuffers(1, &mut fb_id);
        gl::BindFramebuffer(gl::FRAMEBUFFER, fb_id);
        gl::FramebufferTexture2D(
          gl::FRAMEBUFFER,
          gl::COLOR_ATTACHMENT0,
          gl::TEXTURE_2D,
          tex_id,
          0,
        );
        BackgroundImage {
          bo: bgbo,
          egl_image,
          tex_id,
          fb_id,
          width: aligned_width as i32,
          height: aligned_height as i32,
        }
      };
    let displays: HashMap<String, Display> = HashMap::new();
    let context = GpuContext {
      card,
      gbm,
      egl,
      egldisplay,
      eglctx,
      eglconfig,
      bg,
      displays,
      layout: TaffyTree::new(),
      leaf_ids: Vec::new(),
    };
    contexts.push(context);
  }
  let end = Instant::now() + Duration::from_secs(5);
  let mut frame = 0usize;
  loop {
    // Update configuration and such
    match rx.try_recv() {
      Ok(Ok(NotifyEvent { .. })) => {
        if let Ok(new_config) = Config::new(&config_path) {
          config = new_config;
          // displays = todo![];
        }
      },
      Ok(Err(_)) => { },
      Err(crossbeam::channel::TryRecvError::Empty) => (),
      Err(_) => {
        if frame % 120 == 0 {
          tracing::warn!["Config watcher channel disconnected somehow. Reconnecting."];
          (tx, rx) = crossbeam::channel::bounded(2);
          config_watcher = notify::recommended_watcher(tx).ok();
          config_watcher.map(
            |mut watcher| watcher.watch(&config_path, RecursiveMode::NonRecursive).ok(),
          );
        }
      },
    }
    for context in contexts.iter_mut() {
      context.upkeep(&config);
    }
    if Instant::now().checked_duration_since(end).is_some() {
      break;
    }
    frame += 1;
  }
}
