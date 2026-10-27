//! A checked, ordered reconstruction plan shared by video analysis and exports.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByteRange {
    pub source_byte_offset: u64,
    pub length_bytes: u64,
}

pub fn validate(ranges: &[ByteRange], capacity: u64) -> Result<u64> {
    ensure!(
        !ranges.is_empty() && ranges.len() <= 4096,
        "Specify 1..4096 byte ranges in output order"
    );
    let mut total = 0u64;
    let mut intervals = Vec::with_capacity(ranges.len());
    for range in ranges {
        let end = range
            .source_byte_offset
            .checked_add(range.length_bytes)
            .context("Byte range overflow")?;
        ensure!(
            range.length_bytes > 0 && end <= capacity,
            "Byte range exceeds source capacity"
        );
        intervals.push((range.source_byte_offset, end));
        total = total
            .checked_add(range.length_bytes)
            .context("Reconstructed size overflow")?;
    }
    intervals.sort_unstable();
    ensure!(
        !intervals.windows(2).any(|p| p[0].1 > p[1].0),
        "Source byte ranges overlap; duplicate evidence is not a reconstruction"
    );
    Ok(total)
}

pub struct ByteView<'a, R> {
    source: &'a mut R,
    ranges: &'a [ByteRange],
    length: u64,
    position: u64,
}
impl<'a, R> ByteView<'a, R> {
    pub fn new(source: &'a mut R, ranges: &'a [ByteRange], capacity: u64) -> Result<Self> {
        Ok(Self {
            source,
            ranges,
            length: validate(ranges, capacity)?,
            position: 0,
        })
    }
}
impl<R: Read + Seek> Read for ByteView<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length || output.is_empty() {
            return Ok(0);
        }
        let mut base = 0;
        for range in self.ranges {
            if self.position < base + range.length_bytes {
                let within = self.position - base;
                let count = (range.length_bytes - within).min(output.len() as u64) as usize;
                self.source
                    .seek(SeekFrom::Start(range.source_byte_offset + within))?;
                let read = self.source.read(&mut output[..count])?;
                self.position += read as u64;
                return Ok(read);
            }
            base += range.length_bytes;
        }
        Err(io::Error::other("Invalid reconstruction position"))
    }
}
impl<R> Seek for ByteView<'_, R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        if position < 0 || position > self.length as i128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Seek outside reconstruction",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reordered_unaligned_ranges_are_read_exactly_across_boundaries() {
        let ranges = vec![
            ByteRange {
                source_byte_offset: 7,
                length_bytes: 3,
            },
            ByteRange {
                source_byte_offset: 1,
                length_bytes: 2,
            },
        ];
        let mut source = io::Cursor::new(b"0123456789".to_vec());
        let mut view = ByteView::new(&mut source, &ranges, 10).unwrap();
        let mut output = Vec::new();
        view.read_to_end(&mut output).unwrap();
        assert_eq!(output, b"78912");
        view.seek(SeekFrom::End(-3)).unwrap();
        let mut end = [0; 3];
        view.read_exact(&mut end).unwrap();
        assert_eq!(&end, b"912");
        assert!(view.seek(SeekFrom::End(1)).is_err());
    }
    #[test]
    fn overlap_overflow_empty_and_out_of_bounds_are_rejected() {
        assert!(validate(&[], 100).is_err());
        for ranges in [
            vec![ByteRange {
                source_byte_offset: u64::MAX,
                length_bytes: 2,
            }],
            vec![ByteRange {
                source_byte_offset: 99,
                length_bytes: 2,
            }],
            vec![ByteRange {
                source_byte_offset: 0,
                length_bytes: 0,
            }],
            vec![
                ByteRange {
                    source_byte_offset: 0,
                    length_bytes: 3
                };
                2
            ],
        ] {
            assert!(validate(&ranges, 100).is_err());
        }
    }
}
