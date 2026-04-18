use crate::crawler::{self, RawFile};
use crate::trigram;
use anyhow::Result;
use crossbeam_channel::unbounded;
use dashmap::DashMap;
use rayon::prelude::*;
use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct IndexBuilder;

pub struct BuildStats {
    pub file_count: u64,
    pub trigram_count: u64,
    pub index_size_bytes: u64,
}

const BATCH_SIZE: usize = 2048;
/// Progress printed to stderr every N files (indexing large trees can take minutes).
const PROGRESS_EVERY: u64 = 5000;

impl IndexBuilder {
    pub fn new() -> Self {
        IndexBuilder
    }

    pub async fn build(&self, root: &Path, output_path: &Path) -> Result<BuildStats> {
        // Unbounded queue: a bounded channel deadlocks here — the main thread blocks inside
        // Rayon (`extract_batch` / posting `par_iter`) while crawler threads fill the buffer
        // and block on `send`, so nobody receives. Unbounded avoids backpressure deadlock;
        // for huge trees, narrow the path or use `.cixignore` to cap memory.
        let (tx, rx) = unbounded::<RawFile>();

        let root_owned = root.to_path_buf();
        let crawler_handle = std::thread::spawn(move || {
            crawler::crawl(&root_owned, tx)
        });

        let posting_lists: DashMap<u32, RoaringBitmap> = DashMap::with_capacity(500_000);
        let mut doc_paths: Vec<PathBuf> = Vec::with_capacity(50_000);
        let mut batch: Vec<RawFile> = Vec::with_capacity(BATCH_SIZE);
        let mut processed_files: u64 = 0;

        loop {
            batch.clear();
            for file in rx.iter().take(BATCH_SIZE) {
                batch.push(file);
            }

            if batch.is_empty() {
                break;
            }

            let batch_len = batch.len() as u64;
            let before = processed_files;
            processed_files += batch_len;
            if before / PROGRESS_EVERY != processed_files / PROGRESS_EVERY {
                eprintln!(
                    "   … {} files indexed (still running — large trees can take minutes)",
                    processed_files
                );
            }

            let batch_files = std::mem::take(&mut batch);
            let trigram_files = trigram::extract_batch(batch_files);

            let base_id = doc_paths.len() as u32;
            let mut trigram_vecs: Vec<Vec<u32>> = Vec::with_capacity(trigram_files.len());
            for tfile in trigram_files {
                doc_paths.push(tfile.path);
                trigram_vecs.push(tfile.trigrams);
            }

            trigram_vecs
                .par_iter()
                .enumerate()
                .for_each(|(i, trigrams)| {
                    let doc_id = base_id + i as u32;
                    for &t in trigrams {
                        posting_lists
                            .entry(t)
                            .or_insert_with(RoaringBitmap::new)
                            .insert(doc_id);
                    }
                });
        }

        let _crawl_stats = crawler_handle
            .join()
            .map_err(|_| anyhow::anyhow!("Crawler thread panicked"))??;

        let file_count = doc_paths.len() as u64;
        let trigram_count = posting_lists.len() as u64;

        let posting_lists: FxHashMap<u32, RoaringBitmap> = posting_lists.into_iter().collect();
        let index_size_bytes = write_index(output_path, &doc_paths, &posting_lists)?;

        Ok(BuildStats {
            file_count,
            trigram_count,
            index_size_bytes,
        })
    }
}

fn write_index(
    path: &Path,
    doc_paths: &[PathBuf],
    posting_lists: &FxHashMap<u32, RoaringBitmap>,
) -> Result<u64> {
    let file = std::fs::File::create(path)?;
    let mut w = BufWriter::with_capacity(8 * 1024 * 1024, file);

    // ── PASS 1: Path data ────────────────────────────────────────────────────

    let mut path_data: Vec<u8> = Vec::new();
    let mut path_offsets: Vec<(u64, u32)> = Vec::with_capacity(doc_paths.len());

    for p in doc_paths {
        let s = p.to_string_lossy();
        let bytes = s.as_bytes();
        let offset = path_data.len() as u64;
        let len = bytes.len() as u32;
        path_data.extend_from_slice(bytes);
        path_offsets.push((offset, len));
    }

    // ── PASS 2: Serialize bitmaps (sequential — avoids zeroing a huge buffer + union overhead) ──

    let mut sorted_trigrams: Vec<u32> = posting_lists.keys().copied().collect();
    sorted_trigrams.sort_unstable();

    let mut bitmap_data: Vec<u8> = Vec::new();
    let mut trigram_entries: Vec<(u32, u64, u32)> = Vec::with_capacity(sorted_trigrams.len());

    for &t in &sorted_trigrams {
        let offset = bitmap_data.len() as u64;
        let len_before = bitmap_data.len();
        posting_lists[&t].serialize_into(&mut bitmap_data)?;
        let serialized_len = (bitmap_data.len() - len_before) as u32;
        trigram_entries.push((t, offset, serialized_len));
    }

    // ── PASS 3: Compute offsets ──────────────────────────────────────────────

    let header_size: u64 = 32;
    let doc_table_offset = header_size;
    let doc_table_size = doc_paths.len() as u64 * 12;
    let path_data_offset = doc_table_offset + doc_table_size;
    let path_data_size = path_data.len() as u64;
    let trigram_table_offset = path_data_offset + path_data_size;
    let trigram_table_size = trigram_entries.len() as u64 * 16;
    let bitmap_data_offset = trigram_table_offset + trigram_table_size;

    // ── WRITE ────────────────────────────────────────────────────────────────

    use byteorder::{LittleEndian, WriteBytesExt};

    w.write_all(b"CIX1")?;
    w.write_u32::<LittleEndian>(1)?;
    w.write_u32::<LittleEndian>(doc_paths.len() as u32)?;
    w.write_u32::<LittleEndian>(trigram_entries.len() as u32)?;
    w.write_u64::<LittleEndian>(doc_table_offset)?;
    w.write_u64::<LittleEndian>(trigram_table_offset)?;

    for (offset, len) in &path_offsets {
        w.write_u64::<LittleEndian>(*offset)?;
        w.write_u32::<LittleEndian>(*len)?;
    }

    w.write_all(&path_data)?;

    for (trigram, bm_offset, bm_len) in &trigram_entries {
        w.write_u32::<LittleEndian>(*trigram)?;
        w.write_u64::<LittleEndian>(bitmap_data_offset + bm_offset)?;
        w.write_u32::<LittleEndian>(*bm_len)?;
    }

    w.write_all(&bitmap_data)?;
    w.flush()?;

    Ok(bitmap_data_offset + bitmap_data.len() as u64)
}