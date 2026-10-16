use std::io::Read;

use crate::backup::{config, run, s3 as s3_client};
use crate::dtos::response::AppError;
use crate::state::state::SharedState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadFormat {
    Tar,
    Zip,
    SevenZip,
}

impl DownloadFormat {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "tar" => Some(Self::Tar),
            "zip" => Some(Self::Zip),
            "7z" => Some(Self::SevenZip),
            _ => None,
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Tar => "application/x-tar",
            Self::Zip => "application/zip",
            Self::SevenZip => "application/x-7z-compressed",
        }
    }

    fn ext(&self) -> &'static str {
        match self {
            Self::Tar => "tar",
            Self::Zip => "zip",
            Self::SevenZip => "7z",
        }
    }
}

pub async fn download_backup(
    state: &SharedState,
    backup_id: &str,
    format: DownloadFormat,
) -> Result<(Vec<u8>, String, &'static str), AppError> {
    let s3_config = config::load(state).await?.ok_or_else(|| {
        AppError::ValidationError("Configure S3 storage before downloading backups".into())
    })?;

    let backup = run::get(state, backup_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    if backup.status != "success" {
        return Err(AppError::ValidationError(
            "Only a successful backup can be downloaded".into(),
        ));
    }
    let s3_key = backup
        .s3_key
        .ok_or_else(|| AppError::ValidationError("This backup has no stored object".into()))?;

    let bucket = s3_client::client(&s3_config)?;
    let response = bucket
        .get_object(&s3_key)
        .await
        .map_err(|error| AppError::InternalError(format!("S3 download failed: {error}")))?;
    let tar_bytes = response.bytes().to_vec();

    let short_id = &backup_id[..std::cmp::min(8, backup_id.len())];
    let filename = format!("backup-{short_id}.{}", format.ext());

    let output_bytes = match format {
        DownloadFormat::Tar => tar_bytes,
        DownloadFormat::Zip => tar_to_zip(&tar_bytes)?,
        DownloadFormat::SevenZip => tar_to_7z(&tar_bytes)?,
    };

    Ok((output_bytes, filename, format.mime_type()))
}

fn tar_to_zip(tar_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    use std::io::{Cursor, Write};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    let mut tar_archive = tar::Archive::new(Cursor::new(tar_bytes));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));

    for entry in tar_archive.entries().map_err(tar_err)? {
        let mut entry = entry.map_err(tar_err)?;
        let path = entry.path().map_err(tar_err)?.into_owned();

        if !entry.header().entry_type().is_file() {
            continue;
        }

        zip.start_file(path.to_string_lossy(), options)
            .map_err(|e| AppError::InternalError(format!("zip start_file: {e}")))?;

        let mut content = Vec::new();
        entry.read_to_end(&mut content).map_err(tar_err)?;

        zip.write_all(&content)
            .map_err(|e| AppError::InternalError(format!("zip write: {e}")))?;
    }

    let cursor = zip
        .finish()
        .map_err(|e| AppError::InternalError(format!("zip finalize: {e}")))?;
    Ok(cursor.into_inner())
}

fn tar_to_7z(tar_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
    use std::io::Cursor;

    let mut tar_archive = tar::Archive::new(Cursor::new(tar_bytes));
    let mut sz = ArchiveWriter::new(Cursor::new(Vec::new()))
        .map_err(|e| AppError::InternalError(format!("7z writer init: {e}")))?;

    for entry in tar_archive.entries().map_err(tar_err)? {
        let mut entry = entry.map_err(tar_err)?;
        let path = entry.path().map_err(tar_err)?.into_owned();

        if !entry.header().entry_type().is_file() {
            continue;
        }

        let mut content = Vec::new();
        entry.read_to_end(&mut content).map_err(tar_err)?;

        let archive_entry = ArchiveEntry {
            name: path.to_string_lossy().into_owned().into(),
            ..Default::default()
        };
        sz.push_archive_entry(archive_entry, Some(Cursor::new(content)))
            .map_err(|e| AppError::InternalError(format!("7z write entry: {e}")))?;
    }

    let cursor = sz
        .finish()
        .map_err(|e| AppError::InternalError(format!("7z finalize: {e}")))?;
    Ok(cursor.into_inner())
}

fn tar_err(e: std::io::Error) -> AppError {
    AppError::InternalError(format!("tar read: {e}"))
}
