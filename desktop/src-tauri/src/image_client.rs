//! Fixed generation JSON client for image generation and single-image edit.
//!
//! Uses the trusted channel to send `POST /v1/images/generations` requests
//! with a fixed JSON contract: `{model, prompt, n:1}` for generation and
//! `{model, prompt, n:1, image: "data:..."}` for edit.

use crate::image_limits::{MAX_PROMPT_BYTES, MAX_RESPONSE_BYTES};
use crate::image_validation::{self, ImageValidationError, ValidatedImage};
use crate::trusted_channel::{TrustedChannel, TrustedChannelError};
use serde::Deserialize;

/// Stable error classification for image generation failures.
/// Messages are sanitized and never contain key, prompt, base64, or upstream body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationError {
    NotReady,
    InvalidPrompt,
    InvalidModel,
    InvalidReferenceImage,
    ChannelError(TrustedChannelError),
    ResponseParseFailed,
    ResponseUrlOnly,
    ResponseEmpty,
    ResponseMultipleImages,
    ResponseUnknownFormat,
    ResponseExceedsLimits,
    ResponseDimensionsUnreadable,
    Cancelled,
    Timeout,
}

impl std::fmt::Display for GenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady => write!(f, "image generation is not ready"),
            Self::InvalidPrompt => write!(f, "prompt is empty or exceeds size limit"),
            Self::InvalidModel => write!(f, "model is not a valid preset"),
            Self::InvalidReferenceImage => write!(f, "reference image failed validation"),
            Self::ChannelError(e) => write!(f, "trusted channel error: {e}"),
            Self::ResponseParseFailed => write!(f, "failed to parse generation response"),
            Self::ResponseUrlOnly => write!(f, "response contains URL only"),
            Self::ResponseEmpty => write!(f, "response image is empty"),
            Self::ResponseMultipleImages => write!(f, "response contains multiple images"),
            Self::ResponseUnknownFormat => write!(f, "response image format is not supported"),
            Self::ResponseExceedsLimits => write!(f, "response image exceeds size or pixel limits"),
            Self::ResponseDimensionsUnreadable => {
                write!(f, "cannot read response image dimensions")
            }
            Self::Cancelled => write!(f, "generation was cancelled"),
            Self::Timeout => write!(f, "generation timed out"),
        }
    }
}

impl std::error::Error for GenerationError {}

impl From<TrustedChannelError> for GenerationError {
    fn from(e: TrustedChannelError) -> Self {
        match e {
            TrustedChannelError::Timeout => Self::Timeout,
            _ => Self::ChannelError(e),
        }
    }
}

impl From<ImageValidationError> for GenerationError {
    fn from(e: ImageValidationError) -> Self {
        match e {
            ImageValidationError::Empty => Self::ResponseEmpty,
            ImageValidationError::UnknownFormat => Self::ResponseUnknownFormat,
            ImageValidationError::Animated => Self::ResponseUnknownFormat,
            ImageValidationError::Base64Invalid => Self::ResponseParseFailed,
            ImageValidationError::ExceedsMaxBytes
            | ImageValidationError::ExceedsMaxDimension
            | ImageValidationError::ExceedsMaxPixels => Self::ResponseExceedsLimits,
            ImageValidationError::DimensionsUnreadable => Self::ResponseDimensionsUnreadable,
        }
    }
}

/// Request for image generation or edit.
#[derive(Clone, Debug)]
pub struct GenerationRequest {
    pub model: String,
    pub prompt: String,
    pub reference_image_data_uri: Option<String>,
}

/// Validated generation result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationResult {
    pub image_bytes: Vec<u8>,
    pub validated: ValidatedImage,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGenerationResponse {
    #[serde(rename = "created")]
    _created: u64,
    #[serde(default)]
    data: Vec<RawImageData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImageData {
    #[serde(default)]
    b64_json: String,
    #[serde(default)]
    url: String,
}

/// Validates a generation request before sending.
pub fn validate_request(req: &GenerationRequest) -> Result<(), GenerationError> {
    let prompt_bytes = req.prompt.as_bytes();
    if req.prompt.trim().is_empty() || prompt_bytes.len() > MAX_PROMPT_BYTES {
        return Err(GenerationError::InvalidPrompt);
    }
    if crate::image_models::find_preset(&req.model).is_none() {
        return Err(GenerationError::InvalidModel);
    }
    if let Some(ref image) = req.reference_image_data_uri {
        let Some((prefix, encoded)) = image.split_once(',') else {
            return Err(GenerationError::InvalidReferenceImage);
        };
        if !matches!(
            prefix,
            "data:image/png;base64" | "data:image/jpeg;base64" | "data:image/webp;base64"
        ) {
            return Err(GenerationError::InvalidReferenceImage);
        }
        image_validation::validate_b64_image(encoded)
            .map_err(|_| GenerationError::InvalidReferenceImage)?;
    }
    Ok(())
}

/// Builds the JSON request body for the generation request.
pub fn build_request_body(req: &GenerationRequest) -> Vec<u8> {
    if let Some(image) = &req.reference_image_data_uri {
        serde_json::to_vec(&serde_json::json!({
            "model": req.model,
            "prompt": req.prompt,
            "n": 1,
            "image": image,
        }))
        .expect("fixed generation request is serializable")
    } else {
        serde_json::to_vec(&serde_json::json!({
            "model": req.model,
            "prompt": req.prompt,
            "n": 1,
        }))
        .expect("fixed generation request is serializable")
    }
}

/// Sends a generation request through the trusted channel and validates the response.
pub async fn execute(
    channel: &mut TrustedChannel,
    api_key: &str,
    req: &GenerationRequest,
) -> Result<GenerationResult, GenerationError> {
    validate_request(req)?;
    let body = build_request_body(req);
    let response = channel.generate(api_key, &body, MAX_RESPONSE_BYTES).await?;
    parse_generation_response(&response)
}

/// Parses and validates a generation response body.
pub fn parse_generation_response(body: &[u8]) -> Result<GenerationResult, GenerationError> {
    let raw: RawGenerationResponse =
        serde_json::from_slice(body).map_err(|_| GenerationError::ResponseParseFailed)?;
    if raw.data.is_empty() {
        return Err(GenerationError::ResponseEmpty);
    }
    if raw.data.len() > 1 {
        return Err(GenerationError::ResponseMultipleImages);
    }
    let item = &raw.data[0];
    if item.b64_json.is_empty() {
        if !item.url.is_empty() {
            return Err(GenerationError::ResponseUrlOnly);
        }
        return Err(GenerationError::ResponseEmpty);
    }
    let (image_bytes, validated) = image_validation::decode_and_validate_b64_image(&item.b64_json)?;
    Ok(GenerationResult {
        image_bytes,
        validated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_png_b64(width: u32, height: u32) -> String {
        let mut data = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R',
        ];
        data.extend(width.to_be_bytes());
        data.extend(height.to_be_bytes());
        data.extend(&[8, 2, 0, 0, 0]);
        base64::engine::general_purpose::STANDARD.encode(&data)
    }

    #[test]
    fn validates_generation_request() {
        let req = GenerationRequest {
            model: "cx/gpt-5.5-image".into(),
            prompt: "a sunset".into(),
            reference_image_data_uri: None,
        };
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn rejects_empty_prompt() {
        let req = GenerationRequest {
            model: "cx/gpt-5.5-image".into(),
            prompt: "".into(),
            reference_image_data_uri: None,
        };
        assert_eq!(validate_request(&req), Err(GenerationError::InvalidPrompt));
    }

    #[test]
    fn rejects_non_preset_model() {
        let req = GenerationRequest {
            model: "cx/gpt-5.6-image".into(),
            prompt: "a sunset".into(),
            reference_image_data_uri: None,
        };
        assert_eq!(validate_request(&req), Err(GenerationError::InvalidModel));
    }

    #[test]
    fn rejects_invalid_reference_image() {
        let req = GenerationRequest {
            model: "cx/gpt-5.5-image".into(),
            prompt: "edit this".into(),
            reference_image_data_uri: Some("not-base64".into()),
        };
        assert_eq!(
            validate_request(&req),
            Err(GenerationError::InvalidReferenceImage)
        );
    }

    #[test]
    fn builds_generation_body_without_image() {
        let req = GenerationRequest {
            model: "cx/gpt-5.5-image".into(),
            prompt: "a cat".into(),
            reference_image_data_uri: None,
        };
        let body = build_request_body(&req);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["model"], "cx/gpt-5.5-image");
        assert_eq!(json["prompt"], "a cat");
        assert_eq!(json["n"], 1);
        assert!(json.get("image").is_none());
    }

    #[test]
    fn builds_edit_body_with_image() {
        let b64 = make_png_b64(2, 2);
        let req = GenerationRequest {
            model: "ag/gemini-3.1-flash-image".into(),
            prompt: "edit this".into(),
            reference_image_data_uri: Some(format!("data:image/png;base64,{b64}")),
        };
        let body = build_request_body(&req);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["model"], "ag/gemini-3.1-flash-image");
        assert_eq!(json["n"], 1);
        assert!(json["image"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn parses_valid_generation_response() {
        let b64 = make_png_b64(2, 2);
        let body = format!(r#"{{"created":1719500000,"data":[{{"b64_json":"{b64}"}}]}}"#);
        let result = parse_generation_response(body.as_bytes()).unwrap();
        assert_eq!(result.validated.width, 2);
        assert_eq!(result.validated.height, 2);
        assert_eq!(
            result.image_bytes,
            base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .unwrap()
        );
    }

    #[test]
    fn maps_only_typed_channel_timeouts_to_generation_timeout() {
        assert_eq!(
            GenerationError::from(TrustedChannelError::Timeout),
            GenerationError::Timeout
        );
        assert_eq!(
            GenerationError::from(TrustedChannelError::ConnectFailed("timeout".into())),
            GenerationError::ChannelError(TrustedChannelError::ConnectFailed("timeout".into()))
        );
    }

    #[test]
    fn rejects_url_only_response() {
        let body = br#"{"created":1,"data":[{"url":"https://example.com/img.png"}]}"#;
        assert_eq!(
            parse_generation_response(body),
            Err(GenerationError::ResponseUrlOnly)
        );
    }

    #[test]
    fn rejects_empty_response() {
        let body = br#"{"created":1,"data":[]}"#;
        assert_eq!(
            parse_generation_response(body),
            Err(GenerationError::ResponseEmpty)
        );
    }

    #[test]
    fn rejects_multiple_images() {
        let b64 = make_png_b64(1, 1);
        let body =
            format!(r#"{{"created":1,"data":[{{"b64_json":"{b64}"}},{{"b64_json":"{b64}"}}]}}"#);
        assert_eq!(
            parse_generation_response(body.as_bytes()),
            Err(GenerationError::ResponseMultipleImages)
        );
    }

    #[test]
    fn sanitizes_prompt_with_quotes() {
        let req = GenerationRequest {
            model: "cx/gpt-5.5-image".into(),
            prompt: r#"a "quote" inside"#.into(),
            reference_image_data_uri: None,
        };
        let body = build_request_body(&req);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["prompt"], r#"a "quote" inside"#);
    }

    #[test]
    fn serializes_all_json_control_characters() {
        let req = GenerationRequest {
            model: "cx/gpt-5.5-image".into(),
            prompt: "before\u{0001}after".into(),
            reference_image_data_uri: None,
        };
        let body = build_request_body(&req);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["prompt"], "before\u{0001}after");
    }
}
