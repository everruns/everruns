/// Repeatable string flags take one value per occurrence. A caller who passes
/// the API's JSON array (`["company", "ci"]`) as that one value gets those
/// strings, not a single tag made of brackets and quotes.
pub(crate) fn expand_json_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .flat_map(|value| {
            everruns_cli_contract::json_string_array(&value).unwrap_or_else(|| vec![value])
        })
        .collect()
}

pub mod agents;
pub mod api;
pub mod chat;
pub mod connections;
pub mod files;
pub mod login;
pub mod logout;
pub mod orgs;
pub mod sessions;
pub mod status;
