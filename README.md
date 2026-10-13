# unduplicator

A bulletproof, resumable CLI tool that organizes files from a slow external HDD into a clean folder structure — with intelligent duplicate detection, full error resilience, and SQLite-backed progress tracking so interrupted runs pick up exactly where they left off.

## destination
-- media files go into media folder
-- other files go to other folder

## media folder organisation
-- example /2022/2022-10/2022-10-31/file.jpg 

## other folder
-- goes just in there /

## file classification

### Supported formats

| Type   | Extensions |
|--------|-----------|
| media:Images | `.jpg` `.jpeg` `.png` `.heic` `.heif` `.gif` `.bmp` `.tiff` `.tif` `.webp` |
| media:Videos | `.mp4` `.mov` `.avi` `.mkv` `.wmv` `.m4v` `.3gp` `.flv` |
| other  | everything else → `/Other/` |


## Key features

    Date-based folder structure — YYYY/YYYY-MM/YYYY-MM-DD/ using the actual capture date
    Smart date extraction — reads EXIF/video metadata first, falls back to filename patterns, then file timestamps
    Intelligent duplicate detection — three-tier strategy (size → partial hash → full SHA256) so only colliding files are hashed
    Fully resumable
    Copy-only — source files are never modified or deleted; the destination is built separately
    Per-file error isolation — one bad file never stops the run; errors are recorded in     
    Non-media files — anything not a recognized image or video goes to /Other/
    Unknown dates (only for media) — files with no extractable date go to /Unknown/

## How date extraction works

For each file, the following sources are tried in order — stopping at the first success:

| Priority | Source | Example |
|----------|--------|---------|
| 1 | EXIF `DateTimeOriginal` or `DateTimeDigitized` (images) | JPEG, HEIC, PNG |
| 2 | Video container creation date | MP4, MOV |
| 3 | Filename pattern | `IMG_20231015_143022.jpg`, `Screenshot_20231015-143022.png`, `2023-10-15 12.34.56.jpg` |
| 4 | File system creation timestamp | if after 1990-01-01 |
| 5 | File system last-modified timestamp | if after 1990-01-01 |
| — | No date found | → placed in `/Unknown/` |


## Design decisions

**Why SQLite?** 100k+ files need durable, queryable state. In-memory state is lost on crash; a text log can't be queried. SQLite gives atomic writes, indexed lookups by status/hash/path, and requires no server setup. WAL mode allows the status command to read while a phase is writing.

**Why copy instead of move?** Copying is safe and idempotent. If something goes wrong — power failure, disk full, a bug — your source is completely untouched.

**Why three-tier hashing?** On a slow spinning HDD, reading and hashing every file is prohibitively slow. Files with unique sizes are provably unique with zero reads. Only the minority of size-collision candidates need any I/O for deduplication.

**Why three phases?** A single streaming pass would require reading content while walking directories and mixing concerns. Separating scan (directory entries only), analyze (date extraction + duplicate analysis with bounded hash parallelism), and organize (copying to final paths) keeps each phase resumable and tunable for different storage speeds.
