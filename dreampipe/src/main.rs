mod buffer;
mod card;
mod config;
mod display;
mod error;
mod fourcc;
mod gpu;
mod util;
mod render;

use config::Config;
use taffy::NodeId;
use crate::card::Card;
use crate::config::CompositorConfig;
use crate::display::Display;
use crate::gpu::GpuContext;
use crate::util::BackgroundImage;
use crate::util::layout_displays;
use drm::buffer::DrmFourcc;
use drm::control::Device as ControlDevice;
use gbm::BufferObjectFlags;
use image::DynamicImage;
use image::GenericImage;
use nix::fcntl::FcntlArg;
use nix::fcntl::OFlag;
use nix::fcntl::fcntl;
use notify::Event as NotifyEvent;
use notify::RecursiveMode;
use notify::Watcher;
use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Duration;
use std::time::Instant;
use taffy::TaffyTree;

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
    // Put the opaque card pointer on the heap to allow us to move owned pointers to it without moving the card memory itself
    let card: Box<Card> = Box::new(card);

    // Consume the box without deallocating it, returning a pointer to the heap memory
    let card_ptr: *mut Card = Box::leak(card);

    // Clone the raw pointer as const
    let card_ptr_clone: *const Card = card_ptr as *const Card;

    // Convert the pointer to a static reference (lives for lifetime of program)
    let card_ref: &'static Card = unsafe {
      &*card_ptr_clone
    };

    // Take back ownership of the heap memory so it is freed when the program exits
    let card: Box<Card> = unsafe {
      Box::from_raw(card_ptr)
    };

    // Use the static reference
    let gbm: gbm::Device<&'static Card> =
      gbm::Device::new(card_ref).expect("Failed to init GBM with device");
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
    let displays: Vec<Display> = Vec::new();
    let context = GpuContext {
      card,
      gbm,
      displays,
    };
    contexts.push(context);
  }
  let end = Instant::now() + Duration::from_secs(5);
  let mut frame = 0usize;
  let mut layout: TaffyTree<String> = TaffyTree::new();
  let mut leaf_ids: Vec<NodeId> = Vec::new();
  loop {
    let mut displays_changed = false;

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
      displays_changed |= context.update();
      displays_changed |= context.init_displays(&config);
    }
    if Instant::now().checked_duration_since(end).is_some() {
      break;
    }
    frame += 1;
    if displays_changed {
      let displays: HashMap<String, &mut Display> =
        contexts
          .iter_mut()
          .map(|context| context.displays_mut())
          .flatten()
          .map(|display| (display.name.to_owned(), display))
          .collect();
      (layout, leaf_ids) = layout_displays(displays);
    }
  }
}
