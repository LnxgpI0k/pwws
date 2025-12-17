#![allow(dead_code)]

use ash::vk;
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

#[derive(Debug)]
pub enum CompositorError {
  OpenCard(PathBuf, IoError),
  GpuCard,
  VulkanApi,
  VulkanImageDim,
  VulkanImageCreate(vk::Result),
  VulkanMemoryAlloc(vk::Result),
  VulkanMemoryTypeIndex,
  VulkanBindMemory(vk::Result),
  ClientCapability(ClientCapability, IoError),
  ResourcesError(IoError),
  NoQualifiedConnectors,
  GbmCreation(IoError),
  GbmFd(InvalidFdError),
  GbmSurfaceCreate(IoError),
  GbmModifier,
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

impl Display for CompositorError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let msg =
      match self {
        Self::OpenCard(path, error) => format![
          "Unable to open card at {path:?}: {error:#?}"
        ],
        Self::GpuCard => format!["No matching card for selected GPU"],
        Self::VulkanApi => format!["Vulkan not supported"],
        Self::VulkanImageDim => format!["Invalid DMA-BUF dimensions"],
        Self::VulkanImageCreate(e) => format!["Failed to create vulkan image: {e}"],
        Self::VulkanMemoryAlloc(e) => format![
          "Failed to allocate memory for DMA-BUF: {e}"
        ],
        Self::VulkanMemoryTypeIndex => format![
          "Failed to find suitable memory type for DMA-BUF"
        ],
        Self::VulkanBindMemory(e) => format!["Failed to bind vulkan memory: {e}"],
        Self::ClientCapability(client_capability, error) => format![
          "Unable to request {client_capability:#?}: {error:#?}"
        ],
        Self::ResourcesError(error) => format![
          "Could not load normal resource IDs: {error:#?}"
        ],
        Self::NoQualifiedConnectors => format!["No active connectors found."],
        Self::GbmCreation(error) => format![
          "Failed to create GBM buffer object: {error:#?}"
        ],
        Self::GbmFd(error) => format!["Invalid GBM buffer Fd: {error}"],
        Self::GbmSurfaceCreate(error) => format![
          "Failed to create GBM surface: {error:#?}"
        ],
        Self::GbmModifier => format!["Failed to get GBM buffer modifiers"],
        Self::FrontBufferLock => format!["Failed to lock front buffer"],
        Self::AddFrameBuffer(error) => format![
          "Failed to add framebuffer to card: {error:#?}"
        ],
        Self::GetPlanes(error) => format!["Failed to get planes: {error:#?}"],
        Self::NoCompatiblePrimaryPlane(info) => format![
          "Failed to get compatible plane for CRTC. CRTC Info:\n{info:#?}"
        ],
        Self::UnknownPlaneType(val) => format!["Unkown plane type '{val:x}'"],
        Self::PlaneNotFound(planetype) => format![
          "Plane type {planetype:#?} not available."
        ],
        Self::GetConnectorProperties(handle, error) => format![
          "Failed to get properties for connector {handle:#?}: {error:#?}"
        ],
        Self::GetConnectorInfo(handle, error) => format![
          "Failed to get info for connector {handle:#?}: {error:#?}"
        ],
        Self::GetCrtcProperties(handle, error) => format![
          "Failed to get properties for CRTC {handle:#?}: {error:#?}"
        ],
        Self::GetCrtcInfo(handle, error) => format![
          "Failed to get info for CRTC {handle:#?}: {error:#?}"
        ],
        Self::GetEncoderInfo(handle, error) => format![
          "Failed to get info for encoder {handle:#?}: {error:#?}"
        ],
        Self::GetPlaneProperties(handle, error) => format![
          "Failed to get properties for plane {handle:#?}: {error:#?}"
        ],
        Self::PropsToHashMap(error) => format![
          "Failed to convert props to hashmap: {error:#?}"
        ],
        Self::AtomicCommitFailed(error) => format![
          "Failed to commit request to CRTC: {error:#?}"
        ],
        Self::ConfigOpen(error) => format!["Failed to open configuration file: {error}"],
        Self::ConfigRead(error) => format!["Failed to read configuration file: {error}"],
        Self::ConfigMissing(k) => format!["Missing {k} in config"],
        Self::ConfigConvert(k, error) => format!["Failed to convert key {k}: {error}"],
      };
    write![f, "{msg}"]
  }
}
