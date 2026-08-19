# The container

A XEX wraps a PE style image that is normally both compressed and encrypted.
`xenolith-xex` turns that file into an image addressable by 32-bit Xbox 360
virtual address, which is the form every later stage consumes.

## What the file holds

A fixed header, then a directory of optional headers keyed by a 32-bit value
whose low byte says whether the entry is a value stored inline or an offset to
data elsewhere in the file. The entries that matter here are the security info,
the file format info, the execution info, and the import libraries.

The security info carries the load address, the image size, the wrapped session
key, and an array of page descriptors. The page descriptors are what give the
image a section table: each describes a run of pages and the permissions on
them, and consecutive runs sharing permissions are what this project reports as
a section.

## Decoding

Two steps, in order.

**Decryption.** The session key in the header is itself encrypted with a static
key, so it is unwrapped first and the image decrypted with the result. This is
AES-128 in CBC mode with a zero initialization vector. Why the static key is not
here is covered in [Key material](/guide/keys).

**Decompression.** Two schemes exist. The basic scheme is a run of blocks, each
a length of data followed by a length of zeroes, which reconstructs a sparse
image without storing its holes. Both test titles use it. The normal scheme uses
LZX inside a framing that records a digest per block; it is not implemented,
because no LZX compressed title was available to check an implementation
against, and shipping one untested would be worse than reporting it unsupported.

The image is zero filled out to the declared size, because the page descriptors
cover that whole range and a section must not claim addresses the byte buffer
cannot serve.

## The address space

Everything downstream addresses the image by virtual address, never by file
offset. A read that is not fully inside the image fails, including when the
address arithmetic would leave the 32-bit space, and it fails as a value rather
than a panic. Input here is whatever file the caller pointed at, so nothing is
trusted.

## How this is checked

- Both test titles parse, and the arithmetic of every optional header offset is
  confirmed to land inside the file rather than assumed to.
- The page descriptors are checked to account for the whole declared image, so a
  gap in the section table would be a failure rather than a silently unmapped
  range.
- The decoded image is compared byte for byte against a reference produced by an
  independent implementation. This is the only check that covers decryption and
  reconstruction together against ground truth rather than against our own
  expectations.
- A property test truncates a synthetic container at every length and asserts
  that each yields an error and never a panic.
- Fuzz targets cover the parser.

## What is not implemented

- **LZX decompression**, for want of a title that uses it.
- **Title update patches.** A shipped game normally has a separate `.xexp` file
  holding a delta against the base image. Applying one is implemented up to the
  point where a sample was needed.

Both are reported clearly rather than failing obscurely. `inspect` names the
compression scheme and says outright when decoding will fail.
