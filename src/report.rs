use crate::matching::Group;
use crate::media::MediaKind;
use crate::util::human_bytes;
use console::style;
use serde::Serialize;

const DIVIDER: &str = "────────────────────────────────────────────────";

pub struct ReportContext<'a> {
    pub paths: &'a [String],
    pub hash_algo: &'a str,
    pub verbose: bool,
    pub ffprobe_used: bool,
}

/// Aggregate counters captured from the scan phase (the entry list itself is
/// consumed by the hashing pipeline).
pub struct ScanStats {
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub dirs_skipped: u64,
    pub files_skipped: u64,
    pub dir_read_errors: Vec<String>,
}

pub fn group_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Other => "file",
    }
}

pub fn dup_file_count(groups: &[Group]) -> u64 {
    groups
        .iter()
        .map(|g| (g.members.len() - g.keep_count()) as u64)
        .sum()
}

pub fn reclaimable_bytes(groups: &[Group]) -> u64 {
    groups.iter().map(Group::dup_bytes).sum()
}

pub fn print_human(ctx: &ReportContext, stats: &ScanStats, groups: &[Group]) {
    let reclaim = reclaimable_bytes(groups);
    let dups = dup_file_count(groups);

    println!();
    let summary = format!(
        "Scanned {} path(s) · {} files ({}) · {} duplicate group(s) · {} duplicate file(s)",
        ctx.paths.len(),
        stats.files_scanned,
        human_bytes(stats.bytes_scanned),
        groups.len(),
        dups,
    );
    println!("{}", style(summary).bold());
    if reclaim > 0 {
        println!(
            "{}",
            style(format!("Reclaimable with --delete: {}", human_bytes(reclaim)))
                .yellow()
                .bold()
        );
    }
    if stats.dirs_skipped > 0 || stats.files_skipped > 0 {
        println!(
            "{}",
            style(format!(
                "Skipped {} file(s) and {} directory tree(s) via filters.",
                stats.files_skipped, stats.dirs_skipped
            ))
            .dim()
        );
    }
    if !stats.dir_read_errors.is_empty() {
        println!(
            "{} {}",
            style("⚠").yellow(),
            style(format!(
                "{} path(s) could not be read (first: {})",
                stats.dir_read_errors.len(),
                stats.dir_read_errors.first().unwrap_or(&String::new())
            ))
            .yellow()
        );
    }
    if !ctx.ffprobe_used {
        println!(
            "{}",
            style(
                "Note: ffprobe not found on PATH; image/video resolution and duration are not reported. Install FFmpeg to enable media metadata."
            )
            .dim()
        );
    }

    if groups.is_empty() {
        println!();
        println!("{}", style("✔ No duplicate files found.").green().bold());
        return;
    }

    for group in groups {
        let short_hash = &group.hash[..group.hash.len().min(12)];
        let keeper_media = group.members.first().and_then(|m| m.media.clone());

        println!();
        println!("{}", style(DIVIDER).dim());
        let kind = style(group_kind_name(group.media_kind).to_uppercase())
            .magenta()
            .bold();
        // Similar groups are identified by their similarity %, exact groups by
        // the content hash.
        let ident = match group.similarity {
            Some(s) => style(format!("{:.1}% similar", s * 100.0))
                .yellow()
                .bold()
                .to_string(),
            None => format!("{} {short_hash}", ctx.hash_algo),
        };
        println!(
            "{} {} · {kind} · {} files · {} each · {} reclaimable · {ident}",
            style("◆").cyan().bold(),
            style(format!("Group #{}", group.index)).cyan().bold(),
            group.members.len(),
            human_bytes(group.members[0].size),
            human_bytes(group.dup_bytes()),
        );
        if let Some(info) = &keeper_media {
            println!(
                "{} {}",
                style("  ↳").dim(),
                style(info.summary()).dim()
            );
        }

        // Align the size column across the group's members.
        let path_w = group
            .members
            .iter()
            .map(|m| m.path.display().to_string().chars().count())
            .max()
            .unwrap_or(0);
        for member in &group.members {
            let (glyph, badge, color) = if member.keep {
                ("✓", "KEEP", console::Style::new().green())
            } else {
                ("✗", "DUP", console::Style::new().yellow())
            };
            let mut line = format!(
                "  {} {}  {:<width$}  {}",
                color.apply_to(glyph),
                color.bold().apply_to(badge),
                member.path.display(),
                style(human_bytes(member.size)).magenta(),
                width = path_w,
            );
            if let Some(sim) = member.similarity
                && !member.keep
            {
                line.push_str(&format!(
                    "  {} {}",
                    style("·").dim(),
                    style(format!("{:.1}% similar", sim * 100.0)).dim()
                ));
            }
            if ctx.verbose
                && let Some(info) = &member.media
            {
                let mut extra = info.summary();
                if let Some(codec) = &info.codec {
                    extra.push_str(&format!(", {codec}"));
                }
                line.push_str(&format!("  {} {}", style("·").dim(), style(extra).dim()));
            }
            println!("{line}");
        }
    }

    if dups > 0 {
        println!();
        println!(
            "{} run with {} to remove the {} duplicate file(s), or {} to prefer the smallest copy.",
            style("Tip:").cyan().bold(),
            style("--delete").green(),
            dups,
            style("--delete --keep-smaller").green(),
        );
    }
}

#[derive(Serialize)]
struct JsonReport<'a> {
    hash_algorithm: &'a str,
    paths: &'a [String],
    files_scanned: usize,
    bytes_scanned: u64,
    dirs_skipped: u64,
    files_skipped: u64,
    dir_read_errors: &'a [String],
    duplicate_groups: usize,
    duplicate_files: u64,
    reclaimable_bytes: u64,
    ffprobe_used: bool,
    groups: &'a [Group],
}

pub fn print_json(ctx: &ReportContext, stats: &ScanStats, groups: &[Group]) {
    let report = JsonReport {
        hash_algorithm: ctx.hash_algo,
        paths: ctx.paths,
        files_scanned: stats.files_scanned,
        bytes_scanned: stats.bytes_scanned,
        dirs_skipped: stats.dirs_skipped,
        files_skipped: stats.files_skipped,
        dir_read_errors: &stats.dir_read_errors,
        duplicate_groups: groups.len(),
        duplicate_files: dup_file_count(groups),
        reclaimable_bytes: reclaimable_bytes(groups),
        ffprobe_used: ctx.ffprobe_used,
        groups,
    };
    let json = serde_json::to_string(&report).expect("report serialization cannot fail");
    println!("{json}");
}