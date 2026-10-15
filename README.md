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

The script runs the full test suite, builds a production release, and installs `unduplicator` in `~/.local/bin`. Run the same script again to update an existing installation. It replaces the installed binary only after testing and building succeed. If `~/.local/bin` is not in your `PATH`, the script tells you how to add it. The current version is `0.1.1`.

## Before importing

Keep the catalog and its `.lock` file on reliable storage. Put the source, destination, catalog, and staging area at separate, non-overlapping paths. Keep the source unchanged until the import finishes.

## Commands at a glance

| Command | What it is for |
| --- | --- |
| `import` | Start a new source-drive import; `--verify` checks destination hashes after copying. |
| `resume` | Continue an interrupted or failed run using its saved settings; add `--verify` to check the result. |
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

### Continue a run

Use this after an interruption or a failed copy. Replace `1` with the printed Run ID.

```sh
unduplicator resume --catalog /ssd/catalog.sqlite --run 1 --verify
```

### Check destination files again

This rereads every unique destination file required by the run and compares its size and SHA-256 hash with the catalog. It can take as long as a full read of that content.

```sh
unduplicator verify --catalog /ssd/catalog.sqlite --run 1
```

### Find a run and read its review

`runs` is useful when you forgot a Run ID. `report` displays the saved review; it does **not** perform a new verification.

```sh
unduplicator runs --catalog /ssd/catalog.sqlite
unduplicator report --catalog /ssd/catalog.sqlite --run 1
unduplicator report --catalog /ssd/catalog.sqlite --run 1 --full
```

## Reading the result

The default summary shows elapsed time, files found, destination content ready, files copied during the last invocation, source duplicates, current errors, and the verdict. A verdict gives the result and its reasons:

- **PASS:** The inventory and content counts reconcile, all inventoried files are complete, the latest destination verification passed, and no entries were skipped.
- **REVIEW:** The run has no passing verification yet, work remains unfinished, or symlinks or special files were skipped. Read the reasons and decide whether those entries were expected.
- **FAIL:** The run reported an error, destination verification failed, or recorded counts do not reconcile. Fix the cause and resume or verify again.

The copied-file count and written bytes cover the last invocation only, so they reset on resume or verify. Older saved reports include written bytes but lack a copied-file count. Content groups without an earlier-run match are a run-level catalog comparison, not a copy history. The `--full` view gives paths, breakdowns, recorded phase, and reconciliation details.

A report is a saved catalog snapshot. If files could have changed since its timestamp, run `verify` for a fresh check. A zero exit status means the requested command finished; exit status `1` means work or verification failed, `2` means invalid arguments or paths, and `130` means interruption.

## More detail

See [design and operating notes](docs/design.md) for the import sequence, storage layout, recovery guarantees, media date rules, and performance behavior. See [test coverage](tests/README.md) for the existing test suites and their limits.
