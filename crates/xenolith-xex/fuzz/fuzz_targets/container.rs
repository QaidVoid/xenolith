//! Asserts that container parsing is total over arbitrary bytes.
//!
//! A XEX is whatever file the user points the tool at, so parsing has to return
//! an error for every input it cannot handle rather than panicking, looping
//! forever, or reading out of bounds.

#![no_main]

use libfuzzer_sys::fuzz_target;
use xenolith_xex::Container;

fuzz_target!(|data: &[u8]| {
    let Ok(container) = Container::parse(data) else {
        return;
    };

    // Every accessor must stay within the input that was accepted.
    let _ = container.format();
    let _ = container.module_flags();
    let _ = container.entry_point();
    let _ = container.image_base_address();
    let _ = container.encryption();
    let _ = container.compression();
    let _ = container.execution_info();
    let _ = container.sections();
    let _ = container.body();

    for header in container.optional_headers() {
        let _ = header.value.data();
    }
    for library in container.import_libraries() {
        assert!(library.imports.len() <= data.len());
    }

    let security = container.security_info();
    let _ = security.total_pages();
    let _ = security.export_table_address();
});
