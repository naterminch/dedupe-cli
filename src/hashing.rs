use crate::cli::HashAlgo;
use crate::scan::FileEntry;
use anyhow::Result;
use indicatif::ProgressBar;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

/// Number of leading bytes used for the cheap partial-hash pre-filter.
/// Files that share size and partial hash are then fully hashed.
const PARTIAL_BYTES: u64 = 64 * 1024;
const IO_CHUNK: usize = 256 * 1024;

/// Wraps a hash algorithm and knows how to hash files, partially or fully.
pub struct HashEngine {
    algo: HashAlgo,
}

impl HashEngine {
    pub fn new(algo: HashAlgo) -> Self {
        Self { algo }
    }

    pub fn name(&self) -> &'static str {
        match self.algo {
            HashAlgo::Blake3 => "blake3",
            HashAlgo::Sha256 => "sha256",
            HashAlgo::Md5 => "md5",
        }
    }

    fn digest_bytes(&self, data: &[u8]) -> String {
        match self.algo {
            HashAlgo::Blake3 => blake3::hash(data).to_hex().to_string(),
            HashAlgo::Sha256 => {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(data);
                format!("{:x}", hasher.finalize())
            }
            HashAlgo::Md5 => {
                use md5::{Digest, Md5};
                let mut hasher = Md5::new();
                hasher.update(data);
                format!("{:x}", hasher.finalize())
            }
        }
    }

    fn hash_reader<R: Read>(&self, mut reader: R) -> io::Result<String> {
        let mut buf = [0u8; IO_CHUNK];
        match self.algo {
            HashAlgo::Blake3 => {
                let mut hasher = blake3::Hasher::new();
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                Ok(hasher.finalize().to_hex().to_string())
            }
            HashAlgo::Sha256 => {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            }
            HashAlgo::Md5 => {
                use md5::{Digest, Md5};
                let mut hasher = Md5::new();
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            }
        }
    }

    /// Hash the first `PARTIAL_BYTES` of a file.
    pub fn partial(&self, path: &Path) -> io::Result<String> {
        let file = File::open(path)?;
        let mut data = Vec::with_capacity(PARTIAL_BYTES as usize);
        file.take(PARTIAL_BYTES).read_to_end(&mut data)?;
        Ok(self.digest_bytes(&data))
    }

    /// Hash the entire file.
    pub fn full(&self, path: &Path) -> io::Result<String> {
        self.hash_reader(BufReader::new(File::open(path)?))
    }
}

/// One set of files proven (by full hash) to be identical.
#[derive(Debug)]
pub struct DuplicateGroup {
    pub hash: String,
    pub members: Vec<FileEntry>,
}

/// The full detection pipeline:
/// 1. group candidate files by size,
/// 2. partial-hash the candidates in parallel and keep only same-partial groups,
/// 3. full-hash the survivors in parallel; identical full hashes are duplicates.
pub fn find_duplicate_groups(
    entries: Vec<FileEntry>,
    engine: &HashEngine,
    progress: Option<&ProgressBar>,
) -> Result<Vec<DuplicateGroup>> {
    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    for entry in entries {
        by_size.entry(entry.size).or_default().push(entry);
    }
    let candidate_groups: Vec<Vec<FileEntry>> = by_size
        .into_values()
        .filter(|g| g.len() >= 2)
        .collect();

    let total_candidates: usize = candidate_groups.iter().map(Vec::len).sum();
    if let Some(bar) = progress {
        bar.set_length(total_candidates as u64);
        bar.set_message("hashing (partial)");
    }

    // Stage 2: partial hashes.
    struct PartialBucket {
        files: Vec<FileEntry>,
    }
    let buckets: Vec<PartialBucket> = candidate_groups
        .par_iter()
        .flat_map(|group| {
            let mut by_partial: HashMap<String, Vec<FileEntry>> = HashMap::new();
            for entry in group {
                if let Ok(h) = engine.partial(&entry.path) {
                    by_partial.entry(h).or_default().push(entry.clone());
                }
            }
            if let Some(bar) = progress {
                bar.inc(group.len() as u64);
            }
            by_partial
                .into_iter()
                .filter(|(_, files)| files.len() >= 2)
                .map(|(_, files)| PartialBucket { files })
                .collect::<Vec<_>>()
        })
        .collect();

    // Stage 3: full hashes on partial survivors. Reset the bar so progress
    // stays within 0..len instead of counting files twice against one length.
    let survivors: usize = buckets.iter().map(|b| b.files.len()).sum();
    if let Some(bar) = progress {
        bar.set_position(0);
        bar.set_length(survivors as u64);
        bar.set_message("hashing (full)");
    }
    let groups: Vec<DuplicateGroup> = buckets
        .par_iter()
        .flat_map(|bucket| {
            let mut by_full: HashMap<String, Vec<FileEntry>> = HashMap::new();
            for entry in &bucket.files {
                if let Ok(h) = engine.full(&entry.path) {
                    by_full.entry(h).or_default().push(entry.clone());
                }
            }
            if let Some(bar) = progress {
                bar.inc(bucket.files.len() as u64);
            }
            by_full
                .into_iter()
                .filter(|(_, files)| files.len() >= 2)
                .map(|(hash, files)| DuplicateGroup { hash, members: files })
                .collect::<Vec<_>>()
        })
        .collect();

    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dedupe-hash-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(path: std::path::PathBuf, size: u64) -> FileEntry {
        FileEntry {
            path,
            size,
            mtime_secs: None,
        }
    }

    #[test]
    fn pipeline_finds_exact_duplicates_only() {
        let dir = tmpdir("pipeline");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let c = dir.join("c.txt");
        fs::write(&a, b"same content here").unwrap();
        fs::write(&b, b"same content here").unwrap();
        fs::write(&c, b"different content!").unwrap();
        // Bigger file sharing only the prefix must NOT match a.
        let d = dir.join("d.txt");
        fs::write(&d, b"same content here but longer and different").unwrap();

        let entries = vec![
            entry(a.clone(), fs::metadata(&a).unwrap().len()),
            entry(b.clone(), fs::metadata(&b).unwrap().len()),
            entry(c.clone(), fs::metadata(&c).unwrap().len()),
            entry(d.clone(), fs::metadata(&d).unwrap().len()),
        ];
        let engine = HashEngine::new(HashAlgo::Blake3);
        let groups = find_duplicate_groups(entries, &engine, None).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 2);
        let mut paths: Vec<&std::path::Path> = groups[0].members.iter().map(|m| m.path.as_path()).collect();
        paths.sort();
        assert_eq!(paths, vec![a.as_path(), b.as_path()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_hash_equals_full_for_small_files() {
        let dir = tmpdir("partial");
        let f = dir.join("small.txt");
        fs::write(&f, b"tiny").unwrap();
        let engine = HashEngine::new(HashAlgo::Blake3);
        assert_eq!(engine.partial(&f).unwrap(), engine.full(&f).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_bar_position_never_exceeds_length() {
        // Regression: every candidate survives partial hashing (identical
        // small files), which used to tick the bar in BOTH stages against a
        // single length — 8 + 8 = 16/8. The bar must reset between stages so
        // position never exceeds length.
        let dir = tmpdir("bar");
        let mut entries = Vec::new();
        for i in 0..8 {
            let f = dir.join(format!("f{i}.txt"));
            fs::write(&f, b"same content").unwrap();
            entries.push(entry(f, 12));
        }
        let engine = HashEngine::new(HashAlgo::Blake3);
        let bar = indicatif::ProgressBar::new(0);
        let groups = find_duplicate_groups(entries, &engine, Some(&bar)).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 8);
        assert!(
            bar.position() <= bar.length().unwrap_or(0),
            "bar overflowed: position {} > length {:?}",
            bar.position(),
            bar.length()
        );

        let _ = fs::remove_dir_all(&dir);
    }
}