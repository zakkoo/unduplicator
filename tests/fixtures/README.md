# Media fixture provenance

These tiny synthetic blue frames contain no third-party media. Generated locally:

```sh
ffmpeg -f lavfi -i color=c=blue:s=16x16:d=0.1 -frames:v 1 capture.jpg
ffmpeg -f lavfi -i color=c=blue:s=16x16:d=0.1 -c:v libx264 -pix_fmt yuv420p \
  -metadata creation_time=2020-02-29T12:34:56Z capture.mp4
exiftool -overwrite_original '-EXIF:DateTimeOriginal=2001:01:02 12:34:56' \
  '-XMP:DateCreated=2002:03:04 00:00:00+14:00' capture.jpg
```

ExifTool writes only these test fixtures during generation. The application
invokes ExifTool read-only. FFmpeg is needed only to regenerate fixtures.
Real metadata extraction is tested for JPEG and MP4, including an extension-less
staged JPEG. Other supported extensions have classification tests only.
