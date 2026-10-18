//! On-device crop previews through the operating system's foreground instance masks.
//! No model download, external program, or upload. macOS 14 / iOS 17 or newer.

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple {
    use objc2::encode::{Encoding, RefEncode};
    use objc2::msg_send;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use std::ffi::c_void;
    static INFERENCE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[repr(C)]
    struct PixelBuffer {
        _opaque: [u8; 0],
    }
    unsafe impl RefEncode for PixelBuffer {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CVBuffer", &[]));
    }
    #[link(name = "Vision", kind = "framework")]
    extern "C" {}
    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        fn CVPixelBufferLockBaseAddress(buffer: *mut PixelBuffer, flags: u64) -> i32;
        fn CVPixelBufferUnlockBaseAddress(buffer: *mut PixelBuffer, flags: u64) -> i32;
        fn CVPixelBufferGetWidth(buffer: *const PixelBuffer) -> usize;
        fn CVPixelBufferGetHeight(buffer: *const PixelBuffer) -> usize;
        fn CVPixelBufferGetBytesPerRow(buffer: *const PixelBuffer) -> usize;
        fn CVPixelBufferGetBaseAddress(buffer: *const PixelBuffer) -> *const c_void;
        fn CVPixelBufferGetPixelFormatType(buffer: *const PixelBuffer) -> u32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: *const c_void);
    }

    struct OwnedPixelBuffer(*mut PixelBuffer);
    impl Drop for OwnedPixelBuffer {
        fn drop(&mut self) {
            unsafe {
                CFRelease(self.0.cast());
            }
        }
    }

    pub fn available() -> bool {
        AnyClass::get(c"VNGenerateForegroundInstanceMaskRequest").is_some()
    }

    /// Binary response: six little-endian u32s (width, height, x0, y0, x1, y1), then a grey PNG.
    pub fn preview(
        bytes: &[u8],
        focus: f64,
        threshold: f64,
        region: Option<[f64; 4]>,
    ) -> Result<Vec<u8>, String> {
        if !available() {
            return Err("foreground-unavailable".into());
        }
        if bytes.len() > 32 * 1024 * 1024
            || !focus.is_finite()
            || !(0.3..=1.0).contains(&focus)
            || !threshold.is_finite()
            || !(0.1..=0.9).contains(&threshold)
        {
            return Err("invalid-foreground-input".into());
        }
        if region.is_some_and(|r| {
            r.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || r[0] >= r[2]
                || r[1] >= r[3]
        }) {
            return Err("invalid-foreground-input".into());
        }
        // A settings change may leave a previous preview in flight. Serialize model work.
        let _inference = INFERENCE
            .lock()
            .map_err(|_| "foreground-failed".to_string())?;
        let photo = crispmesh_dense::photos::staging::decode_photo(
            std::path::Path::new("capture.jpg"),
            bytes,
        )
        .map_err(|e| e.to_string())?;
        let (width, height) = (photo.width(), photo.height());
        if width * height > 40_000_000 {
            return Err("foreground-image-too-large".into());
        }
        autoreleasepool(|_| {
            objc2::exception::catch(std::panic::AssertUnwindSafe(|| unsafe {
            let class =
                |name| AnyClass::get(name).ok_or_else(|| "foreground-unavailable".to_string());
            let (data_class, dict_class, array_class) = (
                class(c"NSData")?,
                class(c"NSDictionary")?,
                class(c"NSArray")?,
            );
            let handler_class = class(c"VNImageRequestHandler")?;
            let request_class = class(c"VNGenerateForegroundInstanceMaskRequest")?;
            let index_class = class(c"NSIndexSet")?;
            // A centre focus is an explicit crop of the image sent to Vision, with all offsets restored below.
            let (left, top, cw, ch) = if let Some([x0,y0,x1,y1]) = region {
                let left = (x0*width as f64).floor() as usize;
                let top = (y0*height as f64).floor() as usize;
                let right = ((x1*width as f64).ceil() as usize).min(width);
                let bottom = ((y1*height as f64).ceil() as usize).min(height);
                (left, top, right-left, bottom-top)
            } else {
                let cw = ((width as f64 * focus).round() as usize).max(1);
                let ch = ((height as f64 * focus).round() as usize).max(1);
                ((width-cw)/2, (height-ch)/2, cw, ch)
            };
            let mut rgb = Vec::with_capacity(cw * ch * 3);
            for y in top..top + ch {
                rgb.extend_from_slice(
                    &photo.rgb.as_raw()[(y * width + left) * 3..(y * width + left + cw) * 3],
                );
            }
            let encoded =
                crispmesh_dense::storage::png_bytes(cw, ch, 3, &rgb).map_err(|e| e.to_string())?;
            let data: Retained<AnyObject> = msg_send![data_class, dataWithBytes: encoded.as_ptr().cast::<c_void>(), length: encoded.len()];
            let options: Retained<AnyObject> = msg_send![dict_class, new];
            let alloc: *mut AnyObject = msg_send![handler_class, alloc];
            let handler_raw: *mut AnyObject =
                msg_send![alloc, initWithData: &*data, options: &*options];
            let handler =
                Retained::from_raw(handler_raw).ok_or_else(|| "foreground-failed".to_string())?;
            let request: Retained<AnyObject> = msg_send![request_class, new];
            let requests: Retained<AnyObject> = msg_send![array_class, arrayWithObject: &*request];
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let ok: bool = msg_send![&*handler, performRequests: &*requests, error: &mut error];
            if !ok {
                return Err("foreground-failed".into());
            }
            let results: Option<Retained<AnyObject>> = msg_send![&*request, results];
            let results = results.ok_or_else(|| "foreground-not-found".to_string())?;
            let count: usize = msg_send![&*results, count];
            if count == 0 {
                return Err("foreground-not-found".into());
            }
            let result: Retained<AnyObject> = msg_send![&*results, objectAtIndex: 0usize];
            let instances: Retained<AnyObject> = msg_send![&*result, allInstances];
            let mut index: usize = msg_send![&*instances, firstIndex];
            let mut best: Option<(f64, Vec<u8>, [usize; 4])> = None;
            // NSIndexSet uses NSNotFound (NSIntegerMax), not NSUIntegerMax.
            while index != isize::MAX as usize {
                let chosen: Retained<AnyObject> = msg_send![index_class, indexSetWithIndex: index];
                let buffer: *mut PixelBuffer = msg_send![&*result, generateScaledMaskForImageForInstances: &*chosen, fromRequestHandler: &*handler, error: &mut error];
                if !buffer.is_null() {
                    let _owned = OwnedPixelBuffer(buffer);
                    let locked = CVPixelBufferLockBaseAddress(buffer, 1) == 0;
                    if locked {
                        let (bw, bh) = (
                            CVPixelBufferGetWidth(buffer),
                            CVPixelBufferGetHeight(buffer),
                        );
                        let format = CVPixelBufferGetPixelFormatType(buffer);
                        let stride = CVPixelBufferGetBytesPerRow(buffer);
                        let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
                        let mut mask = vec![0u8; width * height];
                        let (mut area, mut sx, mut sy) = (0usize, 0usize, 0usize);
                        if bw == cw
                            && bh == ch
                            && !base.is_null()
                            && [0x4c303038, 0x4c303066].contains(&format)
                        {
                            for y in 0..ch {
                                for x in 0..cw {
                                    let value = if format == 0x4c303038 {
                                        *base.add(y * stride + x) as f64 / 255.0
                                    } else {
                                        std::ptr::read_unaligned(
                                            base.add(y * stride + 4 * x).cast::<f32>(),
                                        ) as f64
                                    };
                                    if value >= threshold {
                                        mask[(top + y) * width + left + x] = 255;
                                        area += 1;
                                        sx += left + x;
                                        sy += top + y;
                                    }
                                }
                            }
                        }
                        CVPixelBufferUnlockBaseAddress(buffer, 1);
                        if area >= 100 && area * 10 < cw * ch * 9 {
                            let bbox = crispmesh_dense::photos::scene_writer::foreground_box(
                                &mask, width, height, 0.0,
                            )
                            .map_err(|e| e.to_string())?;
                            // Reject the image border / an object clipped by the search area. A complete
                            // close-up can fill most of the frame; area alone must not reject it.
                            if bbox[0] <= left + 1
                                || bbox[1] <= top + 1
                                || bbox[2] >= left + cw - 1
                                || bbox[3] >= top + ch - 1
                            {
                                index = msg_send![&*instances, indexGreaterThanIndex: index];
                                continue;
                            }
                            let distance = ((sx as f64 / area as f64 - left as f64) / cw as f64 - 0.5).powi(2)
                                + ((sy as f64 / area as f64 - top as f64) / ch as f64 - 0.5).powi(2);
                            let score =
                                (area as f64 / (width * height) as f64).sqrt() / (0.03 + distance);
                            if best.as_ref().is_none_or(|v| score > v.0) {
                                best = Some((score, mask, bbox));
                            }
                        }
                    }
                }
                index = msg_send![&*instances, indexGreaterThanIndex: index];
            }
            let (_, mask, bbox) = best.ok_or_else(|| "foreground-not-found".to_string())?;
            let mut response = Vec::new();
            for value in [width, height, bbox[0], bbox[1], bbox[2], bbox[3]] {
                response.extend_from_slice(&(value as u32).to_le_bytes());
            }
            response.extend(
                crispmesh_dense::storage::png_bytes(width, height, 1, &mask)
                    .map_err(|e| e.to_string())?,
            );
            Ok(response)
            })).map_err(|_| "foreground-failed".to_string())?
        })
    }
}

pub fn available() -> bool {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        apple::available()
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        false
    }
}
pub fn preview(bytes: &[u8], focus: f64, threshold: f64) -> Result<Vec<u8>, String> {
    preview_region(bytes, focus, threshold, None)
}
/// An optional normalized search region follows a previously detected object. Output stays in the full photo frame.
pub fn preview_region(
    bytes: &[u8],
    focus: f64,
    threshold: f64,
    region: Option<[f64; 4]>,
) -> Result<Vec<u8>, String> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        apple::preview(bytes, focus, threshold, region)
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        let _ = (bytes, focus, threshold, region);
        Err("foreground-unavailable".into())
    }
}
