/** @jest-environment node */

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";

const SOURCE_ROOT = path.resolve(__dirname, "../..");
const NESTED_LINK_BUTTON = /<Link\b[^>]*>\s*<Button\b/;

function tsxFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      return tsxFiles(entryPath);
    }
    return entry.name.endsWith(".tsx") ? [entryPath] : [];
  });
}

function unwrapParentheses(expression: ts.Expression): ts.Expression {
  let current = expression;
  while (ts.isParenthesizedExpression(current)) {
    current = current.expression;
  }
  return current;
}

function isLinkElement(expression: ts.Expression): boolean {
  const unwrapped = unwrapParentheses(expression);
  if (ts.isJsxSelfClosingElement(unwrapped)) {
    return ts.isIdentifier(unwrapped.tagName) && unwrapped.tagName.text === "Link";
  }
  if (ts.isJsxElement(unwrapped)) {
    const tagName = unwrapped.openingElement.tagName;
    return ts.isIdentifier(tagName) && tagName.text === "Link";
  }
  return false;
}

function buttonRendersLink(source: string, filePath = "fixture.tsx"): boolean {
  const sourceFile = ts.createSourceFile(
    filePath,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let found = false;

  function visit(node: ts.Node) {
    if (
      (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) &&
      ts.isIdentifier(node.tagName) &&
      node.tagName.text === "Button"
    ) {
      const renderAttribute = node.attributes.properties.find(
        (property): property is ts.JsxAttribute =>
          ts.isJsxAttribute(property) &&
          ts.isIdentifier(property.name) &&
          property.name.text === "render",
      );
      const initializer = renderAttribute?.initializer;
      if (
        initializer &&
        ts.isJsxExpression(initializer) &&
        initializer.expression &&
        isLinkElement(initializer.expression)
      ) {
        found = true;
        return;
      }
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return found;
}
test("link-styled buttons do not nest interactive elements", () => {
  const violations = tsxFiles(SOURCE_ROOT)
    .filter((filePath) => NESTED_LINK_BUTTON.test(readFileSync(filePath, "utf8")))
    .map((filePath) => path.relative(SOURCE_ROOT, filePath));

  expect(violations).toEqual([]);
});

test.each([
  [
    "callback before render",
    '<Button onClick={() => track()} render={<Link href="/target" />}>Open</Button>',
  ],
  [
    "callback after render",
    '<Button render={<Link href="/target" />} onClick={() => track()}>Open</Button>',
  ],
])("detects a rendered Link with a %s prop", (_name, source) => {
  expect(buttonRendersLink(source)).toBe(true);
});

test.each([
  ["native Button", "<Button onClick={() => track()}>Run</Button>"],
  ["semantic LinkButton", '<LinkButton href="/target">Open</LinkButton>'],
  ["non-Link render element", "<Button render={<span />}>Label</Button>"],
])("allows a %s", (_name, source) => {
  expect(buttonRendersLink(source)).toBe(false);
});

test("link-styled buttons use anchors instead of the Base UI Button render prop", () => {
  const violations = tsxFiles(SOURCE_ROOT)
    .filter((filePath) => buttonRendersLink(readFileSync(filePath, "utf8"), filePath))
    .map((filePath) => path.relative(SOURCE_ROOT, filePath));

  expect(violations).toEqual([]);
});
