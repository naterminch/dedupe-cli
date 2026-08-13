use crate::cli::Cli;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// A single discovered file.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    /// Modification time in whole seconds since the Unix epoch, if readable.
    pub mtime_secs: Option<i64>,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub entries: Vec<FileEntry>,
    pub dirs_skipped: u64,
    pub files_skipped: u64,
    pub dir_read_errors: Vec<String>,
}

/// Build a set of lower-cased extensions (dots stripped) from `--types`.
pub fn extension_filter(exts: &[String]) -> HashSet<String> {
    exts.iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

fn matches_extension(path: &Path, filter: &HashSet<String>) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    filter.contains(&ext.to_ascii_lowercase())
}

impl ScanResult {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Recursively collect candidate files under `cli.paths`.
///
/// Files outside the size range, or excluded by `--exclude*` / `--types`
/// filters, are skipped. Directory read errors are collected (not fatal).
pub fn scan(cli: &Cli, size_min: u64, size_max: u64) -> ScanResult {
    let ext_filter = extension_filter(&cli.types);
    let type_filtered = !ext_filter.is_empty();
    let mut result = ScanResult::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for raw_path in &cli.paths {
        let path = Path::new(raw_path);

        // A directly-given file: include it if it passes the filters.
        if path.is_file() {
            collect_file(
                path,
                cli,
                &ext_filter,
                type_filtered,
                size_min,
                size_max,
                &mut seen,
                &mut result,
            );
            continue;
        }
        if !path.exists() {
            result
                .dir_read_errors
                .push(format!("path does not exist: {}", path.display()));
            continue;
        }

        use std::sync::atomic::{AtomicU64, Ordering};
        let dirs_skipped = AtomicU64::new(0);
        let walker = WalkDir::new(path)
            // walkdir counts the root as depth 0 and the user's --max-depth 0
            // means "direct children only", hence the +1 offset.
            .max_depth(cli.max_depth.map(|d| d + 1).unwrap_or(usize::MAX))
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true; // never prune the root itself
                }
                if e.file_type().is_dir() && matches_excluded_dir(e.path(), cli) {
                    dirs_skipped.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
                true
            });
        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    result
                        .dir_read_errors
                        .push(format!("cannot read: {err}"));
                    continue;
                }
            };

            if entry.file_type().is_dir() || !entry.file_type().is_file() {
                continue;
            }
            collect_file(
                entry.path(),
                cli,
                &ext_filter,
                type_filtered,
                size_min,
                size_max,
                &mut seen,
                &mut result,
            );
        }
        result.dirs_skipped += dirs_skipped.load(Ordering::Relaxed);
    }

    result
}

fn matches_excluded_dir(path: &Path, cli: &Cli) -> bool {
    if cli.exclude_dir.is_empty() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    cli.exclude_dir.iter().any(|ex| name.contains(ex.as_str()))
}

fn matches_excluded_path(path: &Path, cli: &Cli) -> bool {
    let raw = path.to_string_lossy();
    cli.exclude_path.iter().any(|ex| raw.contains(ex.as_str()))
}

#[allow(clippy::too_many_arguments)]
fn collect_file(
    path: &Path,
    cli: &Cli,
    ext_filter: &HashSet<String>,
    type_filtered: bool,
    size_min: u64,
    size_max: u64,
    seen: &mut HashSet<PathBuf>,
    result: &mut ScanResult,
) {
    if type_filtered && !matches_extension(path, ext_filter) {
        result.files_skipped += 1;
        return;
    }
    if matches_excluded_path(path, cli) {
        result.files_skipped += 1;
        return;
    }
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            result.files_skipped += 1;
            return;
        }
    };
    let size = meta.len();
    if size < size_min || (size_max > 0 && size > size_max) {
        result.files_skipped += 1;
        return;
    }
    if !seen.insert(path.to_path_buf()) {
        return;
    }
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    result.entries.push(FileEntry {
        path: path.to_path_buf(),
        size,
        mtime_secs,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use std::fs;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dedupe-scan-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_extension_filter() {
        let set = extension_filter(&[".JPG".into(), "png".into(), " MP4 ".into()]);
        assert!(set.contains("jpg"));
        assert!(set.contains("png"));
        assert!(set.contains("mp4"));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn scan_filters_by_type_and_size() {
        let dir = tmpdir("filters");
        fs::write(dir.join("a.jpg"), vec![0u8; 100]).unwrap();
        fs::write(dir.join("b.png"), vec![0u8; 100]).unwrap();
        fs::write(dir.join("c.txt"), vec![0u8; 100]).unwrap();
        fs::write(dir.join("big.png"), vec![0u8; 5000]).unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("d.JPG"), vec![0u8; 100]).unwrap();

        let cli = Cli {
            paths: vec![dir.display().to_string()],
            max_depth: None,
            types: vec!["jpg".into()],
            ..zero_cli()
        };
        let result = scan(&cli, 0, 0);
        let mut names: Vec<String> = result
            .entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.jpg", "d.JPG"]);

        // Size filter: only files >= 1KB and <= 100KB.
        let cli = Cli {
            paths: vec![dir.display().to_string()],
            max_depth: None,
            types: vec!["png".into()],
            min_size: Some("1KB".into()),
            max_size: Some("100KB".into()),
            ..zero_cli()
        };
        let result = scan(&cli, 1_000, 102_400);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].size, 5_000);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_depth_zero_does_not_descend() {
        let dir = tmpdir("depth");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.join("root.txt"), b"x").unwrap();
        fs::write(sub.join("child.txt"), b"y").unwrap();

        let cli = Cli {
            paths: vec![dir.display().to_string()],
            max_depth: Some(0),
            ..zero_cli()
        };
        let result = scan(&cli, 0, 0);
        let names: Vec<String> = result
            .entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["root.txt"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclude_dir_prunes_subtrees() {
        let dir = tmpdir("exclude");
        fs::create_dir_all(dir.join("node_modules").join("deep")).unwrap();
        fs::write(dir.join("a.txt"), b"x").unwrap();
        fs::write(dir.join("node_modules").join("b.txt"), b"y").unwrap();
        fs::write(dir.join("node_modules").join("deep").join("c.txt"), b"z").unwrap();
        fs::write(dir.join("notes.txt"), b"w").unwrap();

        let cli = Cli {
            paths: vec![dir.display().to_string()],
            max_depth: None,
            exclude_dir: vec!["node_modules".into()],
            ..zero_cli()
        };
        let result = scan(&cli, 0, 0);
        let names: Vec<String> = result
            .entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.txt", "notes.txt"]);
        assert_eq!(result.dirs_skipped, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclude_path_filters_files() {
        let dir = tmpdir("exclude-path");
        fs::write(dir.join("report-final.txt"), b"x").unwrap();
        fs::write(dir.join("report-draft.txt"), b"y").unwrap();

        let cli = Cli {
            paths: vec![dir.display().to_string()],
            max_depth: None,
            exclude_path: vec!["draft".into()],
            ..zero_cli()
        };
        let result = scan(&cli, 0, 0);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].path.file_name().unwrap(), "report-final.txt");
        let _ = fs::remove_dir_all(&dir);
    }

    fn zero_cli() -> Cli {
        Cli {
            paths: vec![],
            max_depth: None,
            types: vec![],
            keep_smaller: false,
            delete: false,
            yes: false,
            hash: crate::cli::HashAlgo::Blake3,
            exact: false,
            similarity: 97.0,
            no_cache: false,
            min_size: None,
            max_size: None,
            exclude_dir: vec![],
            exclude_path: vec![],
            json: false,
            verbose: false,
            quiet: true,
        }
    }
}