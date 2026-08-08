struct Calculator;

impl Calculator {
    /// Has a `self` receiver.
    #[everruns::tool]
    async fn add(&self, x: i64) -> i64 {
        x
    }
}

fn main() {}
