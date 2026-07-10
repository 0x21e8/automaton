use std::future::Future;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

pub(crate) fn normalize_evm_address(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    let valid = trimmed.len() == 42
        && trimmed.starts_with("0x")
        && trimmed
            .as_bytes()
            .iter()
            .skip(2)
            .all(|byte| byte.is_ascii_hexdigit());
    if !valid {
        return Err("address must be a 0x-prefixed 20-byte hex string".to_string());
    }
    Ok(trimmed)
}

pub(crate) fn normalize_hex_blob(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    let without_prefix = trimmed
        .strip_prefix("0x")
        .ok_or_else(|| format!("{field} must be 0x-prefixed hex"))?;
    if without_prefix.len() % 2 != 0 {
        return Err(format!("{field} hex length must be even"));
    }
    if !without_prefix
        .as_bytes()
        .iter()
        .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("{field} must be valid hex"));
    }
    Ok(trimmed)
}

pub(crate) fn normalize_selector_hex(raw: &str) -> Result<String, String> {
    let compact = raw.trim().to_ascii_lowercase();
    let normalized = compact.strip_prefix("0x").unwrap_or(&compact);
    if normalized.len() != 8 {
        return Err("selector must be exactly 4 bytes hex".to_string());
    }
    if !normalized
        .as_bytes()
        .iter()
        .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("selector must be valid hex".to_string());
    }
    Ok(format!("0x{normalized}"))
}

pub(crate) fn block_on_with_spin<F: Future>(future: F) -> F::Output {
    unsafe fn clone(_ptr: *const ()) -> RawWaker {
        dummy_raw_waker()
    }
    unsafe fn wake(_ptr: *const ()) {}
    unsafe fn wake_by_ref(_ptr: *const ()) {}
    unsafe fn drop(_ptr: *const ()) {}

    fn dummy_raw_waker() -> RawWaker {
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }

    let waker = unsafe { Waker::from_raw(dummy_raw_waker()) };
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);

    for _ in 0..10_000 {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::hint::spin_loop(),
        }
    }

    panic!("future did not complete in polling loop");
}
