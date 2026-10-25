//! Host-provided indexed retrieval alongside exact filesystem grep.
use async_trait::async_trait;
use everruns_contracts::{
    knowledge::{KnowledgeCatalogQuery, KnowledgeQuery, KnowledgeRuntime},
    typed_id::SessionId,
};
use std::sync::Arc;
pub struct KnowledgeBuiltin {
    pub service: Arc<dyn KnowledgeRuntime>,
    pub session: SessionId,
}
const HELP: &str = "knowledge search [--collection ID] [--path PREFIX] [--limit 1..25] [--metadata JSON] [--lexical] QUERY\nknowledge files --collection ID [--path PREFIX] [--after CURSOR] [--limit 1..100] [--metadata]\nReturns JSON with immutable citations. Browse /knowledge/index.md or use grep -r for exact matches.\n";
#[async_trait]
impl bashkit::Builtin for KnowledgeBuiltin {
    async fn execute(
        &self,
        ctx: bashkit::BuiltinContext<'_>,
    ) -> bashkit::Result<bashkit::ExecResult> {
        let args = ctx.args;
        if args.is_empty() || args.iter().any(|v| v == "--help") {
            return Ok(bashkit::ExecResult::ok(HELP));
        }
        if args[0] == "files" {
            let mut query = KnowledgeCatalogQuery {
                collection: String::new(),
                path: None,
                after: None,
                limit: 50,
                include_metadata: false,
            };
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--metadata" {
                    query.include_metadata = true;
                    i += 1;
                    continue;
                }
                let Some(value) = args.get(i + 1) else {
                    return Ok(bashkit::ExecResult::err(HELP, 2));
                };
                match args[i].as_str() {
                    "--collection" => query.collection = value.clone(),
                    "--path" => query.path = Some(value.clone()),
                    "--after" => query.after = Some(value.clone()),
                    "--limit" => match value.parse::<usize>() {
                        Ok(n) if (1..=100).contains(&n) => query.limit = n,
                        _ => return Ok(bashkit::ExecResult::err(HELP, 2)),
                    },
                    _ => return Ok(bashkit::ExecResult::err(HELP, 2)),
                }
                i += 2;
            }
            if query.collection.is_empty() {
                return Ok(bashkit::ExecResult::err(HELP, 2));
            }
            return match self.service.files(self.session, query).await {
                Ok(page) => Ok(bashkit::ExecResult::ok(format!(
                    "{}\n",
                    serde_json::to_string(&page).unwrap_or_default()
                ))),
                Err(_) => Ok(bashkit::ExecResult::err(
                    "Knowledge catalog unavailable; check the attachment or narrow the path prefix.\n",
                    1,
                )),
            };
        }
        if args[0] != "search" {
            return Ok(bashkit::ExecResult::err(HELP, 2));
        }
        let mut query = KnowledgeQuery::default();
        let mut words = Vec::new();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--lexical" => query.lexical_only = true,
                "--collection" | "--path" | "--limit" | "--metadata" => {
                    let flag = &args[i];
                    i += 1;
                    let Some(value) = args.get(i) else {
                        return Ok(bashkit::ExecResult::err(HELP, 2));
                    };
                    match flag.as_str() {
                        "--collection" => query.collections.push(value.clone()),
                        "--path" => query.path = Some(value.clone()),
                        "--metadata" => match serde_json::from_str(value) {
                            Ok(metadata) => query.metadata = metadata,
                            Err(_) => return Ok(bashkit::ExecResult::err(HELP, 2)),
                        },
                        _ => match value.parse::<usize>() {
                            Ok(n) if (1..=25).contains(&n) => query.limit = n,
                            _ => return Ok(bashkit::ExecResult::err(HELP, 2)),
                        },
                    }
                }
                "--" => {
                    words.extend(args[i + 1..].iter().cloned());
                    break;
                }
                value if value.starts_with('-') => return Ok(bashkit::ExecResult::err(HELP, 2)),
                value => words.push(value.to_string()),
            }
            i += 1;
        }
        query.query = words.join(" ");
        match self.service.search(self.session, query).await {
            Ok(result) => Ok(bashkit::ExecResult::ok(format!(
                "{}\n",
                serde_json::to_string(&result).unwrap_or_default()
            ))),
            Err(_) => Ok(bashkit::ExecResult::err(
                "Knowledge search unavailable; inspect its index status or browse the mounted files.\n",
                1,
            )),
        }
    }
    fn llm_hint(&self) -> Option<&'static str> {
        Some(
            "knowledge files pages attached catalogs without indexing; knowledge search retrieves ranked passages; grep retains exact line matching",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::{knowledge::KnowledgeResults, typed_id::MessageId};
    struct Retrieval(
        std::sync::Mutex<Vec<KnowledgeQuery>>,
        std::sync::Mutex<Vec<KnowledgeCatalogQuery>>,
    );
    #[async_trait]
    impl KnowledgeRuntime for Retrieval {
        async fn search(
            &self,
            _: SessionId,
            q: KnowledgeQuery,
        ) -> everruns_contracts::error::Result<KnowledgeResults> {
            self.0.lock().unwrap().push(q);
            Ok(KnowledgeResults::default())
        }
        async fn files(
            &self,
            _: SessionId,
            query: KnowledgeCatalogQuery,
        ) -> everruns_contracts::error::Result<everruns_contracts::knowledge::KnowledgeCatalog>
        {
            self.1.lock().unwrap().push(query.clone());
            Ok(everruns_contracts::knowledge::KnowledgeCatalog {
                collection_id: query.collection,
                files: vec![],
                next_cursor: None,
                warnings: vec![],
            })
        }
        async fn automatic_context(
            &self,
            _: SessionId,
            _: MessageId,
            _: String,
        ) -> everruns_contracts::error::Result<Option<String>> {
            Ok(None)
        }
    }
    #[tokio::test]
    async fn shell_search_uses_the_host_retrieval_port_and_rejects_invalid_limits() {
        let service = Arc::new(Retrieval(Default::default(), Default::default()));
        let mut shell = bashkit::Bash::builder()
            .builtin(
                "knowledge",
                Box::new(KnowledgeBuiltin {
                    service: service.clone(),
                    session: SessionId::from_uuid(uuid::Uuid::now_v7()),
                }),
            )
            .build();
        let result=shell.exec("knowledge search --collection kn_0123456789abcdef0123456789abcdef --limit 3 --lexical --metadata '{\"team\":\"support\"}' 'refund policy'").await.unwrap();
        assert_eq!(result.exit_code, 0);
        {
            let queries = service.0.lock().unwrap();
            assert_eq!(queries.len(), 1);
            assert_eq!(queries[0].query, "refund policy");
            assert_eq!(queries[0].limit, 3);
            assert!(queries[0].lexical_only);
            assert_eq!(queries[0].metadata["team"], "support");
        }

        assert_eq!(
            shell
                .exec("knowledge search --limit 0 test")
                .await
                .unwrap()
                .exit_code,
            2
        );
        assert_eq!(service.0.lock().unwrap().len(), 1);
        assert_eq!(shell.exec("knowledge files --collection kn_0123456789abcdef0123456789abcdef --path /docs --after /docs/a.md --limit 3 --metadata").await.unwrap().exit_code,0);
        {
            let pages = service.1.lock().unwrap();
            assert_eq!(pages.len(), 1);
            assert_eq!(pages[0].path.as_deref(), Some("/docs"));
            assert_eq!(pages[0].after.as_deref(), Some("/docs/a.md"));
            assert!(pages[0].include_metadata);
            assert_eq!(pages[0].limit, 3);
        }
        assert_eq!(
            shell
                .exec("knowledge files --limit 0")
                .await
                .unwrap()
                .exit_code,
            2
        );
    }
}
