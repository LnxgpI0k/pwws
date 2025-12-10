pub use drm::control::Device as ControlDevice;
use std::os::fd::AsFd;
use std::os::fd::BorrowedFd;
use drm::Device;
use crate::error::CompositorError;
use crate::error::CompositorResult;

fn get_pci_ids_from_card(card_num: u32) -> Option<(u32, u32)> {
   let sys_path = format!("/sys/class/drm/card{}/device", card_num);
   let vendor = std::fs::read_to_string(format!("{}/vendor", sys_path)).ok()?;
   let device = std::fs::read_to_string(format!("{}/device", sys_path)).ok()?;
   let vendor_id =
      u32::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok()?;
   let device_id =
      u32::from_str_radix(device.trim().trim_start_matches("0x"), 16).ok()?;
   Some((vendor_id, device_id))
}

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
            options
               .open(path)
               .map_err(|e| CompositorError::OpenCard(path.into(), e))?,
            card_num,
         ),
      )
   }

   pub fn open_all() -> Vec<Card> {
      let mut cards = Vec::with_capacity(16);
      for card_num in 0 .. 16 {
         if let Ok(card) = Card::open(card_num) {
            if let Some((vendor_id, device_id)) = get_pci_ids_from_card(card_num) {
               println![
                  "Opened card{card_num}: vendor=0x{vendor_id:x}, device=0x{device_id:x}"
               ];
               cards.push(card);
            }
         }
      }
      cards
   }

   // pub fn from_raw(fd: RawFd) -> Self {
   //    Self(unsafe {
   //       std::fs::File::from_raw_fd(fd)
   //    }, u32::MAX)
   // }
   pub fn num(&self) -> u32 {
      self.1
   }
}

impl Device for Card { }

impl ControlDevice for Card { }
