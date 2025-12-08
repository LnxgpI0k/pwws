use drm::buffer::DrmFourcc;

pub trait FourCc {
   fn depth(&self) -> u32;
   fn bpp(&self) -> u32;
}

// depth is bits used for encoding
// bpp is storage size or stride
impl FourCc for DrmFourcc {
   fn depth(&self) -> u32 {
      match self {
         DrmFourcc::Big_endian => 0,
         DrmFourcc::Rgb332 | DrmFourcc::Bgr233 | DrmFourcc::C8 | DrmFourcc::R8 => 8,
         DrmFourcc::Yvu410 | DrmFourcc::Yuv410 => 9,
         DrmFourcc::X0l0 |
         DrmFourcc::Y0l0 |
         DrmFourcc::Q401 |
         DrmFourcc::X0l2 |
         DrmFourcc::Y0l2 => 10,
         DrmFourcc::Yvu420 |
         DrmFourcc::Yuv420 |
         DrmFourcc::Yvu411 |
         DrmFourcc::Yuv411 |
         DrmFourcc::Nv21 |
         DrmFourcc::Nv12 |
         DrmFourcc::Yuv420_8bit => 12,
         DrmFourcc::P010 | DrmFourcc::Nv15 | DrmFourcc::Yuv420_10bit => 15,
         DrmFourcc::Rgba5551 |
         DrmFourcc::Bgra5551 |
         DrmFourcc::Rgbx5551 |
         DrmFourcc::Bgrx5551 |
         DrmFourcc::Nv61 |
         DrmFourcc::Yvu422 |
         DrmFourcc::Yuv422 |
         DrmFourcc::Rgba4444 |
         DrmFourcc::Bgra4444 |
         DrmFourcc::Argb4444 |
         DrmFourcc::Xrgb4444 |
         DrmFourcc::Abgr4444 |
         DrmFourcc::Xbgr4444 |
         DrmFourcc::Rgbx4444 |
         DrmFourcc::Bgrx4444 |
         DrmFourcc::Argb1555 |
         DrmFourcc::Xrgb1555 |
         DrmFourcc::Abgr1555 |
         DrmFourcc::Xbgr1555 |
         DrmFourcc::Rgb565 |
         DrmFourcc::Bgr565 |
         DrmFourcc::R16 |
         DrmFourcc::Nv16 |
         DrmFourcc::Rg88 |
         DrmFourcc::Gr88 |
         DrmFourcc::Yvyu |
         DrmFourcc::Yuyv |
         DrmFourcc::Vyuy |
         DrmFourcc::Uyvy => 16,
         DrmFourcc::P012 => 18,
         DrmFourcc::P210 | DrmFourcc::Y210 => 20,
         DrmFourcc::Y212 |
         DrmFourcc::Nv42 |
         DrmFourcc::Nv24 |
         DrmFourcc::Yvu444 |
         DrmFourcc::Yuv444 |
         DrmFourcc::P016 |
         DrmFourcc::Rgb888 |
         DrmFourcc::Bgr888 |
         DrmFourcc::Vuy888 |
         DrmFourcc::Rgb565_a8 |
         DrmFourcc::Bgr565_a8 => 24,
         DrmFourcc::Vuy101010 | DrmFourcc::Q410 | DrmFourcc::Y410 => 30,
         DrmFourcc::Argb2101010 |
         DrmFourcc::Xrgb2101010 |
         DrmFourcc::Abgr2101010 |
         DrmFourcc::Xbgr2101010 |
         DrmFourcc::Xvyu2101010 |
         DrmFourcc::Rgba1010102 |
         DrmFourcc::Bgra1010102 |
         DrmFourcc::Rgbx1010102 |
         DrmFourcc::Bgrx1010102 |
         DrmFourcc::Y216 |
         DrmFourcc::Rg1616 |
         DrmFourcc::Gr1616 |
         DrmFourcc::Rgba8888 |
         DrmFourcc::Bgra8888 |
         DrmFourcc::Argb8888 |
         DrmFourcc::Xrgb8888 |
         DrmFourcc::Abgr8888 |
         DrmFourcc::Xbgr8888 |
         DrmFourcc::Xyuv8888 |
         DrmFourcc::Rgbx8888 |
         DrmFourcc::Bgrx8888 |
         DrmFourcc::Rgb888_a8 |
         DrmFourcc::Bgr888_a8 |
         DrmFourcc::Ayuv => 32,
         DrmFourcc::Xrgb8888_a8 |
         DrmFourcc::Xbgr8888_a8 |
         DrmFourcc::Rgbx8888_a8 |
         DrmFourcc::Bgrx8888_a8 => 40,
         DrmFourcc::Y412 | DrmFourcc::Xvyu12_16161616 => 48,
         DrmFourcc::Axbxgxrx106106106106 |
         DrmFourcc::Y416 |
         DrmFourcc::Xvyu16161616 |
         DrmFourcc::Argb16161616f |
         DrmFourcc::Xrgb16161616f |
         DrmFourcc::Abgr16161616f |
         DrmFourcc::Xbgr16161616f => 64,
      }
   }

   fn bpp(&self) -> u32 {
      match self {
         DrmFourcc::Big_endian => 0,
         DrmFourcc::Rgb332 | DrmFourcc::Bgr233 | DrmFourcc::C8 | DrmFourcc::R8 => 8,
         DrmFourcc::Yvu410 | DrmFourcc::Yuv410 => 9,
         DrmFourcc::X0l0 |
         DrmFourcc::Y0l0 |
         DrmFourcc::Q401 |
         DrmFourcc::X0l2 |
         DrmFourcc::Y0l2 => 10,
         DrmFourcc::Yvu420 |
         DrmFourcc::Yuv420 |
         DrmFourcc::Yvu411 |
         DrmFourcc::Yuv411 |
         DrmFourcc::Nv21 |
         DrmFourcc::Nv12 |
         DrmFourcc::Yuv420_8bit => 12,
         DrmFourcc::P010 | DrmFourcc::Nv15 | DrmFourcc::Yuv420_10bit => 15,
         DrmFourcc::Rgba5551 |
         DrmFourcc::Bgra5551 |
         DrmFourcc::Rgbx5551 |
         DrmFourcc::Bgrx5551 |
         DrmFourcc::Nv61 |
         DrmFourcc::Yvu422 |
         DrmFourcc::Yuv422 |
         DrmFourcc::Rgba4444 |
         DrmFourcc::Bgra4444 |
         DrmFourcc::Argb4444 |
         DrmFourcc::Xrgb4444 |
         DrmFourcc::Abgr4444 |
         DrmFourcc::Xbgr4444 |
         DrmFourcc::Rgbx4444 |
         DrmFourcc::Bgrx4444 |
         DrmFourcc::Argb1555 |
         DrmFourcc::Xrgb1555 |
         DrmFourcc::Abgr1555 |
         DrmFourcc::Xbgr1555 |
         DrmFourcc::Rgb565 |
         DrmFourcc::Bgr565 |
         DrmFourcc::R16 |
         DrmFourcc::Nv16 |
         DrmFourcc::Rg88 |
         DrmFourcc::Gr88 |
         DrmFourcc::Yvyu |
         DrmFourcc::Yuyv |
         DrmFourcc::Vyuy |
         DrmFourcc::Uyvy => 16,
         DrmFourcc::P012 => 18,
         DrmFourcc::P210 | DrmFourcc::Y210 => 20,
         DrmFourcc::Y212 |
         DrmFourcc::Nv42 |
         DrmFourcc::Nv24 |
         DrmFourcc::Yvu444 |
         DrmFourcc::Yuv444 |
         DrmFourcc::P016 |
         DrmFourcc::Rgb888 |
         DrmFourcc::Bgr888 |
         DrmFourcc::Vuy888 |
         DrmFourcc::Rgb565_a8 |
         DrmFourcc::Bgr565_a8 => 24,
         DrmFourcc::Vuy101010 | DrmFourcc::Q410 => 30,
         DrmFourcc::Argb2101010 |
         DrmFourcc::Xrgb2101010 |
         DrmFourcc::Abgr2101010 |
         DrmFourcc::Xbgr2101010 |
         DrmFourcc::Xvyu2101010 |
         DrmFourcc::Y410 |
         DrmFourcc::Rgba1010102 |
         DrmFourcc::Bgra1010102 |
         DrmFourcc::Rgbx1010102 |
         DrmFourcc::Bgrx1010102 |
         DrmFourcc::Y216 |
         DrmFourcc::Rg1616 |
         DrmFourcc::Gr1616 |
         DrmFourcc::Rgba8888 |
         DrmFourcc::Bgra8888 |
         DrmFourcc::Argb8888 |
         DrmFourcc::Xrgb8888 |
         DrmFourcc::Abgr8888 |
         DrmFourcc::Xbgr8888 |
         DrmFourcc::Xyuv8888 |
         DrmFourcc::Rgbx8888 |
         DrmFourcc::Bgrx8888 |
         DrmFourcc::Rgb888_a8 |
         DrmFourcc::Bgr888_a8 |
         DrmFourcc::Ayuv => 32,
         DrmFourcc::Xrgb8888_a8 |
         DrmFourcc::Xbgr8888_a8 |
         DrmFourcc::Rgbx8888_a8 |
         DrmFourcc::Bgrx8888_a8 => 40,
         DrmFourcc::Y412 | DrmFourcc::Xvyu12_16161616 => 48,
         DrmFourcc::Axbxgxrx106106106106 |
         DrmFourcc::Y416 |
         DrmFourcc::Xvyu16161616 |
         DrmFourcc::Argb16161616f |
         DrmFourcc::Xrgb16161616f |
         DrmFourcc::Abgr16161616f |
         DrmFourcc::Xbgr16161616f => 64,
      }
   }
}
