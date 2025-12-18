mod buffer;
mod context;
mod display;
mod error;
mod fourcc;
mod gpu;
mod util;

use crate::context::AppContext;
use crate::context::Card;
use crate::display::Display;
use crate::util::config::CompositorConfig;
use crate::util::config::Config;
use crate::util::layout_displays;
use nix::fcntl::FcntlArg;
use nix::fcntl::OFlag;
use nix::fcntl::fcntl;
use notify::Event as NotifyEvent;
use notify::RecursiveMode;
use notify::Watcher;
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;
use taffy::NodeId;
use taffy::TaffyTree;

// 65536x65536
pub const VIRTUAL_SCREEN_EXTENTS: (i32, i32) = (0x10000, 0x10000);

#[tokio::main]
async fn main() {
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

  // Create gbm devices and default bg buffers
  let mut contexts: Vec<AppContext> = Vec::new();

  // Create gbm contexts and copy bg into gpu memory
  for card in cards.into_iter() {
    let context: AppContext = AppContext::init(card).await;
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
