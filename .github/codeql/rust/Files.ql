/** Lists extracted source files with semantic information and actual AST nodes. */
import rust

string semantics(ExtractedFile f) {
  if f.hasSemantics() then result = "true" else result = "false"
}

from ExtractedFile f
where exists(f.getRelativePath())
select f.getRelativePath() as path, semantics(f) as semantics,
  count(AstNode n | n.getFile() = f) as nodes
