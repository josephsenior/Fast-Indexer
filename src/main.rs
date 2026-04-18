mod crawler;
mod trigram;
mod index;
mod query;
mod watcher;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "cix")]
#[command(about = "Code Index, eXtreme — sub-millisecond codebase search")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build or rebuild the index for a directory
    Index {
        /// Root directory to index (respects .gitignore; add .cixignore for extra excludes)
        path: PathBuf,

        /// Output index file (default: .cix-index in target dir)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Search the index
    Search {
        /// Query string
        query: String,

        /// Index file to search
        #[arg(short, long)]
        index: Option<PathBuf>,

        /// Show line numbers (slower — requires re-reading files)
        #[arg(short, long)]
        lines: bool,
    },

    /// Watch a directory and keep the index up to date
    Watch {
        /// Root directory to watch
        path: PathBuf,

        #[arg(short, long)]
        index: Option<PathBuf>,
    },

    /// Print index statistics
    Stats {
        #[arg(short, long)]
        index: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Index { path, output } => {
            let index_path = output.unwrap_or_else(|| path.join(".cix-index"));

            println!("📂 Indexing: {}", path.display());
            let t = Instant::now();

            let builder = index::IndexBuilder::new();
            let stats = builder.build(&path, &index_path).await?;

            println!(
                "✅ Done in {:.2}s — {} files, {} trigrams, {:.1} MB",
                t.elapsed().as_secs_f64(),
                stats.file_count,
                stats.trigram_count,
                stats.index_size_bytes as f64 / 1_048_576.0
            );
        }

        Command::Search { query, index, lines } => {
            let index_path = index.unwrap_or_else(|| PathBuf::from(".cix-index"));
            let store = index::IndexStore::open(&index_path).with_context(|| {
                format!(
                    "could not open index {:?} (os error usually means wrong path).\n\
                     Default is `.\\cix-index` in the current directory.\n\
                     If you built the index elsewhere, use: --index \"C:\\\\path\\\\to\\\\.cix-index\"",
                    index_path
                )
            })?;

            println!(
                "📇 Index: {} — {} documents",
                index_path.display(),
                store.doc_count
            );

            let t = Instant::now();
            let results = query::search(&store, &query)?;
            let elapsed = t.elapsed();

            if results.is_empty() {
                println!("No results for {:?}", query);
                println!(
                    "  (Searched the index above. If you meant another project, pass --index to its .cix-index file.)"
                );
            } else {
                for path in &results {
                    println!("{}", path.display());
                }
                println!(
                    "\n{} result(s) in {:.3}ms",
                    results.len(),
                    elapsed.as_secs_f64() * 1000.0
                );
            }

            if lines {
                println!("(--lines not yet implemented in this draft)");
            }
        }

        Command::Watch { path, index } => {
            let index_path = index.unwrap_or_else(|| path.join(".cix-index"));
            println!("👀 Watching: {} → {}", path.display(), index_path.display());
            watcher::watch(&path, &index_path).await?;
        }

        Command::Stats { index } => {
            let index_path = index.unwrap_or_else(|| PathBuf::from(".cix-index"));
            let store = index::IndexStore::open(&index_path).with_context(|| {
                format!(
                    "could not open index {:?}. Use --index if the file lives in another folder.",
                    index_path
                )
            })?;
            store.print_stats();
        }
    }

    Ok(())
}
