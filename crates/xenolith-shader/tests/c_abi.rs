//! The C face of this crate.
//!
//! A title hands a shader over while it is running, and the program that is
//! running is C. So this crate has one function a C caller reaches, through
//! the static library `xenolith-host` builds, which carries it.

#[test]
fn the_function_a_c_caller_reaches_has_a_c_signature() {
    // Naming the function through a C function pointer type is what fails to
    // compile where the signature drifts, which is the thing worth catching:
    // the C side reads the answer back by hand and shares no header.
    let reachable: unsafe extern "C" fn(u32, *const u32, u32, *mut u32, u32) -> u32 =
        xenolith_shader::running::xenolith_say_a_shader;
    let _ = reachable;
}
