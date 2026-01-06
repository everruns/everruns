// Export OpenAPI specification as JSON
//
// Usage: cargo run --bin export-openapi > docs/api/openapi.json
//
// This binary generates the OpenAPI spec without starting the full API server.
// It's useful for CI/CD pipelines and documentation builds.

use everruns_control_plane::openapi::ApiDoc;

fn main() {
    println!("{}", ApiDoc::to_json());
}
