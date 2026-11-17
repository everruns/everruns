//! `everruns agent-skill`: the Everruns skill for coding agents, as shipped
//! with this CLI.
//!
//! Decision: the CLI carries the copy of `skills/everruns/` it was built with,
//! so the guidance a coding agent reads matches the commands this binary has.
//! `install` replaces only a folder that already holds this skill (its
//! SKILL.md is named `everruns`), never an unrelated directory.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::user_dirs;

const FILES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/everruns_skill.rs"));
const SKILL_NAME: &str = "everruns";
const PUBLISHED: &str = "npx skills add everruns/everruns";

#[derive(Subcommand)]
pub enum AgentSkillCommand {
    /// Write the skill into this project's .agents/skills and .claude/skills
    Install {
        /// Install for your user (~/.agents/skills and ~/.claude/skills) instead of this project
        #[arg(long, conflicts_with = "dir")]
        global: bool,

        /// Write into this skills directory only, as <DIR>/everruns
        #[arg(long)]
        dir: Option<PathBuf>,
    },

    /// Print the skill, or one of its files such as references/install.md
    Show {
        /// File inside the skill (default SKILL.md)
        file: Option<String>,
    },

    /// List the skill's files
    Files,
}

pub fn run(command: AgentSkillCommand) -> Result<()> {
    if FILES.is_empty() {
        bail!(
            "this build of the CLI does not carry the Everruns skill; install it with `{PUBLISHED}`"
        );
    }
    match command {
        AgentSkillCommand::Install { global, dir } => {
            let targets = match dir {
                Some(dir) => vec![dir],
                None => {
                    let base = if global {
                        user_dirs::home_dir().context("no home directory to install into")?
                    } else {
                        std::env::current_dir()?
                    };
                    vec![
                        base.join(".agents").join("skills"),
                        base.join(".claude").join("skills"),
                    ]
                }
            };
            for skills_dir in targets {
                let path = install(&skills_dir)?;
                println!("Installed the everruns skill in {}", path.display());
            }
            Ok(())
        }
        AgentSkillCommand::Show { file } => {
            let wanted = file.as_deref().unwrap_or("SKILL.md");
            let Some((_, content)) = FILES.iter().find(|(path, _)| *path == wanted) else {
                bail!("no file {wanted} in the skill; see `everruns agent-skill files`");
            };
            std::io::stdout().write_all(content.as_bytes())?;
            Ok(())
        }
        AgentSkillCommand::Files => {
            for (path, _) in FILES {
                println!("{path}");
            }
            Ok(())
        }
    }
}

/// Write the skill to `<skills_dir>/everruns`, replacing an earlier copy.
fn install(skills_dir: &Path) -> Result<PathBuf> {
    let target = skills_dir.join(SKILL_NAME);
    if target.exists() {
        let existing = fs::read_to_string(target.join("SKILL.md")).unwrap_or_default();
        if !is_everruns_skill(&existing) {
            bail!(
                "{} exists and is not the everruns skill; move it or pass --dir",
                target.display()
            );
        }
        fs::remove_dir_all(&target)
            .with_context(|| format!("remove the earlier copy at {}", target.display()))?;
    }
    for (relative, content) in FILES {
        let path = target.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(target)
}

fn is_everruns_skill(skill_md: &str) -> bool {
    skill_md
        .strip_prefix("---")
        .and_then(|rest| rest.split_once("\n---"))
        .is_some_and(|(frontmatter, _)| {
            frontmatter
                .lines()
                .any(|line| line.trim() == format!("name: {SKILL_NAME}"))
        })
}

#[cfg(all(test, everruns_has_workspace_skill))]
mod tests {
    use super::*;

    #[test]
    fn embeds_the_workspace_skill_and_its_references() {
        let skill = FILES
            .iter()
            .find(|(path, _)| *path == "SKILL.md")
            .unwrap()
            .1;
        assert!(is_everruns_skill(skill));
        assert!(
            FILES
                .iter()
                .any(|(path, _)| *path == "references/install.md")
        );
        assert!(
            FILES
                .iter()
                .any(|(path, _)| *path == "references/framework.md")
        );
    }

    #[test]
    fn install_replaces_an_earlier_copy_and_drops_stale_files() {
        let dir = tempfile::tempdir().unwrap();
        let first = install(dir.path()).unwrap();
        fs::write(first.join("references/stale.md"), "old").unwrap();
        let second = install(dir.path()).unwrap();
        assert!(second.join("SKILL.md").is_file());
        assert!(second.join("references/cli.md").is_file());
        assert!(!second.join("references/stale.md").exists());
    }

    #[test]
    fn install_refuses_to_replace_another_folder() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = dir.path().join(SKILL_NAME);
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("notes.txt"), "mine").unwrap();
        assert!(install(dir.path()).is_err());
        assert!(foreign.join("notes.txt").is_file());
    }
}
