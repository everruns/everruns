use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::params_from_request;
use crate::domains::common::{Command, CommandError, CommandMeta, Ctx};

#[derive(Debug, Deserialize, Serialize, ToSchema, PartialEq)]
struct Probe {
    id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    name: Option<String>,
}

impl Command for Probe {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "probe",
            category: "test",
            description: "probe",
            method: "PATCH",
            path: "/test/probe/{id}",
        }
    }

    async fn execute(self, _ctx: &Ctx) -> Result<(), CommandError> {
        Ok(())
    }
}

fn query(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn path(id: &str) -> Vec<(String, String)> {
    vec![("id".to_string(), id.to_string())]
}

#[test]
fn merges_path_query_and_body() {
    let probe: Probe = params_from_request(
        path("p_1"),
        query(&[("limit", "20"), ("archived", "true")]),
        br#"{"name":"x"}"#,
    )
    .unwrap();
    assert_eq!(
        probe,
        Probe {
            id: "p_1".into(),
            limit: Some(20),
            archived: true,
            name: Some("x".into()),
        }
    );
}

#[test]
fn path_parameter_wins_over_body_field() {
    let probe: Probe =
        params_from_request(path("from_path"), BTreeMap::new(), br#"{"id":"from_body"}"#).unwrap();
    assert_eq!(probe.id, "from_path");
}

#[test]
fn empty_body_is_no_fields() {
    let probe: Probe = params_from_request(path("p"), BTreeMap::new(), b"  ").unwrap();
    assert_eq!(probe.name, None);
}

#[test]
fn query_text_that_is_not_a_number_is_rejected() {
    let err = params_from_request::<Probe>(path("p"), query(&[("limit", "lots")]), b"")
        .expect_err("non-numeric limit");
    assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[test]
fn malformed_json_body_is_bad_request() {
    let err = params_from_request::<Probe>(path("p"), BTreeMap::new(), b"{not json")
        .expect_err("malformed body");
    assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[test]
fn non_object_body_with_path_params_is_bad_request() {
    let err =
        params_from_request::<Probe>(path("p"), BTreeMap::new(), b"[1,2]").expect_err("array body");
    assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[test]
fn body_of_the_wrong_shape_is_unprocessable() {
    let err = params_from_request::<Probe>(path("p"), BTreeMap::new(), br#"{"archived":"maybe"}"#)
        .expect_err("wrong type");
    assert_eq!(err.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn generated_operations_reach_the_openapi_document() {
    use utoipa::OpenApi;
    let doc = crate::openapi::ApiDoc::openapi();
    let item = doc
        .paths
        .paths
        .get("/v1/skills/{id}/delete")
        .expect("destroy_skill is generated from #[command]");
    let op = item.post.as_ref().expect("POST");
    assert_eq!(op.operation_id.as_deref(), Some("destroy_skill"));
    let list = doc.paths.paths.get("/v1/skills").expect("list path");
    let params: Vec<_> = list
        .get
        .as_ref()
        .and_then(|op| op.parameters.as_ref())
        .into_iter()
        .flatten()
        .filter_map(|p| match p {
            utoipa::openapi::RefOr::T(p) => Some(p.name.clone()),
            _ => None,
        })
        .collect();
    assert!(params.contains(&"search".to_string()), "{params:?}");
    assert!(
        params.contains(&"include_archived".to_string()),
        "{params:?}"
    );
}
