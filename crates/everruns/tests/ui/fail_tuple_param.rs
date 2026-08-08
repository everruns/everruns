/// A tuple-destructuring parameter pattern is not supported.
#[everruns::tool]
async fn add_pair((a, b): (i64, i64)) -> i64 {
    a + b
}

fn main() {}
