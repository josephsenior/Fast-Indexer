/// Layer 3: Index Store
///
/// The index is a single binary file with this layout:
///
/// ┌─────────────────────────────────────────┐
/// │  HEADER (32 bytes)                      │
/// │  magic: [u8; 4] = b"CIX1"              │
/// │  version: u32                           │
/// │  doc_count: u32                         │
/// │  trigram_count: u32                     │
/// │  doc_table_offset: u64                  │
/// │  trigram_table_offset: u64              │
/// ├─────────────────────────────────────────┤
/// │  DOCUMENT TABLE                         │
/// │  [doc_id → (path_offset: u64,          │
/// │             path_len: u32)]             │
/// ├─────────────────────────────────────────┤
/// │  PATH DATA                              │
/// │  Concatenated UTF-8 file paths          │
/// ├─────────────────────────────────────────┤
/// │  TRIGRAM TABLE                          │
/// │  Sorted array of:                       │
/// │  (trigram: u32, bitmap_offset: u64,     │
/// │   bitmap_len: u32)                      │
/// ├─────────────────────────────────────────┤
/// │  BITMAP DATA                            │
/// │  Serialized RoaringBitmaps —            │
/// │  each bitmap = set of doc IDs           │
/// │  that contain this trigram              │
/// └─────────────────────────────────────────┘
///
/// The entire file is memory-mapped at query time.
/// No parsing, no deserialization — the OS demand-pages
/// exactly the bytes we access.

mod builder;
mod store;

pub use builder::{IndexBuilder, BuildStats};
pub use store::IndexStore;
