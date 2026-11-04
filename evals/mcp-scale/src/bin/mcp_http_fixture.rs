#[path = "../fixture.rs"]
mod fixture;

fn main() -> std::io::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    fixture::serve_http(&args)
}
