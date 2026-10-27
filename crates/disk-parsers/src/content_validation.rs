//! Bounded file-container validation for evidence-led carving.
//! Original implementation; no decoder or arbitrary fragment ordering is implied.
use serde_json::{json, Value};
use std::io::{self, BufReader, Read, Seek, SeekFrom};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PNG_CHUNKS: usize = 16384;
const MAX_JPEG_MARKERS: usize = 65536;
const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn unsupported(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message)
}
struct Input<'a, R> {
    reader: BufReader<io::Take<&'a mut R>>,
    length: u64,
    position: u64,
    budget_exhausted: bool,
}
impl<'a, R: Read> Input<'a, R> {
    fn new(reader: &'a mut R, length: u64) -> Self {
        Self {
            reader: BufReader::with_capacity(65536, reader.take(length.min(MAX_BYTES))),
            length,
            position: 0,
            budget_exhausted: false,
        }
    }
    fn bytes(&mut self, output: &mut [u8]) -> io::Result<()> {
        let end = self
            .position
            .checked_add(output.len() as u64)
            .ok_or_else(|| invalid("Read position overflow"))?;
        if end > self.length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Container field exceeds supplied candidate length",
            ));
        }
        if end > MAX_BYTES {
            self.budget_exhausted = true;
            return Err(io::Error::other(
                "64 MiB container-validation byte budget exceeded",
            ));
        }
        self.reader.read_exact(output)?;
        self.position = end;
        Ok(())
    }
    fn fixed<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut output = [0; N];
        self.bytes(&mut output)?;
        Ok(output)
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.fixed::<1>()?[0])
    }
    fn quota(&mut self, message: &str) -> io::Error {
        self.budget_exhausted = true;
        io::Error::other(message)
    }
}

fn png<R: Read>(input: &mut Input<'_, R>) -> io::Result<Value> {
    if &input.fixed::<8>()? != PNG_MAGIC {
        return Err(invalid("PNG signature mismatch"));
    }
    let mut chunks = 0;
    let mut ihdr = false;
    let mut palette = false;
    let mut idat_seen = false;
    let mut idat_closed = false;
    let mut image_bytes = 0u64;
    let mut color = 0;
    let mut depth = 0;
    let mut width = 0;
    let mut height = 0;
    let mut zlib_prefix = Vec::with_capacity(2);
    let mut animated = false;
    let mut buffer = [0; 65536];
    loop {
        if chunks >= MAX_PNG_CHUNKS {
            return Err(input.quota("PNG chunk-count budget exceeded"));
        }
        let header = input.fixed::<8>()?;
        let length = u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|_| invalid("Invalid PNG length"))?,
        );
        let kind: [u8; 4] = header[4..]
            .try_into()
            .map_err(|_| invalid("Invalid PNG chunk type"))?;
        if length > 0x7fff_ffff
            || !kind.iter().all(u8::is_ascii_alphabetic)
            || !kind[2].is_ascii_uppercase()
        {
            return Err(invalid("Invalid PNG chunk length/type/reserved bit"));
        }
        if !ihdr && &kind != b"IHDR" {
            return Err(invalid("IHDR must be the first PNG chunk"));
        }
        if kind[0].is_ascii_uppercase() && !matches!(&kind, b"IHDR" | b"PLTE" | b"IDAT" | b"IEND") {
            return Err(unsupported("Unknown PNG critical chunk"));
        }
        let required = length as u64 + 4;
        if required > input.length - input.position {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "PNG chunk payload/CRC is truncated",
            ));
        }
        if &kind == b"IHDR" && (ihdr || length != 13) {
            return Err(invalid("Duplicate or wrong-size IHDR"));
        }
        if &kind == b"PLTE"
            && (palette
                || idat_seen
                || !(3..=768).contains(&length)
                || !length.is_multiple_of(3)
                || matches!(color, 0 | 4))
        {
            return Err(invalid("Invalid PNG palette length/order/color type"));
        }
        if &kind == b"PLTE" && color == 3 && length / 3 > (1u32 << depth) {
            return Err(invalid("PNG palette exceeds indexed bit depth"));
        }
        if &kind == b"IDAT" && (idat_closed || color == 3 && !palette) {
            return Err(invalid("PNG IDAT sequence or indexed palette is invalid"));
        }
        if idat_seen && &kind != b"IDAT" {
            idat_closed = true;
        }
        if &kind == b"IEND" && (length != 0 || !idat_seen || image_bytes < 2) {
            return Err(invalid(
                "PNG IEND requires nonempty image data and zero length",
            ));
        }
        let mut crc = crc32fast::Hasher::new();
        crc.update(&kind);
        let mut remaining = length as usize;
        let mut head = [0; 13];
        let mut copied = 0;
        while remaining > 0 {
            let amount = remaining.min(buffer.len());
            input.bytes(&mut buffer[..amount])?;
            crc.update(&buffer[..amount]);
            if copied < head.len() {
                let take = amount.min(head.len() - copied);
                head[copied..copied + take].copy_from_slice(&buffer[..take]);
                copied += take;
            }
            if &kind == b"IDAT" {
                let needed = (2 - zlib_prefix.len()).min(amount);
                zlib_prefix.extend_from_slice(&buffer[..needed]);
            }
            remaining -= amount;
        }
        if crc.finalize() != u32::from_be_bytes(input.fixed::<4>()?) {
            return Err(invalid("PNG chunk CRC32 mismatch"));
        }
        chunks += 1;
        match &kind {
            b"IHDR" => {
                width = u32::from_be_bytes(
                    head[..4]
                        .try_into()
                        .map_err(|_| invalid("Invalid PNG width"))?,
                );
                height = u32::from_be_bytes(
                    head[4..8]
                        .try_into()
                        .map_err(|_| invalid("Invalid PNG height"))?,
                );
                depth = head[8];
                color = head[9];
                let depth_valid = match color {
                    0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
                    2 | 4 | 6 => matches!(depth, 8 | 16),
                    3 => matches!(depth, 1 | 2 | 4 | 8),
                    _ => false,
                };
                if width == 0
                    || height == 0
                    || width > 0x7fff_ffff
                    || height > 0x7fff_ffff
                    || !depth_valid
                    || head[10] != 0
                    || head[11] != 0
                    || head[12] > 1
                {
                    return Err(invalid("Invalid PNG dimensions, bit depth, color or compression/filter/interlace method"));
                }
                ihdr = true;
            }
            b"PLTE" => palette = true,
            b"IDAT" => {
                idat_seen = true;
                image_bytes += length as u64;
            }
            b"acTL" => animated = true,
            b"IEND" => {
                let header = u16::from_be_bytes([zlib_prefix[0], zlib_prefix[1]]);
                if zlib_prefix[0] & 0x0f != 8
                    || zlib_prefix[0] >> 4 > 7
                    || !header.is_multiple_of(31)
                    || zlib_prefix[1] & 0x20 != 0
                {
                    return Err(invalid("Invalid PNG zlib stream header"));
                }
                return Ok(
                    json!({"width":width,"height":height,"chunks_checked":chunks,"crc32_verified":true,"compressed_image_bytes":image_bytes,"animated_png":animated,"validation_scope":"chunk CRC32, critical chunk order and zlib header; pixel decompression/APNG frames not verified"}),
                );
            }
            _ => (),
        }
    }
}

fn jpeg<R: Read>(input: &mut Input<'_, R>) -> io::Result<Value> {
    if input.fixed::<2>()? != [0xff, 0xd8] {
        return Err(invalid("JPEG SOI signature mismatch"));
    }
    let mut markers = 1;
    let mut scans = 0;
    let mut scan_data_bytes = 0u64;
    let mut in_scan = false;
    let mut frame = None;
    let mut components: Vec<(u8, u8)> = Vec::new();
    let mut quantization = 0u8;
    let mut huffman = [0u8; 2];
    let mut restart_interval = 0u16;
    let mut restart_expected = 0xd0;
    let mut restart_markers = 0;
    loop {
        if markers >= MAX_JPEG_MARKERS {
            return Err(input.quota("JPEG marker-count budget exceeded"));
        }
        if in_scan {
            let first = input.byte()?;
            if first != 0xff {
                scan_data_bytes += 1;
                continue;
            }
        } else if input.byte()? != 0xff {
            return Err(invalid("JPEG segment does not start with marker prefix"));
        }
        let mut marker = input.byte()?;
        while marker == 0xff {
            marker = input.byte()?;
        }
        if marker == 0 && in_scan {
            scan_data_bytes += 1;
            continue;
        }
        if marker == 0 {
            return Err(invalid("Stuffed JPEG byte appears outside entropy data"));
        }
        markers += 1;
        if (0xd0..=0xd7).contains(&marker) {
            if !in_scan || restart_interval == 0 || marker != restart_expected {
                return Err(invalid(
                    "JPEG restart marker sequence/interval is inconsistent",
                ));
            }
            restart_expected = 0xd0 + (marker - 0xd0 + 1) % 8;
            restart_markers += 1;
            continue;
        }
        if in_scan && scan_data_bytes == 0 {
            return Err(invalid("JPEG scan has no entropy payload"));
        }
        in_scan = false;
        if marker == 0xd9 {
            let (_, width, height) =
                frame.ok_or_else(|| invalid("JPEG EOI before frame header"))?;
            if scans == 0 {
                return Err(invalid("JPEG EOI before any scan"));
            }
            return Ok(
                json!({"width":width,"height":height,"markers_checked":markers,"scans":scans,"restart_markers":restart_markers,"validation_scope":"SOF/SOS/table references, marker bounds, entropy stuffing/restarts and EOI; Huffman coefficients and pixels not decoded"}),
            );
        }
        if marker == 0xd8 {
            return Err(invalid("Duplicate JPEG SOI inside candidate"));
        }
        if marker == 0x01 {
            return Err(unsupported(
                "JPEG arithmetic TEM marker is outside supported Huffman frame profiles",
            ));
        }
        if !matches!(
            marker,
            0xc0 | 0xc1 | 0xc2 | 0xc4 | 0xdb | 0xdd | 0xda | 0xe0..=0xef | 0xfe
        ) {
            return Err(unsupported(
                "JPEG marker/frame profile is not supported by this structural validator",
            ));
        }
        let length = u16::from_be_bytes(input.fixed::<2>()?) as usize;
        if length < 2 {
            return Err(invalid("JPEG segment length is smaller than length field"));
        }
        let mut payload = vec![0; length - 2];
        input.bytes(&mut payload)?;
        match marker {
            0xc0..=0xc2 => {
                if frame.is_some() || payload.len() < 6 {
                    return Err(invalid("Duplicate/truncated JPEG frame header"));
                }
                let precision = payload[0];
                let height = u16::from_be_bytes([payload[1], payload[2]]);
                let width = u16::from_be_bytes([payload[3], payload[4]]);
                let count = payload[5] as usize;
                if count > 4 || height == 0 {
                    return Err(unsupported(
                        "JPEG component count or deferred-height frame is unsupported",
                    ));
                }
                if width == 0
                    || count == 0
                    || payload.len() != 6 + 3 * count
                    || !matches!(precision, 8 | 12)
                    || marker == 0xc0 && precision != 8
                {
                    return Err(invalid(
                        "JPEG frame dimensions, precision or component length invalid",
                    ));
                }
                for component in payload[6..].as_chunks::<3>().0 {
                    let h = component[1] >> 4;
                    let v = component[1] & 15;
                    if !(1..=4).contains(&h)
                        || !(1..=4).contains(&v)
                        || component[2] > 3
                        || components.iter().any(|(id, _)| *id == component[0])
                    {
                        return Err(invalid("JPEG component identifiers, sampling or quantization selectors invalid"));
                    }
                    components.push((component[0], component[2]));
                }
                frame = Some((marker, width, height));
            }
            0xdb => {
                let mut at = 0;
                while at < payload.len() {
                    let descriptor = payload[at];
                    at += 1;
                    let precision = descriptor >> 4;
                    let id = descriptor & 15;
                    let bytes = if precision == 0 { 64 } else { 128 };
                    if precision > 1 || id > 3 || bytes > payload.len() - at {
                        return Err(invalid("JPEG quantization table length/selector invalid"));
                    }
                    if precision == 0 && payload[at..at + bytes].contains(&0)
                        || precision == 1
                            && payload[at..at + bytes].as_chunks::<2>().0.contains(&[0, 0])
                    {
                        return Err(invalid("JPEG quantization table contains zero"));
                    }
                    quantization |= 1 << id;
                    at += bytes;
                }
            }
            0xc4 => {
                let mut at = 0;
                while at < payload.len() {
                    if payload.len() - at < 17 {
                        return Err(invalid("JPEG Huffman table header truncated"));
                    }
                    let descriptor = payload[at];
                    let class = (descriptor >> 4) as usize;
                    let id = descriptor & 15;
                    if class > 1 || id > 3 {
                        return Err(invalid("JPEG Huffman table class/selector invalid"));
                    }
                    let counts = &payload[at + 1..at + 17];
                    let symbols: usize = counts.iter().map(|n| *n as usize).sum();
                    if symbols == 0 || symbols > 256 || symbols > payload.len() - at - 17 {
                        return Err(invalid("JPEG Huffman symbol count invalid"));
                    }
                    let mut slots = 1i32;
                    for count in counts {
                        slots = slots * 2 - *count as i32;
                        if slots < 0 {
                            return Err(invalid("JPEG Huffman table is oversubscribed"));
                        }
                    }
                    huffman[class] |= 1 << id;
                    at += 17 + symbols;
                }
            }
            0xdd => {
                if payload.len() != 2 {
                    return Err(invalid("JPEG restart interval segment length invalid"));
                }
                restart_interval = u16::from_be_bytes([payload[0], payload[1]]);
            }
            0xda => {
                let (kind, _, _) =
                    frame.ok_or_else(|| invalid("JPEG scan appears before frame header"))?;
                if payload.len() < 4 {
                    return Err(invalid("Truncated JPEG scan header"));
                }
                let count = payload[0] as usize;
                if count == 0 || count > 4 || payload.len() != 4 + 2 * count {
                    return Err(invalid("JPEG scan component count/length invalid"));
                }
                let parameters = &payload[payload.len() - 3..];
                let ss = parameters[0];
                let se = parameters[1];
                let ah = parameters[2] >> 4;
                let al = parameters[2] & 15;
                if kind != 0xc2 && parameters != [0, 63, 0]
                    || kind == 0xc2
                        && (ss > se
                            || se > 63
                            || count > 1 && (ss != 0 || se != 0)
                            || ah > 13
                            || al > 13
                            || ah != 0 && ah != al + 1)
                {
                    return Err(invalid("JPEG scan spectral/successive parameters invalid"));
                }
                let mut ids = Vec::new();
                for component in payload[1..1 + 2 * count].as_chunks::<2>().0 {
                    let quant = components
                        .iter()
                        .find(|(id, _)| *id == component[0])
                        .map(|(_, q)| *q)
                        .ok_or_else(|| invalid("JPEG scan references unknown frame component"))?;
                    let dc = component[1] >> 4;
                    let ac = component[1] & 15;
                    if ids.contains(&component[0]) || dc > 3 || ac > 3 {
                        return Err(invalid("JPEG scan component/table selectors invalid"));
                    }
                    ids.push(component[0]);
                    if quantization & (1 << quant) == 0
                        || ss == 0 && ah == 0 && huffman[0] & (1 << dc) == 0
                        || se > 0 && huffman[1] & (1 << ac) == 0
                    {
                        return Err(unsupported(
                            "JPEG uses absent/external quantization or Huffman tables",
                        ));
                    }
                }
                scans += 1;
                scan_data_bytes = 0;
                in_scan = true;
                restart_expected = 0xd0;
            }
            _ => (),
        }
    }
}

/// Analyze the supplied view starting at logical offset zero. Candidate length
/// identifies a structurally bounded prefix, not verified decoded content.
pub fn analyze<R: Read + Seek>(reader: &mut R, length: u64, format: &str) -> io::Result<Value> {
    let requested = format.to_ascii_uppercase();
    if !matches!(requested.as_str(), "AUTO" | "JPEG" | "PNG" | "MP4" | "MOV") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Supported formats: AUTO/JPEG/PNG/MP4/MOV",
        ));
    }
    reader.seek(SeekFrom::Start(0))?;
    let mut magic = [0; 12];
    let count = length.min(magic.len() as u64) as usize;
    if let Err(error) = reader.read_exact(&mut magic[..count]) {
        if error.kind() != io::ErrorKind::UnexpectedEof {
            return Err(error);
        }
        return Ok(
            json!({"format":requested,"classification":"TRUNCATED","candidate_length_bytes":null,"decode_verified":false,"issues":["Source is shorter than supplied length"]}),
        );
    }
    reader.seek(SeekFrom::Start(0))?;
    let detected = if count >= 8 && &magic[..8] == PNG_MAGIC {
        "PNG"
    } else if count >= 2 && magic[..2] == [0xff, 0xd8] {
        "JPEG"
    } else if count >= 8
        && matches!(
            &magic[4..8],
            b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"moof"
        )
    {
        "MP4"
    } else {
        "UNKNOWN"
    };
    let selected = if requested == "AUTO" {
        detected
    } else {
        &requested
    };
    if selected == "UNKNOWN" {
        return Ok(
            json!({"format":"UNKNOWN","classification":"UNKNOWN_OR_UNSUPPORTED_FORMAT","supplied_length_bytes":length,"candidate_length_bytes":null,"decode_verified":false,"issues":["No supported file header at view offset zero; PDF/ZIP and other formats require independent validators"]}),
        );
    }
    if matches!(selected, "MP4" | "MOV") {
        let report = crate::iso_bmff::analyze(reader, length)?;
        return Ok(
            json!({"format":selected,"classification":report.classification,"supplied_length_bytes":length,"candidate_length_bytes":if report.classification == "STRUCTURE_CONSISTENT" {Some(length)} else {None},"boundary_evidence":"caller_supplied_view","decode_verified":false,"details":report}),
        );
    }
    let mut input = Input::new(reader, length);
    let result = if selected == "PNG" {
        png(&mut input)
    } else {
        jpeg(&mut input)
    };
    match result {
        Ok(details) => Ok(
            json!({"format":selected,"classification":"STRUCTURE_CONSISTENT","supplied_length_bytes":length,"candidate_length_bytes":input.position,"trailing_bytes":length-input.position,"boundary_evidence":if selected=="PNG" {"CRC-checked IEND"} else {"EOI after checked marker/scan structure"},"bytes_examined":input.position,"decode_verified":false,"details":details}),
        ),
        Err(error) => {
            let classification = if input.budget_exhausted {
                "BUDGET_EXCEEDED"
            } else {
                match error.kind() {
                    io::ErrorKind::InvalidData => "INVALID_STRUCTURE",
                    io::ErrorKind::UnexpectedEof => "TRUNCATED",
                    io::ErrorKind::Unsupported => "UNSUPPORTED_FEATURE",
                    _ => return Err(error),
                }
            };
            Ok(
                json!({"format":selected,"classification":classification,"supplied_length_bytes":length,"candidate_length_bytes":null,"bytes_examined":input.position,"byte_budget":MAX_BYTES,"decode_verified":false,"issues":[error.to_string()]}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut crc = crc32fast::Hasher::new();
        crc.update(kind);
        crc.update(payload);
        [
            (payload.len() as u32).to_be_bytes().as_slice(),
            kind,
            payload,
            &crc.finalize().to_be_bytes(),
        ]
        .concat()
    }
    fn png_fixture() -> Vec<u8> {
        let ihdr = [
            1u32.to_be_bytes().as_slice(),
            &1u32.to_be_bytes(),
            &[8, 2, 0, 0, 0],
        ]
        .concat();
        [
            PNG_MAGIC.as_slice(),
            &chunk(b"IHDR", &ihdr),
            &chunk(
                b"IDAT",
                &[0x78, 0x9c, 0x63, 0x60, 0x60, 0x60, 0, 0, 0, 4, 0, 1],
            ),
            &chunk(b"IEND", &[]),
        ]
        .concat()
    }
    fn jpeg_fixture() -> Vec<u8> {
        // Locally generated 1x1 baseline JPEG fixture, not copied upstream code.
        let hex = "ffd8ffe000104a46494600010100000100010000ffdb004300030202020202030202020303030304060404040404080606050609080a0a090809090a0c0f0c0a0b0e0b09090d110d0e0f101011100a0c12131210130f101010ffdb00430103030304030408040408100b090b1010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010ffc00011080001000103011100021101031101ffc40014000100000000000000000000000000000008ffc40014100100000000000000000000000000000000ffc4001501010100000000000000000000000000000709ffc40014110100000000000000000000000000000000ffda000c03010002110311003f003a03154dffd9";
        hex.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    fn report(data: &[u8], format: &str) -> Value {
        analyze(&mut Cursor::new(data), data.len() as u64, format).unwrap()
    }

    #[test]
    fn png_crc_checked_iend_yields_exact_boundary_and_separates_trailing_bytes() {
        let original = png_fixture();
        let mut data = original.clone();
        data.extend_from_slice(b"not part of image");
        let result = report(&data, "AUTO");
        assert_eq!(result["classification"], "STRUCTURE_CONSISTENT");
        assert_eq!(result["candidate_length_bytes"], original.len());
        assert_eq!(result["trailing_bytes"], 17);
        assert_eq!(result["details"]["crc32_verified"], true);
        assert_eq!(result["decode_verified"], false);
    }

    #[test]
    fn png_corruption_truncation_and_critical_order_are_not_passing_candidates() {
        let original = png_fixture();
        let mut bad_crc = original.clone();
        bad_crc[29] ^= 1;
        assert_eq!(
            report(&bad_crc, "PNG")["classification"],
            "INVALID_STRUCTURE"
        );
        assert_eq!(
            report(&original[..original.len() - 1], "PNG")["classification"],
            "TRUNCATED"
        );
        let bad_order = [
            PNG_MAGIC.as_slice(),
            &chunk(b"IDAT", &[0x78, 0x9c]),
            &chunk(b"IEND", &[]),
        ]
        .concat();
        assert_eq!(
            report(&bad_order, "PNG")["classification"],
            "INVALID_STRUCTURE"
        );
        let unknown_critical = [
            original[..33].to_vec(),
            chunk(b"ABCD", &[]),
            original[33..].to_vec(),
        ]
        .concat();
        assert_eq!(
            report(&unknown_critical, "PNG")["classification"],
            "UNSUPPORTED_FEATURE"
        );
    }

    #[test]
    fn png_chunk_budget_prevents_endless_empty_ancillary_sequences() {
        let mut data = png_fixture()[..33].to_vec();
        let ancillary = chunk(b"tEXt", &[]);
        for _ in 0..MAX_PNG_CHUNKS {
            data.extend_from_slice(&ancillary);
        }
        assert_eq!(report(&data, "PNG")["classification"], "BUDGET_EXCEEDED");
    }

    #[test]
    fn jpeg_marker_parser_handles_real_image_and_ignores_eoi_inside_app_payload() {
        let original = jpeg_fixture();
        let mut data = original[..2].to_vec();
        data.extend_from_slice(&[0xff, 0xe1, 0, 6, 0xff, 0xd9, 0, 0]);
        data.extend_from_slice(&original[2..]);
        let actual = data.len();
        data.extend_from_slice(b"other file");
        let result = report(&data, "AUTO");
        assert_eq!(result["classification"], "STRUCTURE_CONSISTENT");
        assert_eq!(result["candidate_length_bytes"], actual);
        assert_eq!(result["trailing_bytes"], 10);
        assert_eq!(result["details"]["width"], 1);
    }

    #[test]
    fn jpeg_stuffing_restart_sequences_and_missing_eoi_are_checked() {
        let original = jpeg_fixture();
        assert_eq!(
            report(&original[..original.len() - 1], "JPEG")["classification"],
            "TRUNCATED"
        );
        let sos = original
            .windows(2)
            .position(|bytes| bytes == [0xff, 0xda])
            .unwrap();
        let mut data = original[..sos].to_vec();
        data.extend_from_slice(&[0xff, 0xdd, 0, 4, 0, 1]);
        data.extend_from_slice(&original[sos..original.len() - 2]);
        data.extend_from_slice(&[0xff, 0, 0xff, 0xd0, 0x25, 0xff, 0xd9]);
        assert_eq!(
            report(&data, "JPEG")["classification"],
            "STRUCTURE_CONSISTENT"
        );
        let restart = data
            .windows(2)
            .position(|bytes| bytes == [0xff, 0xd0])
            .unwrap();
        data[restart + 1] = 0xd2;
        assert_eq!(report(&data, "JPEG")["classification"], "INVALID_STRUCTURE");
        assert_eq!(
            report(&[0xff, 0xd8, 0xff, 0xd9], "JPEG")["classification"],
            "INVALID_STRUCTURE"
        );
    }

    #[test]
    fn unknown_formats_and_underlying_io_failures_remain_distinct() {
        assert_eq!(
            report(b"%PDF document", "AUTO")["classification"],
            "UNKNOWN_OR_UNSUPPORTED_FORMAT"
        );
        assert!(analyze(&mut Cursor::new(b""), 0, "ZIP").is_err());
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "test read denied",
                ))
            }
        }
        impl Seek for Broken {
            fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
                Ok(0)
            }
        }
        assert_eq!(
            analyze(&mut Broken, 16, "AUTO").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        for length in 0..32 {
            let _ = report(&vec![0xff; length], "JPEG");
        }
    }

    #[test]
    fn buffered_reads_cannot_prefetch_outside_the_supplied_view() {
        struct Counted {
            source: Cursor<Vec<u8>>,
            maximum_requested_end: u64,
        }
        impl Read for Counted {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                self.maximum_requested_end = self
                    .maximum_requested_end
                    .max(self.source.position() + output.len() as u64);
                self.source.read(output)
            }
        }
        impl Seek for Counted {
            fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
                self.source.seek(from)
            }
        }
        let mut data = png_fixture();
        let supplied_length = data.len() as u64;
        data.extend(vec![0xaa; 1024 * 1024]);
        let mut counted = Counted {
            source: Cursor::new(data),
            maximum_requested_end: 0,
        };
        let result = analyze(&mut counted, supplied_length, "PNG").unwrap();
        assert_eq!(result["classification"], "STRUCTURE_CONSISTENT");
        assert_eq!(counted.maximum_requested_end, supplied_length);
    }
}
