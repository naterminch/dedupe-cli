use crate::hashing::HashEngine;
use crate::matching::Group;
use crate::util::human_bytes;
use anyhow::Result;
use console::style;
use std::fs;

#[derive(Debug, Default)]
pub struct DeletionStats {
    pub files_deleted: u64,
    pub bytes_freed: u64,
    pub files_skipped: u64,
}

/// Delete every non-keep member of each group.
///
/// Safety: every file is re-hashed immediately before deletion and skipped
/// (with a warning) if it no longer matches the group's hash, so a file that
/// changed between the scan and the delete is never removed.
///
/// Unless `yes` is set, the user is prompted per group:
///   y = delete this group, a = delete this and all remaining, q = quit.
pub fn delete_duplicates(groups: &[Group], engine: &HashEngine, yes: bool) -> Result<DeletionStats> {
    let mut stats = DeletionStats::default();
    let mut assume_yes = yes;

    'groups: for group in groups {
        let targets: Vec<usize> = group
            .members
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.keep)
            .map(|(i, _)| i)
            .collect();
        if targets.is_empty() {
            continue;
        }
        let bytes: u64 = targets.iter().map(|&i| group.members[i].size).sum();

        if !assume_yes {
            let sim = group
                .similarity
                .map(|s| format!(" ({:.1}% similar)", s * 100.0))
                .unwrap_or_default();
            let answer = prompt(&format!(
                "{} Delete {} file(s) ({} reclaimable{sim}) from group #{}? {} ",
                style("?").yellow().bold(),
                targets.len(),
                human_bytes(bytes),
                group.index,
                style("[y/n/a/q]").dim(),
            ));
            match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => {}
                "a" | "all" => assume_yes = true,
                "q" | "quit" => break 'groups,
                _ => continue,
            }
        }

        for &i in &targets {
            let member = &group.members[i];
            // Similar groups share no content hash, so each file is verified
            // against its own hash recorded during the scan.
            let expected = member
                .content_hash
                .as_deref()
                .or(Some(group.hash.as_str()));
            match engine.full(&member.path) {
                Ok(h) if expected == Some(h.as_str()) => match fs::remove_file(&member.path) {
                    Ok(()) => {
                        stats.files_deleted += 1;
                        stats.bytes_freed += member.size;
                    }
                    Err(e) => {
                        eprintln!(
                            "{} {}: {}",
                            style("⚠").yellow(),
                            style("cannot delete").yellow().bold(),
                            style(format!("{} ({e})", member.path.display())).dim()
                        );
                        stats.files_skipped += 1;
                    }
                },
                Ok(_) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("changed since the scan, skipping (not deleted)").yellow().bold(),
                        style(member.path.display()).dim()
                    );
                    stats.files_skipped += 1;
                }
                Err(e) => {
                    eprintln!(
                        "{} {}: {}",
                        style("⚠").yellow(),
                        style("cannot re-hash, skipping").yellow().bold(),
                        style(format!("{} ({e})", member.path.display())).dim()
                    );
                    stats.files_skipped += 1;
                }
            }
        }
    }

    Ok(stats)
}

fn prompt(message: &str) -> String {
    use std::io::Write;
    print!("{message}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line
}