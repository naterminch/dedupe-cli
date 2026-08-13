mod actions;
mod cache;
mod cli;
mod hashing;
mod matching;
mod media;
mod report;
mod scan;
mod similar;
mod util;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use console::style;
use report::ScanStats;

fn main() {
    init_colors();
    let cli = cli::Cli::parse();
    if cli.paths.is_empty() {
        // No scan path supplied: show the full usage/help (to stderr, keeping
        // stdout clean for scripted output) and exit with the standard usage
        // error code. `dedupe --help` / `dedupe --version` are still handled by
        // clap and exit 0.
        let mut cmd = cli::Cli::command();
        let _ = cmd.write_help(&mut std::io::stderr());
        eprintln!();
        let msg = "a scan path is required. Run `dedupe <path>` (e.g. `dedupe .`)";
        eprintln!("{} {msg}", style("error:").red().bold());
        std::process::exit(2);
    }
    if let Err(e) = run(&cli) {
        eprintln!("{} {e:#}", style("error:").red().bold());
        std::process::exit(1);
    }
}

/// Colors are enabled only when stdout is a terminal and NO_COLOR is unset,
/// so redirected / piped output and CI logs stay plain and parseable.
fn init_colors() {
    let no_color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let enabled = !no_color && console::Term::stdout().is_term();
    console::set_colors_enabled(enabled);
}

/// Build the animated progress bar used during scanning/hashing/fingerprinting,
/// or `None` when progress output is disabled (quiet/JSON modes).
fn new_progress_bar(enabled: bool) -> Option<indicatif::ProgressBar> {
    if !enabled {
        return None;
    }
    let bar = indicatif::ProgressBar::new(0);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.cyan} [{bar:40.cyan/blue}] {pos}/{len} files {msg:.dim}",
        )
        .ok()?
        .progress_chars("█▉▊▋▌▍▎▏  ")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    Some(bar)
}

fn run(cli: &cli::Cli) -> Result<()> {
    if !(0.0..=100.0).contains(&cli.similarity) {
        anyhow::bail!("--similarity must be between 0 and 100 (got {})", cli.similarity);
    }
    let similar_threshold = cli.similarity / 100.0;
    let size_min = cli
        .min_size
        .as_deref()
        .map(util::parse_size)
        .transpose()?
        .unwrap_or(0);
    let size_max = cli
        .max_size
        .as_deref()
        .map(util::parse_size)
        .transpose()?
        .unwrap_or(0);

    if !cli.quiet && !cli.json {
        eprintln!(
            "{} scanning {} path(s)...",
            style("◆").cyan().bold(),
            cli.paths.len()
        );
    }
    let scanned = scan::scan(cli, size_min, size_max);

    let has_media_candidates = scanned
        .entries
        .iter()
        .any(|e| media::classify(&e.path) != media::MediaKind::Other);
    let ffprobe_used = has_media_candidates && media::ffprobe_available();
    if has_media_candidates && !ffprobe_used && !cli.quiet && !cli.json {
        eprintln!(
            "{} ffprobe not found on PATH; image/video resolution and duration metadata will not be reported (install FFmpeg to enable)",
            style("⚠").yellow()
        );
    }

    let stats = ScanStats {
        files_scanned: scanned.entries.len(),
        bytes_scanned: scanned.entries.iter().map(|e| e.size).sum(),
        dirs_skipped: scanned.dirs_skipped,
        files_skipped: scanned.files_skipped,
        dir_read_errors: scanned.dir_read_errors,
    };

    let progress_bar = new_progress_bar(!cli.quiet && !cli.json);

    let engine = hashing::HashEngine::new(cli.hash);
    let entries = scanned.entries;
    let similar_entries = if cli.exact {
        Vec::new()
    } else {
        entries.clone()
    };
    let groups = hashing::find_duplicate_groups(entries, &engine, progress_bar.as_ref())?;
    if let Some(bar) = &progress_bar {
        bar.finish_and_clear();
    }

    let mut groups = matching::assemble_groups(groups, cli.keep_smaller, ffprobe_used);

    // Perceptual fingerprint cache: loaded only when the similar pass runs,
    // saved (best-effort) after it. --no-cache / --exact bypass it entirely.
    let mut cache = if cli.exact || cli.no_cache {
        None
    } else {
        Some(cache::FingerprintCache::load())
    };

    if !cli.exact {
        if !cli.quiet && !cli.json {
            eprintln!(
                "{} comparing media by perceptual hash (threshold {}%)...",
                style("◆").cyan().bold(),
                cli.similarity
            );
        }
        let has_videos = similar_entries
            .iter()
            .any(|e| media::classify(&e.path) == media::MediaKind::Video);
        if has_videos && !media::ffmpeg_available() && !cli.quiet && !cli.json {
            eprintln!(
                "{} ffmpeg not found on PATH; videos will not be compared for similarity (images still are)",
                style("⚠").yellow()
            );
        }
        let exact_paths: std::collections::HashSet<std::path::PathBuf> = groups
            .iter()
            .flat_map(|g| g.members.iter().map(|m| m.path.clone()))
            .collect();
        let candidates: Vec<scan::FileEntry> = similar_entries
            .into_iter()
            .filter(|e| !exact_paths.contains(&e.path))
            .collect();
        let similar_cfg = similar::SimilarConfig {
            threshold: similar_threshold,
        };
        // One bar spans the whole similar phase: fingerprinting, then (if any
        // groups were found) building them.
        let similar_bar = new_progress_bar(!cli.quiet && !cli.json);
        let (similar, writes) = similar::find_similar(
            candidates,
            &similar_cfg,
            ffprobe_used,
            cache.as_ref(),
            similar_bar.as_ref(),
        );
        if let Some(c) = &mut cache {
            for w in writes {
                c.insert_write(w);
            }
            // A cache write failure (read-only home dir, ...) is non-fatal.
            if let Err(e) = c.save()
                && !cli.quiet
                && !cli.json
            {
                eprintln!(
                    "{} could not write fingerprint cache: {e}",
                    style("⚠").yellow()
                );
            }
        }
        let built = if !similar.is_empty() {
            similar::build_similar_groups(
                similar,
                cli.keep_smaller,
                ffprobe_used,
                &engine,
                groups.len() + 1,
                similar_bar.as_ref(),
            )?
        } else {
            Vec::new()
        };
        if let Some(bar) = &similar_bar {
            bar.finish_and_clear();
        }
        groups.extend(built);
    }

    let ctx = report::ReportContext {
        paths: &cli.paths,
        hash_algo: engine.name(),
        verbose: cli.verbose,
        ffprobe_used,
    };

    if cli.json {
        report::print_json(&ctx, &stats, &groups);
    } else {
        report::print_human(&ctx, &stats, &groups);
    }

    if cli.delete {
        if groups.is_empty() {
            if !cli.quiet {
                eprintln!("{}", style("Nothing to delete.").dim());
            }
        } else {
            let deletion = actions::delete_duplicates(&groups, &engine, cli.yes)?;
            println!(
                "{} Deleted {} file(s) ({} freed); {} skipped.",
                style("✔").green().bold(),
                style(deletion.files_deleted).green().bold(),
                style(util::human_bytes(deletion.bytes_freed)).green().bold(),
                deletion.files_skipped
            );
        }
    }

    Ok(())
}