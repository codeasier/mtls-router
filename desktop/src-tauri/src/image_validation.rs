//! Bounded base64 decoding and PNG/JPEG/WebP magic-byte, format, byte, and
//! pixel validation. Rejects URL-only, empty, multi-image, and unknown
//! responses.

use crate::image_limits::{MAX_IMAGE_BYTES, MAX_IMAGE_DIMENSION, MAX_IMAGE_PIXELS};
use base64::Engine;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedImage {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageValidationError {
    Empty,
    Animated,
    UnknownFormat,
    Base64Invalid,
    ExceedsMaxBytes,
    ExceedsMaxDimension,
    ExceedsMaxPixels,
    DimensionsUnreadable,
}

impl std::fmt::Display for ImageValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "image data is empty"),
            Self::Animated => write!(f, "animated images are not supported"),
            Self::UnknownFormat => write!(f, "image format is not PNG, JPEG, or WebP"),
            Self::Base64Invalid => write!(f, "base64 data is invalid"),
            Self::ExceedsMaxBytes => write!(f, "decoded image exceeds maximum byte limit"),
            Self::ExceedsMaxDimension => write!(f, "image dimension exceeds maximum"),
            Self::ExceedsMaxPixels => write!(f, "image pixel count exceeds maximum"),
            Self::DimensionsUnreadable => write!(f, "cannot read image dimensions"),
        }
    }
}

impl std::error::Error for ImageValidationError {}

/// Validates a single `b64_json` field from a generation response.
/// Returns the decoded bytes, format, and dimensions.
pub fn validate_b64_image(b64: &str) -> Result<ValidatedImage, ImageValidationError> {
    if b64.is_empty() {
        return Err(ImageValidationError::Empty);
    }
    let padding = b64
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .take(2)
        .count();
    let decoded_upper_bound = b64
        .len()
        .div_ceil(4)
        .saturating_mul(3)
        .saturating_sub(padding);
    if decoded_upper_bound > MAX_IMAGE_BYTES {
        return Err(ImageValidationError::ExceedsMaxBytes);
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| ImageValidationError::Base64Invalid)?;
    if decoded.len() > MAX_IMAGE_BYTES {
        return Err(ImageValidationError::ExceedsMaxBytes);
    }
    validate_image_bytes(&decoded)
}

/// Validates raw image bytes: magic bytes, format, dimensions, and size.
pub fn validate_image_bytes(data: &[u8]) -> Result<ValidatedImage, ImageValidationError> {
    if data.is_empty() {
        return Err(ImageValidationError::Empty);
    }
    if data.len() > MAX_IMAGE_BYTES {
        return Err(ImageValidationError::ExceedsMaxBytes);
    }
    let format = detect_format(data)?;
    reject_animation(data, &format)?;
    let (width, height) = read_dimensions(data, &format)?;
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(ImageValidationError::ExceedsMaxDimension);
    }
    let pixels = width as u64 * height as u64;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(ImageValidationError::ExceedsMaxPixels);
    }
    Ok(ValidatedImage {
        format,
        width,
        height,
        byte_len: data.len(),
    })
}

fn reject_animation(data: &[u8], format: &ImageFormat) -> Result<(), ImageValidationError> {
    match format {
        ImageFormat::Png => {
            let mut offset = 8_usize;
            while offset.checked_add(12).is_some_and(|end| end <= data.len()) {
                let length = u32::from_be_bytes([
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ]) as usize;
                if &data[offset + 4..offset + 8] == b"acTL" {
                    return Err(ImageValidationError::Animated);
                }
                let Some(next) = offset
                    .checked_add(12)
                    .and_then(|value| value.checked_add(length))
                else {
                    break;
                };
                if next > data.len() {
                    break;
                }
                offset = next;
            }
        }
        ImageFormat::Webp => {
            if data.get(12..16) == Some(b"VP8X")
                && data.get(20).is_some_and(|flags| flags & 0x02 != 0)
            {
                return Err(ImageValidationError::Animated);
            }
        }
        ImageFormat::Jpeg => {}
    }
    Ok(())
}

fn detect_format(data: &[u8]) -> Result<ImageFormat, ImageValidationError> {
    if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        return Ok(ImageFormat::Png);
    }
    if data.len() >= 3 && data[0] == 0xff && data[1] == 0xd8 && data[2] == 0xff {
        return Ok(ImageFormat::Jpeg);
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Ok(ImageFormat::Webp);
    }
    Err(ImageValidationError::UnknownFormat)
}

fn read_dimensions(data: &[u8], format: &ImageFormat) -> Result<(u32, u32), ImageValidationError> {
    match format {
        ImageFormat::Png => read_png_dimensions(data),
        ImageFormat::Jpeg => read_jpeg_dimensions(data),
        ImageFormat::Webp => read_webp_dimensions(data),
    }
}

fn read_png_dimensions(data: &[u8]) -> Result<(u32, u32), ImageValidationError> {
    if data.len() < 24 {
        return Err(ImageValidationError::DimensionsUnreadable);
    }
    // IHDR chunk starts at byte 8, length(4) + type(4) + width(4) + height(4)
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    if width == 0 || height == 0 {
        return Err(ImageValidationError::DimensionsUnreadable);
    }
    Ok((width, height))
}

fn read_jpeg_dimensions(data: &[u8]) -> Result<(u32, u32), ImageValidationError> {
    let mut i = 2; // skip SOI marker (0xFF 0xD8)
    while i + 1 < data.len() {
        if data[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        i += 2;
        // SOF markers: 0xC0-0xCF (except 0xC4, 0xC8, 0xCC)
        if (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc {
            if i + 7 > data.len() {
                return Err(ImageValidationError::DimensionsUnreadable);
            }
            let height = u16::from_be_bytes([data[i + 3], data[i + 4]]) as u32;
            let width = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            if width == 0 || height == 0 {
                return Err(ImageValidationError::DimensionsUnreadable);
            }
            return Ok((width, height));
        }
        if marker == 0xd8 || marker == 0xd9 {
            continue;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if i + 1 >= data.len() {
            return Err(ImageValidationError::DimensionsUnreadable);
        }
        let seg_len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        i += seg_len;
    }
    Err(ImageValidationError::DimensionsUnreadable)
}

fn read_webp_dimensions(data: &[u8]) -> Result<(u32, u32), ImageValidationError> {
    if data.len() < 30 {
        return Err(ImageValidationError::DimensionsUnreadable);
    }
    let fourcc = &data[12..16];
    match fourcc {
        b"VP8 " => {
            // Lossy VP8
            let width = u16::from_le_bytes([data[26], data[27]]) as u32 & 0x3fff;
            let height = u16::from_le_bytes([data[28], data[29]]) as u32 & 0x3fff;
            if width == 0 || height == 0 {
                return Err(ImageValidationError::DimensionsUnreadable);
            }
            Ok((width, height))
        }
        b"VP8L" => {
            // Lossless VP8L
            if data.len() < 25 {
                return Err(ImageValidationError::DimensionsUnreadable);
            }
            // VP8L signature byte at data[21] should be 0x2f
            let bits = u32::from_le_bytes([data[22], data[23], data[24], {
                if data.len() > 25 {
                    data[25]
                } else {
                    0
                }
            }]);
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            if width == 0 || height == 0 {
                return Err(ImageValidationError::DimensionsUnreadable);
            }
            Ok((width, height))
        }
        b"VP8X" => {
            // Extended VP8X
            let width = 1 + u32::from_le_bytes([data[24], data[25], data[26], 0]);
            let height = 1 + u32::from_le_bytes([data[27], data[28], data[29], 0]);
            if width == 0 || height == 0 {
                return Err(ImageValidationError::DimensionsUnreadable);
            }
            Ok((width, height))
        }
        _ => Err(ImageValidationError::DimensionsUnreadable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_minimal_png(width: u32, height: u32) -> Vec<u8> {
        let mut data = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // PNG signature
            0x00, 0x00, 0x00, 0x0d, // IHDR length
            b'I', b'H', b'D', b'R', // IHDR type
        ];
        data.extend(width.to_be_bytes());
        data.extend(height.to_be_bytes());
        data.push(8); // bit depth
        data.push(2); // color type (RGB)
        data.push(0); // compression
        data.push(0); // filter
        data.push(0); // interlace
        data
    }

    #[test]
    fn validates_png_magic_and_dimensions() {
        let png = make_minimal_png(4, 4);
        let result = validate_image_bytes(&png).unwrap();
        assert_eq!(result.format, ImageFormat::Png);
        assert_eq!(result.width, 4);
        assert_eq!(result.height, 4);
    }

    #[test]
    fn rejects_empty_data() {
        assert_eq!(validate_image_bytes(b""), Err(ImageValidationError::Empty));
    }

    #[test]
    fn rejects_unknown_format() {
        assert_eq!(
            validate_image_bytes(b"not an image"),
            Err(ImageValidationError::UnknownFormat)
        );
    }

    #[test]
    fn rejects_apng_animation_control_chunk() {
        let mut png = make_minimal_png(4, 4);
        png.extend([0; 4]);
        png.extend([0, 0, 0, 8, b'a', b'c', b'T', b'L']);
        png.extend([0; 12]);
        assert_eq!(
            validate_image_bytes(&png),
            Err(ImageValidationError::Animated)
        );
    }

    #[test]
    fn validates_b64_image_roundtrip() {
        let png = make_minimal_png(2, 2);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let result = validate_b64_image(&b64).unwrap();
        assert_eq!(result.format, ImageFormat::Png);
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
        assert_eq!(result.byte_len, png.len());
    }

    #[test]
    fn rejects_empty_b64() {
        assert_eq!(validate_b64_image(""), Err(ImageValidationError::Empty));
    }

    #[test]
    fn rejects_invalid_base64() {
        assert_eq!(
            validate_b64_image("!!!not-base64!!!"),
            Err(ImageValidationError::Base64Invalid)
        );
    }

    #[test]
    fn rejects_dimensions_over_limit() {
        let png = make_minimal_png(MAX_IMAGE_DIMENSION + 1, 1);
        assert_eq!(
            validate_image_bytes(&png),
            Err(ImageValidationError::ExceedsMaxDimension)
        );
    }

    #[test]
    fn detects_jpeg_format() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
        let result = validate_image_bytes(&jpeg);
        assert!(matches!(
            result,
            Err(ImageValidationError::DimensionsUnreadable)
        ));
    }

    #[test]
    fn detects_webp_format() {
        let mut webp = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        webp.extend(b"VP8 X\x00\x00\x00\x00");
        let result = validate_image_bytes(&webp);
        // VP8 X with space is not a valid fourcc, should fail dimensions
        assert!(matches!(
            result,
            Err(ImageValidationError::DimensionsUnreadable)
        ));
    }

    #[test]
    fn fixture_png_validates() {
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEElEQVR4nGP4z8AARwzEcQCukw/x0F8jngAAAABJRU5ErkJggg==";
        let result = validate_b64_image(b64).unwrap();
        assert_eq!(result.format, ImageFormat::Png);
        assert_eq!(result.width, 4);
        assert_eq!(result.height, 4);
    }
}
