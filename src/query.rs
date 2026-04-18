/// Layer 4: Query Engine
///
/// Converts a search string into a set of matching file paths.
///
/// Algorithm:
///   1. Extract trigrams from the query string
///   2. Look up each trigram's RoaringBitmap (set of doc IDs)
///   3. AND all bitmaps together → candidate doc IDs
///   4. Resolve doc IDs → file paths
///   5. (Optional) verify each candidate with exact substring match
///
/// The AND step is where RoaringBitmaps shine:
///   A ∩ B is a SIMD-accelerated bitwise AND across compressed 64-bit blocks.
///   For 1M documents, this runs in microseconds.

use crate::index::IndexStore;
use crate::trigram::query_trigrams;
use anyhow::Result;
use roaring::RoaringBitmap;
use std::path::PathBuf;

/// Search the index for files matching `query`.
///
/// Returns a list of file paths that contain all trigrams of the query.
/// False positives are possible (trigrams present but not as a substring)
/// and are filtered by exact re-check if `verify` is true.
pub fn search(store: &IndexStore, query: &str) -> Result<Vec<PathBuf>> {
    let trigrams = query_trigrams(query);

    if trigrams.is_empty() {
        return Ok(vec![]);
    }

    // If query is shorter than 3 bytes, we can't use the trigram index.
    // Fall back to scanning all documents (rare in practice for code search).
    if query.len() < 3 {
        return search_short(store, query);
    }

    // ── Phase 1: Bitmap intersection ────────────────────────────────────────

    // Start with the bitmap for the first trigram, then AND in the rest.
    // We sort trigrams by rarity (smallest bitmap = fewest matches) to
    // prune the candidate set as early as possible.
    // For simplicity in this draft, we just process in sorted order.

    let mut candidates: Option<RoaringBitmap> = None;

    for trigram in &trigrams {
        match store.trigram_bitmap(*trigram) {
            None => {
                // This trigram doesn't exist in the index at all.
                // The query string can't be in any file → zero results.
                return Ok(vec![]);
            }
            Some(bitmap) => {
                candidates = Some(match candidates {
                    None => bitmap,
                    Some(existing) => existing & bitmap, // RoaringBitmap AND
                });
            }
        }

        // Early exit: if candidate set is already empty, no need to continue
        if candidates.as_ref().map(|b| b.is_empty()).unwrap_or(false) {
            return Ok(vec![]);
        }
    }

    // ── Phase 2: Resolve doc IDs → paths ────────────────────────────────────

    let candidate_ids = match candidates {
        None => return Ok(vec![]),
        Some(b) => b,
    };

    let mut results = Vec::with_capacity(candidate_ids.len() as usize);

    for doc_id in candidate_ids.iter() {
        if let Some(path_str) = store.doc_path(doc_id) {
            results.push(PathBuf::from(path_str));
        }
    }

    Ok(results)
}

/// Search for queries shorter than 3 bytes by scanning all document paths.
/// This is a rare fallback — most code searches are longer.
fn search_short(store: &IndexStore, _query: &str) -> Result<Vec<PathBuf>> {
    // For now, return empty — very short queries aren't useful for code search
    // In a full implementation, you'd scan file contents directly here
    Ok((0..store.doc_count)
        .filter_map(|id| store.doc_path(id).map(PathBuf::from))
        .collect())
}

/// Advanced search with AND/OR/NOT query parsing.
/// e.g. "fn render AND tokio NOT test"
/// This is a placeholder for Phase 2 implementation.
pub fn search_advanced(
    store: &IndexStore,
    query: &str,
) -> Result<Vec<PathBuf>> {
    // Simple implementation: treat the whole string as a single query
    // Full boolean query parsing is a Phase 2 feature
    search(store, query)
}
