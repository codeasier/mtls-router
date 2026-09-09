use super::error::{SafeKind, WorkbenchError};
use super::store::is_sha256;

pub fn asset_id_from_uri(uri: &str) -> Result<String, WorkbenchError> {
    if uri.contains("..") || uri.contains('\\') {
        return Err(WorkbenchError::new(SafeKind::ImageInvalid));
    }
    let path = if let Some(rest) = uri.strip_prefix("image-asset://localhost/") {
        rest
    } else if let Some(rest) = uri.strip_prefix("http://image-asset.localhost/") {
        rest
    } else if let Some(rest) = uri.strip_prefix("https://image-asset.localhost/") {
        rest
    } else {
        return Err(WorkbenchError::new(SafeKind::ImageInvalid));
    };
    let id = path
        .split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("");
    if !is_sha256(id) {
        return Err(WorkbenchError::new(SafeKind::ImageInvalid));
    }
    Ok(id.to_owned())
}

pub fn serve_asset_response(mime: &str, bytes: Vec<u8>) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(200)
        .header(http::header::CONTENT_TYPE, mime)
        .header("X-Content-Type-Options", "nosniff")
        .header(http::header::CACHE_CONTROL, "no-store, private")
        .body(bytes)
        .unwrap_or_else(|_| not_found())
}

pub fn not_found() -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(404)
        .header("X-Content-Type-Options", "nosniff")
        .header(http::header::CACHE_CONTROL, "no-store")
        .body(Vec::new())
        .expect("empty")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_accepts_sha() {
        let id = "a".repeat(64);
        assert!(asset_id_from_uri(&format!("image-asset://localhost/{id}")).is_ok());
        assert!(asset_id_from_uri(&format!("http://image-asset.localhost/{id}")).is_ok());
        assert!(asset_id_from_uri("image-asset://localhost/../secrets").is_err());
        assert!(asset_id_from_uri("image-asset://localhost//tmp/abs").is_err());
        assert!(asset_id_from_uri("image-asset://localhost/not-hex").is_err());
    }
}
