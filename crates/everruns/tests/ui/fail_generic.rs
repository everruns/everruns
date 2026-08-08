/// Generic parameters are not supported.
#[everruns::tool]
async fn identity<T>(value: T) -> T {
    value
}

fn main() {}
