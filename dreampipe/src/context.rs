pub use drm::control::Device as ControlDevice;
use crate::display::Display;
use crate::error::CompositorError;
use crate::error::CompositorResult;
use crate::gpu::init_gpu;
use crate::gpu::load_default_bg;
use crate::util::DisplayPosition;
use crate::util::config::CompositorConfig;
use crate::util::config::Config;
use drm::Device;
use drm::control::AtomicCommitFlags;
use drm::control::atomic;
use drm::control::connector;
use std::collections::HashSet;
use std::os::fd::AsFd;
use std::os::fd::BorrowedFd;

// Throw this thing wherever you need it!
pub struct Card(std::fs::File, u32);

impl AsFd for Card {
  fn as_fd(&self) -> BorrowedFd<'_> {
    self.0.as_fd()
  }
}

impl Card {
  pub fn open(card_num: u32) -> CompositorResult<Card> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    options.write(true);
    let path = &format!["/dev/dri/card{card_num}"];
    Ok(
      Card(
        options.open(path).map_err(|e| CompositorError::OpenCard(path.into(), e))?,
        card_num,
      ),
    )
  }

  pub fn open_all() -> Vec<Card> {
    let mut cards = Vec::with_capacity(16);
    for card_num in 0 .. 16 {
      if let Ok(card) = Card::open(card_num) {
        if let Some((vendor_id, device_id)) = card.pci_ids() {
          println![
            "Opened card{card_num}: vendor=0x{vendor_id:x}, device=0x{device_id:x}"
          ];
          cards.push(card);
        }
      }
    }
    cards
  }

  pub fn pci_ids(&self) -> Option<(u32, u32)> {
    let sys_path = format!("/sys/class/drm/card{}/device", self.num());
    let vendor = std::fs::read_to_string(format!("{}/vendor", sys_path)).ok()?;
    let device = std::fs::read_to_string(format!("{}/device", sys_path)).ok()?;
    let vendor_id =
      u32::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok()?;
    let device_id =
      u32::from_str_radix(device.trim().trim_start_matches("0x"), 16).ok()?;
    Some((vendor_id, device_id))
  }

  pub fn num(&self) -> u32 {
    self.1
  }
}

impl Device for Card { }

impl ControlDevice for Card { }

pub struct AppContext {
  pub card: Box<Card>,
  pub gbm: gbm::Device<&'static Card>,
  pub gpu: wgpu::Device,
  pub adapter: wgpu::Adapter,
  pub queue: wgpu::Queue,
  pub bg_bindgroup: wgpu::BindGroup,
  pub displays: Vec<Display>,
}

impl AppContext {
  pub async fn init(card: Card) -> Self {
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
    let (gpu, adapter, queue) =
      init_gpu(card_ref).await.expect("Failed to init wgpu");
    let bg_bindgroup = load_default_bg(card_ref, &gpu, &queue);
    let displays: Vec<Display> = Vec::new();
    AppContext {
      card,
      gbm,
      gpu,
      adapter,
      queue,
      bg_bindgroup,
      displays,
    }
  }

  /// Returns true if display state was updated (eg a display disconnected)
  pub fn update(&mut self) -> bool {
    let events = match self.card.receive_events() {
      Ok(events) => {
        println!["Ready to receive events!"];
        events.peekable()
      },
      Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
        println!["Would block"];
        return false;
      },
      Err(e) => panic!["{e}"],
    };
    let mut disconnected = false;
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
                    disconnected = true;
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
    disconnected
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
      Display::init_displays(
        ignore_list,
        &self.card,
        &self.gbm,
        &self.gpu,
      ).unwrap_or_else(|e| {
        tracing::warn!["Failed to init displays: {e}"];
        Default::default()
      });
    for display in new_displays.iter_mut() {
      println!["Found display: {} {:?}", display.name, display.size];
      let mut atomic_req = atomic::AtomicModeReq::new();
      display.init_req(&self.card, &mut atomic_req).expect("Failed to init display");
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
