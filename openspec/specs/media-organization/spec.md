# media-organization Specification

## Purpose

Organize imported content into predictable media date directories and prevent filename collisions from overwriting files.

## Requirements

### Requirement: Media classification
The tool SHALL recognize image extensions .jpg, .jpeg, .png, .heic, .heif, .gif, .bmp, .tiff, .tif, .webp and video extensions .mp4, .mov, .avi, .mkv, .wmv, .m4v, .3gp, .flv case-insensitively. Other extensions SHALL be classified as non-media.

#### Scenario: Supported and other extensions
- **WHEN** a JPG, heic, MKV, or txt file is processed
- **THEN** the first three are media and the txt file belongs under Other

### Requirement: Date selection
The tool SHALL select the first plausible date from media metadata, filename, then Unknown. Normal imports SHALL extract media metadata from unique staged content. Source and staging filesystem timestamps SHALL NOT determine media dates.

#### Scenario: Metadata takes priority
- **WHEN** plausible media metadata disagrees with the filename
- **THEN** the media metadata date determines the directory

#### Scenario: Unusable metadata
- **WHEN** media metadata is absent or malformed
- **THEN** the tool uses a plausible filename date, otherwise Unknown

#### Scenario: Filesystem timestamps are available but no capture date is usable
- **WHEN** media has no plausible metadata or filename date, but has valid source birth and modification timestamps
- **THEN** its date is Unknown

### Requirement: Plausible date candidates
Dates SHALL be valid calendar dates from 1900-01-01 through tomorrow's UTC calendar date, inclusive. Invalid, implausible or missing dates SHALL silently permit fallback without rejection or uncertainty reports. Filename matching SHALL exclude UUID segments and dates embedded in alphanumeric identifiers.

#### Scenario: Implausible dates are rejected silently
- **WHEN** a metadata or filename date predates 1900-01-01 or exceeds tomorrow's UTC calendar date
- **THEN** the candidate is skipped without a rejection or uncertainty report
- **AND** the next eligible metadata tag or filename candidate is considered

#### Scenario: Identifier digits resemble a date
- **WHEN** a UUID or alphanumeric identifier contains digits resembling a plausible calendar date
- **THEN** those digits do not determine the media date

### Requirement: Destination layout
The tool SHALL place dated media under Media/YYYY/YYYY-MM/YYYY-MM-DD/, undated media under Media/Unknown/<filename-pattern>/, and other files under Other/, relative to the selected destination root. Original basenames SHALL be preserved.

#### Scenario: Dated and undated media
- **WHEN** media named IMG_1234.jpg resolves to 2020-02-29 or has no plausible date
- **THEN** its directory is Media/2020/2020-02/2020-02-29/ or Media/Unknown/IMG_#/ respectively

### Requirement: Unknown filename patterns
Unknown filename patterns SHALL ignore extensions, uppercase ASCII letters, replace each run of digits with #, retain _ and -, and replace other characters with _. Labels SHALL trim leading and trailing separators, be limited to 80 ASCII bytes, form safe single path components and be independent of other files in the import.

#### Scenario: Similar unknown filenames share a folder
- **WHEN** IMG_1234.jpg and img_56789.png have Unknown dates
- **THEN** both files are placed under Media/Unknown/IMG_#/ with their original basenames

### Requirement: Unknown identifier and unmatched groups
Stems containing a UUID SHALL use UUID; otherwise stems containing a 32/40/64/128-character hexadecimal token SHALL use Hash. Remaining stems without digits and non-UTF-8 names SHALL use Other.

#### Scenario: Unknown identifiers and unmatched names
- **WHEN** unknown media have a UUID name, a hexadecimal hash name, or a name without digits
- **THEN** they use Media/Unknown/UUID/, Media/Unknown/Hash/, or Media/Unknown/Other/ respectively

### Requirement: Deterministic representatives and collisions
The tool SHALL choose duplicate representatives and destination names independently of worker completion order. Different content with the same destination name SHALL receive a deterministic SHA-256 suffix without overwriting any existing file.

#### Scenario: Equal names with different content
- **WHEN** two unique files resolve to the same directory and basename
- **THEN** both survive and the colliding file receives a stable digest-derived suffix

#### Scenario: Worker timing changes
- **WHEN** the same recorded inventory and catalog are processed with different worker completion timings or resumed
- **THEN** representative choices and intended destination paths remain the same
