mod buffer;
mod card;
mod config;
mod display;
mod error;
mod fourcc;
mod util;

use config::Config;
use gbm::BufferObject;
use nix::fcntl::fcntl;
use nix::fcntl::FcntlArg;
use nix::fcntl::OFlag;
use taffy::TaffyTree;
use crate::card::Card;
use crate::config::CompositorConfig;
use crate::display::Display;
use crate::display::init_displays;
use crate::fourcc::FourCc;
use crate::util::BackgroundImage;
use crate::util::Bounds2;
use crate::util::GlFns;
use crate::util::create_context;
use drm::buffer::DrmFourcc;
use drm::control::AtomicCommitFlags;
use drm::control::Device as ControlDevice;
use drm::control::atomic;
use drm::control::connector;
use gbm::AsRaw;
use gbm::BufferObjectFlags;
use image::DynamicImage;
use image::GenericImage;
use khregl::ATTRIB_NONE;
use khregl::ClientBuffer;
use khregl::Context;
use notify::Event as NotifyEvent;
use notify::RecursiveMode;
use notify::Watcher;
use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

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
   let cards =
      Card::open_all()
         .into_iter()
         .map(
            |card| {
               let flags =
                  fcntl(&card, FcntlArg::F_GETFL).expect("Failed to get card FD flags");
               let new_flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
               fcntl(&card, FcntlArg::F_SETFL(new_flags)).expect("Failed to set new flags");
               card
            },
         )
         .collect::<Vec<_>>();
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
   let cards = Box::leak(Box::new(cards));

   // Create gbm devices and default bg buffers
   let mut contexts = Vec::new();
   let bg = load_default_bg();

   // Alignment required for frame buffers
   let aligned_width = (bg.width() + 15) & !15;
   let aligned_height = (bg.height() + 15) & !15;
   let mut padded = DynamicImage::new_rgba8(aligned_width, aligned_height);
   padded.copy_from(&bg, 0, 0).unwrap();

   // Create gbm contexts and copy bg into gpu memory
   for card in cards.iter() {
      let gbm = gbm::Device::new(card).expect("Failed to init GBM with device");
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
            }
         };
      contexts.push((card, gbm, egl, egldisplay, eglctx, eglconfig, bg));
   }
   let end = Instant::now() + Duration::from_secs(5);
   let mut frame = 0usize;
   let mut display_tree: TaffyTree<Display> = TaffyTree::new();
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
               tracing::warn![
                  "Config watcher channel disconnected somehow. Reconnecting."
               ];
               (tx, rx) = crossbeam::channel::bounded(2);
               config_watcher = notify::recommended_watcher(tx).ok();
               config_watcher.map(
                  |mut watcher| watcher
                     .watch(&config_path, RecursiveMode::NonRecursive)
                     .ok(),
               );
            }
         },
      }
      // Need these vars to get new displays and receive events
      for (card, gbm, egl, egldisplay, eglctx, eglconfig, bg) in contexts.iter_mut() {
         // Don't re-initialize displays we are already using
         let ignore_list =
            HashSet::<String>::from_iter(
               displays
                  .select_all_items()
                  .iter()
                  .map(|(_, display)| display.name().to_owned()),
            );
         let mut new_displays =
            init_displays(ignore_list, card, gbm, egl, egldisplay, eglconfig)
               .expect("Failed to init displays")
               .into_iter()
               .collect::<Vec<_>>();
         // Modeset all newly connected displays.
         for (display, initial_primary_bo, initial_cursor_bo) in new_displays.iter_mut() {
            println!["Found display {}", display.name()];
            egl
               .make_current(
                  *egldisplay,
                  Some(display.primary.eglsurface),
                  Some(display.primary.eglsurface),
                  Some(*eglctx),
               )
               .expect("Failed to make surface current");
            let mut atomic_req = atomic::AtomicModeReq::new();
            let initial_primary_fb =
               card
                  .add_framebuffer(
                     initial_primary_bo,
                     DRM_FORMAT.depth(),
                     DRM_FORMAT.bpp(),
                  )
                  .expect("Failed to get initial framebuffer");
            let initial_cursor_fb =
               card
                  .add_framebuffer(
                     initial_cursor_bo,
                     DRM_FORMAT.depth(),
                     DRM_FORMAT.bpp(),
                  )
                  .expect("Failed to get initial framebuffer");
            display
               .init_req(card, initial_primary_fb, &mut atomic_req)
               .expect("Failed to init display");
            display
               .primary
               .init_req(card, initial_primary_fb, &mut atomic_req, display.crtc)
               .expect("Failed to init primary surface");
            display
               .cursor
               .init_req(card, initial_cursor_fb, &mut atomic_req, display.crtc)
               .expect("Failed to init primary surface");
            for overlay in display.overlays.iter() {
               overlay
                  .init_req(card, initial_primary_fb, &mut atomic_req, display.crtc)
                  .expect("Failed to init primary surface");
            }
            card
               .atomic_commit(
                  AtomicCommitFlags::ALLOW_MODESET | AtomicCommitFlags::NONBLOCK |
                     AtomicCommitFlags::PAGE_FLIP_EVENT,
                  atomic_req,
               )
               .expect("Failed to set mode");
            card
               .destroy_framebuffer(initial_primary_fb)
               .expect("Failed to destroy initial framebuffer");
         }
         // If we have new displays, add them to the display tree
         if !new_displays.is_empty() {
            let displays =
               displays
                  .into_objects()
                  .into_iter()
                  .chain(new_displays.into_iter().map(|(display, _, _)| display))
                  .collect();
            displays = todo![];
         }
         // 1. get next buffer(s)
         //
         // 2. render something
         //
         // 3. atomic commit to display
         //
         // 4. wait for vsync
         // BUG: What happens if Window A-1 is in GPU B's display regions?
         let events = match card.receive_events() {
            Ok(events) => events.peekable(),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
               continue
            },
            Err(e) => panic!["{e}"],
         };
         events.for_each(
            |event| {
               match event {
                  drm::control::Event::PageFlip(event) => {
                     let mut to_remove = Vec::new();
                     for (id, display) in displays.select_all_items_mut() {
                        if display.crtc != event.crtc {
                           continue;
                        }
                        // Draw to the back buffer
                        egl
                           .make_current(
                              *egldisplay,
                              Some(display.primary.eglsurface),
                              None,
                              Some(*eglctx),
                           )
                           .expect("Failed to make surface current");
                        unsafe {
                           gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, 0);
                           gl::BindFramebuffer(gl::READ_FRAMEBUFFER, bg.fb_id);
                           gl::BlitFramebuffer(
                              0,
                              0,
                              aligned_width as i32,
                              aligned_height as i32,
                              0,
                              0,
                              aligned_width as i32,
                              aligned_height as i32,
                              gl::COLOR_BUFFER_BIT,
                              gl::LINEAR,
                           );
                           // Can blit more after this as well, just change the read buffer!
                        }
                        // Swap the buffers
                        //? SAFETY: This is safe here because we are calling it right after a page flip
                        //? event, indicating the hardware is no longer using it
                        match unsafe {
                           display.primary.swap(card, egl, egldisplay)
                        } {
                           Ok(()) => (),
                           // Probably disconnected: remove the display from the list
                           Err(_) => if let Ok(info) = card.get_connector(display.connector, false) {
                              if info.state() != connector::State::Connected {
                                 to_remove.push(id);
                              }
                           },
                        }
                     }
                     for id in to_remove {
                        // Quadtree.remove only overwrites location with null, does not rearrange displays
                        if let Some(display) = displays.remove(id) {
                           for fb in display
                              .primary
                              .fbs
                              .into_inner()
                              .values()
                              .chain(display.cursor.fbs.into_inner().values()) {
                              card.destroy_framebuffer(*fb).ok();
                           }
                           for overlay in display.overlays.into_iter() {
                              for fb in overlay.fbs.into_inner().values() {
                                 card.destroy_framebuffer(*fb).ok();
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
      if Instant::now().checked_duration_since(end).is_some() {
         break;
      }
      frame += 1;
   }
}
