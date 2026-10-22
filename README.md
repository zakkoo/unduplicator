# unduplicator

A copy-only tool for collecting files from several drives into one destination. It identifies duplicates by SHA-256 content, stores one copy of each unique file, and can resume interrupted work. It never changes source files.

## Install or update

Use Linux with Rust 1.85 or newer, Cargo, a C compiler, and ExifTool on `PATH`. On Omarchy, install the prerequisites with:

```sh
omarchy pkg add base-devel rust perl-image-exiftool
```

From this repository, run:

```sh
./scripts/install.sh
unduplicator --version
```

The script runs the full test suite, builds a production release, and installs `unduplicator` in `~/.local/bin`. Run the same script again to update an existing installation. It replaces the installed binary only after testing and building succeed. If `~/.local/bin` is not in your `PATH`, the script tells you how to add it. The current version is `0.2.2`.

## Before importing

Keep the catalog and its `.lock` file on reliable storage. Put the source, destination, catalog, and staging area at separate, non-overlapping paths. Keep the source unchanged until the import finishes.

## Commands at a glance

| Command | What it is for |
| --- | --- |
| `import` | Start a new source-drive import; `--verify` checks destination hashes after copying. |
| `resume` | Continue an interrupted or failed run using its saved settings; add `--verify` to check the result. `--verify` is not a saved setting: pass it on every invocation that should verify. |
| `verify` | Start a fresh, full destination hash check for a completed run. |
| `runs` | List run IDs, phases, source paths, and saved-report times. |
| `report` | Show a short saved review; add `--full` for the detailed report. Neither reads the drives again. |
| `--version` | Show the installed program version. |
| `--help` | Show general help; use `import --help`, `resume --help`, and so on for options. |

```sh
unduplicator --help
unduplicator import --help
```

### Start an import

Change the four paths to your own. The staging budget is in bytes; `1073741824` is 1 GiB.

```sh
unduplicator import \
  --source /mnt/source \
  --destination /mnt/destination \
  --catalog /ssd/catalog.sqlite \
  --staging /ssd/staging \
  --staging-budget 1073741824 \
  --verify
```

The program prints a **Run ID**. Save it for the commands below. To import another source drive, run `import` again with that source and the same destination and catalog. The tool will reuse content already in the catalog.

### How files are placed

- **Identify:** Inventory the source and hash every file with SHA-256. Duplicate content reuses its stored copy, path, and date.
- **Place:** Images and videos go under `Media/YYYY/YYYY-MM/YYYY-MM-DD/`; other files go under `Other/`. Name clashes get a hash suffix.
- **Protect:** Source files stay untouched; existing destination files are never overwritten. Symlinked destination directories are rejected.

For media, use the first valid date in this order:

| Priority | Date source |
| --- | --- |
| 1 | Embedded EXIF capture, XMP creation, QuickTime creation, then other embedded creation dates. |
| 2 | Date in the filename. |
| 3 | No usable date: `Media/Unknown/<filename-pattern>/`. |

Dates must be between January 1, 1900 and tomorrow (UTC); unusable dates are
silently skipped. Filename matching excludes UUIDs and dates embedded in
alphanumeric identifiers. Source and staging filesystem times are never used
to infer capture dates.

Unknown files are grouped by their filename pattern: number runs become `#`,
letters are uppercased, and extensions are ignored. For example, `IMG_1234.jpg`
and `img_56789.png` share `Media/Unknown/IMG_#/`, while
`00011101_00005341.jpg` goes under `Media/Unknown/#_#/`. UUID and hash names use
`UUID/` and `Hash/`; names without a numeric pattern use `Other/`.

### Continue a run

Use this after fixing the cause of an interruption or failed copy. Replace `1` with the printed Run ID.

```sh
unduplicator resume --catalog /ssd/catalog.sqlite --run 1 --verify
```

`--verify` applies only to the invocation it is given to. An `import --verify` that was interrupted and then resumed without `--verify` finishes the copy but skips verification, and the report shows REVIEW. Run `verify` afterwards to get PASS.

### Check destination files again

This rereads every unique destination file required by the run and compares its size and SHA-256 hash with the catalog. It can take as long as a full read of that content.

```sh
unduplicator verify --catalog /ssd/catalog.sqlite --run 1
```

### Find a run and read its review

- `runs` finds a forgotten Run ID.
- `report --full` shows paths, current file errors, latest verification failures, date sources, and completion counts. It needs only the catalog, so you can inspect failures later.
- Reports are snapshots, not fresh checks. Resume or verify replaces the previous report; after a crash without a saved report, `report` builds one from the catalog.

```sh
unduplicator runs --catalog /ssd/catalog.sqlite
unduplicator report --catalog /ssd/catalog.sqlite --run 1
unduplicator report --catalog /ssd/catalog.sqlite --run 1 --full
```

## Reading the result

The default summary shows elapsed time, files found, destination content ready, files copied during the last invocation, source duplicates, current errors, and the verdict. A verdict gives the result and its reasons:

- **PASS:** The inventory and content counts reconcile, all inventoried files are complete, the latest destination verification passed, and no entries were skipped.
- **REVIEW:** The run has no passing verification yet, work remains unfinished, or symlinks or special files were skipped. Read the reasons and decide whether those entries were expected.
- **FAIL:** The run reported an error, destination verification failed, or recorded counts do not reconcile. Read the full report and resolve the cause.

| Situation | Next step |
| --- | --- |
| Import or copy unfinished | Read `report --full`, fix the cause, then run `resume --verify`. |
| Verification failed after copying | Restore the affected destination file to the expected content, then run `verify`. |
| Files may have changed since the report | Run `verify` for a fresh destination check. |

Exit status `1` means work or verification failed, `2` means invalid arguments or paths, and `130` means interruption.

## More detail

See [design and operating notes](docs/design.md) for the import sequence, storage layout, recovery guarantees, media date rules, and performance behavior. See [test coverage](tests/README.md) for the existing test suites and their limits.
