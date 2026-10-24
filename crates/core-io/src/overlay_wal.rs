//! Framed, checksummed write-ahead journal. Only complete synced records replay.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum WalRecord {
    Header {
        version: u32,
        sector_size: usize,
        length: u64,
        source_identity: String,
    },
    Patch {
        lba: u64,
        offset: usize,
        size: usize,
        before: Vec<u8>,
        after: Vec<u8>,
        description: String,
        id: u64,
    },
    Undo {
        id: u64,
    },
    Clear,
}
pub struct OverlayWal {
    file: File,
    poisoned: bool,
}
impl OverlayWal {
    pub fn open(
        path: &Path,
        sector_size: usize,
        length: u64,
        identity: &str,
    ) -> io::Result<(Self, Vec<WalRecord>)> {
        let created = !path.exists();
        let mut file = if created {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)?
        } else {
            OpenOptions::new().read(true).write(true).open(path)?
        };
        file.try_lock().map_err(io::Error::other)?;
        if file.metadata()?.len() > 256 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Overlay WAL exceeds replay budget",
            ));
        }
        let mut records = Vec::new();
        let mut end = 0u64;
        loop {
            let mut header = [0u8; 8];
            match file.read_exact(&mut header) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error),
            }
            let len =
                u32::from_le_bytes(header[..4].try_into().map_err(io::Error::other)?) as usize;
            let crc = u32::from_le_bytes(header[4..].try_into().map_err(io::Error::other)?);
            if len == 0 || len > 131072 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid WAL record length",
                ));
            }
            let mut payload = vec![0; len];
            match file.read_exact(&mut payload) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error),
            }
            if crc32fast::hash(&payload) != crc {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Overlay WAL checksum mismatch",
                ));
            }
            records.push(
                serde_json::from_slice(&payload)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            );
            if records.len() > 131073 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WAL record count exceeds budget",
                ));
            }
            end = file.stream_position()?;
        }
        if !records.is_empty() {
            match records.first() {
                Some(WalRecord::Header {
                    version: 1,
                    sector_size: size,
                    length: bytes,
                    source_identity,
                }) if *size == sector_size && *bytes == length && source_identity == identity => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "WAL source identity or sector geometry mismatch",
                    ))
                }
            }
        } else if !created {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing durable WAL header",
            ));
        }
        file.set_len(end)?;
        file.seek(SeekFrom::Start(end))?;
        let mut wal = Self {
            file,
            poisoned: false,
        };
        if records.is_empty() {
            if !created {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Missing durable WAL header",
                ));
            }
            wal.append(&WalRecord::Header {
                version: 1,
                sector_size,
                length,
                source_identity: identity.into(),
            })?;
        } else {
            match records.first() {
                Some(WalRecord::Header {
                    version: 1,
                    sector_size: size,
                    length: bytes,
                    source_identity,
                }) if *size == sector_size && *bytes == length && source_identity == identity => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "WAL source identity or sector geometry mismatch",
                    ))
                }
            }
            records.remove(0);
        }
        Ok((wal, records))
    }
    pub fn append(&mut self, record: &WalRecord) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(
                "WAL requires recovery after failed append",
            ));
        }
        let payload = serde_json::to_vec(record).map_err(io::Error::other)?;
        if payload.len() > 131072 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "WAL record too large",
            ));
        }
        let position = self.file.stream_position()?;
        if position + payload.len() as u64 + 8 > 256 * 1024 * 1024 {
            return Err(io::Error::other("WAL storage budget exceeded"));
        }
        let result = (|| {
            self.file.write_all(&(payload.len() as u32).to_le_bytes())?;
            self.file
                .write_all(&crc32fast::hash(&payload).to_le_bytes())?;
            self.file.write_all(&payload)?;
            self.file.sync_all()
        })();
        if result.is_err() {
            self.poisoned = self
                .file
                .set_len(position)
                .and_then(|()| self.file.seek(SeekFrom::Start(position)).map(|_| ()))
                .and_then(|()| self.file.sync_all())
                .is_err();
        }
        result
    }
}
