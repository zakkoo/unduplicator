# Spec Delta

## Purpose

Organize imported content into predictable media date directories and prevent filename collisions from overwriting files.

## ADDED Requirements

### Requirement: Media classification
The tool SHALL recognize image extensions .jpg, .jpeg, .png, .heic, .heif, .gif, .bmp, .tiff, .tif, .webp and video extensions .mp4, .mov, .avi, .mkv, .wmv, .m4v, .3gp, .flv case-insensitively. Other extensions SHALL be classified as non-media.

#### Scenario: Supported and other extensions
- **WHEN** a JPG, heic, MKV, or txt file is processed
- **THEN** the first three are media and the txt file belongs under Other

### Requirement: Date selection
The tool SHALL select the first valid date from media metadata, filename, original source filesystem timestamps, then Unknown. Normal imports SHALL extract media metadata from unique staged content. Invalid or missing metadata SHALL permit fallback, and staging timestamps SHALL never substitute for original timestamps.

#### Scenario: Metadata takes priority
- **WHEN** valid media metadata disagrees with the filename and original filesystem date
- **THEN** the media metadata date determines the directory

#### Scenario: Unusable metadata
- **WHEN** media metadata is absent or malformed
- **THEN** the tool uses a valid filename date, otherwise an original source timestamp, otherwise Unknown

### Requirement: Destination layout
The tool SHALL place dated media under Media/YYYY/YYYY-MM/YYYY-MM-DD/, undated media under Media/Unknown/, and other files under Other/, relative to the selected destination root.

#### Scenario: Dated and undated media
- **WHEN** media resolves to 2020-02-29 or has no valid date
- **THEN** its directory is Media/2020/2020-02/2020-02-29/ or Media/Unknown/ respectively

### Requirement: Deterministic representatives and collisions
The tool SHALL choose duplicate representatives and destination names independently of worker completion order. Different content with the same destination name SHALL receive a deterministic SHA-256 suffix without overwriting any existing file.

#### Scenario: Equal names with different content
- **WHEN** two unique files resolve to the same directory and basename
- **THEN** both survive and the colliding file receives a stable digest-derived suffix

#### Scenario: Worker timing changes
- **WHEN** the same recorded inventory and catalog are processed with different worker completion timings or resumed
- **THEN** representative choices and intended destination paths remain the same
