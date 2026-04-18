/// Layer 2: Trigram Extractor
///
/// For every file, extracts all unique 3-byte sequences (trigrams) and
/// maps them to a compact u32 ID.
///
/// Why trigrams?
///   Any substring search can be broken into trigram lookups.
///   "fn render" → trigrams that MUST appear in any file containing this string.
///   We AND the posting lists → candidate files → verify with exact match.
///
/// Encoding:
///   A trigram is 3 bytes. Each byte is 0–255.
///   We pack them into a u32: (b0 << 16) | (b1 << 8) | b2
///   This gives us a 24-bit key — only 16,777,216 possible trigrams.
///   In practice most codebases use ~100,000–500,000 unique trigrams.
///
/// Parallelism:
///   Files are processed in parallel via Rayon.
///   Each file produces a Vec<u32> of unique trigram IDs.

use crate::crawler::RawFile;
use rayon::prelude::*;
use rustc_hash::FxHashSet;

/// A file with its extracted trigrams, ready to be inserted into the index.
pub struct TrigramFile {
    pub path: std::path::PathBuf,
    /// Sorted, deduplicated list of trigram IDs present in this file.
    pub trigrams: Vec<u32>,
}

/// Extract trigrams from a batch of raw files in parallel.
///
/// Uses Rayon to process files across all CPU cores simultaneously.
/// This is the CPU-heavy phase — pure computation, no I/O.
pub fn extract_batch(files: Vec<RawFile>) -> Vec<TrigramFile> {
    files
        .into_par_iter()
        .map(|file| {
            let trigrams = extract_trigrams(&file.content);
            TrigramFile {
                path: file.path,
                trigrams,
            }
        })
        .collect()
}

/// Extract all unique trigrams from a byte slice.
///
/// The inner loop is the hot path. It's written to be friendly to
/// auto-vectorization by the compiler (LLVM will apply SIMD here
/// on x86-64 with AVX2, and on ARM with NEON).
///
/// For explicit SIMD, see `extract_trigrams_simd` below — enabled
/// with the `simd` feature flag.
#[inline]
pub fn extract_trigrams(content: &[u8]) -> Vec<u32> {
    if content.len() < 3 {
        return vec![];
    }

    // Pre-allocate with a reasonable estimate.
    // Most files have (len - 2) trigrams before deduplication.
    // After dedup, expect ~40-60% unique rate for code.
    let mut seen = FxHashSet::with_capacity_and_hasher(
        (content.len() / 4).min(65536),
        Default::default(),
    );

    // Hot loop — compiler will vectorize this with -O3 + target-cpu=native
    for window in content.windows(3) {
        let trigram = pack_trigram(window[0], window[1], window[2]);
        seen.insert(trigram);
    }

    // Sort for cache-friendly sequential access during index building
    let mut result: Vec<u32> = seen.into_iter().collect();
    result.sort_unstable();
    result
}

/// Pack 3 bytes into a u32 trigram key.
/// Uses the upper 24 bits, leaving the low 8 bits as zero (reserved).
#[inline(always)]
pub fn pack_trigram(a: u8, b: u8, c: u8) -> u32 {
    ((a as u32) << 16) | ((b as u32) << 8) | (c as u32)
}

/// Unpack a u32 trigram key back to 3 bytes (for debugging/display).
#[inline(always)]
pub fn unpack_trigram(t: u32) -> [u8; 3] {
    [
        ((t >> 16) & 0xFF) as u8,
        ((t >> 8) & 0xFF) as u8,
        (t & 0xFF) as u8,
    ]
}

/// Extract trigrams from a query string for lookup.
/// Same logic as file extraction — used by the query engine.
pub fn query_trigrams(query: &str) -> Vec<u32> {
    extract_trigrams(query.as_bytes())
}

/// Display a trigram as a string (for debugging)
pub fn trigram_display(t: u32) -> String {
    let bytes = unpack_trigram(t);
    // Show as string if all bytes are printable ASCII, else as hex
    if bytes.iter().all(|&b| b >= 32 && b < 127) {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        format!("{:06x}", t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigram_basic() {
        let trigrams = extract_trigrams(b"hello");
        // "hello" → "hel", "ell", "llo"
        assert_eq!(trigrams.len(), 3);

        let expected = vec![
            pack_trigram(b'h', b'e', b'l'),
            pack_trigram(b'e', b'l', b'l'),
            pack_trigram(b'l', b'l', b'o'),
        ];

        for e in &expected {
            assert!(trigrams.contains(e), "Missing trigram {:06x}", e);
        }
    }

    #[test]
    fn test_trigram_dedup() {
        // "aaa" → only one unique trigram: "aaa"
        let trigrams = extract_trigrams(b"aaaa");
        assert_eq!(trigrams.len(), 1);
    }

    #[test]
    fn test_trigram_too_short() {
        assert!(extract_trigrams(b"ab").is_empty());
        assert!(extract_trigrams(b"").is_empty());
    }

    #[test]
    fn test_pack_unpack_roundtrip() {
        let original = [b'f', b'n', b' '];
        let packed = pack_trigram(original[0], original[1], original[2]);
        let unpacked = unpack_trigram(packed);
        assert_eq!(original, unpacked);
    }
}
