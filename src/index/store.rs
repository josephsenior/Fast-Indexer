/// The read-only index store.
///
/// Opens the index file with mmap — the OS maps the file into virtual
/// memory. No data is loaded until accessed. The kernel's page cache
/// handles eviction automatically.
///
/// On second query (warm cache), the index is already in RAM — load
/// time is effectively zero.

use anyhow::{bail, Result};
use memmap2::Mmap;
use roaring::RoaringBitmap;
use std::path::Path;

pub struct IndexStore {
    /// The memory-mapped file. Keeps the mapping alive.
    _mmap: Mmap,

    /// Pointer to the raw bytes — safe to use as long as _mmap is alive.
    data: *const u8,
    data_len: usize,

    // Parsed header fields (just u64/u32 — read once at open time)
    pub doc_count: u32,
    pub trigram_count: u32,
    doc_table_offset: u64,
    trigram_table_offset: u64,
}

// SAFETY: The mmap is read-only and the pointer is valid as long as
// the IndexStore is alive. We never mutate through this pointer.
unsafe impl Send for IndexStore {}
unsafe impl Sync for IndexStore {}

impl IndexStore {
    /// Open an index file. The mmap call is ~instantaneous regardless
    /// of file size — the OS doesn't load the data until you read it.
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < 32 {
            bail!("Index file too small — corrupted?");
        }

        // Validate magic bytes
        if &mmap[0..4] != b"CIX1" {
            bail!("Not a valid CIX index file");
        }

        use byteorder::{LittleEndian, ReadBytesExt};
        let mut cursor = std::io::Cursor::new(&mmap[..]);
        cursor.set_position(4); // skip magic

        let _version           = cursor.read_u32::<LittleEndian>()?;
        let doc_count          = cursor.read_u32::<LittleEndian>()?;
        let trigram_count      = cursor.read_u32::<LittleEndian>()?;
        let doc_table_offset   = cursor.read_u64::<LittleEndian>()?;
        let trigram_table_offset = cursor.read_u64::<LittleEndian>()?;

        let data = mmap.as_ptr();
        let data_len = mmap.len();

        Ok(IndexStore {
            _mmap: mmap,
            data,
            data_len,
            doc_count,
            trigram_count,
            doc_table_offset,
            trigram_table_offset,
        })
    }

    /// Look up the file path for a document ID.
    pub fn doc_path(&self, doc_id: u32) -> Option<&str> {
        if doc_id >= self.doc_count {
            return None;
        }

        // Doc table entry: u64 offset + u32 len = 12 bytes
        let entry_offset = self.doc_table_offset as usize + doc_id as usize * 12;
        let bytes = self.slice(entry_offset, 12)?;

        use byteorder::{LittleEndian, ReadBytesExt};
        let mut c = std::io::Cursor::new(bytes);
        let path_offset = c.read_u64::<LittleEndian>().ok()? as usize;
        let path_len    = c.read_u32::<LittleEndian>().ok()? as usize;

        // Path data starts after doc table
        let doc_table_size = self.doc_count as usize * 12;
        let path_data_base = self.doc_table_offset as usize + doc_table_size;
        let path_bytes = self.slice(path_data_base + path_offset, path_len)?;

        std::str::from_utf8(path_bytes).ok()
    }

    /// Look up the RoaringBitmap for a trigram.
    /// Returns None if the trigram doesn't exist in the index.
    pub fn trigram_bitmap(&self, trigram: u32) -> Option<RoaringBitmap> {
        // Binary search in the sorted trigram table
        // Each entry: u32 trigram + u64 offset + u32 len = 16 bytes

        let table_base = self.trigram_table_offset as usize;
        let entry_count = self.trigram_count as usize;

        // Binary search
        let mut lo = 0usize;
        let mut hi = entry_count;

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let entry_offset = table_base + mid * 16;
            let bytes = self.slice(entry_offset, 4)?;
            let stored_trigram = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);

            match stored_trigram.cmp(&trigram) {
                std::cmp::Ordering::Equal => {
                    // Found — read bitmap offset and len
                    let full_entry = self.slice(entry_offset, 16)?;
                    let bm_offset = u64::from_le_bytes(
                        full_entry[4..12].try_into().ok()?
                    ) as usize;
                    let bm_len = u32::from_le_bytes(
                        full_entry[12..16].try_into().ok()?
                    ) as usize;

                    let bm_bytes = self.slice(bm_offset, bm_len)?;
                    return RoaringBitmap::deserialize_from(bm_bytes).ok();
                }
                std::cmp::Ordering::Less    => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }

        None // Trigram not in index
    }

    /// Print human-readable index statistics.
    pub fn print_stats(&self) {
        println!("Index Statistics");
        println!("  Documents  : {}", self.doc_count);
        println!("  Trigrams   : {}", self.trigram_count);
        println!(
            "  Avg trigrams/doc : {:.0}",
            if self.doc_count > 0 {
                self.trigram_count as f64 / self.doc_count as f64
            } else {
                0.0
            }
        );
    }

    /// Safe slice into the mmap buffer.
    #[inline]
    fn slice(&self, offset: usize, len: usize) -> Option<&[u8]> {
        let end = offset.checked_add(len)?;
        if end > self.data_len {
            return None;
        }
        // SAFETY: offset+len is within bounds, data is valid for the
        // lifetime of self (mmap is kept alive in _mmap field).
        Some(unsafe { std::slice::from_raw_parts(self.data.add(offset), len) })
    }
}
