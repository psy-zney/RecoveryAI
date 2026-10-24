#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::{self, Read, Seek, SeekFrom};
struct Memory(Vec<u8>);
impl core_io::RawStorageReader for Memory {
    fn len_bytes(&self) -> io::Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_physical_sectors(&mut self, lba: u64, count: usize, out: &mut [u8]) -> io::Result<()> {
        let start = lba
            .checked_mul(512)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| io::Error::other("overflow"))?;
        let bytes = count
            .checked_mul(512)
            .ok_or_else(|| io::Error::other("overflow"))?;
        let source = self
            .0
            .get(start..start.saturating_add(bytes))
            .ok_or_else(|| io::Error::other("range"))?;
        out.get_mut(..bytes)
            .ok_or_else(|| io::Error::other("buffer"))?
            .copy_from_slice(source);
        Ok(())
    }
}
fuzz_target!(|input: &[u8]| {
    let sector = if input.first().copied().unwrap_or(0) & 1 == 0 {
        512
    } else {
        4096
    };
    let mut disk =
        core_io::VirtualDiskOverlay::with_sector_size(Memory(vec![0; 8192]), sector).unwrap();
    let offset = input
        .get(..8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0);
    disk.seek(SeekFrom::Start(offset)).ok();
    disk.read(&mut [0; 32]).ok();
    disk.apply_virtual_patch(
        0,
        (offset % 8192) as usize,
        input.get(..input.len().min(4096)).unwrap_or(&[]),
        String::new(),
    )
    .ok();
});
