use anyhow::{bail, Result};

/// Format a byte count as a human-readable string (decimal units).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Format milliseconds as `H:MM:SS` (or `M:SS` when under an hour).
pub fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = total_secs / 3600;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins}:{secs:02}")
    }
}

/// Parse a human size string like `512`, `1.5KB`, `2 MB`, `3GiB`, `4G`.
/// Decimal suffixes (KB, MB, ...) multiply by powers of 1000;
/// binary suffixes (KiB, MiB, ...) and bare letters (K, M, ...) by powers of 1024.
pub fn parse_size(raw: &str) -> Result<u64> {
    let s = raw.trim();
    if s.is_empty() {
        bail!("empty size");
    }
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num_part, suffix) = s.split_at(split);
    let value: f64 = num_part
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid size: '{raw}'"))?;
    let suffix = suffix.trim().to_ascii_uppercase();
    let multiplier: f64 = match suffix.as_str() {
        "" | "B" => 1.0,
        "KB" => 1_000.0_f64,
        "MB" => 1_000.0_f64.powi(2),
        "GB" => 1_000.0_f64.powi(3),
        "TB" => 1_000.0_f64.powi(4),
        "K" | "KIB" | "KI" => 1024.0_f64,
        "M" | "MIB" | "MI" => 1024.0_f64.powi(2),
        "G" | "GIB" | "GI" => 1024.0_f64.powi(3),
        "T" | "TIB" | "TI" => 1024.0_f64.powi(4),
        other => bail!("unknown size suffix '{other}' in '{raw}'"),
    };
    let bytes = (value * multiplier).round();
    if bytes < 0.0 || bytes > u64::MAX as f64 {
        bail!("size out of range: '{raw}'");
    }
    Ok(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500), "1.50 KB");
        assert_eq!(human_bytes(2_621_440), "2.62 MB");
    }

    #[test]
    fn parse_size_variants() {
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("1.5KB").unwrap(), 1_500);
        assert_eq!(parse_size("2 MB").unwrap(), 2_000_000);
        assert_eq!(parse_size("3GiB").unwrap(), 3 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("4G").unwrap(), 4 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("0").unwrap(), 0);
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1XB").is_err());
    }

    #[test]
    fn format_duration() {
        assert_eq!(format_duration_ms(65_000), "1:05");
        assert_eq!(format_duration_ms(3_661_000), "1:01:01");
    }
}