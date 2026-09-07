use std::sync::{Mutex, OnceLock};

pub const SIMPLIFY_DEFAULT: &str = "True";

fn stored() -> &'static Mutex<String> {
    static VALUE: OnceLock<Mutex<String>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(SIMPLIFY_DEFAULT.to_owned()))
}

pub fn parse_simplify() -> Result<bool, &'static str> {
    let value = stored().lock().expect("simplify lock").clone();
    parse_simplify_value(&value)
}

pub fn parse_simplify_value(value: &str) -> Result<bool, &'static str> {
    if value.bytes().any(|byte| byte > 0x7f) {
        return Err("invalid embedded simplify value");
    }
    if value.eq_ignore_ascii_case("true") {
        return Ok(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return Ok(false);
    }
    Err("invalid embedded simplify value")
}

#[cfg(test)]
pub fn set_simplify_for_test(value: impl Into<String>) -> impl Drop {
    let previous = stored().lock().expect("simplify lock").clone();
    *stored().lock().expect("simplify lock") = value.into();
    SimplifyGuard { previous }
}

#[cfg(test)]
struct SimplifyGuard {
    previous: String,
}

#[cfg(test)]
impl Drop for SimplifyGuard {
    fn drop(&mut self) {
        *stored().lock().expect("simplify lock") = self.previous.clone();
    }
}
