//! Persistent SQL, citable knowledge and named memory with a real model.
//! cargo run -p everruns --features openai,sql-database,knowledge --example service_data
use everruns::{
    Agent, Engine, MemoryDirectory, Model, OpenAI,
    database::SqlDatabase,
    knowledge::{Document, Knowledge},
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".local/framework-data".into());
    std::fs::create_dir_all(format!("{root}/memory"))?;
    let policy = Document::new(
        "Refund policy",
        "Refunds are accepted within 30 days of purchase. Escalate disputed charges to billing.",
        "https://example.com/policy/refunds",
    );
    let agent = Agent::builder()
        .instructions(concat!(
            "Consult the refund policy with search_knowledge and cite its source. ",
            "Save a small refund summary in SQL using sql_execute, query it to verify, ",
            "then write a brief note in /memory/refunds/latest.md. Explain the result.",
        ))
        .model(Model::new("gpt-6.2-sol", OpenAI::from_env()?))
        .sql_database(SqlDatabase::open(format!("{root}/databases"))?)
        .knowledge(Knowledge::documents("refunds", vec![policy]))
        .memory(MemoryDirectory::directory(
            "refunds",
            format!("{root}/memory"),
        ))
        .capability("session_file_system")
        .build()?;
    let engine = Engine::new();
    let session = engine.create(agent);
    let turn = session
        .run("A purchase was made 12 days ago. Is a refund allowed? Record your decision.")
        .await?;
    if !turn.success {
        return Err(format!("Agent failed: {:?}", turn.error).into());
    }
    println!("session: {}\n{}", session.id(), turn.response);
    Ok(())
}
