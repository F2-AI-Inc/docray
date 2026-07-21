use docray_core::ExtractError;
use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use zip::ZipArchive;

const MAX_ENTRIES: usize = 4096;
const MAX_ENTRY_SIZE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_SIZE: u64 = 512 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 200;

pub(crate) struct Package<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    bytes_read: u64,
}

impl<'a> Package<'a> {
    pub(crate) fn open(bytes: &'a [u8]) -> Result<Self, ExtractError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| ExtractError::ParseFailure(format!("invalid ZIP container: {e}")))?;
        if archive.len() > MAX_ENTRIES {
            return Err(limit(format!(
                "OPC entry count limit exceeded: {} > {MAX_ENTRIES}",
                archive.len()
            )));
        }

        let mut total = 0_u64;
        let mut names = BTreeSet::new();
        for index in 0..archive.len() {
            let entry = archive.by_index_raw(index).map_err(|e| {
                ExtractError::ParseFailure(format!("cannot inspect ZIP entry {index}: {e}"))
            })?;
            let name = entry.name();
            if name.starts_with('/') || name.contains("..") {
                return Err(limit(format!("unsafe OPC entry name rejected: {name:?}")));
            }
            if !names.insert(name.to_owned()) {
                return Err(ExtractError::ParseFailure(format!(
                    "duplicate OPC entry name rejected: {name:?}"
                )));
            }
            let size = entry.size();
            let compressed = entry.compressed_size();
            if size > MAX_ENTRY_SIZE {
                return Err(limit(format!(
                    "OPC per-entry inflated size limit exceeded for {name:?}: {size} > {MAX_ENTRY_SIZE}"
                )));
            }
            total = total
                .checked_add(size)
                .ok_or_else(|| limit("OPC total inflated size limit overflowed".to_string()))?;
            if total > MAX_TOTAL_SIZE {
                return Err(limit(format!(
                    "OPC total inflated size limit exceeded: {total} > {MAX_TOTAL_SIZE}"
                )));
            }
            if size > 0
                && (compressed == 0
                    || u128::from(size)
                        > u128::from(compressed) * u128::from(MAX_COMPRESSION_RATIO))
            {
                return Err(limit(format!(
                    "OPC compression-ratio limit exceeded for {name:?}: {size}:{compressed} > {MAX_COMPRESSION_RATIO}:1"
                )));
            }
        }

        Ok(Self {
            archive,
            bytes_read: 0,
        })
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.archive.index_for_name(name).is_some()
    }

    /// Reads a single exact OPC part. Entries are never extracted to disk and
    /// callers cannot request directory prefixes or wildcard matches.
    pub(crate) fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, ExtractError> {
        let Ok(mut entry) = self.archive.by_name(name) else {
            return Ok(None);
        };
        let expected = entry.size();
        if expected > MAX_ENTRY_SIZE {
            return Err(limit(format!(
                "OPC per-entry inflated size limit exceeded for {name:?}: {expected} > {MAX_ENTRY_SIZE}"
            )));
        }
        let mut bytes = Vec::with_capacity(expected.min(1024 * 1024) as usize);
        entry
            .by_ref()
            .take(MAX_ENTRY_SIZE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                ExtractError::ParseFailure(format!("cannot inflate OPC part {name:?}: {e}"))
            })?;
        if bytes.len() as u64 > MAX_ENTRY_SIZE {
            return Err(limit(format!(
                "OPC per-entry inflated size limit exceeded while reading {name:?}"
            )));
        }
        self.bytes_read = self
            .bytes_read
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| limit("OPC total inflated size limit overflowed".to_string()))?;
        if self.bytes_read > MAX_TOTAL_SIZE {
            return Err(limit(format!(
                "OPC total inflated size limit exceeded while reading parts: {} > {MAX_TOTAL_SIZE}",
                self.bytes_read
            )));
        }
        Ok(Some(bytes))
    }

    pub(crate) fn read_required(&mut self, name: &str) -> Result<Vec<u8>, ExtractError> {
        self.read(name)?.ok_or_else(|| {
            ExtractError::ParseFailure(format!("required OPC part is missing: {name}"))
        })
    }
}

fn limit(message: String) -> ExtractError {
    ExtractError::ParseFailure(message)
}
