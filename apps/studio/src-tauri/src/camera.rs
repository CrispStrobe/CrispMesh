//! The field of view of this device's camera, for photos taken in the app.
//!
//! *Take photos* saves frames of the web view's `getUserMedia` stream, which carry no EXIF focal
//! length. Where the system knows the camera's field of view (iOS: `AVCaptureDeviceFormat`'s
//! `videoFieldOfView`), the run can use it as the lens (`--calibration fov:DEG`) instead of
//! finding the focal length from the turn. Only device and format discovery is used: no camera
//! permission is needed and no capture session is started. Everywhere else, and when no format
//! fits the frame, the answer is `None`.

/// Horizontal field of view in degrees across a `width` x `height` frame, given the camera's
/// formats as (width, height, horizontal field of view across the format's width). A format of
/// exactly that size (either way round) wins; else one of the same aspect ratio, nearest in size.
/// When the frame is turned against the format (portrait frame, landscape format), the field of
/// view across the frame's width is the one across the format's height.
pub fn field_of_view_for(width: u32, height: u32, formats: &[(u32, u32, f64)]) -> Option<f64> {
    if width == 0 || height == 0 {
        return None;
    }
    let (long, short) = (width.max(height) as f64, width.min(height) as f64);
    let usable = formats.iter().filter(|(w, h, fov)| *w > 0 && *h > 0 && fov.is_finite() && *fov > 1.0 && *fov < 179.0);
    let exact = usable.clone().find(|(w, h, _)| (*w, *h) == (width, height) || (*h, *w) == (width, height));
    let (w, h, fov) = *exact.or_else(|| {
        usable
            .filter(|(w, h, _)| {
                let (fl, fs) = ((*w).max(*h) as f64, (*w).min(*h) as f64);
                (fl / fs / (long / short) - 1.0).abs() < 0.01
            })
            .min_by(|a, b| {
                let gap = |f: &(u32, u32, f64)| ((f.0.max(f.1) as f64) - long).abs();
                gap(a).total_cmp(&gap(b))
            })
    })?;
    // The field of view across the format's width; the frame's width is its long or short side.
    let half = (fov.to_radians() / 2.0).tan();
    let across_long = if w >= h { half } else { half * h as f64 / w as f64 };
    let across_width = if width >= height { across_long } else { across_long * short / long };
    Some((2.0 * across_width.atan()).to_degrees())
}

/// The formats of the camera `getUserMedia` most likely opens: on iOS the back built-in wide-angle
/// camera (`facingMode: environment`), elsewhere the default video device. Empty without a camera.
#[cfg(any(target_os = "ios", target_os = "macos"))]
pub fn camera_formats() -> Vec<(u32, u32, f64)> {
    use objc2::encode::{Encoding, RefEncode};
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, sel};

    #[repr(C)]
    struct OpaqueFormatDescription {
        _private: [u8; 0],
    }
    // SAFETY: CMFormatDescriptionRef is a pointer to this opaque struct.
    unsafe impl RefEncode for OpaqueFormatDescription {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("opaqueCMFormatDescription", &[]));
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Dimensions {
        width: i32,
        height: i32,
    }
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeVideo: &'static AnyObject;
        #[cfg(target_os = "ios")]
        static AVCaptureDeviceTypeBuiltInWideAngleCamera: &'static AnyObject;
    }
    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        fn CMVideoFormatDescriptionGetDimensions(description: *const OpaqueFormatDescription) -> Dimensions;
    }

    autoreleasepool(|_| {
        let Some(class) = AnyClass::get(c"AVCaptureDevice") else {
            return Vec::new();
        };
        // SAFETY: documented AVCaptureDevice class methods; the statics are AVFoundation's constants.
        let device: Option<Retained<AnyObject>> = unsafe {
            #[cfg(target_os = "ios")]
            let back: Option<Retained<AnyObject>> = msg_send![
                class,
                defaultDeviceWithDeviceType: AVCaptureDeviceTypeBuiltInWideAngleCamera,
                mediaType: AVMediaTypeVideo,
                position: 1isize
            ];
            #[cfg(not(target_os = "ios"))]
            let back: Option<Retained<AnyObject>> = None;
            back.or_else(|| msg_send![class, defaultDeviceWithMediaType: AVMediaTypeVideo])
        };
        let Some(device) = device else {
            return Vec::new();
        };
        // SAFETY: `formats` is an NSArray of AVCaptureDeviceFormat; each call below is a documented
        // getter. `videoFieldOfView` is not declared on macOS, hence the check that it responds.
        unsafe {
            let formats: Option<Retained<AnyObject>> = msg_send![&*device, formats];
            let Some(formats) = formats else {
                return Vec::new();
            };
            let count: usize = msg_send![&*formats, count];
            let mut found = Vec::with_capacity(count);
            for index in 0..count {
                let format: Retained<AnyObject> = msg_send![&*formats, objectAtIndex: index];
                let answers: bool = msg_send![&*format, respondsToSelector: sel!(videoFieldOfView)];
                if !answers {
                    continue;
                }
                let fov: f32 = msg_send![&*format, videoFieldOfView];
                let description: *const OpaqueFormatDescription = msg_send![&*format, formatDescription];
                if description.is_null() {
                    continue;
                }
                let size = CMVideoFormatDescriptionGetDimensions(description);
                if size.width > 0 && size.height > 0 {
                    found.push((size.width as u32, size.height as u32, fov as f64));
                }
            }
            found
        }
    })
}

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
pub fn camera_formats() -> Vec<(u32, u32, f64)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_format_of_the_frame() {
        let formats = [(1920, 1080, 61.0), (4032, 3024, 69.4), (1920, 1440, 69.0), (640, 480, 0.0)];
        assert!((field_of_view_for(4032, 3024, &formats).unwrap() - 69.4).abs() < 1e-9);
        // Same aspect ratio, no exact size: the nearest in size.
        assert!((field_of_view_for(2016, 1512, &formats).unwrap() - 69.0).abs() < 1e-9);
        assert!((field_of_view_for(1280, 720, &formats).unwrap() - 61.0).abs() < 1e-9);
        // A portrait frame of a landscape format: the field of view across the format's height.
        let portrait = field_of_view_for(3024, 4032, &formats).unwrap();
        let expected = 2.0 * ((69.4f64.to_radians() / 2.0).tan() * 3024.0 / 4032.0).atan().to_degrees();
        assert!((portrait - expected).abs() < 1e-9, "{portrait} {expected}");
        // The exact format's field of view is unknown (0): another of the same shape answers.
        assert!((field_of_view_for(640, 480, &formats).unwrap() - 69.0).abs() < 1e-9);
        // No format of that shape, only unknown fields of view (this Mac) or no camera: no answer.
        assert_eq!(field_of_view_for(1000, 1000, &formats), None);
        assert_eq!(field_of_view_for(1280, 720, &[(1280, 720, 0.0), (640, 480, 0.0)]), None);
        assert_eq!(field_of_view_for(640, 480, &[]), None);
        assert_eq!(field_of_view_for(0, 480, &formats), None);
    }

    /// Asks this computer's camera and prints the answer. Must not crash without a camera. On the
    /// M1 Mac this was written on, the built-in camera's formats answer videoFieldOfView (not
    /// declared for macOS in the SDK) with 0, "unknown", so the answer is None there.
    #[test]
    fn this_device() {
        let formats = camera_formats();
        println!("camera formats with a field of view: {formats:?}");
        println!("1920x1080: {:?}", field_of_view_for(1920, 1080, &formats));
    }
}
