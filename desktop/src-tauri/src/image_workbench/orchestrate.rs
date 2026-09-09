use serde_json::{json, Value};
use unicode_segmentation::UnicodeSegmentation;

use super::error::{SafeKind, WorkbenchError};

pub const MAX_TEXT_BYTES: usize = 20 * 1024;
pub const MAX_CHAT_JSON_BYTES: usize = 256 * 1024;
pub const TITLE_GRAPHEMES: usize = 18;
pub const EMPTY_REF_ZH: &str = "根据这张参考图继续改。";
pub const EMPTY_REF_EN: &str = "Continue editing from this reference image.";
pub const SYSTEM_PROMPT: &str = "你是对话生图助手。既能闲聊，也能出图。\n\n规则：\n1. 用一两句简洁中文回复。可见文字里不要写英文画面描述，不要复述 prompt，不要贴 JSON。\n2. 需要出图或改图时，只在回复最末尾输出：\n```image\n{\"action\":\"generate\",\"prompt\":\"完整画面描述\",\"size\":\"1024x1024\"}\n```\naction 只能是 generate 或 edit。改图用 edit，并写出完整新 prompt。\n3. prompt 只放在上面的 JSON 里，要具体：主体、构图、光线、材质、色彩、风格。\n4. 只聊天、不需要图时，禁止输出 image 块。\n5. 用户附带了参考图时，必须用 action=edit，并基于参考图写完整新 prompt。\n6. 不要使用 markdown 图片或外链。不要解释这个协议。";

const IMAGE_HINT: &str = "画|绘|生成|出一张|做一张|来一张|重画|改成|改一下|换成|imagine|draw|generate|render|paint|illustration";
const EDIT_HINT_ZH: &str = "改成|改一下|换成|重画成|变成";
const EDIT_HINT_EN: &[&str] = &[
    "change it to",
    "make it",
    "turn it into",
    "replace with",
    "redo as",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageSpec {
    pub action: String,
    pub prompt: String,
    pub size: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobDecision {
    pub action: String,
    pub prompt: String,
    pub size: String,
    pub reference_asset_id: Option<String>,
    pub from_model: bool,
}

#[derive(Clone, Debug)]
pub struct HistoryTurn {
    pub role: String,
    pub visible_text: String,
    pub generated_prompts: Vec<String>,
    pub reference_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RebuildInput<'a> {
    pub history: &'a [HistoryTurn],
    pub last_image_prompt: Option<&'a str>,
    pub force_image: bool,
    pub explicit_reference: bool,
    pub size: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Extracted {
    pub display: String,
    pub specs: Vec<ImageSpec>,
}

pub fn fallback_edit_text(english: bool) -> &'static str {
    if english {
        EMPTY_REF_EN
    } else {
        EMPTY_REF_ZH
    }
}

pub fn title_from(text: &str) -> String {
    text.graphemes(true).take(TITLE_GRAPHEMES).collect()
}

pub fn bounded_text(text: &str) -> Result<(), WorkbenchError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(WorkbenchError::new(SafeKind::InputTooLarge));
    }
    Ok(())
}

pub fn looks_like_image_request(text: &str) -> bool {
    contains_any(text, IMAGE_HINT.split('|'))
}

pub fn looks_like_implicit_edit(text: &str, english: bool) -> bool {
    if contains_any(text, EDIT_HINT_ZH.split('|')) {
        return true;
    }
    if english {
        let lower = text.to_ascii_lowercase();
        return EDIT_HINT_EN.iter().any(|hint| lower.contains(hint));
    }
    false
}

pub fn extract_image_payload(raw: &str, default_size: &str) -> Extracted {
    let mut specs = Vec::new();
    let mut display = raw.to_owned();
    display = replace_fences(&display, default_size, &mut specs);
    display = strip_open_fence(&display, default_size, &mut specs);
    let mut cursor = 0usize;
    let mut cleaned = String::new();
    let mut stripped = false;
    for (start, end, text) in find_json_objects(&display) {
        cleaned.push_str(&display[cursor..start]);
        cursor = end;
        if let Some(spec) = coerce_spec(&text, default_size) {
            if !spec.prompt.is_empty() {
                specs.push(spec);
                stripped = true;
                continue;
            }
        }
        cleaned.push_str(&text);
    }
    cleaned.push_str(&display[cursor..]);
    if stripped {
        display = cleaned;
    }
    display = strip_prompt_echo(&display, &specs);
    Extracted { display, specs }
}

pub fn rebuild_chat_messages(input: RebuildInput<'_>) -> Result<Vec<Value>, WorkbenchError> {
    let mut messages = vec![json!({"role":"system","content": SYSTEM_PROMPT})];
    if let Some(prompt) = input.last_image_prompt {
        messages.push(json!({
            "role": "system",
            "content": format!("上一张已生成的画面描述：{prompt}。用户若改图，action 用 edit，并写出完整新 prompt。")
        }));
    }
    for turn in input.history {
        if turn.visible_text.is_empty()
            && turn.generated_prompts.is_empty()
            && turn.reference_hint.is_none()
        {
            continue;
        }
        let mut content = turn.visible_text.clone();
        if !turn.generated_prompts.is_empty() {
            content.push_str("\n\n[已生成图片] ");
            content.push_str(&turn.generated_prompts.join(" / "));
        }
        if let Some(hint) = &turn.reference_hint {
            if hint.is_empty() {
                content.push_str("\n\n[用户附带了参考图。改图必须 action=edit]");
            } else {
                content.push_str(&format!(
                    "\n\n[用户附带了参考图：{hint}。改图必须 action=edit]"
                ));
            }
        }
        messages.push(json!({"role": turn.role, "content": content}));
    }
    if input.force_image && !input.explicit_reference {
        messages.push(json!({
            "role": "system",
            "content": "用户要求直接出图。必须输出 image 块，action=generate。先用一句话确认。"
        }));
    }
    if input.explicit_reference {
        messages.push(json!({
            "role": "system",
            "content": "用户附带了参考图。必须输出 image 块，action=edit。先用一句话确认。"
        }));
    }
    fit_messages(messages)
}

pub fn decide_jobs(
    specs: &[ImageSpec],
    user_text: &str,
    force_image: bool,
    explicit_ref: Option<&str>,
    last_image: Option<&str>,
    default_size: &str,
    english: bool,
) -> Result<Vec<JobDecision>, WorkbenchError> {
    let mut jobs: Vec<JobDecision> = specs
        .iter()
        .filter(|spec| !spec.prompt.is_empty())
        .map(|spec| JobDecision {
            action: spec.action.clone(),
            prompt: spec.prompt.clone(),
            size: if spec.size.is_empty() {
                default_size.to_owned()
            } else {
                spec.size.clone()
            },
            reference_asset_id: None,
            from_model: true,
        })
        .collect();
    if jobs.is_empty()
        && (force_image || explicit_ref.is_some() || looks_like_image_request(user_text))
    {
        let implicit_edit = !force_image
            && explicit_ref.is_none()
            && last_image.is_some()
            && looks_like_implicit_edit(user_text, english);
        jobs.push(JobDecision {
            action: if implicit_edit { "edit" } else { "generate" }.to_owned(),
            prompt: user_text.to_owned(),
            size: default_size.to_owned(),
            reference_asset_id: None,
            from_model: false,
        });
    }
    if let Some(asset) = explicit_ref {
        for job in &mut jobs {
            job.action = "edit".to_owned();
            job.reference_asset_id = Some(asset.to_owned());
        }
    } else {
        for job in &mut jobs {
            if job.action == "edit" {
                if let Some(last) = last_image {
                    job.reference_asset_id = Some(last.to_owned());
                } else {
                    return Err(WorkbenchError::new(SafeKind::NoReference));
                }
            }
        }
    }
    for job in &jobs {
        bounded_text(&job.prompt)?;
    }
    Ok(jobs)
}

fn fit_messages(mut messages: Vec<Value>) -> Result<Vec<Value>, WorkbenchError> {
    loop {
        let encoded = serde_json::to_vec(&messages).unwrap_or_default();
        if encoded.len() <= MAX_CHAT_JSON_BYTES {
            return Ok(messages);
        }
        let Some(index) = messages.iter().position(|item| item["role"] != "system") else {
            return Err(WorkbenchError::new(SafeKind::InputTooLarge));
        };
        messages.remove(index);
    }
}

fn replace_fences(display: &str, default_size: &str, specs: &mut Vec<ImageSpec>) -> String {
    let mut out = String::new();
    let lower = display.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find("```") {
        let start = cursor + rel;
        out.push_str(&display[cursor..start]);
        let after = &display[start + 3..];
        let tag_end = after.find('\n').unwrap_or(after.len());
        let tag = after[..tag_end].trim().to_ascii_lowercase();
        if !tag.is_empty() && tag != "image" && tag != "json" && tag != "jsonc" {
            out.push_str(&display[start..start + 3]);
            cursor = start + 3;
            continue;
        }
        let rest = &after[tag_end..];
        if let Some(close_rel) = rest.find("```") {
            let body = rest[..close_rel].trim();
            if let Some(spec) = coerce_spec(body, default_size) {
                if !spec.prompt.is_empty() {
                    specs.push(spec);
                }
            }
            cursor = start + 3 + tag_end + close_rel + 3;
            out.push('\n');
        } else {
            break;
        }
    }
    if cursor < display.len() && !out.is_empty() {
        // closed-fence loop ended without consuming an open tail
    }
    if cursor < display.len() {
        out.push_str(&display[cursor..]);
    }
    if out.is_empty() {
        display.to_owned()
    } else {
        out
    }
}

fn strip_open_fence(display: &str, default_size: &str, specs: &mut Vec<ImageSpec>) -> String {
    let lower = display.to_ascii_lowercase();
    if let Some(start) = lower.rfind("```") {
        let after = display[start + 3..].trim_start();
        let body = after
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .trim_start_matches(|c: char| c == '\n' || c == '\r');
        if let Some(spec) = coerce_spec(body, default_size) {
            if !spec.prompt.is_empty() {
                specs.push(spec);
                return display[..start].to_owned();
            }
        }
        return display[..start].to_owned();
    }
    display.to_owned()
}

fn coerce_spec(text: &str, default_size: &str) -> Option<ImageSpec> {
    let trimmed = text.trim();
    let parsed = serde_json::from_str::<Value>(trimmed).ok().or_else(|| {
        let loose = regex_lite_trailing_comma(trimmed);
        serde_json::from_str(&loose).ok()
    })?;
    if !parsed.get("prompt").is_some() && !parsed.get("action").is_some() {
        return None;
    }
    let action = if parsed.get("action").and_then(Value::as_str) == Some("edit") {
        "edit"
    } else {
        "generate"
    };
    Some(ImageSpec {
        action: action.to_owned(),
        prompt: parsed
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned(),
        size: parsed
            .get("size")
            .and_then(Value::as_str)
            .unwrap_or(default_size)
            .to_owned(),
    })
}

fn regex_lite_trailing_comma(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_json_objects(text: &str) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let mut depth = 0i32;
        let mut in_str = false;
        let mut esc = false;
        let mut j = i;
        while j < bytes.len() {
            let c = bytes[j];
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
            } else if c == b'"' {
                in_str = true;
            } else if c == b'{' {
                depth += 1;
            } else if c == b'}' {
                depth -= 1;
                if depth == 0 {
                    out.push((i, j + 1, text[i..j + 1].to_owned()));
                    i = j;
                    break;
                }
            }
            j += 1;
        }
        i += 1;
    }
    out
}

fn strip_prompt_echo(display: &str, specs: &[ImageSpec]) -> String {
    let mut text = display.to_owned();
    for spec in specs {
        if spec.prompt.len() < 12 {
            continue;
        }
        text = text.replace(&spec.prompt, "");
        let head: String = spec.prompt.chars().take(36).collect();
        text = text
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return true;
                }
                if trimmed.contains(&head)
                    || head.contains(&trimmed.chars().take(36).collect::<String>())
                {
                    return false;
                }
                let letters = trimmed.chars().filter(|c| c.is_ascii_alphabetic()).count();
                !(trimmed.len() > 36 && letters * 100 / trimmed.len() > 55)
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let collapsed = text.replace("\n\n\n", "\n\n");
    collapsed.trim().to_owned()
}

fn contains_any<'a>(text: &str, mut needles: impl Iterator<Item = &'a str>) -> bool {
    let lower = text.to_ascii_lowercase();
    needles.any(|needle| {
        if needle.chars().all(|c| c.is_ascii()) {
            lower.contains(&needle.to_ascii_lowercase())
        } else {
            text.contains(needle)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_image_fence_and_prompt_echo() {
        let raw = "好的，这就出图。\n```image\n{\"action\":\"generate\",\"prompt\":\"a red cube on a wooden table, soft daylight\",\"size\":\"1024x1024\"}\n```\na red cube on a wooden table, soft daylight";
        let extracted = extract_image_payload(raw, "1024x1024");
        assert_eq!(extracted.specs.len(), 1);
        assert_eq!(extracted.specs[0].action, "generate");
        assert!(!extracted.display.contains("```"));
        assert!(!extracted.display.contains("a red cube"));
        assert!(extracted.display.contains("好的"));
    }

    #[test]
    fn decide_matrix_matches_spec() {
        let generate = ImageSpec {
            action: "generate".into(),
            prompt: "a lamp".into(),
            size: "1024x1024".into(),
        };
        let explicit = decide_jobs(
            &[generate.clone()],
            "改成蓝色",
            false,
            Some("asset-a"),
            Some("asset-b"),
            "1024x1024",
            false,
        )
        .unwrap();
        assert_eq!(explicit[0].action, "edit");
        assert_eq!(explicit[0].reference_asset_id.as_deref(), Some("asset-a"));

        let implicit = decide_jobs(
            &[],
            "改成蓝色的天空",
            false,
            None,
            Some("asset-b"),
            "1024x1024",
            false,
        )
        .unwrap();
        assert_eq!(implicit[0].action, "edit");
        assert_eq!(implicit[0].reference_asset_id.as_deref(), Some("asset-b"));

        let force = decide_jobs(
            &[],
            "随便画一个",
            true,
            None,
            Some("asset-b"),
            "1024x1024",
            false,
        )
        .unwrap();
        assert_eq!(force[0].action, "generate");
        assert_eq!(force[0].reference_asset_id, None);

        let missing = decide_jobs(
            &[ImageSpec {
                action: "edit".into(),
                prompt: "make it gold".into(),
                size: "1024x1024".into(),
            }],
            "hi",
            false,
            None,
            None,
            "1024x1024",
            true,
        );
        assert!(missing.is_err());
    }

    #[test]
    fn rebuild_includes_last_image_and_edit_requirement() {
        let history = [HistoryTurn {
            role: "user".into(),
            visible_text: "画一只猫".into(),
            generated_prompts: vec![],
            reference_hint: Some("旧图".into()),
        }];
        let messages = rebuild_chat_messages(RebuildInput {
            history: &history,
            last_image_prompt: Some("a sitting cat"),
            force_image: true,
            explicit_reference: true,
            size: "1024x1024",
        })
        .unwrap();
        let encoded = serde_json::to_string(&messages).unwrap();
        assert!(encoded.contains("上一张已生成的画面描述：a sitting cat"));
        assert!(encoded.contains("改图必须 action=edit"));
        assert!(encoded.contains("必须输出 image 块，action=edit"));
        assert!(!encoded.contains("sk-"));
    }

    #[test]
    fn title_is_local_grapheme_truncation() {
        assert_eq!(
            title_from("一二三四五六七八九十abcdefghij"),
            "一二三四五六七八九十abcdefgh"
        );
    }
}
