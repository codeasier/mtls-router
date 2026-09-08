use super::error::{SafeKind, WorkbenchError};

pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_DECODED_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_EDGE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 64_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedImage {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

pub fn validate_image(bytes: &[u8]) -> Result<ValidatedImage, WorkbenchError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(WorkbenchError::new(SafeKind::ImageTooLarge));
    }
    let (format, width, height) = dimensions(bytes)?;
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(WorkbenchError::new(SafeKind::ImageInvalid));
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > MAX_PIXELS || pixels.saturating_mul(4) > MAX_DECODED_BYTES {
        return Err(WorkbenchError::new(SafeKind::ImageTooLarge));
    }
    Ok(ValidatedImage {
        format,
        width,
        height,
        bytes: bytes.to_vec(),
    })
}

fn dimensions(bytes: &[u8]) -> Result<(ImageFormat, u32, u32), WorkbenchError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_dimensions(bytes);
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return jpeg_dimensions(bytes);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return webp_dimensions(bytes);
    }
    Err(WorkbenchError::new(SafeKind::ImageInvalid))
}

fn png_dimensions(bytes: &[u8]) -> Result<(ImageFormat, u32, u32), WorkbenchError> {
    if bytes.len() < 33 {
        return Err(WorkbenchError::new(SafeKind::ImageInvalid));
    }
    if &bytes[12..16] != b"IHDR" {
        return Err(WorkbenchError::new(SafeKind::ImageInvalid));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    let mut offset = 8;
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        if kind == b"acTL" {
            return Err(WorkbenchError::new(SafeKind::ImageInvalid));
        }
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        offset = offset.saturating_add(12).saturating_add(length);
    }
    Ok((ImageFormat::Png, width, height))
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(ImageFormat, u32, u32), WorkbenchError> {
    let mut i = 2usize;
    while i + 4 < bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xd8 || marker == 0xd9 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            i += 2;
            continue;
        }
        if i + 4 > bytes.len() {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            break;
        }
        if (0xc0..=0xc3).contains(&marker) && len >= 7 {
            let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Ok((ImageFormat::Jpeg, width, height));
        }
        i += 2 + len;
    }
    Err(WorkbenchError::new(SafeKind::ImageInvalid))
}

fn webp_dimensions(bytes: &[u8]) -> Result<(ImageFormat, u32, u32), WorkbenchError> {
    if bytes.len() < 16 {
        return Err(WorkbenchError::new(SafeKind::ImageInvalid));
    }
    let kind = &bytes[12..16];
    if kind == b"VP8X" {
        if bytes.len() < 30 {
            return Err(WorkbenchError::new(SafeKind::ImageInvalid));
        }
        let flags = bytes[20];
        if flags & 0x02 != 0 {
            return Err(WorkbenchError::new(SafeKind::ImageInvalid));
        }
        let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
        let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
        return Ok((ImageFormat::Webp, width, height));
    }
    if kind == b"VP8 " && bytes.len() >= 30 {
        let width = u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3fff;
        let height = u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3fff;
        return Ok((ImageFormat::Webp, width, height));
    }
    if kind == b"VP8L" && bytes.len() >= 25 {
        let bits = u32::from_le_bytes(bytes[21..25].try_into().unwrap());
        let width = (bits & 0x3fff) + 1;
        let height = ((bits >> 14) & 0x3fff) + 1;
        return Ok((ImageFormat::Webp, width, height));
    }
    Err(WorkbenchError::new(SafeKind::ImageInvalid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_gif_and_oversized_encoded() {
        assert!(validate_image(b"GIF89a....").is_err());
        assert!(validate_image(&vec![0u8; MAX_IMAGE_BYTES + 1]).is_err());
    }

    #[test]
    fn accepts_fixture_png() {
        let png = include_bytes!("../../../../internal/proxy/testdata/generation_binary.png");
        let image = validate_image(png).expect("png");
        assert_eq!(image.format, ImageFormat::Png);
        assert_eq!((image.width, image.height), (1, 1));
    }
}
