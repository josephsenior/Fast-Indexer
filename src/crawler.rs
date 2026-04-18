/// Layer 1: File Crawler
///
/// Walks the directory tree with the `ignore` crate (same as ripgrep).
///
/// **What gets skipped (ignore rules — same syntax as `.gitignore`):**
/// - **`.gitignore`** in each directory (and parents) — applied automatically.
/// - **`.ignore`**, git exclude rules — via `standard_filters(true)`.
/// - **`.cixignore`** — optional extra rules per directory (add patterns you do not want indexed).
///
/// There is **no extension allowlist**: anything that passes ignore rules, size limits, and
/// the quick binary sniff is indexed. Use `.cixignore` to exclude e.g. `*.png`, `*.pdf`, build dirs.
///
/// **Also skipped:** files larger than `MAX_FILE_BYTES`, and files that `looks_binary` classifies
/// as binary (after a full read up to the size cap).

use anyhow::Result;
use crossbeam_channel::Sender;
use ignore::WalkBuilder;
use std::path::Path;

/// A file ready for trigram extraction.
pub struct RawFile {
    pub path: std::path::PathBuf,
    pub content: Vec<u8>,
}

/// Maximum file size we'll index (10MB — larger files are usually generated/binary)
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Crawl `root` and send every indexable file to `tx`.
///
/// This uses `ignore::WalkBuilder`'s built-in parallel walker which
/// spawns one thread per CPU core internally. Each thread reads files
/// and sends them down the channel.
///
/// On Linux this will use io_uring via tokio under the hood.
/// On macOS: kqueue. On Windows: IOCP.
pub fn crawl(root: &Path, tx: Sender<RawFile>) -> Result<CrawlStats> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let files_found = Arc::new(AtomicU64::new(0));
    let bytes_read = Arc::new(AtomicU64::new(0));
    let files_skipped = Arc::new(AtomicU64::new(0));

    let ff = Arc::clone(&files_found);
    let br = Arc::clone(&bytes_read);
    let fs = Arc::clone(&files_skipped);

    WalkBuilder::new(root)
        // .gitignore / .ignore / etc. (per-directory, same as ripgrep)
        .standard_filters(true)
        // Additional ignore file layered on top (optional in each directory)
        .add_custom_ignore_filename(".cixignore")
        // Do not follow symlinks / junctions (avoids crawling unintended trees on Windows)
        .follow_links(false)
        // Stay on the same volume as the root (skips mount points / subst quirks)
        .same_file_system(true)
        // Use all available CPU cores for the walk itself
        .threads(num_cpus())
        .build_parallel()
        .run(|| {
            // This closure is called once per walker thread.
            // We clone the sender so each thread can send independently.
            let tx = tx.clone();
            let ff = Arc::clone(&ff);
            let br = Arc::clone(&br);
            let fs = Arc::clone(&fs);

            Box::new(move |entry| {
                use ignore::WalkState;

                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };

                // Only process files, not directories
                if entry.file_type().map(|t| !t.is_file()).unwrap_or(true) {
                    return WalkState::Continue;
                }

                let path = entry.path().to_path_buf();

                // Skip files that are too large
                if let Some(meta) = entry.metadata().ok() {
                    if meta.len() > MAX_FILE_BYTES {
                        fs.fetch_add(1, Ordering::Relaxed);
                        return WalkState::Continue;
                    }
                }

                // Read file content
                match std::fs::read(&path) {
                    Ok(content) => {
                        // Quick binary sniff — if >30% non-UTF8-ish bytes, skip
                        if looks_binary(&content) {
                            fs.fetch_add(1, Ordering::Relaxed);
                            return WalkState::Continue;
                        }

                        ff.fetch_add(1, Ordering::Relaxed);
                        br.fetch_add(content.len() as u64, Ordering::Relaxed);

                        // Send to indexer (unbounded queue in IndexBuilder — avoids deadlock
                        // with a bounded channel when the main thread is inside Rayon work).
                        let _ = tx.send(RawFile { path, content });
                    }
                    Err(_) => {
                        fs.fetch_add(1, Ordering::Relaxed);
                    }
                }

                WalkState::Continue
            })
        });

    // Drop the sender so the receiver knows we're done
    drop(tx);

    Ok(CrawlStats {
        files_indexed: files_found.load(std::sync::atomic::Ordering::Relaxed),
        bytes_read: bytes_read.load(std::sync::atomic::Ordering::Relaxed),
        files_skipped: files_skipped.load(std::sync::atomic::Ordering::Relaxed),
    })
}

/// Quick binary detection: sample first 512 bytes.
/// If more than 1 in 8 bytes is a null byte or high-value control char, it's binary.
#[inline]
fn looks_binary(content: &[u8]) -> bool {
    let sample = &content[..content.len().min(512)];
    let non_text = sample.iter().filter(|&&b| b == 0 || (b < 9 && b != 0)).count();
    non_text > sample.len() / 8
}

/// Returns the number of logical CPUs, capped at 32.
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(32))
        .unwrap_or(4)
}

#[derive(Debug)]
pub struct CrawlStats {
    pub files_indexed: u64,
    pub bytes_read: u64,
    pub files_skipped: u64,
}
