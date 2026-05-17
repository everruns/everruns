// Project instruction loader: AGENTS.md / CLAUDE.md / .agents.md
// Decision: read instruction files from the workspace root once at startup and
// fold them into the harness system prompt. Walks toward the workspace root,
// not the filesystem root, so we never leak host-level instructions.

use std::path::Path;

const INSTRUCTION_FILENAMES: &[&str] = &["AGENTS.md", "CLAUDE.md", ".agents.md"];

pub struct ProjectInstructions {
    pub blocks: Vec<(String, String)>,
}

impl ProjectInstructions {
    pub fn load(root: &Path) -> Self {
        let mut blocks = Vec::new();
        for name in INSTRUCTION_FILENAMES {
            let path = root.join(name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    blocks.push((name.to_string(), trimmed.to_string()));
                }
            }
        }
        Self { blocks }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn rendered(&self) -> String {
        let mut out = String::new();
        for (name, body) in &self.blocks {
            out.push_str(&format!("<{name}>\n{body}\n</{name}>\n\n"));
        }
        out
    }

    pub fn summary(&self) -> Vec<String> {
        self.blocks
            .iter()
            .map(|(n, b)| format!("{} ({} bytes)", n, b.len()))
            .collect()
    }
}
