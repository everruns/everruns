// Use the repository's TypeScript parser so nested callbacks and test.each are
// inventoried without guessing function boundaries from text.
const fs = require('node:fs');
const ts = require('../../apps/ui/node_modules/typescript');
const files = JSON.parse(fs.readFileSync(0, 'utf8'));
const rows = [];
for (const path of files) {
  const text = fs.readFileSync(path, 'utf8');
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true);
  if (source.parseDiagnostics.length) {
    throw new Error(`Cannot inventory ${path}: ${source.parseDiagnostics.map(d => ts.flattenDiagnosticMessageText(d.messageText, '\n')).join('; ')}`);
  }
  const label = node => ts.isStringLiteralLike(node) ? node.text : node.getText(source);
  function visit(node, suites) {
    if (ts.isCallExpression(node)) {
      const expression = node.expression.getText(source).replace(/\s+/g, '');
      const callback = [...node.arguments].reverse().find(arg => ts.isArrowFunction(arg) || ts.isFunctionExpression(arg));
      const isSuite = /^(?:(?:describe|fdescribe|xdescribe)|test\.describe)(?:\.|\(|$)/.test(expression);
      const isTest = !isSuite && /^(?:test|it|fit|xit|xtest)(?:\.|\(|$)/.test(expression);
      if ((isSuite || isTest) && (callback || expression.endsWith('.todo'))) {
        const title = node.arguments.length ? label(node.arguments[0]) : '(unnamed)';
        if (isTest) {
          rows.push({path, name: [...suites, title].join(' > '), language: 'javascript',
            start: node.getStart(source), end: node.end,
            line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
            end_line: source.getLineAndCharacterOfPosition(node.end).line + 1,
            body: node.getText(source), parameterized: expression.includes('.each'),
            disabled: /\.(?:skip|todo)\b|^(?:xit|xtest)\b/.test(expression)});
        }
        ts.forEachChild(node, child => visit(child, isSuite ? [...suites, title] : suites));
        return;
      }
    }
    ts.forEachChild(node, child => visit(child, suites));
  }
  visit(source, []);
}
process.stdout.write(JSON.stringify(rows));
