#![feature(prelude_import)]
#[macro_use]
extern crate std;
#[prelude_import]
use std::prelude::rust_2024::*;
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
    use gbm::BufferObjectFlags;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use crate::error::CompositorError;
    use crate::error::CompositorResult;
    use crate::card::Card;
    use crate::fourcc::FourCc;
    use crate::DRM_FORMAT;
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
    pub struct TripleBuffer {
        pub draw: usize,
        pub scan: usize,
        pub bos: [gbm::BufferObject<()>; 3],
        pub fbs: [framebuffer::Handle; 3],
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for TripleBuffer {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "TripleBuffer",
                "draw",
                &self.draw,
                "scan",
                &self.scan,
                "bos",
                &self.bos,
                "fbs",
                &&self.fbs,
            )
        }
    }
    impl TripleBuffer {
        pub fn new(
            card: &Card,
            gbm: &gbm::Device<&Card>,
            planetype: PlaneType,
            size: (u32, u32),
        ) -> CompositorResult<Self> {
            let [a, b, c] = std::array::from_fn(|_| make_buffer(
                card,
                gbm,
                planetype,
                size,
            ));
            let buffers = [a?, b?, c?];
            let [a, b, c] = std::array::from_fn(|i| {
                card
                    .add_framebuffer(&buffers[i], DRM_FORMAT.depth(), DRM_FORMAT.bpp())
                    .map_err(|e| CompositorError::AddFrameBuffer(e))
            });
            let framebuffers = [a?, b?, c?];
            Ok(Self {
                scan: 0,
                draw: 0,
                bos: buffers,
                fbs: framebuffers,
            })
        }
        pub fn swap(&mut self) {
            self.scan = self.draw;
            self.draw = (self.draw + 1) % 3;
        }
    }
    pub struct DrmCtx {
        pub plane: plane::Handle,
        pub plane_props: HashMap<String, property::Info>,
        pub size: (u32, u32),
        pub buffers: TripleBuffer,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for DrmCtx {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field4_finish(
                f,
                "DrmCtx",
                "plane",
                &self.plane,
                "plane_props",
                &self.plane_props,
                "size",
                &self.size,
                "buffers",
                &&self.buffers,
            )
        }
    }
    impl DrmCtx {
        pub fn new(
            card: &Card,
            gbm: &gbm::Device<&'static Card>,
            plane: plane::Handle,
            planetype: PlaneType,
            size: (u32, u32),
        ) -> CompositorResult<Self> {
            let plane_props = card
                .get_properties(plane)
                .map_err(|err| CompositorError::GetPlaneProperties(plane, err))?
                .as_hashmap(card)
                .map_err(|err| CompositorError::PropsToHashMap(err))?;
            let buffers = TripleBuffer::new(card, gbm, planetype, size)?;
            Ok(Self {
                plane,
                plane_props,
                size,
                buffers,
            })
        }
        pub fn from_connector(
            card: &Card,
            gbm: &gbm::Device<&'static Card>,
            resources: &ResourceHandles,
            crtc: crtc::Handle,
            planes: &mut HashSet<plane::Handle>,
            planetype: PlaneType,
            size: (u32, u32),
        ) -> CompositorResult<Self> {
            let plane = find_compatible_plane(card, resources, crtc, planes, planetype);
            if let Some(plane) = plane {
                Self::new(card, gbm, plane, planetype, size)
            } else {
                Err(
                    CompositorError::NoCompatiblePrimaryPlane(
                        card
                            .get_crtc(crtc)
                            .map_err(|e| CompositorError::GetCrtcInfo(crtc, e))?,
                    ),
                )
            }
        }
        fn get_draw_fb(&self) -> framebuffer::Handle {
            self.buffers.fbs[self.buffers.draw]
        }
        pub fn init_req(
            &self,
            atomic_req: &mut atomic::AtomicModeReq,
            crtc: crtc::Handle,
        ) -> CompositorResult<()> {
            let plane = self.plane;
            let props = &self.plane_props;
            atomic_req
                .add_property(
                    plane,
                    props["FB_ID"].handle(),
                    property::Value::Framebuffer(Some(self.buffers.fbs[0])),
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
            Ok(())
        }
        pub unsafe fn swap(
            &mut self,
            card: &Card,
            crtc: crtc::Handle,
        ) -> CompositorResult<()> {
            let plane = self.plane;
            let mut atomic_req = atomic::AtomicModeReq::new();
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["FB_ID"].handle(),
                    property::Value::Framebuffer(Some(self.get_draw_fb())),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["CRTC_ID"].handle(),
                    property::Value::CRTC(Some(crtc)),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["SRC_X"].handle(),
                    property::Value::UnsignedRange(0),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["SRC_Y"].handle(),
                    property::Value::UnsignedRange(0),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["SRC_W"].handle(),
                    property::Value::UnsignedRange((self.size.0 << 16) as u64),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["SRC_H"].handle(),
                    property::Value::UnsignedRange((self.size.1 << 16) as u64),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["CRTC_X"].handle(),
                    property::Value::SignedRange(0),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["CRTC_Y"].handle(),
                    property::Value::SignedRange(0),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["CRTC_W"].handle(),
                    property::Value::UnsignedRange(self.size.0 as u64),
                );
            atomic_req
                .add_property(
                    plane,
                    self.plane_props["CRTC_H"].handle(),
                    property::Value::UnsignedRange(self.size.1 as u64),
                );
            card.atomic_commit(
                    AtomicCommitFlags::NONBLOCK | AtomicCommitFlags::PAGE_FLIP_EVENT,
                    atomic_req,
                )
                .map_err(|err| CompositorError::AtomicCommitFailed(err))?;
            self.buffers.swap();
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
                                            "event dreampipe/src/config.rs:54",
                                            "dreampipe::config",
                                            ::tracing::Level::WARN,
                                            ::tracing_core::__macro_support::Option::Some(
                                                "dreampipe/src/config.rs",
                                            ),
                                            ::tracing_core::__macro_support::Option::Some(54u32),
                                            ::tracing_core::__macro_support::Option::Some(
                                                "dreampipe::config",
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
                                        "event dreampipe/src/config.rs:82",
                                        "dreampipe::config",
                                        ::tracing::Level::ERROR,
                                        ::tracing_core::__macro_support::Option::Some(
                                            "dreampipe/src/config.rs",
                                        ),
                                        ::tracing_core::__macro_support::Option::Some(82u32),
                                        ::tracing_core::__macro_support::Option::Some(
                                            "dreampipe::config",
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
    use drm::control::plane;
    use drm::control::property;
    pub use drm::control::Device as ControlDevice;
    use drm::control::PlaneType;
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
    #[repr(C)]
    pub struct Display {
        pub name: String,
        pub size: (u32, u32),
        pub pos: (i32, i32),
        pub connector: connector::Handle,
        pub crtc: crtc::Handle,
        pub connector_props: HashMap<String, property::Info>,
        pub crtc_props: HashMap<String, property::Info>,
        pub mode: control::Mode,
        pub primary: DrmCtx,
        pub cursor: DrmCtx,
        pub overlays: Vec<DrmCtx>,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for Display {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "name",
                "size",
                "pos",
                "connector",
                "crtc",
                "connector_props",
                "crtc_props",
                "mode",
                "primary",
                "cursor",
                "overlays",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.name,
                &self.size,
                &self.pos,
                &self.connector,
                &self.crtc,
                &self.connector_props,
                &self.crtc_props,
                &self.mode,
                &self.primary,
                &self.cursor,
                &&self.overlays,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "Display",
                names,
                values,
            )
        }
    }
    #[repr(C)]
    pub struct ReadonlyDisplay {
        name: String,
        pub size: (u32, u32),
        pub pos: (i32, i32),
        pub connector: connector::Handle,
        pub crtc: crtc::Handle,
        pub connector_props: HashMap<String, property::Info>,
        pub crtc_props: HashMap<String, property::Info>,
        pub mode: control::Mode,
        pub primary: DrmCtx,
        pub cursor: DrmCtx,
        pub overlays: Vec<DrmCtx>,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for ReadonlyDisplay {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "name",
                "size",
                "pos",
                "connector",
                "crtc",
                "connector_props",
                "crtc_props",
                "mode",
                "primary",
                "cursor",
                "overlays",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.name,
                &self.size,
                &self.pos,
                &self.connector,
                &self.crtc,
                &self.connector_props,
                &self.crtc_props,
                &self.mode,
                &self.primary,
                &self.cursor,
                &&self.overlays,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "ReadonlyDisplay",
                names,
                values,
            )
        }
    }
    impl core::ops::Deref for Display {
        type Target = ReadonlyDisplay;
        fn deref(&self) -> &Self::Target {
            unsafe { &*(self as *const Self as *const Self::Target) }
        }
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
        pub fn init_req(
            &self,
            card: &Card,
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
            Ok(())
        }
        pub fn init_displays(
            ignore_list: impl Into<Option<HashSet<String>>>,
            card: &Card,
            gbm: &gbm::Device<&'static Card>,
        ) -> CompositorResult<Vec<Display>> {
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
                    (width, height) => (width as u32, height as u32),
                };
                let primary = DrmCtx::from_connector(
                    card,
                    gbm,
                    &resources,
                    crtc,
                    &mut planes,
                    PlaneType::Primary,
                    (size.0, size.1),
                )?;
                let cursor = DrmCtx::from_connector(
                    card,
                    gbm,
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
                    .push(Display {
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
                    });
            }
            Ok(displays)
        }
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
        GpuCard,
        ClientCapability(ClientCapability, IoError),
        ResourcesError(IoError),
        NoQualifiedConnectors,
        GbmCreation(IoError),
        GbmFd(InvalidFdError),
        GbmSurfaceCreate(IoError),
        FrontBufferLock,
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
                CompositorError::GpuCard => {
                    ::core::fmt::Formatter::write_str(f, "GpuCard")
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
                Self::GpuCard => {
                    ::alloc::__export::must_use({
                        ::alloc::fmt::format(
                            format_args!("No matching card for selected GPU"),
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
mod util {
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
            sorted
                .insert(
                    key,
                    (
                        display.name.to_owned(),
                        Size {
                            width: Dimension::length(display.size.0 as f32),
                            height: Dimension::length(display.size.1 as f32),
                        },
                    ),
                );
        }
        let mut tree: TaffyTree<String> = TaffyTree::<String>::new();
        let mut leafs = Vec::new();
        let mut hnodes = Vec::new();
        let mut vnodes = Vec::new();
        let mut prev_y = 0;
        for ((y, _), (name, size)) in sorted {
            if y > prev_y {
                let node = tree
                    .new_with_children(
                        Style {
                            flex_direction: FlexDirection::Row,
                            flex_grow: 0.0,
                            flex_wrap: taffy::FlexWrap::NoWrap,
                            flex_shrink: 0.0,
                            ..Default::default()
                        },
                        &hnodes,
                    )
                    .unwrap();
                vnodes.push(node);
                leafs.extend(hnodes.drain(..));
            }
            prev_y = y;
            hnodes
                .push(
                    tree
                        .new_leaf_with_context(
                            Style {
                                display: NodeDisplay::Block,
                                size,
                                min_size: size,
                                max_size: size,
                                flex_grow: 0.0,
                                flex_shrink: 0.0,
                                ..Default::default()
                            },
                            name,
                        )
                        .unwrap(),
                );
        }
        {
            let node = tree
                .new_with_children(
                    Style {
                        flex_direction: FlexDirection::Row,
                        flex_grow: 0.0,
                        flex_wrap: taffy::FlexWrap::NoWrap,
                        flex_shrink: 0.0,
                        ..Default::default()
                    },
                    &hnodes,
                )
                .unwrap();
            vnodes.push(node);
            leafs.extend(hnodes.drain(..));
        }
        let root_node = tree
            .new_with_children(
                Style {
                    flex_direction: FlexDirection::Column,
                    ..Default::default()
                },
                &vnodes,
            )
            .unwrap();
        tree.compute_layout(root_node, Size::max_content()).unwrap();
        for leaf in leafs.iter() {
            let nym = tree.get_node_context(*leaf).unwrap();
            let display = displays.get_mut(nym).unwrap();
            let pos = tree.layout(*leaf).unwrap().location;
            display.pos = (pos.x as i32, pos.y as i32);
        }
        (tree, leafs)
    }
}
mod gpu {
    use crate::config::CompositorConfig;
    use crate::config::Config;
    use crate::util::DisplayPosition;
    use crate::card::Card;
    use crate::display::Display;
    use crate::error::CompositorResult;
    use drm::control::AtomicCommitFlags;
    use drm::control::Device as ControlDevice;
    use drm::control::atomic;
    use drm::control::connector;
    use std::collections::HashSet;
    use taffy::NodeId;
    use taffy::TaffyTree;
    use wgpu::TextureFormat;
    const TEXTURE_FORMAT: TextureFormat = TextureFormat::Rgba8Uint;
    const BLIT_SHADER: &str = r#"struct VertexOutput {
   @builtin(position) position: vec4<f32>,
   @location(0) tex_coords: vec2<f32>,
}
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
   var out: VertexOutput;
   let x = f32((vertex_index & 1u) << 2u) - 1.0;
   let x = f32((vertex_index & 2u) << 1u) - 1.0;
   out.position = vec4<f32>(x, y, 0.0, 1.0);
   out.tex_coords = vec2<f32>(x + 1.0, 1.0 - y) * 0.5;
   return out;
}
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var tex_sampler: sampler;
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<32> {
   return textureSample(tex, tex_sampler, in.tex_coords);
}"#;
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
    pub async fn init_gpu(
        card: &Card,
    ) -> CompositorResult<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
        let card_num = card.num();
        if let Some((vendor_id, device_id)) = get_pci_ids_from_card(card_num) {
            {
                ::std::io::_print(
                    format_args!(
                        "Opend card{0}: vendor=0x{1:x}, device=0x{2:x}\n",
                        card_num,
                        vendor_id,
                        device_id,
                    ),
                );
            };
        }
        let instance = wgpu::Instance::new(
            &wgpu::InstanceDescriptor {
                backends: wgpu::Backends::VULKAN,
                ..Default::default()
            },
        );
        let adapters = instance.enumerate_adapters(wgpu::Backends::all());
        {
            ::std::io::_print(format_args!("Available GPUs:\n"));
        };
        for (i, adapter) in adapters.iter().enumerate() {
            let info = adapter.get_info();
            {
                ::std::io::_print(
                    format_args!(
                        "  [{3}] {0} - {1:?} (Backend: {2:?})\n",
                        info.name,
                        info.device_type,
                        info.backend,
                        i,
                    ),
                );
            };
        }
        let adapter = adapters
            .clone()
            .into_iter()
            .find(|a| a.get_info().device_type == wgpu::DeviceType::DiscreteGpu)
            .or_else(|| adapters.into_iter().next())
            .expect("No suitable adapter found");
        let info = adapter.get_info();
        {
            ::std::io::_print(
                format_args!("Selected GPU {0} ({1:?})\n", info.name, info.backend),
            );
        };
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("DMA-BUF Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::defaults(),
                    memory_hints: wgpu::MemoryHints::default(),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    trace: wgpu::Trace::Off,
                },
            )
            .await
            .expect("Failed to create device");
        {
            ::std::io::_print(
                format_args!("Card and GPU successfully initialized in tandem.\n"),
            );
        };
        Ok((adapter, device, queue))
    }
    pub fn create_pipeline(device: &wgpu::Device) {
        let mut encoder = device
            .create_command_encoder(
                &wgpu::CommandEncoderDescriptor {
                    label: Some("Compositor Render Encoder"),
                },
            );
        let shader = device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader"),
                source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
            });
        let pipeline = device
            .create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("Blit Pipeline"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        targets: &[
                            Some(wgpu::ColorTargetState {
                                format: TEXTURE_FORMAT,
                                blend: None,
                                write_mask: wgpu::ColorWrites::ALL,
                            }),
                        ],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                },
            );
    }
    pub struct GpuContext {
        pub card: Box<Card>,
        pub gbm: gbm::Device<&'static Card>,
        pub displays: Vec<Display>,
    }
    impl GpuContext {
        pub fn update(&mut self) {
            let events = match self.card.receive_events() {
                Ok(events) => {
                    {
                        ::std::io::_print(format_args!("Ready to receive events!\n"));
                    };
                    events.peekable()
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    {
                        ::std::io::_print(format_args!("Would block\n"));
                    };
                    return;
                }
                Err(e) => {
                    ::core::panicking::panic_fmt(format_args!("{0}", e));
                }
            };
            events
                .for_each(|event| {
                    match event {
                        drm::control::Event::PageFlip(event) => {
                            {
                                ::std::io::_print(format_args!("Got page flip event\n"));
                            };
                            let mut to_remove: HashSet<String> = HashSet::new();
                            for display in self.displays.iter_mut() {
                                if display.crtc != event.crtc {
                                    continue;
                                }
                                {
                                    ::std::io::_print(
                                        format_args!(
                                            "Blitting to the framebuffer for {0}\n",
                                            display.name,
                                        ),
                                    );
                                };
                                match unsafe {
                                    display.primary.swap(&self.card, display.crtc)
                                } {
                                    Ok(()) => {}
                                    Err(e) => {
                                        if let Ok(info) = self
                                            .card
                                            .get_connector(display.connector, false)
                                        {
                                            {
                                                ::std::io::_print(format_args!("Got an error: {0}\n", e));
                                            };
                                            if info.state() != connector::State::Connected {
                                                to_remove.insert(display.name.to_owned());
                                            }
                                        }
                                    }
                                }
                            }
                            self.displays
                                .retain_mut(|display| {
                                    if to_remove.contains(&display.name) {
                                        for fb in display
                                            .primary
                                            .buffers
                                            .fbs
                                            .iter()
                                            .chain(display.cursor.buffers.fbs.iter())
                                        {
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
                                    }
                                });
                        }
                        _ => {}
                    }
                });
        }
        pub fn displays_mut(&mut self) -> impl Iterator<Item = &mut Display> {
            self.displays.iter_mut()
        }
        /// Returns true if any new displays were acquired
        pub fn init_displays(&mut self, config: &Config) -> bool {
            let ignore_list = HashSet::<
                String,
            >::from_iter(self.displays.iter().map(|display| display.name.to_owned()));
            let mut new_displays = Display::init_displays(
                    ignore_list,
                    &self.card,
                    &self.gbm,
                )
                .unwrap_or_else(|e| {
                    {
                        use ::tracing::__macro_support::Callsite as _;
                        static __CALLSITE: ::tracing::callsite::DefaultCallsite = {
                            static META: ::tracing::Metadata<'static> = {
                                ::tracing_core::metadata::Metadata::new(
                                    "event dreampipe/src/gpu.rs:243",
                                    "dreampipe::gpu",
                                    ::tracing::Level::WARN,
                                    ::tracing_core::__macro_support::Option::Some(
                                        "dreampipe/src/gpu.rs",
                                    ),
                                    ::tracing_core::__macro_support::Option::Some(243u32),
                                    ::tracing_core::__macro_support::Option::Some(
                                        "dreampipe::gpu",
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
                                                    &format_args!("Failed to init displays: {0}", e)
                                                        as &dyn Value,
                                                ),
                                            ),
                                        ],
                                    )
                            });
                        } else {
                        }
                    };
                    Default::default()
                });
            if !new_displays.is_empty() {
                for display in new_displays.iter() {
                    {
                        ::std::io::_print(
                            format_args!(
                                "Found display: {0} {1:?}\n",
                                display.name,
                                display.size,
                            ),
                        );
                    };
                }
            }
            for display in new_displays.iter_mut() {
                let mut atomic_req = atomic::AtomicModeReq::new();
                display
                    .init_req(self.card.as_ref(), &mut atomic_req)
                    .expect("Failed to init display");
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
                self.card
                    .atomic_commit(
                        AtomicCommitFlags::ALLOW_MODESET | AtomicCommitFlags::NONBLOCK
                            | AtomicCommitFlags::PAGE_FLIP_EVENT,
                        atomic_req,
                    )
                    .expect("Failed to set mode");
            }
            if !new_displays.is_empty() {
                self.displays.extend(new_displays.into_iter());
                for display in self.displays.iter_mut() {
                    let name = &display.name;
                    let pos = config
                        .get::<DisplayPosition>(&CompositorConfig::offset_key(name));
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
}
use config::Config;
use taffy::NodeId;
use crate::card::Card;
use crate::config::CompositorConfig;
use crate::display::Display;
use crate::gpu::GpuContext;
use crate::util::BackgroundImage;
use crate::util::GlFns;
use crate::util::layout_displays;
use drm::buffer::DrmFourcc;
use drm::control::Device as ControlDevice;
use gbm::AsRaw;
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
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use taffy::TaffyTree;
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
    let mut contexts = Vec::new();
    let bg = load_default_bg();
    let aligned_width = (bg.width() + 15) & !15;
    let aligned_height = (bg.height() + 15) & !15;
    let mut padded = DynamicImage::new_rgba8(aligned_width, aligned_height);
    padded.copy_from(&bg, 0, 0).unwrap();
    for card in cards.into_iter() {
        let card: Box<Card> = Box::new(card);
        let card_ptr: *mut Card = Box::leak(card);
        let card_ptr_clone: *const Card = card_ptr as *const Card;
        let card_ref: &'static Card = unsafe { &*card_ptr_clone };
        let card: Box<Card> = unsafe { Box::from_raw(card_ptr) };
        let gbm: gbm::Device<&'static Card> = gbm::Device::new(card_ref)
            .expect("Failed to init GBM with device");
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
        let displays: Vec<Display> = Vec::new();
        let context = GpuContext { card, gbm, displays };
        contexts.push(context);
    }
    let end = Instant::now() + Duration::from_secs(5);
    let mut frame = 0usize;
    let mut layout: TaffyTree<String> = TaffyTree::new();
    let mut leaf_ids: Vec<NodeId> = Vec::new();
    loop {
        let mut displays_acquired = false;
        match rx.try_recv() {
            Ok(Ok(NotifyEvent { .. })) => {
                if let Ok(new_config) = Config::new(&config_path) {
                    config = new_config;
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
                                    "event dreampipe/src/main.rs:171",
                                    "dreampipe",
                                    ::tracing::Level::WARN,
                                    ::tracing_core::__macro_support::Option::Some(
                                        "dreampipe/src/main.rs",
                                    ),
                                    ::tracing_core::__macro_support::Option::Some(171u32),
                                    ::tracing_core::__macro_support::Option::Some("dreampipe"),
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
        for context in contexts.iter_mut() {
            context.update();
            displays_acquired |= context.init_displays(&config);
        }
        if Instant::now().checked_duration_since(end).is_some() {
            break;
        }
        frame += 1;
        if displays_acquired {
            let displays: HashMap<String, &mut Display> = contexts
                .iter_mut()
                .map(|context| context.displays_mut())
                .flatten()
                .map(|display| (display.name.to_owned(), display))
                .collect();
            (layout, leaf_ids) = layout_displays(displays);
        }
    }
}
