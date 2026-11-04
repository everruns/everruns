import assert from "node:assert/strict";
import test from "node:test";
import remarkMermaidDiagrams from "./remark-mermaid-diagrams.mjs";

test("converts Mermaid fences and escapes source without changing other code", () => {
  const rust = { type: "code", lang: "rust", value: "fn main() {}" };
  const tree = {
    type: "root",
    children: [
      { type: "paragraph", children: [{ type: "text", value: "Before" }] },
      { type: "code", lang: "MeRmAiD", value: 'flowchart LR\nA["<script>&"] --> B' },
      rust,
    ],
  };

  remarkMermaidDiagrams()(tree);

  assert.equal(tree.children[1].type, "html");
  assert.match(tree.children[1].value, /data-mermaid-diagram/);
  assert.match(tree.children[1].value, /&lt;script&gt;&amp;/);
  assert.doesNotMatch(tree.children[1].value, /<script>/);
  assert.equal(tree.children[2], rust);
});
