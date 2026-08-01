//! Resource limits for the image data plane.
//!
//! These constants are the single source of truth for prompt, image, response,
//! and pixel bounds. They are published in the UI and docs and must not be
//! adjusted without a visible contract change.

/// Maximum UTF-8 encoded prompt length: 20 KiB.
pub const MAX_PROMPT_BYTES: usize = 20 * 1024;

/// Maximum decoded image bytes for a single reference or generated image: 20 MiB.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// Maximum generation JSON response body: 32 MiB. Reads that exceed this are
/// canceled mid-stream.
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Maximum image dimension on a single edge: 16,384 pixels.
pub const MAX_IMAGE_DIMENSION: u32 = 16_384;

/// Maximum total pixel count: 64 megapixels.
pub const MAX_IMAGE_PIXELS: u64 = 64_000_000;

/// HTTP/1.1 `/version` response body limit: 64 KiB (mirrors Go trustedrouter).
pub const VERSION_BODY_LIMIT: usize = 64 * 1024;

/// Maximum `/v1/models/image` response body: 1 MiB.
pub const CATALOG_BODY_LIMIT: usize = 1024 * 1024;

/// Maximum number of entries in the image catalog: 1000.
pub const CATALOG_MAX_ENTRIES: usize = 1000;

/// Maximum length of a single model ID in the catalog: 256 bytes.
pub const CATALOG_MAX_ID_LEN: usize = 256;

/// Generation request timeout: 180 seconds.
pub const GENERATION_TIMEOUT_SECS: u64 = 180;

/// Connection and handshake timeout for the trusted channel: 10 seconds.
pub const CONNECT_TIMEOUT_SECS: u64 = 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_consistent() {
        assert_eq!(MAX_PROMPT_BYTES, 20_480);
        assert_eq!(MAX_IMAGE_BYTES, 20_971_520);
        assert_eq!(MAX_RESPONSE_BYTES, 33_554_432);
        assert_eq!(MAX_IMAGE_DIMENSION, 16_384);
        assert_eq!(MAX_IMAGE_PIXELS, 64_000_000);
        assert!(MAX_RESPONSE_BYTES > MAX_IMAGE_BYTES);
        assert!(MAX_IMAGE_PIXELS > MAX_IMAGE_DIMENSION as u64);
        assert_eq!(GENERATION_TIMEOUT_SECS, 180);
    }
}
