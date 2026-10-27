//! Evidence-led partition/boot recovery proposals. This module never writes.
use anyhow::{ensure, Context, Result};
use core_io::{RawStorageReader, VirtualDiskOverlay};
use disk_parsers::{fat_parser::analyze_fat32, mbr_gpt::GptHeader, ntfs_parser::analyze_ntfs_vbr};
use serde_json::{json, Value};

const MAX_VOLUMES: usize = 3;
const MAX_GPT_COPY_SECTORS: u64 = 64;

fn read_sector<R: RawStorageReader>(disk: &mut VirtualDiskOverlay<R>, lba: u64) -> Result<Vec<u8>> {
    let mut bytes = vec![0; disk.sector_size()];
    disk.read_sectors(lba, 1, &mut bytes)?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}

fn gpt_proposal<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    source_lba: u64,
    target_lba: u64,
    capacity: u64,
) -> Result<Value> {
    let sector = read_sector(disk, source_lba)?;
    let header = GptHeader::from_sector(&sector).context("Validated GPT header disappeared")?;
    // Unknown future header extensions cannot safely be regenerated from v1 fields.
    ensure!(
        header.header_size == 92,
        "GPT proposal supports the standard 92-byte header only"
    );
    ensure!(
        header.partition_entry_size.is_power_of_two(),
        "GPT entry size must be 128 times a power of two"
    );
    let array_bytes = (header.num_partition_entries as u64)
        .checked_mul(header.partition_entry_size as u64)
        .context("GPT array size overflow")?;
    let blocks = array_bytes.div_ceil(disk.sector_size() as u64);
    ensure!(
        blocks <= MAX_GPT_COPY_SECTORS,
        "GPT proposal exceeds 64-sector array-copy budget"
    );
    let target_array = if target_lba == 1 {
        2
    } else {
        target_lba
            .checked_sub(blocks)
            .context("GPT backup metadata underflow")?
    };
    let target_end = target_array
        .checked_add(blocks)
        .context("GPT array overflow")?;
    ensure!(
        target_end <= capacity && target_array != source_lba && target_lba < capacity,
        "GPT target outside metadata bounds"
    );
    ensure!(
        if target_lba == 1 {
            target_end <= header.first_usable_lba
        } else {
            target_array > header.last_usable_lba && target_end <= target_lba
        },
        "Proposed GPT array overlaps usable data"
    );
    let source_array = header.partition_entry_start_lba;
    let mut replacement = sector[..92].to_vec();
    replacement[16..20].fill(0);
    replacement[24..32].copy_from_slice(&target_lba.to_le_bytes());
    replacement[32..40].copy_from_slice(&source_lba.to_le_bytes());
    replacement[72..80].copy_from_slice(&target_array.to_le_bytes());
    let crc = crc32fast::hash(&replacement);
    replacement[16..20].copy_from_slice(&crc.to_le_bytes());
    Ok(json!({
        "scope":"overlay_only", "applied":false,
        "source_header_lba":source_lba, "target_header_lba":target_lba,
        "array_copy":{"copy_from_lba":source_array,"to_lba":target_array,"count_sectors":blocks,"array_bytes":array_bytes},
        "header_patch":{"lba":target_lba,"offset":0,"hex_data":hex(&replacement)},
        "execution":"Copy each array sector through virtual_patch_and_verify; then apply header_patch. Never copy an unmodified backup header over the primary.",
        "verification":"Rerun partition analysis and native filesystem checks; no source write or mount guarantee"
    }))
}

fn ntfs_valid(bytes: &[u8], sector_size: u32, available: u64) -> bool {
    let value = analyze_ntfs_vbr(bytes);
    value.valid && value.bytes_per_sector as u32 == sector_size && value.total_sectors <= available
}

fn fat_valid(bytes: &[u8], sector_size: u32, available: u64) -> bool {
    let value = analyze_fat32(bytes);
    let clusters = (value.total_sectors as u64)
        .saturating_sub(value.data_start_lba as u64)
        .checked_div(value.sectors_per_cluster as u64)
        .unwrap_or(0);
    value.valid
        && value.bytes_per_sector as u32 == sector_size
        && value.total_sectors as u64 <= available
        && value.backup_boot_sector > 0
        && value.backup_boot_sector < value.reserved_sectors
        && (65525..0x0ffffff5).contains(&clusters)
        && value.fat_size_sectors as u64 * sector_size as u64 / 4 >= clusters + 2
        && value.root_cluster >= 2
        && value.root_cluster as u64 <= clusters + 1
}

fn volume<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    start: u64,
    partition_sectors: u64,
    sector_size: u32,
    confirmed_bounds: bool,
) -> Result<Value> {
    let primary = read_sector(disk, start)?;
    let ntfs = analyze_ntfs_vbr(&primary);
    let fat = analyze_fat32(&primary);
    let primary_ntfs = ntfs_valid(&primary, sector_size, partition_sectors);
    let primary_fat = fat_valid(&primary, sector_size, partition_sectors);
    // When the primary is unparseable, these are bounded hypotheses, not facts.
    // NTFS BPB count may exclude the reserved backup sector or reflect a shrunk
    // filesystem. Prefer the partition/image boundary, not BPB.total - 1.
    let ntfs_relative = partition_sectors - 1;
    let fat_relative = if primary_fat {
        fat.backup_boot_sector as u64
    } else {
        6
    };
    let mut candidates = Vec::new();
    for (kind, relative) in [("NTFS", ntfs_relative), ("FAT32", fat_relative)] {
        if relative == 0 || relative >= partition_sectors {
            continue;
        }
        let lba = start
            .checked_add(relative)
            .context("Backup boot address overflow")?;
        let backup = read_sector(disk, lba)?;
        let valid = if kind == "NTFS" {
            ntfs_valid(&backup, sector_size, partition_sectors)
        } else {
            let parsed = analyze_fat32(&backup);
            fat_valid(&backup, sector_size, partition_sectors)
                && parsed.backup_boot_sector as u64 == relative
        };
        let primary_valid = if kind == "NTFS" {
            primary_ntfs
        } else {
            primary_fat
        };
        if !valid && !primary_valid {
            continue;
        }
        let geometry_matches = if kind == "NTFS" {
            primary[11..14] == backup[11..14] && primary[40..80] == backup[40..80]
        } else {
            primary[11..17] == backup[11..17]
                && primary[32..52] == backup[32..52]
                && primary[67..71] == backup[67..71]
        };
        let same_sector = primary == backup;
        let mut candidate = json!({"filesystem":kind,"backup_lba":lba,"primary_valid":primary_valid,"backup_valid":valid,"geometry_and_serial_match":geometry_matches,"sectors_identical":same_sector});
        if valid && !primary_valid {
            candidate["classification"] = json!("BACKUP_BOOT_CANDIDATE");
            candidate["overlay_proposal"] = json!({"copy_from_lba":lba,"to_lba":start,"partition_start_lba":start,"requires_volume_identity_confirmation":!geometry_matches,"requires_boundary_confirmation":!confirmed_bounds,"applied":false});
        } else if primary_valid && valid && !geometry_matches {
            candidate["classification"] = json!("PRIMARY_BACKUP_DISAGREE_NO_AUTOMATIC_REPAIR");
        } else if primary_valid && !valid {
            candidate["classification"] = json!("PRIMARY_VALID_BACKUP_INVALID");
            if kind == "FAT32" || (confirmed_bounds && ntfs.total_sectors < partition_sectors) {
                candidate["overlay_proposal"] = json!({"copy_from_lba":start,"to_lba":lba,"partition_start_lba":start,"requires_volume_identity_confirmation":true,"applied":false});
            } else {
                candidate["proposal_unavailable"] = json!("Cannot prove this target is a reserved backup sector rather than filesystem data");
            }
        } else {
            candidate["classification"] = json!(if same_sector {
                "PRIMARY_BACKUP_MATCH"
            } else {
                "GEOMETRY_MATCH_BOOT_BYTES_DIFFER"
            });
        }
        candidates.push(candidate);
    }
    Ok(
        json!({"partition_start_lba":start,"partition_sectors":partition_sectors,"boundary_source":if confirmed_bounds {"validated_partition_table"} else {"image_remainder_assumption"},"primary_classification":if primary_ntfs {"NTFS"} else if primary_fat {"FAT32"} else {"UNVERIFIED"},"backup_candidates":candidates,"next_step":if primary_ntfs || primary_fat {"Validate native root traversal; boot geometry alone is not a mount test"} else {"Confirm volume identity before any overlay proposal; use VBR hunt when no backup candidate exists"}}),
    )
}

/// Inspect verified partition structures and propose, but never apply, overlay fixes.
pub fn analyze<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    sector_size: u32,
    partition_start_lba: Option<u64>,
) -> Result<Value> {
    ensure!(
        matches!(sector_size, 512 | 4096) && disk.sector_size() == sector_size as usize,
        "Sector size disagrees with session geometry"
    );
    let capacity = disk.len_bytes() / sector_size as u64;
    ensure!(capacity > 0, "Empty source");
    if let Some(start) = partition_start_lba {
        ensure!(start < capacity, "Partition start exceeds source capacity");
    }
    let layout = crate::topology::read_layout(disk, sector_size)?;
    let gpt_classification = match (layout.primary_valid, layout.backup_valid) {
        (true, true) => "CONSISTENT_COPIES",
        (true, false) => "PRIMARY_ONLY",
        (false, true) => "BACKUP_ONLY",
        (false, false) if layout.anomalies.iter().any(|a| a.contains("GPT disagree")) => {
            "VALID_COPIES_CONFLICT"
        }
        _ => "INVALID_OR_MISSING",
    };
    let mut gpt = json!({"classification":gpt_classification,"primary_valid":layout.primary_valid,"backup_valid":layout.backup_valid,"protective_mbr_present":layout.mbr_valid && layout.mbr.is_gpt_protective});
    if layout.primary_valid != layout.backup_valid {
        let (source, target) = if layout.primary_valid {
            (1, capacity - 1)
        } else {
            (capacity - 1, 1)
        };
        match gpt_proposal(disk, source, target, capacity) {
            Ok(proposal) => gpt["overlay_proposal"] = proposal,
            Err(error) => gpt["proposal_unavailable"] = json!(error.to_string()),
        }
    }
    let mut selected: Vec<(u64, u64, bool)> = match partition_start_lba {
        Some(start) => {
            let known = layout.partitions.iter().find(|p| p.start_lba == start);
            vec![(
                start,
                known.map_or(capacity - start, |p| p.sector_count),
                known.is_some(),
            )]
        }
        None if !layout.partitions.is_empty() => layout
            .partitions
            .iter()
            .map(|p| (p.start_lba, p.sector_count, true))
            .collect(),
        None => vec![(0, capacity, false)],
    };
    let total = selected.len();
    selected.truncate(MAX_VOLUMES);
    let mut volumes = Vec::new();
    for (start, count, confirmed) in selected {
        volumes.push(volume(disk, start, count, sector_size, confirmed)?);
    }
    let examined = volumes.len();
    let mut report = json!({"case":"PARTITION_OR_BOOT_DAMAGE","read_only":true,"repairs_applied":false,"sector_size":sector_size,"gpt":gpt,"volumes":volumes,"volumes_examined":examined,"volumes_available":total,"volumes_truncated":total>MAX_VOLUMES,"limits":{"max_volumes":MAX_VOLUMES,"max_gpt_array_copy_sectors":MAX_GPT_COPY_SECTORS},"limitations":["Backup locations without primary geometry are hypotheses; confirm volume identity","GPT repair changes header addresses and CRC; copying the backup header unchanged is invalid","No MBR reconstruction, partition resize or source writes; use an explicit partition_start_lba to inspect another volume"]});
    while serde_json::to_vec(&report)?.len() > 3400 {
        let values = report["volumes"]
            .as_array_mut()
            .context("Invalid report volumes")?;
        if values.is_empty() {
            break;
        }
        values.pop();
        report["volumes_truncated"] = json!(true);
    }
    report["volumes_returned"] = json!(report["volumes"].as_array().map_or(0, Vec::len));
    ensure!(
        serde_json::to_vec(&report)?.len() < 3500,
        "Partition report exceeds compact budget"
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, sync::Arc};
    struct Memory(Arc<Vec<u8>>);
    impl RawStorageReader for Memory {
        fn len_bytes(&self) -> io::Result<u64> {
            Ok(self.0.len() as u64)
        }
        fn read_physical_sectors(
            &mut self,
            lba: u64,
            count: usize,
            output: &mut [u8],
        ) -> io::Result<()> {
            let start = lba as usize * 512;
            let len = count * 512;
            output[..len].copy_from_slice(
                self.0
                    .get(start..start + len)
                    .ok_or_else(|| io::Error::other("read exceeds fixture"))?,
            );
            Ok(())
        }
    }
    fn disk(bytes: Vec<u8>, sector: usize) -> VirtualDiskOverlay<Memory> {
        VirtualDiskOverlay::with_sector_size(Memory(Arc::new(bytes)), sector).unwrap()
    }
    fn ntfs(total: u64, sector: u16) -> Vec<u8> {
        let mut data = vec![0; sector as usize];
        data[3..11].copy_from_slice(b"NTFS    ");
        data[11..13].copy_from_slice(&sector.to_le_bytes());
        data[13] = 8;
        data[21] = 0xf8;
        data[40..48].copy_from_slice(&total.to_le_bytes());
        data[48..56].copy_from_slice(&4u64.to_le_bytes());
        data[56..64].copy_from_slice(&2u64.to_le_bytes());
        data[64] = (-10i8) as u8;
        data[510..512].copy_from_slice(&[0x55, 0xaa]);
        data
    }
    #[test]
    fn damaged_ntfs_primary_proposes_only_overlay_and_checks_backup_geometry() {
        let mut image = vec![0; 128 * 512];
        let boot = ntfs(128, 512);
        image[..512].copy_from_slice(&boot);
        image[510] = 0;
        image[127 * 512..].copy_from_slice(&boot);
        let mut overlay = disk(image, 512);
        let before = overlay.revision();
        let report = analyze(&mut overlay, 512, Some(0)).unwrap();
        let proposal = &report["volumes"][0]["backup_candidates"][0]["overlay_proposal"];
        assert_eq!(proposal["copy_from_lba"], 127);
        assert_eq!(proposal["to_lba"], 0);
        assert_eq!(proposal["requires_volume_identity_confirmation"], false);
        assert_eq!(overlay.revision(), before);
        assert_eq!(overlay.patch_count(), 0);
        assert!(serde_json::to_vec(&report).unwrap().len() < 3500);
    }
    #[test]
    fn wrong_backup_capacity_and_wrong_session_geometry_never_propose_repairs() {
        let mut image = vec![0; 128 * 512];
        image[127 * 512..].copy_from_slice(&ntfs(256, 512));
        let mut overlay = disk(image, 512);
        let report = analyze(&mut overlay, 512, Some(0)).unwrap();
        assert!(report["volumes"][0]["backup_candidates"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(analyze(&mut overlay, 4096, None).is_err());
        assert!(analyze(&mut overlay, 512, Some(128)).is_err());
    }
    #[test]
    fn four_kn_backup_addresses_use_logical_sectors() {
        let mut image = vec![0; 128 * 4096];
        let boot = ntfs(128, 4096);
        image[..4096].copy_from_slice(&boot);
        image[510] = 0;
        image[127 * 4096..].copy_from_slice(&boot);
        let report = analyze(&mut disk(image, 4096), 4096, Some(0)).unwrap();
        assert_eq!(
            report["volumes"][0]["backup_candidates"][0]["backup_lba"],
            127
        );
    }

    #[test]
    fn ntfs_backup_location_uses_partition_boundary_when_bpb_excludes_backup_sector() {
        let mut image = vec![0; 128 * 512];
        let boot = ntfs(127, 512);
        image[..512].copy_from_slice(&boot);
        image[510] = 0;
        image[127 * 512..].copy_from_slice(&boot);
        let report = analyze(&mut disk(image, 512), 512, Some(0)).unwrap();
        assert_eq!(
            report["volumes"][0]["backup_candidates"][0]["backup_lba"],
            127
        );
        assert_eq!(
            report["volumes"][0]["backup_candidates"][0]["overlay_proposal"]["copy_from_lba"],
            127
        );
    }

    fn gpt_image() -> Vec<u8> {
        let sectors = 128u64;
        let mut image = vec![0; sectors as usize * 512];
        image[450] = 0xee;
        image[454..458].copy_from_slice(&1u32.to_le_bytes());
        image[458..462].copy_from_slice(&127u32.to_le_bytes());
        image[510..512].copy_from_slice(&[0x55, 0xaa]);
        let mut entries = vec![0; 128 * 128];
        entries[0] = 1;
        entries[16] = 2;
        entries[32..40].copy_from_slice(&40u64.to_le_bytes());
        entries[40..48].copy_from_slice(&60u64.to_le_bytes());
        for (lba, alternate, array) in [(1u64, 127u64, 2u64), (127, 1, 95)] {
            let mut header = vec![0; 512];
            header[..8].copy_from_slice(b"EFI PART");
            header[8..12].copy_from_slice(&0x10000u32.to_le_bytes());
            header[12..16].copy_from_slice(&92u32.to_le_bytes());
            header[24..32].copy_from_slice(&lba.to_le_bytes());
            header[32..40].copy_from_slice(&alternate.to_le_bytes());
            header[40..48].copy_from_slice(&34u64.to_le_bytes());
            header[48..56].copy_from_slice(&94u64.to_le_bytes());
            header[56..72].copy_from_slice(&[9; 16]);
            header[72..80].copy_from_slice(&array.to_le_bytes());
            header[80..84].copy_from_slice(&128u32.to_le_bytes());
            header[84..88].copy_from_slice(&128u32.to_le_bytes());
            header[88..92].copy_from_slice(&crc32fast::hash(&entries).to_le_bytes());
            let crc = crc32fast::hash(&header[..92]);
            header[16..20].copy_from_slice(&crc.to_le_bytes());
            image[lba as usize * 512..(lba as usize + 1) * 512].copy_from_slice(&header);
            image[array as usize * 512..array as usize * 512 + entries.len()]
                .copy_from_slice(&entries);
        }
        image
    }

    #[test]
    fn gpt_backup_repair_regenerates_locations_and_crc_without_applying() {
        let mut image = gpt_image();
        image[512 + 16] ^= 1;
        let mut overlay = disk(image, 512);
        let report = analyze(&mut overlay, 512, None).unwrap();
        let proposal = &report["gpt"]["overlay_proposal"];
        assert_eq!(proposal["array_copy"]["copy_from_lba"], 95);
        assert_eq!(proposal["array_copy"]["to_lba"], 2);
        assert_eq!(proposal["array_copy"]["count_sectors"], 32);
        let encoded = proposal["header_patch"]["hex_data"].as_str().unwrap();
        let bytes: Vec<u8> = (0..encoded.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).unwrap())
            .collect();
        let header = GptHeader::from_sector(&bytes).unwrap();
        assert_eq!({ header.my_lba }, 1);
        assert_eq!({ header.alternate_lba }, 127);
        assert_eq!({ header.partition_entry_start_lba }, 2);
        assert_eq!(overlay.patch_count(), 0);
        assert!(serde_json::to_vec(&report).unwrap().len() < 3500);
    }

    #[test]
    fn conflicting_valid_gpt_headers_do_not_receive_repair_proposals() {
        let mut image = gpt_image();
        let backup = &mut image[127 * 512..128 * 512];
        backup[56] ^= 1;
        backup[16..20].fill(0);
        let crc = crc32fast::hash(&backup[..92]);
        backup[16..20].copy_from_slice(&crc.to_le_bytes());
        let report = analyze(&mut disk(image, 512), 512, None).unwrap();
        assert!(report["gpt"].get("overlay_proposal").is_none());
        assert_eq!(report["gpt"]["primary_valid"], false);
        assert_eq!(report["gpt"]["backup_valid"], false);
        assert_eq!(report["gpt"]["classification"], "VALID_COPIES_CONFLICT");
    }

    #[test]
    fn independently_applying_a_proposal_restores_gpt_checks_only_in_the_overlay() {
        let mut image = gpt_image();
        image[127 * 512 + 16] ^= 1;
        image[95 * 512] ^= 1;
        let source = Arc::new(image);
        let original = source.as_ref().clone();
        let mut overlay = VirtualDiskOverlay::new(Memory(Arc::clone(&source))).unwrap();
        let report = analyze(&mut overlay, 512, None).unwrap();
        let proposal = &report["gpt"]["overlay_proposal"];
        assert_eq!(proposal["array_copy"]["to_lba"], 95);
        assert_eq!(overlay.patch_count(), 0);
        let from = proposal["array_copy"]["copy_from_lba"].as_u64().unwrap();
        let to = proposal["array_copy"]["to_lba"].as_u64().unwrap();
        let count = proposal["array_copy"]["count_sectors"].as_u64().unwrap();
        for index in 0..count {
            let bytes = read_sector(&mut overlay, from + index).unwrap();
            overlay
                .apply_virtual_patch(to + index, 0, &bytes, "fixture proposal".into())
                .unwrap();
        }
        let patch = &proposal["header_patch"];
        let encoded = patch["hex_data"].as_str().unwrap();
        let bytes: Vec<u8> = (0..encoded.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).unwrap())
            .collect();
        overlay
            .apply_virtual_patch(
                patch["lba"].as_u64().unwrap(),
                0,
                &bytes,
                "fixture header".into(),
            )
            .unwrap();
        let verified = crate::topology::read_layout(&mut overlay, 512).unwrap();
        assert!(verified.primary_valid && verified.backup_valid);
        assert_eq!(source.as_ref(), &original);
    }

    #[test]
    fn fat32_backup_uses_reserved_region_and_rejects_out_of_region_pointer() {
        let sectors = 70000usize;
        let mut boot = vec![0; 512];
        boot[3..11].copy_from_slice(b"MSDOS5.0");
        boot[11..13].copy_from_slice(&512u16.to_le_bytes());
        boot[13] = 1;
        boot[14..16].copy_from_slice(&32u16.to_le_bytes());
        boot[16] = 2;
        boot[32..36].copy_from_slice(&(sectors as u32).to_le_bytes());
        boot[36..40].copy_from_slice(&550u32.to_le_bytes());
        boot[44..48].copy_from_slice(&2u32.to_le_bytes());
        boot[50..52].copy_from_slice(&6u16.to_le_bytes());
        boot[82..90].copy_from_slice(b"FAT32   ");
        boot[510..512].copy_from_slice(&[0x55, 0xaa]);
        let mut image = vec![0; sectors * 512];
        image[..512].copy_from_slice(&boot);
        image[510] = 0;
        image[6 * 512..7 * 512].copy_from_slice(&boot);
        let report = analyze(&mut disk(image, 512), 512, Some(0)).unwrap();
        assert_eq!(
            report["volumes"][0]["backup_candidates"][0]["filesystem"],
            "FAT32"
        );
        assert_eq!(
            report["volumes"][0]["backup_candidates"][0]["overlay_proposal"]["copy_from_lba"],
            6
        );
        boot[50..52].copy_from_slice(&32u16.to_le_bytes());
        assert!(!fat_valid(&boot, 512, sectors as u64));
    }
}
