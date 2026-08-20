//! Parsing and decoding for Xbox 360 XEX executable containers.
//!
//! A XEX wraps a PE style image that is usually both compressed and encrypted,
//! and shipped titles normally carry a separate XEXP file holding a title
//! update as a delta against the base image. This crate turns those files into
//! a decoded image addressable by 32-bit Xbox 360 virtual address, which is the
//! form every later stage of the recompiler consumes.
//!
//! Input is whatever file the caller points at, so nothing here trusts its
//! contents. Every parse is bounds checked and every rejection arrives as an
//! [`Error`] rather than a panic. The crate contains no `unsafe` code.

extern crate alloc;

mod container;
mod crypto;
mod error;
mod exports;
mod headers;
mod image;
mod imports;
mod reader;
mod security;

pub use container::{Container, Format, OptionalHeader, OptionalHeaderKey, OptionalHeaderValue};
pub use crypto::KeyMaterial;
pub use error::{Error, Result};
pub use exports::{Export, ExportKind, export};
pub use headers::{
    BasicBlock, CompressionType, EncryptionType, ExecutionInfo, FileFormatInfo, ImportLibrary,
    NormalCompression, Version, keys,
};
pub use image::{Image, Permissions, Section};
pub use imports::{Import, ImportKind, imports};
pub use security::{PageDescriptor, PageKind, SecurityInfo};
