//! Benign fixtures for the ZIP9 filename API and the PostgreSQL install path.
use std::io::{Cursor, Write};

use postgresql_archive::configuration::zonky;
use postgresql_archive::extractor::{ExtractDirectories, zip_extract};
use regex_lite::Regex;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn archive(entries: &[(&str, &[u8])]) -> anyhow::Result<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer.start_file(*name, SimpleFileOptions::default())?;
        writer.write_all(bytes)?;
    }
    Ok(writer.finish()?.into_inner())
}

#[test]
fn ordinary_zip_extracts_decoded_filename_and_content() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let mut directories = ExtractDirectories::default();
    directories.add_mapping(Regex::new(".*")?, root.path().to_path_buf());
    let bytes = archive(&[("bin/postgres", b"fixture")])?;
    let files = zip_extract(&bytes, &directories)?;
    assert_eq!(files, vec![root.path().join("postgres")]);
    assert_eq!(std::fs::read(&files[0])?, b"fixture");
    Ok(())
}

#[test]
fn zonky_extracts_nested_txz_and_reuses_installation() -> anyhow::Result<()> {
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(7);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "bin/postgres", Cursor::new(b"fixture"))?;
    let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 1);
    encoder.write_all(&tar.into_inner()?)?;
    let txz = encoder.finish()?;
    let bytes = archive(&[("README", b"metadata"), ("postgres.txz", &txz)])?;
    let root = tempfile::tempdir()?;
    let installation = root.path().join("postgresql");
    let mut directories = ExtractDirectories::default();
    directories.add_mapping(Regex::new(".*")?, installation.clone());
    assert_eq!(zonky::extract(&bytes, &directories)?.len(), 1);
    assert_eq!(
        std::fs::read(installation.join("bin/postgres"))?,
        b"fixture"
    );
    assert!(zonky::extract(&bytes, &directories)?.is_empty());
    assert!(!root.path().join("postgresql-archive.lock").exists());
    Ok(())
}

#[test]
fn zonky_refuses_an_archive_without_postgresql_payload() -> anyhow::Result<()> {
    let bytes = archive(&[("README", b"metadata")])?;
    let root = tempfile::tempdir()?;
    let mut directories = ExtractDirectories::default();
    directories.add_mapping(Regex::new(".*")?, root.path().join("postgresql"));
    assert!(zonky::extract(&bytes, &directories).is_err());
    assert!(!root.path().join("postgresql/bin/postgres").exists());
    Ok(())
}
