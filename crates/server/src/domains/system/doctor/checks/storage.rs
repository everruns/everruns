use super::Findings;
use crate::domains::system::doctor::{CheckArea, DoctorCheck, PROBE_TIMEOUT, SystemDoctor};
use crate::storage::blob_store::{BlobMetadata, BlobStore, BlobStoreConfig, ObjectStoreBlobStore};

const AREA: CheckArea = CheckArea::Storage;

/// Where the write probe lands, under the configured prefix, outside the
/// key spaces the blob garbage collector reconciles. The probe deletes its
/// object; one left by a failed delete is a few bytes and harmless.
const PROBE_KEY_PREFIX: &str = "_doctor/probe-";

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let mut found = Findings::default();
    let env = &doctor.env;
    let check = DoctorCheck::new(AREA, "storage.files", "File storage");
    let config = match (env.blob_backend.as_str(), &env.s3) {
        ("db" | "", _) => {
            found.fact("Files in PostgreSQL");
            found.check(check.info(
                "Files are stored in PostgreSQL. Simple, but the database grows with every \
                 upload and recording; set STORAGE_BLOB_BACKEND=s3 for larger installs.",
            ));
            return found;
        }
        ("s3", Some(Ok(config))) => config,
        ("s3", Some(Err(error))) => {
            found.check(
                check
                    .problem(format!(
                        "S3 file storage is selected but not configured: {error}."
                    ))
                    .why("Uploads, images and recordings cannot be saved.")
                    .fix("Set STORAGE_S3_BUCKET and the other STORAGE_S3_* settings."),
            );
            return found;
        }
        (other, _) => {
            found.check(
                check
                    .problem(format!(
                        "STORAGE_BLOB_BACKEND is `{other}`, which is not known."
                    ))
                    .why("The server does not start with an unknown file storage backend.")
                    .fix("Set STORAGE_BLOB_BACKEND to `db` or `s3`."),
            );
            return found;
        }
    };
    found.fact("S3 file storage");
    found.check(
        match tokio::time::timeout(PROBE_TIMEOUT, probe(config)).await {
            Ok(Ok(())) => check.ok(format!(
                "Writing, reading and deleting a test file in bucket `{}` works.",
                config.bucket
            )),
            Ok(Err(error)) => unreachable(check, config, &format!("{error:#}")),
            Err(_) => unreachable(check, config, "no answer within 3 seconds"),
        },
    );
    found
}

/// Write, read back and delete one small object with the configured
/// credentials, so a wrong region, endpoint or missing permission shows.
async fn probe(config: &BlobStoreConfig) -> anyhow::Result<()> {
    let store = ObjectStoreBlobStore::s3_from_config(config)?;
    let key = format!("{PROBE_KEY_PREFIX}{}", uuid::Uuid::now_v7());
    let body = b"everruns doctor".to_vec();
    let metadata = BlobMetadata::new("doctor_probe", serde_json::json!({}));
    store.put(&key, body.clone(), &metadata).await?;
    let read = store.get(&key).await;
    let deleted = store.delete(&key).await;
    anyhow::ensure!(read? == Some(body), "the test file read back differently");
    deleted
}

fn unreachable(check: DoctorCheck, config: &BlobStoreConfig, reason: &str) -> DoctorCheck {
    check
        .problem(format!(
            "Bucket `{}` cannot be used: {reason}.",
            config.bucket
        ))
        .why("Uploads, images and recordings fail to save, and stored ones cannot be opened.")
        .fix(
            "Check STORAGE_S3_REGION, STORAGE_S3_ENDPOINT and the credentials; they need \
             permission to put, get and delete objects in the bucket.",
        )
}
