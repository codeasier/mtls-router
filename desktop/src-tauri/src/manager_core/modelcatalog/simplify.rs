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
fn simplify_test_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

#[cfg(test)]
fn lock_simplify_tests() -> std::sync::MutexGuard<'static, ()> {
    simplify_test_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// The stored value is process-wide. Hold this across a read so a parallel
/// `set_simplify_for_test` cannot change it mid-assertion.
#[cfg(test)]
pub fn with_simplify_lock<R>(body: impl FnOnce() -> R) -> R {
    let _lock = lock_simplify_tests();
    body()
}

#[cfg(test)]
pub fn set_simplify_for_test(value: impl Into<String>) -> impl Drop {
    let lock = lock_simplify_tests();
    let mut slot = stored().lock().expect("simplify lock");
    let previous = slot.clone();
    *slot = value.into();
    drop(slot);
    SimplifyGuard {
        _lock: lock,
        previous,
    }
}

#[cfg(test)]
struct SimplifyGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: String,
}

#[cfg(test)]
impl Drop for SimplifyGuard {
    fn drop(&mut self) {
        *stored().lock().expect("simplify lock") = self.previous.clone();
    }
}
