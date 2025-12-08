#![feature(prelude_import)]
#[prelude_import]
use std::prelude::rust_2024::*;
#[macro_use]
extern crate std;
mod buffer {
    use drm::control::atomic;
    use drm::control::crtc;
    use drm::control::framebuffer;
    use drm::control::plane;
    use drm::control::property;
    use drm::control::AtomicCommitFlags;
    pub use drm::control::Device as ControlDevice;
    use drm::control::PlaneType;
    use drm::control::ResourceHandles;
    use gbm::AsRaw;
    use gbm::BufferObjectFlags;
    use khregl::ATTRIB_NONE;
    use khregl::EGL1_5;
    use std::cell::UnsafeCell;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::os::fd::AsRawFd;
    use crate::error::CompositorError;
    use crate::error::CompositorResult;
    use crate::card::Card;
    use crate::fourcc::FourCc;
    use crate::DRM_FORMAT;
    use std::os::raw::c_void;
    pub const CURSOR_DIM: u32 = 64;
    fn is_plane_compatible_with_crtc(
        card: &Card,
        resources: &ResourceHandles,
        plane: plane::Handle,
        crtc: crtc::Handle,
    ) -> bool {
        let plane_info = card.get_plane(plane).unwrap();
        resources.filter_crtcs(plane_info.possible_crtcs()).contains(&crtc)
    }
    fn find_compatible_plane(
        card: &Card,
        resources: &ResourceHandles,
        crtc: crtc::Handle,
        planes: &mut HashSet<plane::Handle>,
        planetype: PlaneType,
    ) -> Option<plane::Handle> {
        let mut compatible = Vec::new();
        for &plane in planes.iter() {
            if is_plane_compatible_with_crtc(card, &resources, plane, crtc) {
                compatible.push(plane);
            }
        }
        let mut plane = None;
        'outer: for p in compatible.into_iter() {
            if let Ok(plane_props) = card.get_properties(p) {
                for (&id, &val) in plane_props.iter() {
                    if let Ok(info) = card.get_property(id) {
                        if info.name().to_str().map(|x| x == "type").unwrap_or(false) {
                            if val == planetype as u64 {
                                plane = Some(p);
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
        plane
    }
    #[allow(unused)]
    fn make_buffer(
        card: &Card,
        gbm: &gbm::Device<&Card>,
        planetype: PlaneType,
        size: (u32, u32),
    ) -> Result<gbm::BufferObject<()>, CompositorError> {
        let planeflag = match planetype {
            PlaneType::Overlay | PlaneType::Primary => BufferObjectFlags::SCANOUT,
            PlaneType::Cursor => BufferObjectFlags::CURSOR,
        };
        let buffer = gbm
            .create_buffer_object::<
                (),
            >(size.0, size.1, DRM_FORMAT, planeflag | BufferObjectFlags::RENDERING)
            .map_err(|err| { CompositorError::GbmCreation(err) })?;
        Ok(buffer)
    }
    fn make_surfaces(
        gbm: &gbm::Device<&'static Card>,
        egl: &khregl::DynamicInstance<EGL1_5>,
        config: &khregl::Config,
        egldisplay: &khregl::Display,
        planetype: PlaneType,
        (width, height): (u32, u32),
    ) -> CompositorResult<(gbm::Surface<&'static Card>, khregl::Surface)> {
        let planeflag = match planetype {
            PlaneType::Overlay | PlaneType::Primary => BufferObjectFlags::SCANOUT,
            PlaneType::Cursor => BufferObjectFlags::CURSOR,
        };
        let gbmsurface = gbm
            .create_surface::<
                &'static Card,
            >(width, height, DRM_FORMAT, planeflag | BufferObjectFlags::RENDERING)
            .map_err(|e| CompositorError::GbmSurfaceCreate(e))?;
        let eglsurface = unsafe {
            egl.create_platform_window_surface(
                *egldisplay,
                *config,
                gbmsurface.as_raw() as *mut c_void,
                &[ATTRIB_NONE],
            )
        }
            .map_err(|e| CompositorError::EglSurfaceCreate(e))?;
        Ok((gbmsurface, eglsurface))
    }
    pub struct DrmCtx {
        pub plane: Option<plane::Handle>,
        pub plane_props: HashMap<String, property::Info>,
        pub size: (u32, u32),
        pub current_bo: Option<gbm::BufferObject<&'static Card>>,
        pub fbs: UnsafeCell<HashMap<i32, framebuffer::Handle>>,
        pub gbmsurface: gbm::Surface<&'static Card>,
        pub eglsurface: khregl::Surface,
    }
    impl DrmCtx {
        pub fn new(
            card: &Card,
            gbm: &gbm::Device<&'static Card>,
            egl: &khregl::DynamicInstance<EGL1_5>,
            config: &khregl::Config,
            egldisplay: &khregl::Display,
            plane: Option<plane::Handle>,
            planetype: PlaneType,
            size: (u32, u32),
        ) -> CompositorResult<(Self, gbm::BufferObject<()>)> {
            let (gbmsurface, eglsurface) = make_surfaces(
                gbm,
                egl,
                config,
                egldisplay,
                planetype,
                size,
            )?;
            let plane_props = if let Some(plane) = plane {
                card.get_properties(plane)
                    .map_err(|err| CompositorError::GetPlaneProperties(plane, err))?
                    .as_hashmap(card)
                    .map_err(|err| CompositorError::PropsToHashMap(err))?
            } else {
                HashMap::new()
            };
            let initial_buffer = make_buffer(card, gbm, planetype, size)?;
            let fbs = HashMap::new();
            Ok((
                Self {
                    plane,
                    plane_props,
                    size,
                    current_bo: None,
                    fbs: fbs.into(),
                    gbmsurface,
                    eglsurface,
                },
                initial_buffer,
            ))
        }
        pub fn from_connector(
            card: &Card,
            gbm: &gbm::Device<&'static Card>,
            egl: &khregl::DynamicInstance<EGL1_5>,
            config: &khregl::Config,
            egldisplay: &khregl::Display,
            resources: &ResourceHandles,
            crtc: crtc::Handle,
            planes: &mut HashSet<plane::Handle>,
            planetype: PlaneType,
            size: (u32, u32),
        ) -> CompositorResult<(Self, gbm::BufferObject<()>)> {
            let plane = find_compatible_plane(card, resources, crtc, planes, planetype);
            if let Some(plane) = plane {
                let _ = planes.remove(&plane);
            }
            Self::new(card, gbm, egl, config, egldisplay, plane, planetype, size)
        }
        fn get_fb(
            &self,
            card: &Card,
            bo: &gbm::BufferObject<&'static Card>,
        ) -> CompositorResult<framebuffer::Handle> {
            let fd = bo.fd().map_err(|e| CompositorError::GbmFd(e))?.as_raw_fd();
            if let Some(fb) = unsafe { self.fbs.get().as_ref().unwrap().get(&fd) } {
                Ok(*fb)
            } else {
                let fb = card
                    .add_framebuffer(bo, DRM_FORMAT.depth(), DRM_FORMAT.bpp())
                    .map_err(|err| CompositorError::AddFrameBuffer(err))?;
                let fbs = (unsafe { &mut *self.fbs.get() })
                    as &mut HashMap<i32, framebuffer::Handle>;
                fbs.insert(fd, fb);
                Ok(fb)
            }
        }
        pub fn init_req(
            &self,
            card: &Card,
            initial_fb: framebuffer::Handle,
            atomic_req: &mut atomic::AtomicModeReq,
            crtc: crtc::Handle,
        ) -> CompositorResult<()> {
            if let Some(plane) = self.plane {
                let props = &self.plane_props;
                atomic_req
                    .add_property(
                        plane,
                        props["FB_ID"].handle(),
                        property::Value::Framebuffer(Some(initial_fb)),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["CRTC_ID"].handle(),
                        property::Value::CRTC(Some(crtc)),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["SRC_X"].handle(),
                        property::Value::UnsignedRange(0),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["SRC_Y"].handle(),
                        property::Value::UnsignedRange(0),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["SRC_W"].handle(),
                        property::Value::UnsignedRange((self.size.0 as u64) << 16),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["SRC_H"].handle(),
                        property::Value::UnsignedRange((self.size.1 as u64) << 16),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["CRTC_X"].handle(),
                        property::Value::SignedRange(0),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["CRTC_Y"].handle(),
                        property::Value::SignedRange(0),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["CRTC_W"].handle(),
                        property::Value::UnsignedRange(self.size.0 as u64),
                    );
                atomic_req
                    .add_property(
                        plane,
                        props["CRTC_H"].handle(),
                        property::Value::UnsignedRange(self.size.1 as u64),
                    );
            }
            Ok(())
        }
        /// SAFETY: This function must be called exactly once after a page flip event, not
        /// before
        pub unsafe fn swap(
            &mut self,
            card: &Card,
            egl: &khregl::DynamicInstance<EGL1_5>,
            display: &khregl::Display,
        ) -> CompositorResult<()> {
            if let Some(plane) = self.plane {
                self.current_bo.take();
                egl.swap_buffers(*display, self.eglsurface)
                    .map_err(|e| CompositorError::BufferSwap(e))?;
                let bo = unsafe {
                    self.gbmsurface
                        .lock_front_buffer()
                        .map_err(|_| CompositorError::FrontBufferLock)?
                };
                let fb = self.get_fb(card, &bo)?;
                let props = &self.plane_props;
                let mut atomic_req = atomic::AtomicModeReq::new();
                atomic_req
                    .add_property(
                        plane,
                        props["FB_ID"].handle(),
                        property::Value::Framebuffer(Some(fb)),
                    );
                card.atomic_commit(
                        AtomicCommitFlags::NONBLOCK | AtomicCommitFlags::PAGE_FLIP_EVENT,
                        atomic_req,
                    )
                    .map_err(|err| CompositorError::AtomicCommitFailed(err))?;
            }
            Ok(())
        }
    }
}
mod card {
    pub use drm::control::Device as ControlDevice;
    use std::os::fd::AsFd;
    use std::os::fd::BorrowedFd;
    use drm::Device;
    use crate::error::CompositorError;
    use crate::error::CompositorResult;
    fn get_pci_ids_from_card(card_num: u32) -> Option<(u32, u32)> {
        let sys_path = ::alloc::__export::must_use({
            ::alloc::fmt::format(format_args!("/sys/class/drm/card{0}/device", card_num))
        });
        let vendor = std::fs::read_to_string(
                ::alloc::__export::must_use({
                    ::alloc::fmt::format(format_args!("{0}/vendor", sys_path))
                }),
            )
            .ok()?;
        let device = std::fs::read_to_string(
                ::alloc::__export::must_use({
                    ::alloc::fmt::format(format_args!("{0}/device", sys_path))
                }),
            )
            .ok()?;
        let vendor_id = u32::from_str_radix(vendor.trim().trim_start_matches("0x"), 16)
            .ok()?;
        let device_id = u32::from_str_radix(device.trim().trim_start_matches("0x"), 16)
            .ok()?;
        Some((vendor_id, device_id))
    }
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
            let path = &::alloc::__export::must_use({
                ::alloc::fmt::format(format_args!("/dev/dri/card{0}", card_num))
            });
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
            for card_num in 0..16 {
                if let Ok(card) = Card::open(card_num) {
                    if let Some((vendor_id, device_id)) = get_pci_ids_from_card(
                        card_num,
                    ) {
                        {
                            ::std::io::_print(
                                format_args!(
                                    "Opened card{0}: vendor=0x{1:x}, device=0x{2:x}\n",
                                    card_num,
                                    vendor_id,
                                    device_id,
                                ),
                            );
                        };
                        cards.push(card);
                    }
                }
            }
            cards
        }
        pub fn num(&self) -> u32 {
            self.1
        }
    }
    impl Device for Card {}
    impl ControlDevice for Card {}
}
mod config {
    #![allow(unused)]
    use crate::error::CompositorError;
    use crate::error::CompositorResult;
    use microxdg::Xdg;
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::Read;
    use std::path::Path as FsPath;
    use std::path::PathBuf;
    use std::str::FromStr;
    use tracing::warn;
    pub struct CompositorConfig;
    impl CompositorConfig {
        pub fn config_path() -> Option<PathBuf> {
            let xdg = Xdg::new().ok();
            let config_dir = xdg.map(|xdg| xdg.config().ok()).flatten();
            config_dir.map(|dir| dir.to_path_buf().join("pwws").join("pwws.kv"))
        }
        pub fn offset_key(display_name: &str) -> String {
            ::alloc::__export::must_use({
                ::alloc::fmt::format(format_args!("{0}.offset", display_name))
            })
        }
    }
    /// Format is: key = value. Comments are with '#' or just write anything. Errors
    /// won't be raised on "extra" data, equals-sign present or not. Outer whitespace
    /// is stripped.
    pub struct Config {
        data: HashMap<String, String>,
    }
    #[automatically_derived]
    impl ::core::default::Default for Config {
        #[inline]
        fn default() -> Config {
            Config {
                data: ::core::default::Default::default(),
            }
        }
    }
    impl Config {
        /// Load from file
        pub fn from_str(s: &str) -> Self {
            let mut cfg = Self { data: HashMap::new() };
            {
                let data = &mut cfg.data;
                for (i, line) in s.lines().enumerate() {
                    let i = i + 1;
                    let line = line
                        .chars()
                        .take_while(|ch| *ch != '#')
                        .collect::<String>();
                    let line = line.trim();
                    if line != "" {
                        let (k, v) = if let Some(v) = line.split_once("=") {
                            v
                        } else {
                            {
                                use ::tracing::__macro_support::Callsite as _;
                                static __CALLSITE: ::tracing::callsite::DefaultCallsite = {
                                    static META: ::tracing::Metadata<'static> = {
                                        ::tracing_core::metadata::Metadata::new(
                                            "event compositor/src/config.rs:54",
                                            "compositor::config",
                                            ::tracing::Level::WARN,
                                            ::tracing_core::__macro_support::Option::Some(
                                                "compositor/src/config.rs",
                                            ),
                                            ::tracing_core::__macro_support::Option::Some(54u32),
                                            ::tracing_core::__macro_support::Option::Some(
                                                "compositor::config",
                                            ),
                                            ::tracing_core::field::FieldSet::new(
                                                &["message"],
                                                ::tracing_core::callsite::Identifier(&__CALLSITE),
                                            ),
                                            ::tracing::metadata::Kind::EVENT,
                                        )
                                    };
                                    ::tracing::callsite::DefaultCallsite::new(&META)
                                };
                                let enabled = ::tracing::Level::WARN
                                    <= ::tracing::level_filters::STATIC_MAX_LEVEL
                                    && ::tracing::Level::WARN
                                        <= ::tracing::level_filters::LevelFilter::current()
                                    && {
                                        let interest = __CALLSITE.interest();
                                        !interest.is_never()
                                            && ::tracing::__macro_support::__is_enabled(
                                                __CALLSITE.metadata(),
                                                interest,
                                            )
                                    };
                                if enabled {
                                    (|value_set: ::tracing::field::ValueSet| {
                                        let meta = __CALLSITE.metadata();
                                        ::tracing::Event::dispatch(meta, &value_set);
                                    })({
                                        #[allow(unused_imports)]
                                        use ::tracing::field::{debug, display, Value};
                                        let mut iter = __CALLSITE.metadata().fields().iter();
                                        __CALLSITE
                                            .metadata()
                                            .fields()
                                            .value_set(
                                                &[
                                                    (
                                                        &::tracing::__macro_support::Iterator::next(&mut iter)
                                                            .expect("FieldSet corrupted (this is a bug)"),
                                                        ::tracing::__macro_support::Option::Some(
                                                            &format_args!("Missing \'=\' on line {0}", i) as &dyn Value,
                                                        ),
                                                    ),
                                                ],
                                            )
                                    });
                                } else {
                                }
                            };
                            continue;
                        };
                        data.insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
            }
            cfg
        }
        /// Load from file
        pub fn new(path: impl AsRef<FsPath>) -> CompositorResult<Self> {
            let mut buf = String::new();
            {
                let mut f = File::open(path)
                    .map_err(|e| CompositorError::ConfigOpen(e))?;
                f.read_to_string(&mut buf).map_err(|e| CompositorError::ConfigRead(e))?;
            }
            Ok(Self::from_str(&buf))
        }
        /// Get a config value
        pub fn get<N: FromStr>(&self, k: &str) -> Option<N>
        where
            <N as FromStr>::Err: std::fmt::Display,
        {
            if let Some(v) = self.data.get(k) {
                match N::from_str(v) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        {
                            use ::tracing::__macro_support::Callsite as _;
                            static __CALLSITE: ::tracing::callsite::DefaultCallsite = {
                                static META: ::tracing::Metadata<'static> = {
                                    ::tracing_core::metadata::Metadata::new(
                                        "event compositor/src/config.rs:82",
                                        "compositor::config",
                                        ::tracing::Level::ERROR,
                                        ::tracing_core::__macro_support::Option::Some(
                                            "compositor/src/config.rs",
                                        ),
                                        ::tracing_core::__macro_support::Option::Some(82u32),
                                        ::tracing_core::__macro_support::Option::Some(
                                            "compositor::config",
                                        ),
                                        ::tracing_core::field::FieldSet::new(
                                            &["message"],
                                            ::tracing_core::callsite::Identifier(&__CALLSITE),
                                        ),
                                        ::tracing::metadata::Kind::EVENT,
                                    )
                                };
                                ::tracing::callsite::DefaultCallsite::new(&META)
                            };
                            let enabled = ::tracing::Level::ERROR
                                <= ::tracing::level_filters::STATIC_MAX_LEVEL
                                && ::tracing::Level::ERROR
                                    <= ::tracing::level_filters::LevelFilter::current()
                                && {
                                    let interest = __CALLSITE.interest();
                                    !interest.is_never()
                                        && ::tracing::__macro_support::__is_enabled(
                                            __CALLSITE.metadata(),
                                            interest,
                                        )
                                };
                            if enabled {
                                (|value_set: ::tracing::field::ValueSet| {
                                    let meta = __CALLSITE.metadata();
                                    ::tracing::Event::dispatch(meta, &value_set);
                                })({
                                    #[allow(unused_imports)]
                                    use ::tracing::field::{debug, display, Value};
                                    let mut iter = __CALLSITE.metadata().fields().iter();
                                    __CALLSITE
                                        .metadata()
                                        .fields()
                                        .value_set(
                                            &[
                                                (
                                                    &::tracing::__macro_support::Iterator::next(&mut iter)
                                                        .expect("FieldSet corrupted (this is a bug)"),
                                                    ::tracing::__macro_support::Option::Some(
                                                        &format_args!(
                                                            "Failed to parse \'{1}\' as {0}: {2}",
                                                            std::any::type_name::<N>(),
                                                            v,
                                                            e,
                                                        ) as &dyn Value,
                                                    ),
                                                ),
                                            ],
                                        )
                                });
                            } else {
                            }
                        };
                        None
                    }
                }
            } else {
                None
            }
        }
    }
}
mod display {
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
        let prop_vals: HashMap<property::Handle, u64> = card
            .get_properties(plane)
            .unwrap()
            .into_iter()
            .collect();
        let props = card.get_properties(plane).unwrap().as_hashmap(card).unwrap();
        let formats_val = prop_vals[&props["IN_FORMATS"].handle()];
        let blob = card.get_property_blob(formats_val).unwrap();
        {
            ::std::io::_print(format_args!("formats: {0:?}\n", blob.as_slice()));
        };
    }
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
    impl Eq for Display {}
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
            atomic_req
                .add_property(
                    self.connector,
                    self.connector_props["CRTC_ID"].handle(),
                    property::Value::CRTC(Some(self.crtc)),
                );
            let blob = card
                .create_property_blob(&self.mode)
                .expect("Failed to create a blob");
            atomic_req
                .add_property(self.crtc, self.crtc_props["MODE_ID"].handle(), blob);
            atomic_req
                .add_property(
                    self.crtc,
                    self.crtc_props["ACTIVE"].handle(),
                    property::Value::Boolean(true),
                );
            self.primary.init_req(card, initial_fb, atomic_req, self.crtc)?;
            self.cursor.init_req(card, initial_fb, atomic_req, self.crtc)?;
            for overlay in self.overlays.iter() {
                overlay.init_req(card, initial_fb, atomic_req, self.crtc)?;
            }
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
        for (cap, enable) in [
            (ClientCapability::UniversalPlanes, true),
            (ClientCapability::Atomic, true),
        ]
            .into_iter()
        {
            card.set_client_capability(cap, enable)
                .map_err(|err| { CompositorError::ClientCapability(cap, err) })?;
        }
        let resources = card
            .resource_handles()
            .map_err(|err| { CompositorError::ResourcesError(err) })?;
        {
            ::std::io::_print(format_args!("Getting all connected connectors\n"));
        };
        let connected: Vec<connector::Info> = resources
            .connectors()
            .iter()
            .flat_map(|con| card.get_connector(*con, true))
            .filter(|i| {
                i.state() == connector::State::Connected && !i.modes().is_empty()
            })
            .collect();
        if connected.is_empty() {
            Err(CompositorError::NoQualifiedConnectors)?;
        }
        let mut planes: HashSet<plane::Handle> = card
            .plane_handles()
            .map_err(|err| CompositorError::GetPlanes(err))?
            .into_iter()
            .collect();
        {
            ::std::io::_print(format_args!("Organizing the display objects.\n"));
        };
        let max_displays = resources.crtcs().len().min(connected.len());
        let mut displays = Vec::new();
        for (connector, &crtc) in connected
            .into_iter()
            .take(max_displays)
            .zip(resources.crtcs())
        {
            let name = ::alloc::__export::must_use({
                ::alloc::fmt::format(
                    format_args!(
                        "card{0}-{1}-{2}",
                        card.num(),
                        connector.interface().as_str(),
                        connector.interface_id(),
                    ),
                )
            });
            if let Some(ref ignore_list) = ignore_list {
                if ignore_list.contains(&name) {
                    continue;
                }
            }
            let mode = *connector.modes().first().unwrap();
            let size = match mode.size() {
                (width, height) => (width as i32, height as i32),
            };
            let (primary, initial_primary_bo) = DrmCtx::from_connector(
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
            let (cursor, initial_cursor_bo) = DrmCtx::from_connector(
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
            let connector_props = card
                .get_properties(connector.handle())
                .map_err(|err| CompositorError::GetConnectorProperties(
                    connector.handle(),
                    err,
                ))?
                .as_hashmap(card)
                .map_err(|err| CompositorError::PropsToHashMap(err))?;
            let crtc_props = card
                .get_properties(crtc)
                .map_err(|err| CompositorError::GetCrtcProperties(crtc, err))?
                .as_hashmap(card)
                .map_err(|err| CompositorError::PropsToHashMap(err))?;
            let size = size.into();
            displays
                .push((
                    Display {
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
                        overlays: ::alloc::vec::Vec::new(),
                    },
                    initial_primary_bo,
                    initial_cursor_bo,
                ));
        }
        Ok(displays)
    }
}
mod error {
    #![allow(dead_code)]
    use std::fmt::Display;
    use std::path::PathBuf;
    use drm::control::connector;
    use drm::control::crtc;
    use drm::control::encoder;
    use drm::control::plane;
    use drm::control::PlaneType;
    use drm::ClientCapability;
    use gbm::InvalidFdError;
    use std::io::Error as IoError;
    pub type CompositorResult<T> = std::result::Result<T, CompositorError>;
    pub enum CompositorError {
        OpenCard(PathBuf, IoError),
        ClientCapability(ClientCapability, IoError),
        ResourcesError(IoError),
        NoQualifiedConnectors,
        GbmCreation(IoError),
        GbmFd(InvalidFdError),
        GbmSurfaceCreate(IoError),
        FrontBufferLock,
        BufferSwap(khregl::Error),
        EglSurfaceCreate(khregl::Error),
        AddFrameBuffer(IoError),
        GetPlanes(IoError),
        NoCompatiblePrimaryPlane(crtc::Info),
        UnknownPlaneType(u64),
        PlaneNotFound(PlaneType),
        GetConnectorProperties(connector::Handle, IoError),
        GetConnectorInfo(connector::Handle, IoError),
        GetCrtcProperties(crtc::Handle, IoError),
        GetCrtcInfo(crtc::Handle, IoError),
        GetEncoderInfo(encoder::Handle, IoError),
        GetPlaneProperties(plane::Handle, IoError),
        PropsToHashMap(IoError),
        AtomicCommitFailed(IoError),
        ConfigOpen(IoError),
        ConfigRead(IoError),
        ConfigMissing(String),
        ConfigConvert(String, String),
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for CompositorError {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match self {
                CompositorError::OpenCard(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "OpenCard",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::ClientCapability(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "ClientCapability",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::ResourcesError(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "ResourcesError",
                        &__self_0,
                    )
                }
                CompositorError::NoQualifiedConnectors => {
                    ::core::fmt::Formatter::write_str(f, "NoQualifiedConnectors")
                }
                CompositorError::GbmCreation(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "GbmCreation",
                        &__self_0,
                    )
                }
                CompositorError::GbmFd(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "GbmFd",
                        &__self_0,
                    )
                }
                CompositorError::GbmSurfaceCreate(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "GbmSurfaceCreate",
                        &__self_0,
                    )
                }
                CompositorError::FrontBufferLock => {
                    ::core::fmt::Formatter::write_str(f, "FrontBufferLock")
                }
                CompositorError::BufferSwap(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "BufferSwap",
                        &__self_0,
                    )
                }
                CompositorError::EglSurfaceCreate(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "EglSurfaceCreate",
                        &__self_0,
                    )
                }
                CompositorError::AddFrameBuffer(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "AddFrameBuffer",
                        &__self_0,
                    )
                }
                CompositorError::GetPlanes(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "GetPlanes",
                        &__self_0,
                    )
                }
                CompositorError::NoCompatiblePrimaryPlane(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "NoCompatiblePrimaryPlane",
                        &__self_0,
                    )
                }
                CompositorError::UnknownPlaneType(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "UnknownPlaneType",
                        &__self_0,
                    )
                }
                CompositorError::PlaneNotFound(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "PlaneNotFound",
                        &__self_0,
                    )
                }
                CompositorError::GetConnectorProperties(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "GetConnectorProperties",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::GetConnectorInfo(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "GetConnectorInfo",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::GetCrtcProperties(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "GetCrtcProperties",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::GetCrtcInfo(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "GetCrtcInfo",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::GetEncoderInfo(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "GetEncoderInfo",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::GetPlaneProperties(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "GetPlaneProperties",
                        __self_0,
                        &__self_1,
                    )
                }
                CompositorError::PropsToHashMap(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "PropsToHashMap",
                        &__self_0,
                    )
                }
                CompositorError::AtomicCommitFailed(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "AtomicCommitFailed",
                        &__self_0,
                    )
                }
                CompositorError::ConfigOpen(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "ConfigOpen",
                        &__self_0,
                    )
                }
                CompositorError::ConfigRead(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "ConfigRead",
                        &__self_0,
                    )
                }
                CompositorError::ConfigMissing(__self_0) => {
                    ::core::fmt::Formatter::debug_tuple_field1_finish(
                        f,
                        "ConfigMissing",
                        &__self_0,
                    )
                }
                CompositorError::ConfigConvert(__self_0, __self_1) => {
                    ::core::fmt::Formatter::debug_tuple_field2_finish(
                        f,
                        "ConfigConvert",
                        __self_0,
                        &__self_1,
                    )
                }
            }
        }
    }
    impl Display for CompositorError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let msg = match self {
                Self::OpenCard(path, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Unable to open card at {0:?}: {1:#?}",
                                path,
                                error,
                            ),
                        )
                    })
                }
                Self::ClientCapability(client_capability, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Unable to request {0:#?}: {1:#?}",
                                client_capability,
                                error,
                            ),
                        )
                    })
                }
                Self::ResourcesError(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Could not load normal resource IDs: {0:#?}",
                                error,
                            ),
                        )
                    })
                }
                Self::NoQualifiedConnectors => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(format_args!("No active connectors found."))
                    })
                }
                Self::GbmCreation(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to create GBM buffer object: {0:#?}",
                                error,
                            ),
                        )
                    })
                }
                Self::GbmFd(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Invalid GBM buffer Fd: {0}", error),
                        )
                    })
                }
                Self::GbmSurfaceCreate(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Failed to create GBM surface: {0:#?}", error),
                        )
                    })
                }
                Self::FrontBufferLock => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(format_args!("Failed to lock front buffer"))
                    })
                }
                Self::BufferSwap(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Failed to swap buffers: {0:#?}", error),
                        )
                    })
                }
                Self::EglSurfaceCreate(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Failed to create EGL surface: {0:#?}", error),
                        )
                    })
                }
                Self::AddFrameBuffer(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to add framebuffer to card: {0:#?}",
                                error,
                            ),
                        )
                    })
                }
                Self::GetPlanes(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Failed to get planes: {0:#?}", error),
                        )
                    })
                }
                Self::NoCompatiblePrimaryPlane(info) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to get compatible plane for CRTC. CRTC Info:\n{0:#?}",
                                info,
                            ),
                        )
                    })
                }
                Self::UnknownPlaneType(val) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Unkown plane type \'{0:x}\'", val),
                        )
                    })
                }
                Self::PlaneNotFound(planetype) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Plane type {0:#?} not available.", planetype),
                        )
                    })
                }
                Self::GetConnectorProperties(handle, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to get properties for connector {0:#?}: {1:#?}",
                                handle,
                                error,
                            ),
                        )
                    })
                }
                Self::GetConnectorInfo(handle, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to get info for connector {0:#?}: {1:#?}",
                                handle,
                                error,
                            ),
                        )
                    })
                }
                Self::GetCrtcProperties(handle, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to get properties for CRTC {0:#?}: {1:#?}",
                                handle,
                                error,
                            ),
                        )
                    })
                }
                Self::GetCrtcInfo(handle, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to get info for CRTC {0:#?}: {1:#?}",
                                handle,
                                error,
                            ),
                        )
                    })
                }
                Self::GetEncoderInfo(handle, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to get info for encoder {0:#?}: {1:#?}",
                                handle,
                                error,
                            ),
                        )
                    })
                }
                Self::GetPlaneProperties(handle, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to get properties for plane {0:#?}: {1:#?}",
                                handle,
                                error,
                            ),
                        )
                    })
                }
                Self::PropsToHashMap(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to convert props to hashmap: {0:#?}",
                                error,
                            ),
                        )
                    })
                }
                Self::AtomicCommitFailed(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to commit request to CRTC: {0:#?}",
                                error,
                            ),
                        )
                    })
                }
                Self::ConfigOpen(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Failed to open configuration file: {0}", error),
                        )
                    })
                }
                Self::ConfigRead(error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Failed to read configuration file: {0}", error),
                        )
                    })
                }
                Self::ConfigMissing(k) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(format_args!("Missing {0} in config", k))
                    })
                }
                Self::ConfigConvert(k, error) => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("Failed to convert key {0}: {1}", k, error),
                        )
                    })
                }
            };
            f.write_fmt(format_args!("{0}", msg))
        }
    }
}
mod fourcc {
    use drm::buffer::DrmFourcc;
    pub trait FourCc {
        fn depth(&self) -> u32;
        fn bpp(&self) -> u32;
    }
    impl FourCc for DrmFourcc {
        fn depth(&self) -> u32 {
            match self {
                DrmFourcc::Big_endian => 0,
                DrmFourcc::Rgb332 | DrmFourcc::Bgr233 | DrmFourcc::C8 | DrmFourcc::R8 => {
                    8
                }
                DrmFourcc::Yvu410 | DrmFourcc::Yuv410 => 9,
                DrmFourcc::X0l0
                | DrmFourcc::Y0l0
                | DrmFourcc::Q401
                | DrmFourcc::X0l2
                | DrmFourcc::Y0l2 => 10,
                DrmFourcc::Yvu420
                | DrmFourcc::Yuv420
                | DrmFourcc::Yvu411
                | DrmFourcc::Yuv411
                | DrmFourcc::Nv21
                | DrmFourcc::Nv12
                | DrmFourcc::Yuv420_8bit => 12,
                DrmFourcc::P010 | DrmFourcc::Nv15 | DrmFourcc::Yuv420_10bit => 15,
                DrmFourcc::Rgba5551
                | DrmFourcc::Bgra5551
                | DrmFourcc::Rgbx5551
                | DrmFourcc::Bgrx5551
                | DrmFourcc::Nv61
                | DrmFourcc::Yvu422
                | DrmFourcc::Yuv422
                | DrmFourcc::Rgba4444
                | DrmFourcc::Bgra4444
                | DrmFourcc::Argb4444
                | DrmFourcc::Xrgb4444
                | DrmFourcc::Abgr4444
                | DrmFourcc::Xbgr4444
                | DrmFourcc::Rgbx4444
                | DrmFourcc::Bgrx4444
                | DrmFourcc::Argb1555
                | DrmFourcc::Xrgb1555
                | DrmFourcc::Abgr1555
                | DrmFourcc::Xbgr1555
                | DrmFourcc::Rgb565
                | DrmFourcc::Bgr565
                | DrmFourcc::R16
                | DrmFourcc::Nv16
                | DrmFourcc::Rg88
                | DrmFourcc::Gr88
                | DrmFourcc::Yvyu
                | DrmFourcc::Yuyv
                | DrmFourcc::Vyuy
                | DrmFourcc::Uyvy => 16,
                DrmFourcc::P012 => 18,
                DrmFourcc::P210 | DrmFourcc::Y210 => 20,
                DrmFourcc::Y212
                | DrmFourcc::Nv42
                | DrmFourcc::Nv24
                | DrmFourcc::Yvu444
                | DrmFourcc::Yuv444
                | DrmFourcc::P016
                | DrmFourcc::Rgb888
                | DrmFourcc::Bgr888
                | DrmFourcc::Vuy888
                | DrmFourcc::Rgb565_a8
                | DrmFourcc::Bgr565_a8 => 24,
                DrmFourcc::Vuy101010 | DrmFourcc::Q410 | DrmFourcc::Y410 => 30,
                DrmFourcc::Argb2101010
                | DrmFourcc::Xrgb2101010
                | DrmFourcc::Abgr2101010
                | DrmFourcc::Xbgr2101010
                | DrmFourcc::Xvyu2101010
                | DrmFourcc::Rgba1010102
                | DrmFourcc::Bgra1010102
                | DrmFourcc::Rgbx1010102
                | DrmFourcc::Bgrx1010102
                | DrmFourcc::Y216
                | DrmFourcc::Rg1616
                | DrmFourcc::Gr1616
                | DrmFourcc::Rgba8888
                | DrmFourcc::Bgra8888
                | DrmFourcc::Argb8888
                | DrmFourcc::Xrgb8888
                | DrmFourcc::Abgr8888
                | DrmFourcc::Xbgr8888
                | DrmFourcc::Xyuv8888
                | DrmFourcc::Rgbx8888
                | DrmFourcc::Bgrx8888
                | DrmFourcc::Rgb888_a8
                | DrmFourcc::Bgr888_a8
                | DrmFourcc::Ayuv => 32,
                DrmFourcc::Xrgb8888_a8
                | DrmFourcc::Xbgr8888_a8
                | DrmFourcc::Rgbx8888_a8
                | DrmFourcc::Bgrx8888_a8 => 40,
                DrmFourcc::Y412 | DrmFourcc::Xvyu12_16161616 => 48,
                DrmFourcc::Axbxgxrx106106106106
                | DrmFourcc::Y416
                | DrmFourcc::Xvyu16161616
                | DrmFourcc::Argb16161616f
                | DrmFourcc::Xrgb16161616f
                | DrmFourcc::Abgr16161616f
                | DrmFourcc::Xbgr16161616f => 64,
            }
        }
        fn bpp(&self) -> u32 {
            match self {
                DrmFourcc::Big_endian => 0,
                DrmFourcc::Rgb332 | DrmFourcc::Bgr233 | DrmFourcc::C8 | DrmFourcc::R8 => {
                    8
                }
                DrmFourcc::Yvu410 | DrmFourcc::Yuv410 => 9,
                DrmFourcc::X0l0
                | DrmFourcc::Y0l0
                | DrmFourcc::Q401
                | DrmFourcc::X0l2
                | DrmFourcc::Y0l2 => 10,
                DrmFourcc::Yvu420
                | DrmFourcc::Yuv420
                | DrmFourcc::Yvu411
                | DrmFourcc::Yuv411
                | DrmFourcc::Nv21
                | DrmFourcc::Nv12
                | DrmFourcc::Yuv420_8bit => 12,
                DrmFourcc::P010 | DrmFourcc::Nv15 | DrmFourcc::Yuv420_10bit => 15,
                DrmFourcc::Rgba5551
                | DrmFourcc::Bgra5551
                | DrmFourcc::Rgbx5551
                | DrmFourcc::Bgrx5551
                | DrmFourcc::Nv61
                | DrmFourcc::Yvu422
                | DrmFourcc::Yuv422
                | DrmFourcc::Rgba4444
                | DrmFourcc::Bgra4444
                | DrmFourcc::Argb4444
                | DrmFourcc::Xrgb4444
                | DrmFourcc::Abgr4444
                | DrmFourcc::Xbgr4444
                | DrmFourcc::Rgbx4444
                | DrmFourcc::Bgrx4444
                | DrmFourcc::Argb1555
                | DrmFourcc::Xrgb1555
                | DrmFourcc::Abgr1555
                | DrmFourcc::Xbgr1555
                | DrmFourcc::Rgb565
                | DrmFourcc::Bgr565
                | DrmFourcc::R16
                | DrmFourcc::Nv16
                | DrmFourcc::Rg88
                | DrmFourcc::Gr88
                | DrmFourcc::Yvyu
                | DrmFourcc::Yuyv
                | DrmFourcc::Vyuy
                | DrmFourcc::Uyvy => 16,
                DrmFourcc::P012 => 18,
                DrmFourcc::P210 | DrmFourcc::Y210 => 20,
                DrmFourcc::Y212
                | DrmFourcc::Nv42
                | DrmFourcc::Nv24
                | DrmFourcc::Yvu444
                | DrmFourcc::Yuv444
                | DrmFourcc::P016
                | DrmFourcc::Rgb888
                | DrmFourcc::Bgr888
                | DrmFourcc::Vuy888
                | DrmFourcc::Rgb565_a8
                | DrmFourcc::Bgr565_a8 => 24,
                DrmFourcc::Vuy101010 | DrmFourcc::Q410 => 30,
                DrmFourcc::Argb2101010
                | DrmFourcc::Xrgb2101010
                | DrmFourcc::Abgr2101010
                | DrmFourcc::Xbgr2101010
                | DrmFourcc::Xvyu2101010
                | DrmFourcc::Y410
                | DrmFourcc::Rgba1010102
                | DrmFourcc::Bgra1010102
                | DrmFourcc::Rgbx1010102
                | DrmFourcc::Bgrx1010102
                | DrmFourcc::Y216
                | DrmFourcc::Rg1616
                | DrmFourcc::Gr1616
                | DrmFourcc::Rgba8888
                | DrmFourcc::Bgra8888
                | DrmFourcc::Argb8888
                | DrmFourcc::Xrgb8888
                | DrmFourcc::Abgr8888
                | DrmFourcc::Xbgr8888
                | DrmFourcc::Xyuv8888
                | DrmFourcc::Rgbx8888
                | DrmFourcc::Bgrx8888
                | DrmFourcc::Rgb888_a8
                | DrmFourcc::Bgr888_a8
                | DrmFourcc::Ayuv => 32,
                DrmFourcc::Xrgb8888_a8
                | DrmFourcc::Xbgr8888_a8
                | DrmFourcc::Rgbx8888_a8
                | DrmFourcc::Bgrx8888_a8 => 40,
                DrmFourcc::Y412 | DrmFourcc::Xvyu12_16161616 => 48,
                DrmFourcc::Axbxgxrx106106106106
                | DrmFourcc::Y416
                | DrmFourcc::Xvyu16161616
                | DrmFourcc::Argb16161616f
                | DrmFourcc::Xrgb16161616f
                | DrmFourcc::Abgr16161616f
                | DrmFourcc::Xbgr16161616f => 64,
            }
        }
    }
}
mod quadtree {
    #![allow(dead_code)]
    use std::cell::UnsafeCell;
    use std::collections::HashSet;
    use std::{
        collections::HashMap, convert::Infallible, hash::Hash, marker::PhantomData,
    };
    use crate::util::Point2;
    use crate::util::Bounds2;
    type Equality<T> = dyn Fn(&T, &T) -> bool + 'static;
    enum Node<T> {
        Leaf(usize),
        Branch(Box<[Node<T>; 4]>),
        __(Infallible, PhantomData<T>),
    }
    pub struct Quadtree<'a, T> {
        register_left: HashMap<usize, UnsafeCell<T>>,
        register_right: HashMap<&'a T, usize>,
        next_id: usize,
        bounds: Bounds2,
        root: Node<T>,
        _ref: PhantomData<&'a ()>,
    }
    /// `T` should be cheap to clone.
    impl<T: Eq + Hash> Node<T> {
        pub fn insert_with_ignore<'a, F: Fn(&'a T) -> bool + 'a>(
            &mut self,
            register: &'a HashMap<usize, UnsafeCell<T>>,
            id: usize,
            node_bounds: Bounds2,
            area_rect: Bounds2,
            ignore: Option<&F>,
        ) {
            let p = node_bounds.points();
            let q = node_bounds.quarters();
            match self {
                Node::Leaf(nid) => {
                    if *nid == id {
                        return;
                    }
                    if let Some(ignore) = ignore {
                        if ignore(unsafe {
                            (register.get(&nid).unwrap().get()).as_ref().unwrap()
                        }) {
                            return;
                        }
                    }
                    if p.iter().all(|p| area_rect.contains(*p)) {
                        *self = Node::Leaf(id);
                        return;
                    }
                    let mut children = std::array::from_fn::<
                        _,
                        4,
                        _,
                    >(|_| Node::Leaf(nid.clone()));
                    for (node, quadrant) in children.iter_mut().zip(q) {
                        if let Some(_bounds) = area_rect.intersection(&quadrant) {
                            node.insert_with_ignore(
                                register,
                                id,
                                quadrant,
                                area_rect,
                                ignore,
                            );
                        }
                    }
                    *self = Self::Branch(Box::new(children));
                }
                Node::Branch(nodes) => {
                    for (quadrant, node) in q.into_iter().zip(nodes.iter_mut()) {
                        node.insert_with_ignore(
                            register,
                            id,
                            quadrant,
                            area_rect,
                            ignore,
                        );
                    }
                    if nodes
                        .iter()
                        .all(|node| {
                            if let Node::Leaf(nid) = node { *nid == id } else { false }
                        })
                    {
                        *self = Node::Leaf(id);
                    }
                }
                Node::__(..) => {
                    ::core::panicking::panic("internal error: entered unreachable code")
                }
            }
        }
        pub fn select<'a>(
            &self,
            register: &'a HashMap<usize, UnsafeCell<T>>,
            node_bounds: Bounds2,
            rect: Bounds2,
            hopper: &mut HashMap<usize, Vec<Bounds2>>,
        ) {
            if let Some(new_bounds) = node_bounds.intersection(&rect) {
                match self {
                    Node::Leaf(nid) => {
                        hopper.entry(*nid).or_default().push(new_bounds);
                    }
                    Node::Branch(children) => {
                        let q = node_bounds.quarters();
                        for (child, quadrant) in children.iter().zip(q) {
                            child.select(register, quadrant, rect, hopper);
                        }
                    }
                    _ => {
                        ::core::panicking::panic(
                            "internal error: entered unreachable code",
                        )
                    }
                }
            }
        }
        pub fn select_items<'a>(
            &self,
            register: &'a HashMap<usize, UnsafeCell<T>>,
            node_bounds: Bounds2,
            rect: Bounds2,
            hopper: &mut HashSet<usize>,
        ) {
            if let Some(_) = node_bounds.intersection(&rect) {
                match self {
                    Node::Leaf(nid) => {
                        hopper.insert(*nid);
                    }
                    Node::Branch(children) => {
                        let q = node_bounds.quarters();
                        for (child, quadrant) in children.iter().zip(q) {
                            child.select_items(register, quadrant, rect, hopper);
                        }
                    }
                    _ => {
                        ::core::panicking::panic(
                            "internal error: entered unreachable code",
                        )
                    }
                }
            }
        }
        pub fn select_mut<'a>(
            &mut self,
            register: &'a HashMap<usize, UnsafeCell<T>>,
            node_bounds: Bounds2,
            rect: Bounds2,
            hopper: &mut HashMap<usize, Vec<Bounds2>>,
        ) {
            if let Some(new_bounds) = node_bounds.intersection(&rect) {
                match self {
                    Node::Leaf(nid) => {
                        hopper.entry(*nid).or_default().push(new_bounds);
                    }
                    Node::Branch(children) => {
                        let q = node_bounds.quarters();
                        for (child, quadrant) in children.iter_mut().zip(q) {
                            child.select_mut(register, quadrant, rect, hopper);
                        }
                    }
                    _ => {
                        ::core::panicking::panic(
                            "internal error: entered unreachable code",
                        )
                    }
                }
            }
        }
    }
    impl<'a, T: Eq + Hash> Quadtree<'a, T> {
        pub fn new(bounds: Bounds2) -> Self {
            let register_left = HashMap::new();
            let register_right = HashMap::new();
            Self {
                register_left,
                register_right,
                next_id: 1,
                bounds,
                root: Node::Leaf(0),
                _ref: PhantomData,
            }
        }
        pub fn is_empty(&self) -> bool {
            self.register_left.is_empty()
        }
        fn register(&mut self, val: T) -> usize {
            if let Some(id) = self.register_right.get(&val) {
                return *id;
            }
            self.register_left.insert(self.next_id, UnsafeCell::new(val));
            let k = unsafe {
                self.register_left.get(&self.next_id).unwrap().get().as_ref().unwrap()
            };
            self.register_right.insert(k, self.next_id);
            let out = self.next_id;
            self.next_id += 1;
            out
        }
        pub fn insert<'b>(&'b mut self, area_rect: Bounds2, val: T) {
            let id = { self.register(val) };
            self.root
                .insert_with_ignore::<
                    Box<dyn Fn(&T) -> bool>,
                >(&self.register_left, id, self.bounds, area_rect, None)
        }
        pub fn insert_with_ignore(
            &mut self,
            area_rect: Bounds2,
            val: T,
            ignore: Option<&(impl Fn(&T) -> bool + 'a)>,
        ) {
            let ignore = ignore.as_ref();
            let id = self.register(val);
            self.root
                .insert_with_ignore(
                    &self.register_left,
                    id,
                    self.bounds,
                    area_rect,
                    ignore,
                )
        }
        /// Returns references to all values in the bounds
        pub fn select(&self, rect: Bounds2) -> Vec<(&T, Vec<Bounds2>)> {
            let mut regions = HashMap::new();
            self.root.select(&self.register_left, self.bounds, rect, &mut regions);
            let mut output = Vec::new();
            for (k, b) in regions {
                if let Some(t) = self.register_left.get(&k) {
                    output.push((unsafe { t.get().as_ref().unwrap() }, b));
                }
            }
            output
        }
        pub fn select_items(&self, rect: Bounds2) -> Vec<&T> {
            let mut ids = HashSet::new();
            self.root.select_items(&self.register_left, self.bounds, rect, &mut ids);
            let mut output = Vec::new();
            for k in ids {
                if let Some(t) = self.register_left.get(&k) {
                    output.push(unsafe { t.get().as_ref().unwrap() });
                }
            }
            output
        }
        pub fn select_mut(
            &'a mut self,
            rect: Bounds2,
        ) -> Vec<(&'a mut T, Vec<Bounds2>)> {
            let mut regions = HashMap::new();
            self.root
                .select_mut(&mut self.register_left, self.bounds, rect, &mut regions);
            let mut output = Vec::new();
            let reg_ptr = (&mut self.register_left)
                as *mut HashMap<usize, UnsafeCell<T>>;
            for (k, r) in regions.into_iter() {
                let v = unsafe { reg_ptr.as_mut().unwrap().get_mut(&k) };
                output.push((v.unwrap().get_mut(), r));
            }
            output
        }
        pub fn select_all(&self) -> Vec<(&T, Vec<Bounds2>)> {
            self.select(self.bounds)
        }
        pub fn select_all_items(&self) -> Vec<(usize, &T)> {
            self.register_left
                .iter()
                .map(|(k, v)| (*k, unsafe { v.get().as_ref().unwrap() }))
                .collect()
        }
        pub fn select_all_items_mut(&mut self) -> Vec<(usize, &mut T)> {
            self.register_left.iter_mut().map(|(k, v)| (*k, v.get_mut())).collect()
        }
        /// Attempts to merge bounds that have the same width and height
        pub fn select_merged(&self) -> Vec<(&T, Vec<Bounds2>)> {
            let mut work = <[_]>::into_vec(
                ::alloc::boxed::box_new([(self.bounds, &self.root)]),
            );
            let mut result = HashMap::<&T, Vec<Bounds2>>::new();
            while !work.is_empty() {
                let (bounds, node) = work.pop().unwrap();
                match node {
                    Node::Leaf(leaf_id) => {
                        let Some(leaf) = self.register_left.get(leaf_id) else {
                            {
                                ::core::panicking::panic_fmt(
                                    format_args!(
                                        "internal error: entered unreachable code: {0}",
                                        format_args!("This should never happen"),
                                    ),
                                );
                            };
                        };
                        result
                            .entry(unsafe { leaf.get().as_ref().unwrap() })
                            .and_modify(|bounds_array| bounds_array.push(bounds))
                            .or_insert(
                                <[_]>::into_vec(::alloc::boxed::box_new([bounds])),
                            );
                    }
                    Node::Branch(children) => {
                        work.extend(
                            bounds.quarters().iter().copied().zip(children.iter()).rev(),
                        )
                    }
                    _ => {}
                }
            }
            for (_, child) in &mut result {
                if child.len() <= 1 {
                    continue;
                }
                if child.len() == 4 {
                    let mut min_x = 0;
                    let mut max_x = 0;
                    let mut min_y = 0;
                    let mut max_y = 0;
                    for bound in &*child {
                        min_x = bound.min.x.min(min_x);
                        max_x = bound.max.x.max(max_x);
                        min_y = bound.min.y.min(min_y);
                        max_y = bound.max.y.max(max_y);
                    }
                    let new_child = Bounds2::new(
                        (min_x, min_y).into(),
                        (max_x, max_y).into(),
                    );
                    child.clear();
                    child.push(new_child);
                }
            }
            result.into_iter().collect()
        }
        pub fn into_objects(self) -> Vec<T> {
            self.register_left.into_values().map(|v| v.into_inner()).collect()
        }
        pub fn remove(&mut self, id: usize) -> Option<T> {
            if let Some(v) = self.register_left.get(&id) {
                self.root
                    .insert_with_ignore(
                        &self.register_left,
                        0,
                        self.bounds,
                        self.bounds,
                        Some(&|o| unsafe { o != v.get().as_ref().unwrap() }),
                    );
                self.register_right.remove(unsafe { v.get().as_ref().unwrap() });
                Some(self.register_left.remove(&id).unwrap().into_inner())
            } else {
                None
            }
        }
    }
    trait Extra {
        fn points(&self) -> [Point2; 4];
        fn quarters(&self) -> [Bounds2; 4];
    }
    impl Extra for Bounds2 {
        fn points(&self) -> [Point2; 4] {
            [
                self.min,
                (self.min.x, self.max.y).into(),
                self.max,
                (self.max.x, self.min.y).into(),
            ]
        }
        fn quarters(&self) -> [Bounds2; 4] {
            let halfx = (self.min.x + self.max.x) / 2;
            let halfy = (self.min.y + self.max.y) / 2;
            [
                Bounds2 {
                    min: (self.min.x, self.min.y).into(),
                    max: (halfx, halfy).into(),
                },
                Bounds2 {
                    min: (halfx, self.min.y).into(),
                    max: (self.max.x, halfy).into(),
                },
                Bounds2 {
                    min: (self.min.x, halfy).into(),
                    max: (halfx, self.max.y).into(),
                },
                Bounds2 {
                    min: (halfx, halfy).into(),
                    max: (self.max.x, self.max.y).into(),
                },
            ]
        }
    }
}
mod util {
    use crate::config::CompositorConfig;
    use crate::VIRTUAL_SCREEN_EXTENTS;
    use crate::config::Config;
    use crate::display::Display;
    use crate::quadtree::Quadtree;
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
                    String::from(
                        "Cannot parse display position: Missing space separator",
                    ),
                );
            }
            let [x, y, ..] = &split[..] else {
                ::core::panicking::panic("internal error: entered unreachable code")
            };
            Ok(Self {
                x: x
                    .parse()
                    .map_err(|e| ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to parse {0} into u32 for display position: {1}",
                                x,
                                e,
                            ),
                        )
                    }))?,
                y: y
                    .parse()
                    .map_err(|e| ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!(
                                "Failed to parse {0} into u32 for display position: {1}",
                                y,
                                e,
                            ),
                        )
                    }))?,
            })
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
    pub enum Direction {
        East,
        South,
        North,
        West,
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for Direction {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {}
    }
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for Direction {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for Direction {
        #[inline]
        fn eq(&self, other: &Direction) -> bool {
            let __self_discr = ::core::intrinsics::discriminant_value(self);
            let __arg1_discr = ::core::intrinsics::discriminant_value(other);
            __self_discr == __arg1_discr
        }
    }
    pub fn resolve_collisions(
        output: &Quadtree<'_, Display>,
        bounds: &mut euclid::Box2D<i32, ()>,
    ) {
        let mut moved: Option<Direction> = None;
        loop {
            let select = output.select_items(*bounds);
            if select.is_empty() {
                break;
            }
            for intheway in select.into_iter().map(|d| d.bounds()) {
                let dleft = bounds.max.x - intheway.min.x;
                let dright = intheway.max.x - bounds.min.x;
                let dup = bounds.max.y - intheway.min.y;
                let ddown = intheway.max.y - bounds.min.y;
                let min = dleft.min(dright).min(dup).min(ddown);
                match [min == dleft, min == dright, min == dup, min == ddown] {
                    [true, ..] if moved != Some(Direction::East) => {
                        moved = Some(Direction::West);
                        *bounds = bounds.translate((-dleft, 0).into());
                        continue;
                    }
                    [_, true, ..] if moved != Some(Direction::West) => {
                        if (i32::MAX - bounds.size().width) < bounds.max.x {
                            moved = Some(Direction::West);
                            *bounds = bounds.translate((-dleft, 0).into());
                            continue;
                        }
                        moved = Some(Direction::East);
                        *bounds = bounds.translate((dright, 0).into());
                        continue;
                    }
                    [_, _, true, ..] if moved != Some(Direction::South) => {
                        moved = Some(Direction::North);
                        *bounds = bounds.translate((0, dup).into());
                        continue;
                    }
                    [_, _, _, true] if moved != Some(Direction::North) => {
                        if (i32::MAX - bounds.size().height) < bounds.max.y {
                            moved = Some(Direction::North);
                            *bounds = bounds.translate((0, dup).into());
                            continue;
                        }
                        moved = Some(Direction::South);
                        *bounds = bounds.translate((0, -ddown).into());
                        continue;
                    }
                    [..] => {}
                }
            }
        }
    }
    pub fn arrange_displays<'a, 'b>(
        displays: Vec<Display>,
        config: &'a Config,
    ) -> Quadtree<'b, Display> {
        let mut arranging = Quadtree::new(
            Bounds2::new((0, 0).into(), VIRTUAL_SCREEN_EXTENTS.into()),
        );
        if displays.is_empty() {
            return arranging;
        }
        let mut leftover = Vec::new();
        for mut display in displays.into_iter() {
            if let Some(new_pos) = config
                .get::<DisplayPosition>(&CompositorConfig::offset_key(&display.name()))
            {
                display.pos = new_pos.into();
                let mut bounds = display.bounds();
                resolve_collisions(&arranging, &mut bounds);
                display.pos = bounds.min;
                arranging.insert(bounds, display);
            } else {
                display.pos = (0, 0).into();
                leftover.push(display);
            }
        }
        for mut display in leftover.into_iter() {
            let mut bounds = display.bounds();
            resolve_collisions(&arranging, &mut bounds);
            display.pos = bounds.min;
            arranging.insert(bounds, display);
        }
        let mut min = Vec2::zero();
        for (_, display) in arranging.select_all_items() {
            min.x = min.x.min(display.pos.x);
            min.y = min.y.min(display.pos.y);
        }
        let output = if min.x != 0 || min.y != 0 {
            let displays = arranging.into_objects();
            let mut displacing = Quadtree::new(
                Bounds2::new((0, 0).into(), VIRTUAL_SCREEN_EXTENTS.into()),
            );
            for mut display in displays {
                display.pos -= min;
                displacing.insert(display.bounds(), display);
            }
            displacing
        } else {
            arranging
        };
        output
    }
    pub fn create_context(
        egl: &khregl::DynamicInstance,
        display: khregl::Display,
    ) -> (khregl::Context, khregl::Config) {
        let attributes = [
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
        let config = egl
            .choose_first_config(display, &attributes)
            .expect("unable to choose an EGL configuration")
            .expect("no EGL configuration found");
        let context_attributes = [khregl::CONTEXT_CLIENT_VERSION, 3, khregl::NONE];
        let context = egl
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
                        egl.get_proc_address("glEGLImageTargetTexture2DOES").unwrap()
                            as *const c_void,
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
}
use config::Config;
use gbm::BufferObject;
use nix::fcntl::fcntl;
use nix::fcntl::FcntlArg;
use nix::fcntl::OFlag;
use crate::card::Card;
use crate::config::CompositorConfig;
use crate::display::Display;
use crate::display::init_displays;
use crate::fourcc::FourCc;
use crate::quadtree::Quadtree;
use crate::util::BackgroundImage;
use crate::util::Bounds2;
use crate::util::GlFns;
use crate::util::arrange_displays;
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
pub const VIRTUAL_SCREEN_EXTENTS: (i32, i32) = (0x1000, 0x1000);
pub const DRM_FORMAT: DrmFourcc = DrmFourcc::Xrgb8888;
fn load_default_bg() -> DynamicImage {
    let bg = image::load_from_memory_with_format(&[], image::ImageFormat::Png).unwrap();
    bg
}
fn main() {
    let config_path = CompositorConfig::config_path().unwrap_or("/dev/null".into());
    let (mut tx, mut rx) = crossbeam::channel::bounded(2);
    tx.send(Ok(NotifyEvent::default())).unwrap();
    let mut config_watcher = notify::recommended_watcher(tx).ok();
    config_watcher
        .map(|mut watcher| {
            watcher.watch(&config_path, RecursiveMode::NonRecursive).ok()
        });
    let mut config = Config::new(&config_path).unwrap_or_default();
    let cards = Card::open_all()
        .into_iter()
        .map(|card| {
            let flags = fcntl(&card, FcntlArg::F_GETFL)
                .expect("Failed to get card FD flags");
            let new_flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
            fcntl(&card, FcntlArg::F_SETFL(new_flags)).expect("Failed to set new flags");
            card
        })
        .collect::<Vec<_>>();
    let cards = cards
        .into_iter()
        .filter_map(|card| {
            if let Some(res) = card.resource_handles().ok() {
                for &conn in res.connectors() {
                    let info = card.get_connector(conn, false).ok()?;
                    if info.state() == drm::control::connector::State::Connected {
                        return Some(card);
                    }
                }
                None
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let cards = Box::leak(Box::new(cards));
    let mut contexts = Vec::new();
    let bg = load_default_bg();
    let aligned_width = (bg.width() + 15) & !15;
    let aligned_height = (bg.height() + 15) & !15;
    let mut padded = DynamicImage::new_rgba8(aligned_width, aligned_height);
    padded.copy_from(&bg, 0, 0).unwrap();
    for card in cards.iter() {
        let gbm = gbm::Device::new(card).expect("Failed to init GBM with device");
        let egl = unsafe {
            Arc::new(
                khregl::DynamicInstance::<khregl::EGL1_5>::load_required()
                    .expect("unable to load libEGL.so.1"),
            )
        };
        let egldisplay = unsafe {
            egl.get_platform_display(
                EGL_PLATFORM_GBM_KHR,
                gbm.as_raw() as *mut c_void,
                &[ATTRIB_NONE],
            )
        }
            .expect("Failed to get platform display");
        egl.initialize(egldisplay).expect("Failed to initialize display");
        let (eglctx, eglconfig) = create_context(egl.as_ref(), egldisplay);
        gl::load_with(|name| {
            egl
                .get_proc_address(name)
                .map(|ptr| ptr as *const _)
                .unwrap_or(std::ptr::null())
        });
        let gl_fns = GlFns::load(&egl);
        let mut bgbo = gbm
            .create_buffer_object::<
                (),
            >(aligned_width, aligned_height, DRM_FORMAT, BufferObjectFlags::RENDERING)
            .unwrap();
        bgbo.map_mut(
                0,
                0,
                aligned_width,
                aligned_height,
                |map| {
                    map.buffer_mut().copy_from_slice(padded.as_rgba8().unwrap());
                },
            )
            .unwrap();
        let egl_image = unsafe {
            egl.create_image(
                    egldisplay,
                    Context::from_ptr(EGL_NO_CONTEXT),
                    EGL_NATIVE_PIXMAP_KHR,
                    ClientBuffer::from_ptr(bgbo.as_raw() as *mut c_void),
                    &[ATTRIB_NONE],
                )
                .expect("Failed to create EGL image")
        };
        let bg = unsafe {
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
    let mut display_tree: Quadtree<Display> = Quadtree::new(
        Bounds2::new((0, 0).into(), VIRTUAL_SCREEN_EXTENTS.into()),
    );
    loop {
        match rx.try_recv() {
            Ok(Ok(NotifyEvent { .. })) => {
                if let Ok(new_config) = Config::new(&config_path) {
                    config = new_config;
                    display_tree = arrange_displays(
                        display_tree.into_objects(),
                        &config,
                    );
                }
            }
            Ok(Err(_)) => {}
            Err(crossbeam::channel::TryRecvError::Empty) => {}
            Err(_) => {
                if frame % 120 == 0 {
                    {
                        use ::tracing::__macro_support::Callsite as _;
                        static __CALLSITE: ::tracing::callsite::DefaultCallsite = {
                            static META: ::tracing::Metadata<'static> = {
                                ::tracing_core::metadata::Metadata::new(
                                    "event compositor/src/main.rs:221",
                                    "compositor",
                                    ::tracing::Level::WARN,
                                    ::tracing_core::__macro_support::Option::Some(
                                        "compositor/src/main.rs",
                                    ),
                                    ::tracing_core::__macro_support::Option::Some(221u32),
                                    ::tracing_core::__macro_support::Option::Some("compositor"),
                                    ::tracing_core::field::FieldSet::new(
                                        &["message"],
                                        ::tracing_core::callsite::Identifier(&__CALLSITE),
                                    ),
                                    ::tracing::metadata::Kind::EVENT,
                                )
                            };
                            ::tracing::callsite::DefaultCallsite::new(&META)
                        };
                        let enabled = ::tracing::Level::WARN
                            <= ::tracing::level_filters::STATIC_MAX_LEVEL
                            && ::tracing::Level::WARN
                                <= ::tracing::level_filters::LevelFilter::current()
                            && {
                                let interest = __CALLSITE.interest();
                                !interest.is_never()
                                    && ::tracing::__macro_support::__is_enabled(
                                        __CALLSITE.metadata(),
                                        interest,
                                    )
                            };
                        if enabled {
                            (|value_set: ::tracing::field::ValueSet| {
                                let meta = __CALLSITE.metadata();
                                ::tracing::Event::dispatch(meta, &value_set);
                            })({
                                #[allow(unused_imports)]
                                use ::tracing::field::{debug, display, Value};
                                let mut iter = __CALLSITE.metadata().fields().iter();
                                __CALLSITE
                                    .metadata()
                                    .fields()
                                    .value_set(
                                        &[
                                            (
                                                &::tracing::__macro_support::Iterator::next(&mut iter)
                                                    .expect("FieldSet corrupted (this is a bug)"),
                                                ::tracing::__macro_support::Option::Some(
                                                    &format_args!(
                                                        "Config watcher channel disconnected somehow. Reconnecting.",
                                                    ) as &dyn Value,
                                                ),
                                            ),
                                        ],
                                    )
                            });
                        } else {
                        }
                    };
                    (tx, rx) = crossbeam::channel::bounded(2);
                    config_watcher = notify::recommended_watcher(tx).ok();
                    config_watcher
                        .map(|mut watcher| {
                            watcher.watch(&config_path, RecursiveMode::NonRecursive).ok()
                        });
                }
            }
        }
        for (card, gbm, egl, egldisplay, eglctx, eglconfig, bg) in contexts.iter_mut() {
            let ignore_list = HashSet::<
                String,
            >::from_iter(
                display_tree
                    .select_all_items()
                    .iter()
                    .map(|(_, display)| display.name().to_owned()),
            );
            let mut new_displays = init_displays(
                    ignore_list,
                    card,
                    gbm,
                    egl,
                    egldisplay,
                    eglconfig,
                )
                .expect("Failed to init displays")
                .into_iter()
                .collect::<Vec<_>>();
            for (display, initial_primary_bo, initial_cursor_bo) in new_displays
                .iter_mut()
            {
                {
                    ::std::io::_print(
                        format_args!("Found display {0}\n", display.name()),
                    );
                };
                egl.make_current(
                        *egldisplay,
                        Some(display.primary.eglsurface),
                        Some(display.primary.eglsurface),
                        Some(*eglctx),
                    )
                    .expect("Failed to make surface current");
                let mut atomic_req = atomic::AtomicModeReq::new();
                let initial_primary_fb = card
                    .add_framebuffer(
                        initial_primary_bo,
                        DRM_FORMAT.depth(),
                        DRM_FORMAT.bpp(),
                    )
                    .expect("Failed to get initial framebuffer");
                display
                    .init_req(card, initial_primary_fb, &mut atomic_req)
                    .expect("Failed to init display");
                card.atomic_commit(
                        AtomicCommitFlags::ALLOW_MODESET | AtomicCommitFlags::NONBLOCK
                            | AtomicCommitFlags::PAGE_FLIP_EVENT,
                        atomic_req,
                    )
                    .expect("Failed to set mode");
                card.destroy_framebuffer(initial_primary_fb)
                    .expect("Failed to destroy initial framebuffer");
            }
            if !new_displays.is_empty() {
                let displays = display_tree
                    .into_objects()
                    .into_iter()
                    .chain(new_displays.into_iter().map(|(display, _, _)| display))
                    .collect();
                display_tree = arrange_displays(displays, &config);
            }
            let events = match card.receive_events() {
                Ok(events) => events.peekable(),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => {
                    ::core::panicking::panic_fmt(format_args!("{0}", e));
                }
            };
            events
                .for_each(|event| {
                    match event {
                        drm::control::Event::PageFlip(event) => {
                            let mut to_remove = Vec::new();
                            for (id, display) in display_tree.select_all_items_mut() {
                                if display.crtc != event.crtc {
                                    continue;
                                }
                                egl.make_current(
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
                                }
                                match unsafe {
                                    display.primary.swap(card, egl, egldisplay)
                                } {
                                    Ok(()) => {}
                                    Err(_) => {
                                        if let Ok(info) = card
                                            .get_connector(display.connector, false)
                                        {
                                            if info.state() != connector::State::Connected {
                                                to_remove.push(id);
                                            }
                                        }
                                    }
                                }
                            }
                            for id in to_remove {
                                if let Some(display) = display_tree.remove(id) {
                                    for fb in display
                                        .primary
                                        .fbs
                                        .into_inner()
                                        .values()
                                        .chain(display.cursor.fbs.into_inner().values())
                                    {
                                        card.destroy_framebuffer(*fb).ok();
                                    }
                                    for overlay in display.overlays.into_iter() {
                                        for fb in overlay.fbs.into_inner().values() {
                                            card.destroy_framebuffer(*fb).ok();
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                });
        }
        if Instant::now().checked_duration_since(end).is_some() {
            break;
        }
        frame += 1;
    }
}
