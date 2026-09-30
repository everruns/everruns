use serve::prelude::*;
use serve::sim;

/// Reviews a SQL query or a revenue summary before it is shared.
#[agent(sub)]
fn reviewer() -> Agent {
    Agent::builder()
        .model("anthropic/claude-haiku-4-5")
        .instructions(
            "You review revenue SQL and summaries. Check that refunds are subtracted, \
             date ranges are half-open, and figures are in dollars. Answer in two sentences.",
        )
        .tools(Vec::<String>::new())
        .offline(sim::script([sim::reply(
            "Looks right: refunds are subtracted and the date range is half-open. (Offline demo reply.)",
        )]))
        .build()
}
