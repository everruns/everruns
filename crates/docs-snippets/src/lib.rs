// Compile check for the Rust code blocks in `docs/framework/`.
//
// Nothing here is meant to be used. `build.rs` copies each Framework docs page
// into `OUT_DIR` with its Rust fences made `no_run`, and the structs below carry
// those copies as doc comments, so `cargo test -p everruns-docs-snippets`
// compiles every block as a doctest. A failure is reported as
// `pages/<page>.rs - pages::<page>::Page (line N)`, N being the line of the
// block in the Markdown page.
//
// Writing a block: follow the rustdoc doctest conventions. Lines starting with
// `# ` compile but are hidden on the docs site
// (`apps/docs/plugins/remark-strip-rust-hidden-lines.ts`); end a block that uses
// `?` with a hidden `# Ok::<(), Box<dyn std::error::Error>>(())`. Mark a block
// that is deliberately partial `rust ignore`.

#[cfg(doctest)]
mod pages {
    include!(concat!(env!("OUT_DIR"), "/pages.rs"));
}
