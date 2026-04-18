/// Layer 5: Incremental Watcher
///
/// Watches the filesystem for changes and triggers partial re-indexing.
/// This is the feature most existing tools (including Zoekt) handle poorly.
///
/// Strategy:
///   - Watch for create/modify/delete events via the `notify` crate
///   - Debounce rapid changes (e.g. saving a file multiple times in 1 second)
///   - For modified/created files: re-extract trigrams, update posting lists
///   - For deleted files: remove doc ID from all posting lists
///
/// Note: Rebuilding posting lists in-place on the mmap'd file is complex.
/// This draft uses a "dirty flag + rebuild" approach: accumulate changes
/// for 500ms, then rebuild only affected sections.

use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

pub async fn watch(root: &Path, index_path: &Path) -> Result<()> {
    let (tx, rx) = channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
    watcher.watch(root, RecursiveMode::Recursive)?;

    println!("Watching for changes. Press Ctrl+C to stop.");

    // Accumulate changed paths, debounce, then rebuild
    let mut pending_changes: Vec<std::path::PathBuf> = Vec::new();
    let debounce = Duration::from_millis(500);

    loop {
        // Collect events with a timeout for debouncing
        match rx.recv_timeout(debounce) {
            Ok(Ok(event)) => {
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                        for path in event.paths {
                            println!("  Changed: {}", path.display());
                            pending_changes.push(path);
                        }
                    }
                    _ => {}
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Debounce period elapsed — process pending changes
                if !pending_changes.is_empty() {
                    println!(
                        "🔄 Re-indexing {} changed file(s)...",
                        pending_changes.len()
                    );

                    // For this draft: full rebuild
                    // Phase 2: partial update (only changed docs)
                    let builder = crate::index::IndexBuilder::new();
                    match builder.build(root, index_path).await {
                        Ok(stats) => println!(
                            "✅ Index updated: {} files, {} trigrams",
                            stats.file_count, stats.trigram_count
                        ),
                        Err(e) => eprintln!("❌ Re-index failed: {}", e),
                    }

                    pending_changes.clear();
                }
            }
            Ok(Err(e)) => eprintln!("Watch error: {}", e),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}
