use crate::display::Display;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::os::raw::c_void;
use std::str::FromStr;
use taffy::Dimension;
use taffy::Display as NodeDisplay;
use taffy::FlexDirection;
use taffy::NodeId;
use taffy::Size;
use taffy::Style;
use taffy::TaffyTree;

pub struct DisplayPosition {
  x: u32,
  y: u32,
}

impl FromStr for DisplayPosition {
  type Err = String;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let split = s.split(' ').collect::<Vec<_>>();
    if split.len() != 2 {
      return Err(
        String::from("Cannot parse display position: Missing space separator"),
      );
    }
    let [x, y, ..] = &split[..] else {
      unreachable![]
    };
    Ok(
      Self {
        x: x
          .parse()
          .map_err(
            |e| format!["Failed to parse {} into u32 for display position: {e}", x],
          )?,
        y: y
          .parse()
          .map_err(
            |e| format!["Failed to parse {} into u32 for display position: {e}", y],
          )?,
      },
    )
  }
}

impl From<DisplayPosition> for (u32, u32) {
  fn from(value: DisplayPosition) -> Self {
    (value.x, value.y)
  }
}

impl From<DisplayPosition> for (i32, i32) {
  fn from(value: DisplayPosition) -> Self {
    (value.x as i32, value.y as i32)
  }
}

#[derive(Eq, PartialEq)]
pub enum Direction {
  East,
  South,
  North,
  West,
}

#[allow(non_snake_case)]
pub struct GlFns {
  pub EGLImageTargetTexture2DOES: unsafe extern "system" fn(u32, *const c_void),
}

#[allow(dead_code)]
pub struct BackgroundImage {
  pub bo: gbm::BufferObject<()>,
  pub tex_id: u32,
  pub fb_id: u32,
  pub width: i32,
  pub height: i32,
}

pub fn layout_displays(
  mut displays: HashMap<String, &mut Display>,
) -> (TaffyTree<String>, Vec<NodeId>) {
  let mut sorted = BTreeMap::new();
  for (_, display) in displays.iter() {
    let pos: (i32, i32) = display.pos.into();
    let key = (pos.1, pos.0);
    sorted.insert(key, (display.name.to_owned(), Size {
      width: Dimension::length(display.size.0 as f32),
      height: Dimension::length(display.size.1 as f32),
    }));
  }
  let mut tree: TaffyTree<String> = TaffyTree::<String>::new();
  let mut leafs = Vec::new();
  let mut hnodes = Vec::new();
  let mut vnodes = Vec::new();
  let mut prev_y = 0;
  for ((y, _), (name, size)) in sorted {
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
  for leaf in leafs.iter() {
    let nym = tree.get_node_context(*leaf).unwrap();
    let display = displays.get_mut(nym).unwrap();
    let pos = tree.layout(*leaf).unwrap().location;
    display.pos = (pos.x as i32, pos.y as i32);
  }
  (tree, leafs)
}
