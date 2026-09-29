/// A call context after a model argument is rejected.
#[everruns::tool]
async fn late(x: i64, ctx: everruns::ToolCallContext) -> i64 {
    let _ = ctx;
    x
}

fn main() {}
