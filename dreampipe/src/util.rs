use crate::config::CompositorConfig;
use crate::VIRTUAL_SCREEN_EXTENTS;
use crate::config::Config;
use crate::display::Display;
use euclid::Box2D;
use euclid::Point2D;
use euclid::Vector2D;
use khregl::DynamicInstance;
use khregl::EGL1_5;
use std::os::raw::c_void;
use std::str::FromStr;

pub type Point2 = Point2D<i32, ()>;
pub type Vec2 = Vector2D<i32, ()>;
pub type Bounds2 = Box2D<i32, ()>;

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

impl From<DisplayPosition> for Point2 {
   fn from(value: DisplayPosition) -> Self {
      <DisplayPosition as Into<(i32, i32)>>::into(value).into()
   }
}

#[derive(Eq, PartialEq)]
pub enum Direction {
   East,
   South,
   North,
   West,
}

// pub fn resolve_collisions(
//    output: &Quadtree<'_, Display>,
//    bounds: &mut euclid::Box2D<i32, ()>,
// ) {
//    let mut moved: Option<Direction> = None;
//    loop {
//       let select = output.select_items(*bounds);
//       if select.is_empty() {
//          return;
//       }
//       for intheway in select.into_iter().map(|d| d.bounds()) {
//          let dleft = bounds.max.x - intheway.min.x;
//          let dright = intheway.max.x - bounds.min.x;
//          let dup = bounds.max.y - intheway.min.y;
//          let ddown = intheway.max.y - bounds.min.y;
//          let min = dleft.min(dright).min(dup).min(ddown);
//          match [min == dleft, min == dright, min == dup, min == ddown] {
//             [true, ..] if moved != Some(Direction::East) => {
//                moved = Some(Direction::West);
//                *bounds = bounds.translate((-dleft, 0).into());
//                continue;
//             },
//             [_, true, ..] if moved != Some(Direction::West) => {
//                if (i32::MAX - bounds.size().width) < bounds.max.x {
//                   // Reverse
//                   moved = Some(Direction::West);
//                   *bounds = bounds.translate((-dleft, 0).into());
//                   continue;
//                }
//                moved = Some(Direction::East);
//                *bounds = bounds.translate((dright, 0).into());
//                continue;
//             },
//             [_, _, true, ..] if moved != Some(Direction::South) => {
//                moved = Some(Direction::North);
//                *bounds = bounds.translate((0, dup).into());
//                continue;
//             },
//             [_, _, _, true] if moved != Some(Direction::North) => {
//                if (i32::MAX - bounds.size().height) < bounds.max.y {
//                   moved = Some(Direction::North);
//                   *bounds = bounds.translate((0, dup).into());
//                   continue;
//                }
//                moved = Some(Direction::South);
//                *bounds = bounds.translate((0, -ddown).into());
//                continue;
//             },
//             [..] => (),
//          }
//       }
//    }
// }

// pub fn arrange_displays<
//    'a,
//    'b,
// >(displays: Vec<Display>, config: &'a Config) -> Quadtree<'b, Display> {
//    println!["Start"];
//    let mut arranging =
//       Quadtree::new(Bounds2::new((0, 0).into(), VIRTUAL_SCREEN_EXTENTS.into()));
//    if displays.is_empty() {
//       return arranging;
//    }
//    let mut leftover = Vec::new();

//    // Set configured positions first, otherwise mark which ones need to be set
//    for mut display in displays.into_iter() {
//       if let Some(new_pos) =
//          config.get::<DisplayPosition>(
//             &CompositorConfig::offset_key(&display.name()),
//          ) {
//          display.pos = new_pos.into();
//          let mut bounds = display.bounds();
//          resolve_collisions(&arranging, &mut bounds);
//          display.pos = bounds.min;
//          arranging.insert(bounds, display);
//       } else {
//          display.pos = (0, 0).into();
//          leftover.push(display);
//       }
//    }
//    println!["Mid"];
//    for mut display in leftover.into_iter() {
//       let mut bounds = display.bounds();
//       resolve_collisions(&arranging, &mut bounds);
//       display.pos = bounds.min;
//       arranging.insert(bounds, display);
//    }
//    println!["Mid2"];
//    let mut min = Vec2::zero();
//    for (_, display) in arranging.select_all_items() {
//       min.x = min.x.min(display.pos.x);
//       min.y = min.y.min(display.pos.y);
//    }
//    println!["Mid3"];
//    let output =
//       // Will always be 0 or negative
//       if min.x != 0 || min.y != 0 {
//          let displays = arranging.into_objects();
//          let mut displacing =
//             Quadtree::new(
//                Bounds2::new((0, 0).into(), VIRTUAL_SCREEN_EXTENTS.into()),
//             );
//          for mut display in displays {
//             display.pos -= min;
//             displacing.insert(display.bounds(), display);
//          }
//          displacing
//       } else {
//          arranging
//       };
//    println!["Done inserting displays into quadtree"];
//    output
// }

pub fn create_context(
   egl: &khregl::DynamicInstance,
   display: khregl::Display,
) -> (khregl::Context, khregl::Config) {
   let attributes =
      [
         khregl::RED_SIZE,
         8,
         khregl::GREEN_SIZE,
         8,
         khregl::BLUE_SIZE,
         8,
         khregl::ALPHA_SIZE,
         8,
         khregl::SURFACE_TYPE,
         khregl::WINDOW_BIT,
         khregl::RENDERABLE_TYPE,
         khregl::OPENGL_ES3_BIT,
         khregl::NONE,
      ];
   let config =
      egl
         .choose_first_config(display, &attributes)
         .expect("unable to choose an EGL configuration")
         .expect("no EGL configuration found");
   let context_attributes = [khregl::CONTEXT_CLIENT_VERSION, 3, khregl::NONE];
   let context =
      egl
         .create_context(display, config, None, &context_attributes)
         .expect("unable to create an EGL context");
   (context, config)
}

#[allow(non_snake_case)]
pub struct GlFns {
   pub EGLImageTargetTexture2DOES: unsafe extern "system" fn(u32, *const c_void),
}

impl GlFns {
   pub fn load(egl: &DynamicInstance<EGL1_5>) -> GlFns {
      GlFns {
         EGLImageTargetTexture2DOES: unsafe {
            std::mem::transmute(
               egl.get_proc_address("glEGLImageTargetTexture2DOES").unwrap() as
                  *const c_void,
            )
         },
      }
   }
}

#[allow(dead_code)]
pub struct BackgroundImage {
   pub bo: gbm::BufferObject<()>,
   pub egl_image: khregl::Image,
   pub tex_id: u32,
   pub fb_id: u32,
}
