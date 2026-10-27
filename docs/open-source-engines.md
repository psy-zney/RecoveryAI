# Open-source engines and algorithm provenance

The project combines checked native I/O and reconstruction plans with upstream
libraries. It does not claim to implement every algorithm of an external tool.

| Component | Upstream and license | Actual integration |
| --- | --- | --- |
| Aho-Corasick 1.1.5 | https://github.com/BurntSushi/aho-corasick — Unlicense OR MIT | Existing multi-pattern scanner; extended with streaming MP4/MOV box candidates and header bounds |
| ntfs 0.4.0 | https://github.com/ColinFinck/ntfs — MIT OR Apache-2.0 | Existing native allocated-file traversal through the overlay |
| fatfs 0.3.6 | https://github.com/rafalh/rust-fatfs — MIT | Existing FAT traversal with writes explicitly denied |
| FFmpeg / ffprobe | https://ffmpeg.org/ and https://ffmpeg.org/legal.html — LGPL/GPL depending on build | New optional local executable integration in scripts/recovery.py for MOV/MP4 demux and full audio/video decode verification |
| GNU ddrescue | https://www.gnu.org/software/ddrescue/ — GPL-2.0-or-later | Optional external executable in scripts/rescue.py: fast/retry passes, mapfile resume and sealed acquisition; separate from disabled Rust adapter |
| OpenSSH | https://www.openssh.com/ — upstream BSD-style licenses | Installed ssh client for authenticated remote Linux acquisition control and resumable sealed-image transfer; no server/keys bundled |
| ISO BMFF / QuickTime box analysis | Original implementation in crates/disk-parsers/src/iso_bmff.rs | Bounded size32/size64/size0 parsing, container traversal and stco/co64 chunk-start checks; not a codec decoder |
| Ordered byte reconstruction | Original implementation in byte_view.rs and extraction.rs | Reordered unaligned extents, non-overlap validation, cancellable exact-byte export, existing SHA-256 provenance |

Versions and Rust library licenses were checked against the installed registry
package metadata and Cargo.lock. This upgrade adds no Rust/npm/pip dependencies
and leaves the lockfile unchanged. It does not represent a blanket dependency
update or a claim that these are the newest published versions.

The validated cloud uses Debian FFmpeg 7.1.5; Debian's installed copyright notice
states its default builds include GPL components. FFmpeg is invoked as a separately
installed program, not copied, linked or bundled here. Distributing binaries
requires checking their actual build options and obligations. Official probe
reference: https://ffmpeg.org/ffprobe.html; MOV demuxer reference:
https://ffmpeg.org/ffmpeg-formats.html#mov_002fmp4_002f3gp .

PhotoRec/TestDisk (https://www.cgsecurity.org/), The Sleuth Kit
(https://www.sleuthkit.org/) and GNU ddrescue
(https://www.gnu.org/software/ddrescue/) are relevant external engines. Their
legacy adapters in this repository remain disabled with explicit NotImplemented
errors. Installing one does not silently enable it, and the native DD_RESCUE_CLONE
job is a block copier, not GNU ddrescue's recovery algorithm. The separate new
Linux rescue.py CLI explicitly integrates an installed GNU ddrescue executable.
No GPL recovery code
was copied into the project under another license.

Future recovery adapters need reviewed licensing, explicit read-only source
handling, output isolation, argument validation, cancellation, provenance and
real functional tests before being advertised as integrated.


The three-case extension independently researched upstream TianoCore GPT field
layouts, ntfs/ntfs-3g backup-boot behavior, rust-fatfs deletion/allocation semantics,
libpng chunk checks and libjpeg-turbo marker handling. New bounded implementations
do not copy external GPL code or add dependencies. See research/case-1-partition-boot.md,
research/case-2-deleted-metadata.md and research/case-3-fragments-content.md for exact
sources, inspected versions/hashes, implemented checks and omitted capabilities.
