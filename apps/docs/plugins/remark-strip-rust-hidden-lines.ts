// Strip rustdoc hidden lines from Rust code blocks.
//
// Docs pages reuse doctest-style snippets where a line starting with `# ` (or
// a bare `#`) is boilerplate for `cargo test` (e.g. `# fn main() {`). rustdoc
// hides those lines, but Starlight renders fenced blocks literally, so the `#`
// prefixes leak into the page. This remark plugin removes such lines from
// `rust`/`rs` blocks at build time so display and copy output stay clean.
//
// Only `#` followed by a space or end-of-line is a hidden-line marker. Lines
// starting with `#[` / `#![` are real attributes and are preserved. Non-Rust
// blocks (e.g. bash `# comments`) are untouched.

interface MdastNode {
  type: string;
  lang?: string | null;
  value?: string;
  children?: MdastNode[];
}

const RUST_LANG = /^(rust|rs)([^a-z0-9]|$)/i;
const HIDDEN_LINE = /^\s*#($| )/;

function isRustCode(node: MdastNode): node is MdastNode & { value: string } {
  if (node.type !== "code" || typeof node.value !== "string") return false;
  if (typeof node.lang !== "string") return false;
  return RUST_LANG.test(node.lang.toLowerCase());
}

function visit(node: MdastNode, fn: (n: MdastNode) => void): void {
  fn(node);
  if (Array.isArray(node.children)) {
    for (const child of node.children) visit(child, fn);
  }
}

export default function remarkStripRustHiddenLines() {
  return (tree: MdastNode): void => {
    visit(tree, (node) => {
      if (!isRustCode(node)) return;
      node.value = node.value
        .split("\n")
        .filter((line) => !HIDDEN_LINE.test(line))
        .join("\n");
    });
  };
}
