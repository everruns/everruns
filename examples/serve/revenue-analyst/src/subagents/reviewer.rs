use serve::prelude::*;

/// Reviews a SQL query or a revenue summary before it is shared.
#[agent(sub)]
fn reviewer() -> Agent {
    Agent::builder()
        // A lighter model is enough for a two-sentence review.
        .model("openai/gpt-6-luna")
        .instructions(
            "You review revenue SQL and summaries. Check that refunds are subtracted, \
             date ranges are half-open, and figures are in dollars. Answer in two sentences.",
        )
        .tools(Vec::<String>::new())
        .build()
}
