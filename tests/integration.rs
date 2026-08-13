use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dedupe-e2e-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (std::process::Output, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .args(args)
        .output()
        .expect("failed to run dedupe binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    (out, stdout)
}

fn run_with_env(args: &[&str], envs: &[(&str, &str)]) -> (std::process::Output, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dedupe"));
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to run dedupe binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    (out, stdout)
}

#[test]
fn finds_duplicates_recursively_including_subfolders() {
    let dir = tmpdir("recursive");
    fs::create_dir_all(dir.join("sub").join("deeper")).unwrap();
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("sub").join("b.txt"), b"payload").unwrap();
    fs::write(dir.join("sub").join("deeper").join("c.txt"), b"payload").unwrap();
    fs::write(dir.join("unique.txt"), b"totally unique").unwrap();

    let (out, stdout) = run(&["--json", dir.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["duplicate_groups"], 1);
    assert_eq!(parsed["duplicate_files"], 2);
    assert_eq!(parsed["groups"][0]["members"].as_array().unwrap().len(), 3);
    assert!(out.status.success());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn types_flag_limits_scan_to_given_extensions() {
    let dir = tmpdir("types");
    fs::write(dir.join("a.jpg"), b"same").unwrap();
    fs::write(dir.join("b.jpg"), b"same").unwrap();
    fs::write(dir.join("a.png"), b"same").unwrap();
    fs::write(dir.join("b.png"), b"same").unwrap();

    let (_, stdout) = run(&["--json", "--types", "jpg", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 1);
    let group = &parsed["groups"][0];
    let members = group["members"].as_array().unwrap();
    assert!(members.iter().all(|m| {
        m["path"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
            .ends_with(".jpg")
    }));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn delete_with_yes_removes_duplicates_keeps_one() {
    // Exact duplicates always share the same size, but --keep-smaller still
    // deterministically designates the keeper (unit-tested in matching.rs).
    let dir = tmpdir("deletion");
    let alpha = dir.join("alpha.txt");
    let beta = dir.join("beta.txt");
    fs::write(&alpha, b"same payload").unwrap();
    fs::write(&beta, b"same payload").unwrap();

    let (out, _) = run(&["--delete", "--yes", "--keep-smaller", "--quiet", dir.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    // Exactly one copy survives.
    assert!(alpha.exists() ^ beta.exists());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn no_duplicates_yields_empty_result() {
    let dir = tmpdir("clean");
    fs::write(dir.join("a.txt"), b"alpha").unwrap();
    fs::write(dir.join("b.txt"), b"beta").unwrap();

    let (_, stdout) = run(&["--json", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 0);
    assert_eq!(parsed["groups"].as_array().unwrap().len(), 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn human_output_shows_styled_group_report() {
    let dir = tmpdir("human");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("sub").join("b.txt"), b"payload").unwrap();
    fs::write(dir.join("unique.txt"), b"totally unique").unwrap();

    let (out, stdout) = run(&[dir.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    // Styled report markers (glyphs survive even when piped; only ANSI codes
    // are suppressed, and JSON output is never styled).
    assert!(stdout.contains("◆ Group #1"), "stdout: {stdout}");
    assert!(stdout.contains("✓ KEEP"), "stdout: {stdout}");
    assert!(stdout.contains("✗ DUP"), "stdout: {stdout}");
    assert!(stdout.contains("Tip:"), "stdout: {stdout}");
    // No ANSI escape codes when output is piped.
    assert!(!stdout.contains('\u{1b}'), "ANSI escapes leaked into piped stdout: {stdout:?}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn similar_mode_finds_same_image_in_different_formats() {
    use image::ImageBuffer;

    // The same synthetic photo saved as PNG and (lossy) JPEG — byte-identical
    // hashes will NOT match, but the perceptual dHash should be >= 97%.
    let dir = tmpdir("similar");
    let png_path = dir.join("photo.png");
    let jpg_path = dir.join("photo.jpg");
    let img: ImageBuffer<image::Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(160, 120, |x, y| {
        let c = ((x * 2 + y * 3) % 256) as u8;
        image::Rgb([c, c.wrapping_mul(2), 255 - c])
    });
    img.save(&png_path).unwrap();
    img.save(&jpg_path).unwrap();

    let (out, stdout) = run(&["--json", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        parsed["duplicate_groups"], 1,
        "expected one similar group, stdout: {stdout}"
    );
    let group = &parsed["groups"][0];
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
    assert!(
        group["similarity"].as_f64().unwrap() >= 0.97,
        "group similarity below threshold: {group}"
    );
    // Exactly one member is designated the keeper.
    let keep_count = group["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["keep"].as_bool().unwrap())
        .count();
    assert_eq!(keep_count, 1);

    // With --exact the two formats are NOT duplicates (different bytes).
    let (_, plain) = run(&["--exact", "--json", dir.to_str().unwrap()]);
    let plain: serde_json::Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(plain["duplicate_groups"], 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn similar_mode_finds_reencoded_video_in_different_container() {
    use std::process::Command as Proc;

    // Requires real ffmpeg + ffprobe to generate and fingerprint media; skip
    // cleanly on machines without them (like the image test, which runs
    // natively, the video pipeline is only exercised when the tools exist).
    fn tool(name: &str) -> bool {
        Proc::new(name)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    if !tool("ffmpeg") || !tool("ffprobe") {
        eprintln!("skipping: ffmpeg/ffprobe not on PATH");
        return;
    }

    let dir = tmpdir("similar-video");
    let mp4 = dir.join("clip.mp4");
    let mov = dir.join("clip.mov");

    // Deterministic synthetic source, then a re-encode into a different
    // container: same content, different bytes.
    let src = Proc::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=10"])
        .args(["-pix_fmt", "yuv420p", mp4.to_str().unwrap()])
        .status()
        .expect("ffmpeg source encode failed");
    assert!(src.success());
    let re = Proc::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i", mp4.to_str().unwrap()])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", mov.to_str().unwrap()])
        .status()
        .expect("ffmpeg re-encode failed");
    assert!(re.success(), "re-encode to .mov failed");

    let (out, stdout) = run(&["--json", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        parsed["duplicate_groups"], 1,
        "expected one similar video group, stdout: {stdout}"
    );
    let group = &parsed["groups"][0];
    assert_eq!(group["kind"], "video");
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
    assert!(
        group["similarity"].as_f64().unwrap() >= 0.97,
        "re-encoded video similarity below threshold: {group}"
    );
    let keep_count = group["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["keep"].as_bool().unwrap())
        .count();
    assert_eq!(keep_count, 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fingerprint_cache_is_persisted_and_reused_across_runs() {
    use std::process::Command as Proc;

    fn tool(name: &str) -> bool {
        Proc::new(name)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    if !tool("ffmpeg") || !tool("ffprobe") {
        eprintln!("skipping: ffmpeg/ffprobe not on PATH");
        return;
    }

    let dir = tmpdir("cache");
    // Keep the cache file OUTSIDE the scanned directory so it cannot change
    // the file count between runs.
    let cache_file = std::env::temp_dir().join(format!(
        "dedupe-e2e-cache-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    // Two same-resolution, same-duration clips with different content: both
    // land in the same resolution bucket and get fingerprinted (they are not
    // byte-identical, so they reach the similar pass).
    for (name, src) in [
        ("clip1.mp4", "testsrc=duration=2:size=320x240:rate=10"),
        ("clip2.mp4", "testsrc2=duration=2:size=320x240:rate=10"),
    ] {
        let out = Proc::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", src])
            .args(["-pix_fmt", "yuv420p", dir.join(name).to_str().unwrap()])
            .status()
            .expect("ffmpeg encode failed");
        assert!(out.success());
    }

    let envs = [("DEDUPE_CACHE", cache_file.to_str().unwrap())];

    // Run 1: populates the cache with two video fingerprints.
    let (out1, stdout1) = run_with_env(&["--json", dir.to_str().unwrap()], &envs);
    assert!(
        out1.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out1.stderr)
    );
    let first: serde_json::Value = serde_json::from_str(&stdout1).expect("valid JSON");
    assert_eq!(
        first["duplicate_groups"], 0,
        "different clips must not be reported as duplicates"
    );
    let bytes = fs::read(&cache_file).expect("cache file created on first run");
    assert!(
        bytes.len() > 200,
        "cache should hold two video fingerprints, got {} bytes",
        bytes.len()
    );

    // Run 2: unchanged files are served from the cache; results identical.
    let (out2, stdout2) = run_with_env(&["--json", dir.to_str().unwrap()], &envs);
    assert!(
        out2.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout2).unwrap(),
        first,
        "cached run must produce the same report"
    );

    // --no-cache ignores the cache and leaves it untouched.
    let before = fs::read(&cache_file).unwrap();
    let (out3, _) = run_with_env(
        &["--json", "--no-cache", dir.to_str().unwrap()],
        &envs,
    );
    assert!(out3.status.success());
    assert_eq!(
        fs::read(&cache_file).unwrap(),
        before,
        "--no-cache must not rewrite the cache"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bare_invocation_prints_usage_and_fails() {
    // Running the binary without a path shows how to use the tool and exits
    // with the standard usage-error code (2), instead of scanning anything.
    let out = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .output()
        .expect("failed to run dedupe binary");
    assert_eq!(out.status.code(), Some(2), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage:"), "stderr: {stderr}");
    assert!(stderr.contains("--types"), "stderr: {stderr}");
    assert!(stderr.contains("--keep-smaller"), "stderr: {stderr}");
    assert!(stderr.contains("a scan path is required"), "stderr: {stderr}");

    // `--help` still exits 0 and prints to stdout.
    let help = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .arg("--help")
        .output()
        .expect("failed to run dedupe binary");
    assert!(help.status.success());
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    assert!(help_stdout.contains("Usage:"));
}