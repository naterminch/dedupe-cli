use crate::hashing::DuplicateGroup;
use crate::media::{self, MediaInfo, MediaKind};
use rayon::prelude::*;
use serde::Serialize;
use std::path::PathBuf;

/// A file inside a duplicate group, with its keep decision and media metadata.
#[derive(Debug, Clone, Serialize)]
pub struct GroupMember {
    pub path: PathBuf,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime_secs: Option<i64>,
    pub keep: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaInfo>,
    /// Perceptual similarity to the group keeper (0..=1). Set for groups found
    /// by `--similar`; `None` for exact-duplicate groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
    /// The member's own full content hash — used as a deletion safety check
    /// for similar groups (their bytes legitimately differ from each other).
    #[serde(skip_serializing)]
    pub content_hash: Option<String>,
}

/// A set of identical files, ordered with the KEEP member first.
#[derive(Debug, Serialize)]
pub struct Group {
    pub index: usize,
    pub hash: String,
    #[serde(rename = "kind")]
    pub media_kind: MediaKind,
    /// Worst pairwise similarity within the group (0..=1); `None` for exact
    /// groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
    pub members: Vec<GroupMember>,
}

impl Group {
    pub fn keep_count(&self) -> usize {
        self.members.iter().filter(|m| m.keep).count()
    }

    /// Bytes that would be freed by deleting every non-keep member.
    pub fn dup_bytes(&self) -> u64 {
        self.members
            .iter()
            .filter(|m| !m.keep)
            .map(|m| m.size)
            .sum()
    }
}

/// Turn raw hash groups into report-ready groups:
/// - identify the group's media kind (from the member that will be kept),
/// - probe resolution/duration for media groups (in parallel),
/// - choose which member to keep (default: lexicographically first path;
///   with `--keep-smaller`: the smallest file, ties broken by path).
pub fn assemble_groups(
    groups: Vec<DuplicateGroup>,
    keep_smaller: bool,
    probe: bool,
) -> Vec<Group> {
    let groups: Vec<(usize, Group)> = groups
        .into_par_iter()
        .enumerate()
        .map(|(i, group)| {
            let index = i + 1;
            // Keep decision before any reordering.
            let mut members: Vec<GroupMember> = group
                .members
                .into_iter()
                .map(|e| GroupMember {
                    path: e.path,
                    size: e.size,
                    mtime_secs: e.mtime_secs,
                    keep: false,
                    media: None,
                    similarity: None,
                    content_hash: None,
                })
                .collect();

            // Probe media metadata (parallel across members).
            let media_kind = members
                .first()
                .map(|m| media::classify(&m.path))
                .unwrap_or(MediaKind::Other);
            let want_probe = probe && media_kind != MediaKind::Other;
            if want_probe {
                for member in &mut members {
                    member.media = media::probe_media(&member.path);
                }
            }

            // Choose the keeper.
            let keep_idx = choose_keep(&members, keep_smaller);
            members[keep_idx].keep = true;

            // Order: KEEP first, then the rest sorted by path for determinism.
            let keeper = members.remove(keep_idx);
            let mut rest = members;
            rest.sort_by(|a, b| a.path.cmp(&b.path));
            let mut ordered = vec![keeper];
            ordered.extend(rest);

            (index, Group {
                index,
                hash: group.hash,
                media_kind,
                similarity: None,
                members: ordered,
            })
        })
        .collect();

    let mut groups: Vec<Group> = groups.into_iter().map(|(_, g)| g).collect();
    groups.sort_by(|a, b| a.hash.cmp(&b.hash));
    groups
}

fn choose_keep(members: &[GroupMember], keep_smaller: bool) -> usize {
    let mut best = 0usize;
    for (i, member) in members.iter().enumerate().skip(1) {
        let better = if keep_smaller {
            (member.size, &member.path) < (members[best].size, &members[best].path)
        } else {
            member.path < members[best].path
        };
        if better {
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::FileEntry;
    use std::fs;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dedupe-match-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_group(dir: &std::path::Path, names: &[(&str, usize)]) -> DuplicateGroup {
        let members = names
            .iter()
            .map(|(name, size)| {
                let path = dir.join(name);
                fs::write(&path, vec![0u8; *size]).unwrap();
                FileEntry {
                    path,
                    size: *size as u64,
                    mtime_secs: Some(0),
                }
            })
            .collect();
        DuplicateGroup {
            hash: "abc".into(),
            members,
        }
    }

    #[test]
    fn default_keeps_first_path() {
        let dir = tmpdir("default");
        let group = fake_group(&dir, &[("z.txt", 10), ("a.txt", 20), ("m.txt", 5)]);
        let groups = assemble_groups(vec![group], false, false);
        let members = &groups[0].members;
        assert_eq!(members.first().unwrap().path.file_name().unwrap(), "a.txt");
        assert!(members.first().unwrap().keep);
        assert_eq!(members.iter().filter(|m| m.keep).count(), 1);
        // z.txt (10) + m.txt (5) are duplicates; a.txt (20) is kept.
        assert_eq!(groups[0].dup_bytes(), 15);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_smaller_wins() {
        let dir = tmpdir("smaller");
        let group = fake_group(&dir, &[("a.txt", 10), ("b.txt", 20), ("c.txt", 2)]);
        let groups = assemble_groups(vec![group], true, false);
        let members = &groups[0].members;
        assert_eq!(members.first().unwrap().path.file_name().unwrap(), "c.txt");
        assert!(members.first().unwrap().keep);
        assert_eq!(groups[0].dup_bytes(), 30);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn media_kind_detected_from_extension() {
        let dir = tmpdir("kind");
        let group = fake_group(&dir, &[("pic.JPG", 10), ("pic2.jpg", 10)]);
        let groups = assemble_groups(vec![group], false, false);
        // Probe disabled in this test; kind still comes from the first member.
        assert_eq!(groups[0].media_kind, MediaKind::Image);
        let _ = fs::remove_dir_all(&dir);
    }
}