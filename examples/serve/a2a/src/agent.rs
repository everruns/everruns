use serve::prelude::*;
use serve::sim;

/// Researches a topic and answers with short factual notes.
#[agent]
fn researcher() -> Agent {
    Agent::builder()
        // `OPENAI_API_KEY` routes this to OpenAI (any serve gateway works).
        // Without a key, dev and evals use the offline script below.
        .model("openai/gpt-5.6-terra")
        .description("Researches a topic and answers with short factual notes.")
        .instructions(md!("instructions.md"))
        .tools(Vec::<String>::new())
        .offline(sim::script([sim::reply(
            "- Tide pools form where rock holds seawater as the tide goes out.\n\
             - Residents such as anemones, mussels and sea stars tolerate swings in \
             temperature and salinity.\n\
             - Each low tide isolates the pool for hours; each high tide resets it.\n\
             (Offline demo notes from the researcher's simulator.)",
        )]))
        .build()
}
