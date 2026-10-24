//! Append-only hit spool with byte-offset cursors and a bounded JSON page.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
};
pub struct HitWriter {
    file: File,
    count: u64,
}
impl HitWriter {
    pub fn create(path: &Path) -> Result<Self> {
        Ok(Self {
            file: OpenOptions::new().write(true).create_new(true).open(path)?,
            count: 0,
        })
    }
    pub fn append(&mut self, mut value: Value) -> Result<()> {
        value
            .as_object_mut()
            .context("Hit must be an object")?
            .insert("hit_index".into(), self.count.into());
        let mut row = serde_json::to_vec(&value)?;
        ensure!(row.len() <= 1024, "Hit record exceeds spool budget");
        row.push(b'\n');
        self.file.write_all(&row)?;
        self.count += 1;
        Ok(())
    }
    pub fn finish(self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}
pub fn page(path: &Path, cursor: u64, limit: usize) -> Result<Value> {
    ensure!((1..=100).contains(&limit), "limit must be 1..100");
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    ensure!(cursor <= length, "Cursor exceeds stored hit length");
    if cursor > 0 {
        file.seek(SeekFrom::Start(cursor - 1))?;
        let mut byte = [0];
        file.read_exact(&mut byte)?;
        ensure!(byte == *b"\n", "Cursor must point to a JSONL row boundary");
    }
    file.seek(SeekFrom::Start(cursor))?;
    let mut reader = BufReader::new(file);
    let mut rows = Vec::new();
    let mut next = cursor;
    let mut budget = 0;
    while rows.len() < limit {
        let mut row = Vec::new();
        let size = (&mut reader).take(1025).read_until(b'\n', &mut row)?;
        if size == 0 {
            break;
        }
        ensure!(size <= 1025, "Malformed stored hit row");
        if row.last() != Some(&b'\n') {
            ensure!(size < 1025, "Malformed stored hit row");
            break;
        } // Writer has not finished this row yet.
        let value: Value = serde_json::from_slice(&row)?;
        let encoded = serde_json::to_vec(&value)?.len();
        if budget + encoded > 2200 {
            break;
        }
        budget += encoded;
        rows.push(value);
        next += size as u64;
    }
    Ok(
        json!({"hits":rows,"cursor":cursor,"next_cursor":next,"has_more":next<length,"count":rows.len()}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximum_record_and_incomplete_tail_preserve_cursor_boundaries() {
        let path =
            std::env::temp_dir().join(format!("resc-hit-tail-{}.jsonl", uuid::Uuid::new_v4()));
        let empty = json!({"name":"","hit_index":0}).to_string().len();
        let mut writer = HitWriter::create(&path).unwrap();
        writer
            .append(json!({"name":"x".repeat(1024-empty)}))
            .unwrap();
        writer.finish().unwrap();
        let first = page(&path, 0, 1).unwrap();
        assert_eq!(first["count"], 1);
        let cursor = first["next_cursor"].as_u64().unwrap();
        let mut append = OpenOptions::new().append(true).open(&path).unwrap();
        append.write_all(b"{\"hit_index\":1").unwrap();
        let partial = page(&path, cursor, 1).unwrap();
        assert_eq!(partial["count"], 0);
        assert_eq!(partial["next_cursor"], cursor);
        assert_eq!(partial["has_more"], true);
        append.write_all(b"}\n").unwrap();
        assert_eq!(page(&path, cursor, 1).unwrap()["count"], 1);
        drop(append);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn all_hits_are_paginated_without_rescanning() {
        let path = std::env::temp_dir().join(format!("resc-hits-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut writer = HitWriter::create(&path).unwrap();
        for i in 0..1000 {
            writer.append(json!({"lba":i,"extension":"jpg"})).unwrap();
        }
        writer.finish().unwrap();
        let (mut cursor, mut count) = (0, 0);
        loop {
            let page = page(&path, cursor, 100).unwrap();
            assert!(page.to_string().len() < 5000);
            count += page["count"].as_u64().unwrap();
            cursor = page["next_cursor"].as_u64().unwrap();
            if !page["has_more"].as_bool().unwrap() {
                break;
            }
        }
        assert_eq!(count, 1000);
        assert!(page(&path, 1, 10).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
