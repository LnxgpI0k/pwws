use khregl::DynamicInstance;
use khregl::EGL1_5;
use std::os::raw::c_void;
use std::str::FromStr;

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
          egl.get_proc_address("glEGLImageTargetTexture2DOES").unwrap() as *const c_void,
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
  pub width: i32,
  pub height: i32,
}
