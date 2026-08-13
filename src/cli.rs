use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Parser, ValueEnum};

/// Color scheme for the help output. clap emits these only when the target
/// stream is a terminal (ColorChoice::Auto), so piped output stays plain.
fn cli_styles() -> Styles {
    Styles::styled()
        .usage(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .header(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Yellow.on_default())
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Red.on_default())
}

/// Find duplicate files recursively in one or more paths.
///
/// Files are grouped by size, then compared with a partial hash followed by a
/// full content hash (jdupes-style pipeline). Images and videos additionally
/// have their resolution / duration probed via `ffprobe` when available, and
/// that metadata is reported alongside each duplicate group.
#[derive(Debug, Parser)]
#[command(
    name = "dedupe",
    version,
    about = "Find duplicate files by content hash",
    long_about = None,
    override_usage = "dedupe [OPTIONS] <PATH>...",
    styles = cli_styles()
)]
pub struct Cli {
    /// Paths to scan (files or directories). At least one is required;
    /// running the binary without a path prints the usage/help instead.
    #[arg(value_name = "PATH", num_args = 1..)]
    pub paths: Vec<String>,

    /// Maximum recursion depth. 0 = files directly in the given paths only
    /// (default: unlimited).
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,

    /// Only scan files with these extensions (comma-separated, case-insensitive,
    /// e.g. --types jpg,png,mp4,txt).
    #[arg(short, long, value_name = "EXTS", value_delimiter = ',')]
    pub types: Vec<String>,

    /// Within each duplicate group, prefer keeping the smallest file
    /// (for media, files in a group always share the same content, hence the
    /// same resolution -- so the smaller encoding is kept).
    #[arg(short, long)]
    pub keep_smaller: bool,

    /// Delete duplicate files. Prompts per group unless --yes is given.
    #[arg(short = 'D', long)]
    pub delete: bool,

    /// Assume "yes" for all deletion prompts.
    #[arg(short, long)]
    pub yes: bool,

    /// Hash algorithm used for content comparison.
    #[arg(long, value_enum, default_value_t = HashAlgo::Blake3)]
    pub hash: HashAlgo,

    /// Only detect byte-identical duplicates. Near-duplicate images/videos
    /// (the same content re-saved or re-encoded in a different format, e.g.
    /// .png vs .jpg, .mp4 vs .mov) are detected by default via perceptual
    /// hash; this flag disables that pass.
    #[arg(long)]
    pub exact: bool,

    /// Similarity threshold (0-100) above which similar media counts as
    /// duplicated (default: 97).
    #[arg(
        long,
        value_name = "PCT",
        default_value_t = crate::similar::DEFAULT_SIMILARITY_PCT
    )]
    pub similarity: f64,

    /// Do not read or write the perceptual fingerprint cache (stored at
    /// ~/.dedupe/fingerprints.bin). Fingerprints are recomputed from scratch.
    #[arg(long)]
    pub no_cache: bool,

    /// Minimum file size to consider (e.g. 100KB, 1MB, 1GiB). 0 disables.
    #[arg(long, value_name = "SIZE")]
    pub min_size: Option<String>,

    /// Maximum file size to consider (e.g. 2GB). 0 disables.
    #[arg(long, value_name = "SIZE")]
    pub max_size: Option<String>,

    /// Skip directories whose name contains this substring (repeatable).
    #[arg(long, value_name = "NAME", action = clap::ArgAction::Append)]
    pub exclude_dir: Vec<String>,

    /// Skip files whose full path contains this substring (repeatable).
    #[arg(long, value_name = "SUBSTR", action = clap::ArgAction::Append)]
    pub exclude_path: Vec<String>,

    /// Emit machine-readable JSON instead of the human report.
    #[arg(short, long)]
    pub json: bool,

    /// Include per-file media metadata (resolution, duration, codec).
    #[arg(short, long)]
    pub verbose: bool,

    /// Suppress progress output.
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum HashAlgo {
    Blake3,
    Sha256,
    Md5,
}