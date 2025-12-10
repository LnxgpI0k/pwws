use drm::control;
use drm::control::atomic;
use drm::control::crtc;
use drm::control::framebuffer;
use drm::control::plane;
use drm::control::property;
pub use drm::control::Device as ControlDevice;
use drm::control::PlaneType;
use khregl::EGL1_5;
use std::collections::HashMap;
use std::collections::HashSet;
use drm::control::connector;
use drm::ClientCapability;
use drm::Device;
use crate::buffer::DrmCtx;
use crate::buffer::CURSOR_DIM;
use crate::error::CompositorError;
use crate::error::CompositorResult;
use crate::card::Card;
use crate::util::Bounds2;
use crate::util::Point2;
use crate::util::Vec2;

#[allow(unused)]
fn print_formats(card: &Card, plane: plane::Handle) {
   let prop_vals: HashMap<property::Handle, u64> =
      card.get_properties(plane).unwrap().into_iter().collect();
   let props = card.get_properties(plane).unwrap().as_hashmap(card).unwrap();
   let formats_val = prop_vals[&props["IN_FORMATS"].handle()];
   let blob = card.get_property_blob(formats_val).unwrap();
   println!["formats: {:?}", blob.as_slice()];
}

#[derive(Debug)]
pub struct Display {
   name: String,
   pub size: Vec2,
   pub pos: Point2,
   pub connector: connector::Handle,
   pub crtc: crtc::Handle,
   pub connector_props: HashMap<String, property::Info>,
   pub crtc_props: HashMap<String, property::Info>,
   pub mode: control::Mode,
   pub primary: DrmCtx,
   pub cursor: DrmCtx,
   pub overlays: Vec<DrmCtx>,
}

impl std::hash::Hash for Display {
   fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
      self.name.hash(state);
   }
}

impl Eq for Display { }

impl PartialEq for Display {
   fn eq(&self, other: &Self) -> bool {
      self.name == other.name
   }
}

impl Display {
   pub fn name(&self) -> &str {
      &self.name
   }

   pub fn bounds(&self) -> Bounds2 {
      Bounds2::new(self.pos.into(), Point2::from(self.pos) + Vec2::from(self.size))
   }

   pub fn init_req(
      &self,
      card: &Card,
      initial_fb: framebuffer::Handle,
      atomic_req: &mut atomic::AtomicModeReq,
   ) -> CompositorResult<()> {
      atomic_req.add_property(
         self.connector,
         self.connector_props["CRTC_ID"].handle(),
         property::Value::CRTC(Some(self.crtc)),
      );
      let blob =
         card.create_property_blob(&self.mode).expect("Failed to create a blob");
      atomic_req.add_property(self.crtc, self.crtc_props["MODE_ID"].handle(), blob);
      atomic_req.add_property(
         self.crtc,
         self.crtc_props["ACTIVE"].handle(),
         property::Value::Boolean(true),
      );
      Ok(())
   }
}

pub fn init_displays(
   ignore_list: impl Into<Option<HashSet<String>>>,
   card: &Card,
   gbm: &gbm::Device<&'static Card>,
   egl: &khregl::DynamicInstance<EGL1_5>,
   egldisplay: &khregl::Display,
   config: &khregl::Config,
) -> CompositorResult<Vec<(Display, gbm::BufferObject<()>, gbm::BufferObject<()>)>> {
   let ignore_list = ignore_list.into();
   for (
      cap,
      enable,
   ) in [
      (ClientCapability::UniversalPlanes, true),
      (ClientCapability::Atomic, true),
   ].into_iter() {
      card.set_client_capability(cap, enable).map_err(|err| {
         CompositorError::ClientCapability(cap, err)
      })?;
   }
   let resources = card.resource_handles().map_err(|err| {
      CompositorError::ResourcesError(err)
   })?;
   println!["Getting all connected connectors"];
   let connected: Vec<connector::Info> =
      resources
         .connectors()
         .iter()
         .flat_map(|con| card.get_connector(*con, true))
         .filter(|i| i.state() == connector::State::Connected && !i.modes().is_empty())
         .collect();
   if connected.is_empty() {
      Err(CompositorError::NoQualifiedConnectors)?;
   }
   let mut planes: HashSet<plane::Handle> =
      card
         .plane_handles()
         .map_err(|err| CompositorError::GetPlanes(err))?
         .into_iter()
         .collect();
   println!["Organizing the display objects."];
   let max_displays = resources.crtcs().len().min(connected.len());
   let mut displays = Vec::new();
   for (
      connector,
      &crtc,
   ) in connected.into_iter().take(max_displays).zip(resources.crtcs()) {
      let name =
         format![
            "card{}-{}-{}",
            card.num(),
            connector.interface().as_str(),
            connector.interface_id()
         ];
      if let Some(ref ignore_list) = ignore_list {
         if ignore_list.contains(&name) {
            continue;
         }
      }
      let mode = *connector.modes().first().unwrap();
      let size = match mode.size() {
         (width, height) => (width as i32, height as i32),
      };
      let (primary, initial_primary_bo) =
         DrmCtx::from_connector(
            card,
            gbm,
            egl,
            config,
            egldisplay,
            &resources,
            crtc,
            &mut planes,
            PlaneType::Primary,
            (size.0 as u32, size.1 as u32),
         )?;
      let (cursor, initial_cursor_bo) =
         DrmCtx::from_connector(
            card,
            gbm,
            egl,
            config,
            egldisplay,
            &resources,
            crtc,
            &mut planes,
            PlaneType::Cursor,
            (CURSOR_DIM, CURSOR_DIM),
         )?;
      let connector_props =
         card
            .get_properties(connector.handle())
            .map_err(
               |err| CompositorError::GetConnectorProperties(connector.handle(), err),
            )?
            .as_hashmap(card)
            .map_err(|err| CompositorError::PropsToHashMap(err))?;
      let crtc_props =
         card
            .get_properties(crtc)
            .map_err(|err| CompositorError::GetCrtcProperties(crtc, err))?
            .as_hashmap(card)
            .map_err(|err| CompositorError::PropsToHashMap(err))?;
      let size = size.into();
      displays.push((Display {
         name,
         size,
         pos: Default::default(),
         connector: connector.handle(),
         crtc,
         connector_props,
         crtc_props,
         mode,
         primary,
         cursor,
         overlays: vec![],
      }, initial_primary_bo, initial_cursor_bo));
   }
   Ok(displays)
}
