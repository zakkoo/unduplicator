# unduplicator

A resumable, copy-only CLI that imports files from removable drives into one destination. It keeps one SHA-256-identified copy of each unique file and tracks progress in SQLite.

## Import flow

```mermaid
flowchart LR
    L1["Laptop 1<br/>File1, File2, File3"]
    L2["Laptop 2<br/>File1, File3, File4"]
    HD["External HD<br/>File1, File9"]
    D["New computer / destination<br/>File1, File2, File3, File4, File9"]
    L1 --> D
    L2 --> D
    HD --> D
```

Matching SHA-256 content is stored once, across all imports.

## Import sequence

```mermaid
sequenceDiagram
    participant U as User
    participant T as Tool
    participant S as Source
    participant D as Destination

    U->>T: Import Laptop 1
    T->>S: Scan and analyze files
    S-->>T: Metadata and candidate hashes
    T->>D: Check the catalog for matching hashes
    D-->>T: No prior matches
    T->>S: Read new unique files
    S-->>T: File1 File2 File3
    T->>D: Copy files and store SHA256
    T-->>U: Copied

    U->>T: Later import Laptop 2
    T->>S: Scan and analyze files
    S-->>T: Candidate hashes
    T->>D: Check the catalog for matching hashes
    D-->>T: File1 and File3 already exist
    T->>S: Read new unique files
    S-->>T: File4
    T->>D: Copy File4 and store SHA256
    T-->>U: Skip duplicates and copy File4

    opt Run optional verification
        T->>D: Re-read files and compare stored hashes
        D-->>T: Hash results
        T-->>U: Verified if all hashes match
    end
```

## Phases

1. **Scan** — record paths, sizes, extensions, and timestamps; do not read file content.
2. **Analyze** — extract dates and hash same-size duplicate candidates.
3. **Organize** — copy unique files and store their SHA-256 digests.
4. **Verify (optional)** — re-read destination files and compare digests. Mark a run `verified` only if every digest matches.

Keep source files unchanged during an import. The tool never moves or deletes them.

## Destination

- Media: `/Media/YYYY/YYYY-MM/YYYY-MM-DD/`
- Media with unknown dates: `/Media/Unknown/`
- Other files: `/Other/`
- Different content with the same destination name: add a deterministic SHA-256 suffix; never overwrite.

Date priority: image/video metadata, filename, filesystem timestamps, then `Unknown`. Filesystem timestamps may not be capture dates.

## Supported media

- Images: `.jpg` `.jpeg` `.png` `.heic` `.heif` `.gif` `.bmp` `.tiff` `.tif` `.webp`
- Videos: `.mp4` `.mov` `.avi` `.mkv` `.wmv` `.m4v` `.3gp` `.flv`
- Everything else goes to `/Other/`.
