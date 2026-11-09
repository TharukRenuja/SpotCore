use crate::{BindingError, Session, wire};
use std::ffi::{CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn output(json: String) -> *mut c_char {
    CString::new(json)
        .expect("JSON escapes NUL characters")
        .into_raw()
}
fn failure(code: &str, message: &str) -> String {
    wire::response::<()>(Err(BindingError::new(code, message)))
}
fn guarded(action: impl FnOnce() -> String) -> *mut c_char {
    output(
        catch_unwind(AssertUnwindSafe(action))
            .unwrap_or_else(|_| failure("internal", "Native request failed")),
    )
}

/// Creates an opaque session. Any error string is owned by the caller.
/// # Safety
/// `error_out` must be null or writable for one pointer. Free its result with
/// `spotcore_string_free`. A returned session is freed exactly once with `spotcore_session_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spotcore_session_new(error_out: *mut *mut c_char) -> *mut c_void {
    if !error_out.is_null() {
        unsafe {
            *error_out = std::ptr::null_mut();
        }
    }
    let result = catch_unwind(Session::new).unwrap_or_else(|_| {
        Err(BindingError::new(
            "internal",
            "Native session creation failed",
        ))
    });
    match result {
        Ok(session) => Box::into_raw(Box::new(session)).cast(),
        Err(error) => {
            if !error_out.is_null() {
                unsafe {
                    *error_out = output(wire::response::<()>(Err(error)));
                }
            }
            std::ptr::null_mut()
        }
    }
}

/// Runs one blocking request. Event reads use non-blocking `poll_event`.
/// # Safety
/// `handle` must be a live session from `spotcore_session_new`, and cannot be freed
/// during this call. For lengths up to the size cap, `data` must point to `length`
/// readable bytes. Free the response
/// with `spotcore_string_free`; no input pointer is retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spotcore_session_call(
    handle: *mut c_void,
    data: *const u8,
    length: usize,
) -> *mut c_char {
    guarded(|| {
        if handle.is_null() || data.is_null() {
            return failure("invalid_input", "Session and request pointers are required");
        }
        if length > wire::MAX_REQUEST_BYTES {
            return failure("input_too_large", "Request exceeds 1 MiB");
        }
        let bytes = unsafe { std::slice::from_raw_parts(data, length) };
        let Ok(request) = std::str::from_utf8(bytes) else {
            return failure("invalid_input", "Request must be UTF-8");
        };
        let session = unsafe { &*handle.cast::<Session>() };
        futures_executor::block_on(wire::call_c(session, request))
    })
}

/// Shuts down and destroys a session. Null is accepted.
/// # Safety
/// The handle must be live, exclusively owned, and have no calls in flight.
/// It must not be used after this function returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spotcore_session_free(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let session = unsafe { Box::from_raw(handle.cast::<Session>()) };
        let _ = futures_executor::block_on(session.shutdown());
    }));
}

/// Releases a returned string. Null is accepted.
/// # Safety
/// The pointer must be a live result from this API and freed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spotcore_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    unsafe fn read(value: *mut c_char) -> serde_json::Value {
        let json =
            serde_json::from_str(unsafe { CStr::from_ptr(value) }.to_str().unwrap()).unwrap();
        unsafe {
            spotcore_string_free(value);
        }
        json
    }
    #[test]
    fn c_boundary_checks_inputs_and_owns_cleanup() {
        unsafe {
            let mut error = std::ptr::null_mut();
            let session = spotcore_session_new(&mut error);
            assert!(!session.is_null());
            assert!(error.is_null());
            for bytes in [
                &b"{\"op\":\"account\"}"[..],
                &b"{\"op\":\"poll_event\"}"[..],
            ] {
                assert_eq!(
                    read(spotcore_session_call(session, bytes.as_ptr(), bytes.len()))["error"]["code"],
                    "not_authenticated"
                );
            }
            let wait = br#"{"op":"next_event"}"#;
            assert_eq!(
                read(spotcore_session_call(session, wait.as_ptr(), wait.len()))["error"]["code"],
                "unsupported"
            );
            assert_eq!(
                read(spotcore_session_call(session, [255].as_ptr(), 1))["error"]["code"],
                "invalid_input"
            );
            assert_eq!(
                read(spotcore_session_call(
                    session,
                    b"x".as_ptr(),
                    wire::MAX_REQUEST_BYTES + 1
                ))["error"]["code"],
                "input_too_large"
            );
            assert_eq!(
                read(spotcore_session_call(session, std::ptr::null(), 0))["error"]["code"],
                "invalid_input"
            );
            spotcore_session_free(session);
            spotcore_session_free(std::ptr::null_mut());
            spotcore_string_free(std::ptr::null_mut());
        }
    }
}
