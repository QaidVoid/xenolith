//! Checking that the interface emitted code is written against is usable.
//!
//! The header is the contract between this crate and whatever eventually runs
//! the emitted code. If it does not compile, nothing emitted against it can. If
//! its memory accessors read the host's byte order rather than the guest's, the
//! emitted code works on one machine and not another, which is the kind of
//! mistake that is found years later.
//!
//! These tests skip when no C compiler is present, since the crate itself does
//! not need one.

use std::path::PathBuf;
use std::process::Command;

/// The interface, shipped alongside the code emitted against it.
const HEADER: &str = include_str!("../runtime/xenolith.h");

/// Returns a C compiler to check with, if one is installed.
fn compiler() -> Option<&'static str> {
    for candidate in ["clang", "cc", "gcc"] {
        let found = Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if found {
            return Some(candidate);
        }
    }
    None
}

/// Compiles `program` against the header, returning what the compiler said.
fn compiles(name: &str, program: &str) -> Result<(), String> {
    let Some(compiler) = compiler() else {
        return Ok(());
    };

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&directory).map_err(|error| format!("making a directory: {error}"))?;
    std::fs::write(directory.join("xenolith.h"), HEADER)
        .map_err(|error| format!("writing the header: {error}"))?;

    let source = directory.join("main.c");
    std::fs::write(&source, program).map_err(|error| format!("writing the program: {error}"))?;

    let output = Command::new(compiler)
        .args(["-std=c17", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-o")
        .arg(directory.join("program"))
        .arg(&source)
        .output()
        .map_err(|error| format!("running the compiler: {error}"))?;

    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn the_interface_compiles_with_warnings_enabled() {
    let program = r#"
#include "xenolith.h"

void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address, uint32_t from) {
    (void)from;
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_unlifted(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

int main(void) {
    xenolith_context ctx = {0};
    ctx.r[3] = 1;
    return (int)ctx.r[3] - 1;
}
"#;

    if let Err(complaint) = compiles("interface", program) {
        panic!("the interface did not compile:\n{complaint}");
    }
}

/// Guest memory is big endian whatever the host is. A value stored through the
/// interface has to read back with its most significant byte at the lowest
/// address, or every multi byte access in emitted code is silently reversed.
#[test]
fn memory_access_uses_the_guest_byte_order() {
    let program = r#"
#include "xenolith.h"
#include <stdlib.h>

void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address, uint32_t from) {
    (void)from;
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

void xenolith_unlifted(xenolith_context *ctx, uint8_t *base, uint32_t address) {
    (void)ctx;
    (void)base;
    (void)address;
}

int main(void) {
    uint8_t memory[32] = {0};

    xenolith_store32(memory, 0, 0x11223344u);
    if (memory[0] != 0x11 || memory[1] != 0x22 ||
        memory[2] != 0x33 || memory[3] != 0x44) {
        return 1;
    }
    if (xenolith_load32(memory, 0) != 0x11223344u) {
        return 2;
    }

    xenolith_store64(memory, 8, 0x0102030405060708ull);
    if (memory[8] != 0x01 || memory[15] != 0x08) {
        return 3;
    }
    if (xenolith_load64(memory, 8) != 0x0102030405060708ull) {
        return 4;
    }

    xenolith_store16(memory, 20, 0xabcdu);
    if (memory[20] != 0xab || memory[21] != 0xcd) {
        return 5;
    }
    if (xenolith_load16(memory, 20) != 0xabcdu) {
        return 6;
    }

    /* A value written wide and read narrow takes the low part, which is what
     * the instruction set does constantly. */
    xenolith_context ctx = {0};
    ctx.r[3] = 0xdeadbeefcafebabeull;
    if ((uint32_t)ctx.r[3] != 0xcafebabeu) {
        return 7;
    }

    return 0;
}
"#;

    if let Err(complaint) = compiles("byte_order", program) {
        panic!("the check did not compile:\n{complaint}");
    }
    if compiler().is_none() {
        eprintln!("skipping: no C compiler is installed");
        return;
    }
    let program_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("byte_order")
        .join("program");
    let status = Command::new(&program_path)
        .status()
        .unwrap_or_else(|error| panic!("running the check: {error}"));

    assert!(
        status.success(),
        "the interface read or wrote memory in the wrong byte order, check {}",
        status.code().unwrap_or(-1)
    );
}
