//! Bounded ISO BMFF / QuickTime box inspection. Never infer decoded integrity.
//! Uses seekable byte views: media payload is skipped, not loaded into memory.
use serde::Serialize;
use std::io::{self, Read, Seek, SeekFrom};

const MAX_BOXES: usize = 4096;
const MAX_OFFSETS: usize = 65536;

#[derive(Debug, Serialize)]
pub struct BoxInfo {
    pub kind: String,
    pub byte_offset: u64,
    pub length_bytes: u64,
}
#[derive(Debug, Default, Serialize)]
pub struct VideoReport {
    pub classification: String,
    pub length_bytes: u64,
    pub boxes_examined: usize,
    pub top_level_boxes: Vec<BoxInfo>,
    pub boxes_truncated: bool,
    pub has_ftyp: bool,
    pub has_moov: bool,
    pub mdat_count: usize,
    pub tracks: usize,
    pub fragmented_mp4: bool,
    pub chunk_offsets_checked: usize,
    pub chunk_offsets_outside_mdat: usize,
    pub issues: Vec<String>,
    pub decode_verified: bool,
}
struct Parser<'a, R> {
    input: &'a mut R,
    report: VideoReport,
    mdats: Vec<(u64, u64)>,
    offsets: Vec<u64>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn u32be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().expect("fixed four-byte field"))
}
impl<R: Read + Seek> Parser<'_, R> {
    fn read<const N: usize>(&mut self, offset: u64) -> io::Result<[u8; N]> {
        self.input.seek(SeekFrom::Start(offset))?;
        let mut bytes = [0; N];
        self.input.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn walk(&mut self, mut at: u64, end: u64, depth: usize, parent: &[u8; 4]) -> io::Result<()> {
        if depth > 12 {
            return Err(invalid("Box nesting budget exceeded"));
        }
        while at < end {
            if end - at < 8 {
                return Err(invalid(
                    "Truncated box header or trailing bytes; narrow the candidate length",
                ));
            }
            if self.report.boxes_examined >= MAX_BOXES {
                return Err(invalid("Box count budget exceeded"));
            }
            let header = self.read::<8>(at)?;
            let kind: [u8; 4] = header[4..8]
                .try_into()
                .map_err(|_| invalid("Invalid box type"))?;
            let short = u32be(&header[..4]);
            let mut header_size = 8;
            let length = match short {
                0 => end - at,
                1 => {
                    if end - at < 16 {
                        return Err(invalid("Truncated extended box header"));
                    }
                    header_size = 16;
                    u64::from_be_bytes(self.read::<8>(at + 8)?)
                }
                n => n as u64,
            };
            if &kind == b"uuid" {
                header_size += 16;
            }
            if length < header_size || length > end - at {
                return Err(invalid("Box length exceeds candidate or parent bounds (truncated, incorrect fragment order or false signature)"));
            }
            self.report.boxes_examined += 1;
            let payload = at + header_size;
            let next = at + length;
            if depth == 0 {
                if self.report.top_level_boxes.len() < 12 {
                    self.report.top_level_boxes.push(BoxInfo {
                        kind: kind
                            .iter()
                            .map(|b| {
                                if b.is_ascii_graphic() {
                                    *b as char
                                } else {
                                    '?'
                                }
                            })
                            .collect(),
                        byte_offset: at,
                        length_bytes: length,
                    });
                } else {
                    self.report.boxes_truncated = true;
                }
                match &kind {
                    b"ftyp" => {
                        if next - payload < 8 || !(next - payload).is_multiple_of(4) {
                            return Err(invalid("Malformed ftyp brand list"));
                        }
                        self.report.has_ftyp = true;
                    }
                    b"moov" => self.report.has_moov = true,
                    b"mdat" => {
                        self.mdats.push((payload, next));
                        self.report.mdat_count += 1;
                    }
                    b"moof" => self.report.fragmented_mp4 = true,
                    _ => (),
                }
            }
            let expected_parent = match &kind {
                b"trak" => Some(b"moov"),
                b"mdia" => Some(b"trak"),
                b"minf" => Some(b"mdia"),
                b"stbl" => Some(b"minf"),
                b"stco" | b"co64" => Some(b"stbl"),
                _ => None,
            };
            if expected_parent.is_some_and(|expected| expected != parent) {
                return Err(invalid("Box appears outside its required container"));
            }
            if &kind == b"trak" {
                self.report.tracks += 1;
            }
            if &kind == b"mvex" {
                self.report.fragmented_mp4 = true;
            }
            if matches!(&kind, b"stco" | b"co64") {
                if next - payload < 8 {
                    return Err(invalid("Truncated chunk offset table"));
                }
                let table = self.read::<8>(payload)?;
                if table[..4] != [0; 4] {
                    return Err(invalid("Unsupported chunk offset table version/flags"));
                }
                let count = u32be(&table[4..]) as usize;
                let width = if &kind == b"co64" { 8 } else { 4 };
                if count > MAX_OFFSETS - self.offsets.len() {
                    return Err(invalid(
                        "Chunk offset budget exceeded; use an external demuxer for this file",
                    ));
                }
                if count as u64 * width as u64 != next - payload - 8 {
                    return Err(invalid("Chunk offset table length mismatch"));
                }
                // Bounded table reads avoid one disk operation per chunk.
                let mut data = vec![0; count * width];
                self.input.seek(SeekFrom::Start(payload + 8))?;
                self.input.read_exact(&mut data)?;
                for entry in data.chunks_exact(width) {
                    self.offsets.push(if width == 4 {
                        u32be(entry) as u64
                    } else {
                        u64::from_be_bytes(
                            entry
                                .try_into()
                                .map_err(|_| invalid("Invalid co64 entry"))?,
                        )
                    });
                }
            }
            if matches!(
                &kind,
                b"moov"
                    | b"trak"
                    | b"mdia"
                    | b"minf"
                    | b"stbl"
                    | b"edts"
                    | b"dinf"
                    | b"mvex"
                    | b"moof"
                    | b"traf"
            ) {
                self.walk(payload, next, depth + 1, &kind)?;
            }
            at = next;
        }
        Ok(())
    }
}

pub fn analyze<R: Read + Seek>(input: &mut R, length: u64) -> io::Result<VideoReport> {
    let mut parser = Parser {
        input,
        report: VideoReport {
            length_bytes: length,
            ..Default::default()
        },
        mdats: Vec::new(),
        offsets: Vec::new(),
    };
    if let Err(error) = parser.walk(0, length, 0, b"root") {
        if !matches!(
            error.kind(),
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
        ) {
            return Err(error);
        }
        parser.report.issues.push(error.to_string());
    }
    parser.report.chunk_offsets_checked = parser.offsets.len();
    parser.mdats.sort_unstable();
    parser.report.chunk_offsets_outside_mdat = parser
        .offsets
        .iter()
        .filter(|offset| {
            let i = parser.mdats.partition_point(|(start, _)| start <= *offset);
            i == 0 || **offset >= parser.mdats[i - 1].1
        })
        .count();
    let r = &mut parser.report;
    r.classification = if !r.issues.is_empty() {
        "INVALID_OR_BUDGET_EXCEEDED"
    } else if !r.has_moov {
        "MISSING_MOOV"
    } else if r.mdat_count == 0 {
        "MISSING_MEDIA_DATA"
    } else if r.chunk_offsets_outside_mdat > 0 {
        "CHUNK_OFFSETS_OUTSIDE_MDAT"
    } else if r.fragmented_mp4 {
        "FRAGMENTED_MP4_REQUIRES_DEMUXER"
    } else if r.tracks == 0 || r.chunk_offsets_checked == 0 {
        "INSUFFICIENT_SAMPLE_TABLES"
    } else {
        "STRUCTURE_CONSISTENT"
    }
    .into();
    Ok(parser.report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn atom(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        [&((data.len() + 8) as u32).to_be_bytes()[..], kind, data].concat()
    }
    #[test]
    fn identifies_missing_metadata_and_invalid_lengths_without_payload_allocation() {
        for (data, expected) in [
            (atom(b"mdat", &[0; 4]), "MISSING_MOOV"),
            (
                vec![0, 0, 0, 4, b'm', b'd', b'a', b't'],
                "INVALID_OR_BUDGET_EXCEEDED",
            ),
            (
                vec![0, 0, 0, 1, b'm', b'd', b'a', b't'],
                "INVALID_OR_BUDGET_EXCEEDED",
            ),
        ] {
            let report = analyze(&mut io::Cursor::new(&data), data.len() as u64).unwrap();
            assert_eq!(report.classification, expected);
            assert!(!report.decode_verified);
        }
    }
    #[test]
    fn chunk_offsets_must_reference_mdat_payload_not_headers_or_other_boxes() {
        for (offset, expected) in [
            (8u32, "STRUCTURE_CONSISTENT"),
            (0, "CHUNK_OFFSETS_OUTSIDE_MDAT"),
        ] {
            let table = atom(
                b"stco",
                &[&[0u8; 4][..], &1u32.to_be_bytes(), &offset.to_be_bytes()].concat(),
            );
            let data = [
                atom(b"mdat", &[0; 8]),
                atom(
                    b"moov",
                    &atom(
                        b"trak",
                        &atom(b"mdia", &atom(b"minf", &atom(b"stbl", &table))),
                    ),
                ),
            ]
            .concat();
            assert_eq!(
                analyze(&mut io::Cursor::new(&data), data.len() as u64)
                    .unwrap()
                    .classification,
                expected
            );
        }
    }
    #[test]
    fn oversized_tables_and_recursive_boxes_stop_at_resource_budgets() {
        let data = atom(b"stco", &[&[0u8; 4][..], &u32::MAX.to_be_bytes()].concat());
        assert_eq!(
            analyze(&mut io::Cursor::new(&data), data.len() as u64)
                .unwrap()
                .classification,
            "INVALID_OR_BUDGET_EXCEEDED"
        );
        let mut data = atom(b"free", &[]);
        for _ in 0..15 {
            data = atom(b"moov", &data);
        }
        assert!(analyze(&mut io::Cursor::new(&data), data.len() as u64)
            .unwrap()
            .issues[0]
            .contains("nesting"));
    }

    #[test]
    fn extended_sizes_co64_and_fragmented_mp4_have_explicit_outcomes() {
        let mut data = [
            1u32.to_be_bytes().as_slice(),
            b"mdat",
            &24u64.to_be_bytes(),
            &[0; 8],
        ]
        .concat();
        let table = atom(
            b"co64",
            &[&[0u8; 4][..], &1u32.to_be_bytes(), &16u64.to_be_bytes()].concat(),
        );
        data.extend(atom(
            b"moov",
            &atom(
                b"trak",
                &atom(b"mdia", &atom(b"minf", &atom(b"stbl", &table))),
            ),
        ));
        assert_eq!(
            analyze(&mut io::Cursor::new(&data), data.len() as u64)
                .unwrap()
                .classification,
            "STRUCTURE_CONSISTENT"
        );
        let fragmented = [
            atom(b"moov", &atom(b"mvex", &[])),
            atom(b"moof", &atom(b"traf", &[])),
            atom(b"mdat", &[0; 8]),
        ]
        .concat();
        assert_eq!(
            analyze(&mut io::Cursor::new(&fragmented), fragmented.len() as u64)
                .unwrap()
                .classification,
            "FRAGMENTED_MP4_REQUIRES_DEMUXER"
        );
    }
}
