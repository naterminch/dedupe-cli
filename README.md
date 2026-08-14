# dedupe

A small tool that finds duplicate files for you.

Point it at a folder and it scans everything inside, including subfolders.
It finds two kinds of duplicates:

- **Exact duplicates** — files that are the same, byte for byte. A copy, a
  rename, a backup — gone.
- **Similar duplicates** (on by default) — the same photo saved as `.png`
  *and* `.jpg`, or the same video saved as `.mp4` *and* `.mov`. The files are
  different, but the picture you see is the same. Turn this off with `--exact`.

One single binary. No install, no setup. Works on Windows, macOS and Linux.

## Build

You need the Rust toolchain (https://rustup.rs).

```sh
# Windows (PowerShell) — builds the release binary and copies it to the project root:
.\build.ps1

# Any OS — manual build:
cargo build --release
# binary: target/release/dedupe
```

Optional: install [FFmpeg](https://ffmpeg.org). Duplicate detection works
without it — FFmpeg only adds extra media details (resolution, duration,
codec) and enables video comparison.

## Use

```
dedupe [OPTIONS] <PATH>...
```

You must give it a path. Run `dedupe .` to scan the current folder. Running
the binary with no path just prints the help.

### Options

| Flag | What it does |
| --- | --- |
| `-t, --types EXTS` | Scan only these extensions, comma-separated and case-insensitive, e.g. `--types jpg,png,mp4,txt` |
| `-k, --keep-smaller` | Mark the smallest file in each duplicate group as the keeper; for similar media, keep the **highest-resolution** version instead |
| `-D, --delete` | Delete the duplicate files. Prompts per group unless `-y` |
| `-y, --yes` | Assume "yes" for all deletion prompts |
| `--hash ALGO` | Hash algorithm: `blake3` (default), `sha256`, `md5` |
| `--exact` | Only byte-identical duplicates. Similar images/videos (same content in a different format, e.g. `.png` vs `.jpg`, `.mp4` vs `.mov`) are detected **by default**; this flag disables that pass |
| `--similarity PCT` | Similarity threshold (0-100) above which similar media counts as duplicated (default: 97) |
| `--no-cache` | Do not read or write the fingerprint cache (`~/.dedupe/fingerprints.bin`) — recompute fingerprints from scratch |
| `--min-size SIZE` / `--max-size SIZE` | Ignore files outside this size range (`100KB`, `2MB`, `1GiB`, ...) |
| `--max-depth N` | Limit recursion depth (`0` = files directly in the given paths only) |
| `--exclude-dir NAME` | Prune directories whose name contains this substring (repeatable) |
| `--exclude-path SUBSTR` | Skip files whose full path contains this substring (repeatable) |
| `-j, --json` | Machine-readable JSON output |
| `-v, --verbose` | Per-file media metadata (resolution, duration, codec) |
| `-q, --quiet` | Suppress progress output |

### Examples

```sh
# Find all duplicates under /media, subfolders included
dedupe /media

# Only images and videos
dedupe /media --types jpg,png,mp4,mkv

# Find duplicates, keeping the smallest copy, deleting the rest without prompts
dedupe /media --keep-smaller --delete --yes

# Show resolution/duration for media duplicates
dedupe /media --types jpg,mp4 --verbose

# Skip huge or tiny files, ignore node_modules
dedupe /projects --min-size 1KB --max-size 500MB --exclude-dir node_modules

# Scriptable output
dedupe /media --json

# Same photo saved as both PNG and JPG (or a video re-encoded as .mov/.avi) —
# similar-duplicate detection is ON by default
dedupe /media

# Only exact (byte-identical) duplicates
dedupe /media --exact

# Tighter/looser similarity threshold
dedupe /media --similarity 98
```

### The report

```
Scanned 1 path(s) · 4 files (128.3 KB) · 1 duplicate group(s) · 2 duplicate file(s)
Reclaimable with --delete: 62.10 KB

────────────────────────────────────────────────────────────
◆ Group #1 · VIDEO · 2 files · 62.10 KB each · 62.10 KB reclaimable · blake3 69d57daf8d78
  ↳ 1280x720 · 4s · h264
  ✓ KEEP  /media/clip-backup.mp4                    62.10 KB
  ✗ DUP   /media/clip.mp4                           62.10 KB

Tip: run with --delete to remove the 2 duplicate file(s), or --delete --keep-smaller to prefer the smallest copy.
```

`✓ KEEP` is the file that stays. `✗ DUP` is the file that `--delete` would
remove. Without `--keep-smaller`, the first path in the group is the keeper.
Colors show only on a terminal (respecting `NO_COLOR`); piped or `--json`
output stays plain.

## How it finds duplicates

1. **Group by size.** Only files of the same size can be duplicates.
2. **Hash.** Files with the same hash are exact duplicates.
3. **For images and videos, compare what you see, not the bytes:**
   - images get a fingerprint of their brightness pattern,
   - videos get fingerprints of 8 frames, spread evenly over the video.
   A fingerprint match of 97% or more counts as a duplicate (tune with
   `--similarity`).
4. **Remember between runs.** Fingerprints are saved in a cache, so the next
   scan of the same folders is much faster. A cached result is reused only
   while the file's size and modification time are unchanged, so it never
   goes stale. `--no-cache` skips it.

## Tests

```sh
cargo test         # unit + integration tests
cargo clippy --all-targets
```

## License

MIT