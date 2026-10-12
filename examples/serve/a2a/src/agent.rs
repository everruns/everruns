use serve::prelude::*;

/// Researches a topic and answers with short factual notes.
#[agent]
fn researcher() -> Agent {
    Agent::builder()
        // `OPENAI_API_KEY` routes this to OpenAI (any serve gateway works).
        .model("openai/gpt-6.1-sol")
        .description("Researches a topic and answers with short factual notes.")
        .instructions(md!("instructions.md"))
        .tools(Vec::<String>::new())
        .build()
}
