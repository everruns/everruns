use super::{types::*, *};
use anyhow::Result;
use std::time::Duration;
impl KnowledgeService {
    pub async fn run_once(&self) -> Result<bool> {
        let token = Uuid::new_v4();
        let job:Option<Job>=sqlx::query_as("UPDATE knowledge_jobs SET status='running',lease_token=$1,lease_until=now()+interval '5 minutes',attempts=attempts+1 WHERE id=(SELECT id FROM knowledge_jobs WHERE (status='pending' OR (status='running' AND lease_until<now())) AND available_at<=now() ORDER BY available_at,created_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING *").bind(token).fetch_optional(self.db.tx_pool()).await?;
        let Some(job) = job else { return Ok(false) };
        let processing = async {
            let c = match self
                .collection(job.org_id, &public(job.collection_id, "kn_"))
                .await
            {
                Ok(c) => c,
                Err(e) if e.status() == axum::http::StatusCode::NOT_FOUND => return Ok(()),
                Err(e) => return Err(e.into()),
            };
            if c.revision != job.revision {
                return Ok(());
            }
            match job.kind.as_str() {
                "index" => self.index_file(&c, &job).await?,
                "source" => self.sync_source(&c, &job).await?,
                "reindex" => {
                    let mut after = job.cursor.clone();
                    loop {
                        let files:Vec<File>=sqlx::query_as("SELECT * FROM knowledge_files WHERE org_id=$1 AND collection_id=$2 AND deleted_at IS NULL AND path>$3 ORDER BY path LIMIT 100").bind(c.org_id).bind(c.id).bind(&after).fetch_all(self.db.tx_pool()).await?;
                        if files.is_empty() {
                            break;
                        }
                        for file in files {
                            self.enqueue(&c, "index", None, Some(file.id), file.current_version)
                                .await?;
                            after = file.path;
                        }
                        if !self.heartbeat(&job, &after).await? {
                            return Ok(());
                        }
                    }
                    self.publish_generation(&c).await?;
                }
                _ => {}
            };
            anyhow::Ok(())
        };
        // Keep source fetches and slow embedding batches leased independently of
        // their progress checkpoints. Dropping the future fences publication;
        // any already-started blocking fetch can only finish in its temp dir.
        let renew = async {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let live = sqlx::query("UPDATE knowledge_jobs j SET lease_until=now()+interval '5 minutes' WHERE j.id=$1 AND j.lease_token=$2 AND j.status='running' AND j.lease_until>now() AND EXISTS(SELECT 1 FROM knowledge_collections c WHERE c.id=j.collection_id AND c.status='active' AND c.revision=j.revision)")
                    .bind(job.id).bind(token).execute(self.db.tx_pool()).await?;
                if live.rows_affected() != 1 {
                    return Err(anyhow::anyhow!("Knowledge lease superseded"));
                }
            }
        };
        let result = tokio::select! { result = processing => result, result = renew => result };
        match result {
            Ok(()) => {
                sqlx::query("UPDATE knowledge_jobs SET status='done',lease_until=NULL WHERE id=$1 AND lease_token=$2").bind(job.id).bind(token).execute(self.db.tx_pool()).await?;
            }
            Err(_) => {
                let status = if job.attempts < 5 {
                    "pending"
                } else {
                    "failed"
                };
                sqlx::query("UPDATE knowledge_jobs SET status=$3,last_error='Knowledge processing failed; check source access, extraction or indexer configuration',available_at=now()+make_interval(secs=>$4),lease_until=NULL WHERE id=$1 AND lease_token=$2").bind(job.id).bind(token).bind(status).bind((2i32.pow(job.attempts.min(8) as u32)*5) as f64).execute(self.db.tx_pool()).await?;
                if let Some(source) = job.source_id {
                    sqlx::query("UPDATE knowledge_sources SET sync_status='failed',last_error='Source sync failed; check the connection and source coordinates' WHERE org_id=$1 AND id=$2 AND EXISTS(SELECT 1 FROM knowledge_jobs j WHERE j.id=$3 AND j.lease_token=$4 AND j.status IN ('pending','failed'))").bind(job.org_id).bind(source).bind(job.id).bind(token).execute(self.db.tx_pool()).await?;
                }
                tracing::warn!(job_id=%job.id,"Knowledge job failed; retry scheduled where eligible");
            }
        }
        Ok(true)
    }
    pub async fn heartbeat(&self, job: &Job, cursor: &str) -> Result<bool> {
        Ok(sqlx::query("UPDATE knowledge_jobs SET cursor=$3,lease_until=now()+interval '5 minutes' WHERE id=$1 AND lease_token=$2 AND status='running' AND lease_until>now() AND EXISTS(SELECT 1 FROM knowledge_collections c WHERE c.id=knowledge_jobs.collection_id AND c.revision=knowledge_jobs.revision AND c.status='active')").bind(job.id).bind(job.lease_token).bind(cursor).execute(self.db.tx_pool()).await?.rows_affected()==1)
    }
}
pub fn job(service: KnowledgeService) -> crate::background::cluster_jobs::ClusterJob {
    crate::background::cluster_jobs::ClusterJob::every(
        "knowledge-processing",
        "knowledge_processing",
        "Process one resumable Knowledge ingestion or index job",
        Duration::from_secs(2),
        move || {
            let service = service.clone();
            Box::pin(async move {
                use futures::StreamExt;
                futures::stream::iter(0..16)
                    .for_each_concurrent(4, |_| {
                        let service = service.clone();
                        async move {
                            if service.run_once().await.is_err() {
                                tracing::warn!("Knowledge worker unavailable");
                            }
                        }
                    })
                    .await;
            })
        },
    )
}
