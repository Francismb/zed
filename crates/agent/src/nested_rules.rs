use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use collections::HashSet;
use gpui::App;
use project::{Project, ProjectPath};
use prompt_store::RULES_FILE_NAMES;
use util::markdown::MarkdownCodeBlock;
use util::rel_path::RelPath;

/// A sub-directory instruction file (e.g. `AGENTS.md`) discovered in a directory
/// above a file the agent read, but below the worktree root. The worktree root's
/// own rules file is loaded separately into the system prompt, so it is never
/// surfaced here.
pub struct NestedRule {
    /// Worktree-qualified path used purely for display (e.g. `zed/crates/foo/AGENTS.md`).
    pub display_path: String,
    /// Absolute path, used both as the deduplication key and to load the contents.
    pub abs_path: Arc<Path>,
}

/// Discovers instruction files in the directories between the worktree root
/// (exclusive) and the directory containing `accessed_path` (inclusive).
///
/// Files whose absolute path is already present in `already_loaded` are skipped.
/// Newly discovered files are inserted into `already_loaded` *before* returning,
/// so concurrent reads can't surface the same file twice. Results are ordered
/// root-most directory first, so cascading instructions are read in directory
/// order.
pub fn discover_nested_rules(
    project: &Project,
    accessed_path: &ProjectPath,
    already_loaded: &mut HashSet<Arc<Path>>,
    cx: &App,
) -> Vec<NestedRule> {
    let Some(worktree) = project.worktree_for_id(accessed_path.worktree_id, cx) else {
        return Vec::new();
    };
    let worktree = worktree.read(cx);

    // The directory containing the accessed file. A file directly in the
    // worktree root has an empty parent and therefore no nested directories.
    let Some(directory) = accessed_path.path.parent() else {
        return Vec::new();
    };

    // `ancestors` yields the directory itself down to the empty root path;
    // dropping the empty path excludes the worktree root, and reversing puts the
    // root-most directory first.
    let mut directories: Vec<&RelPath> =
        directory.ancestors().filter(|dir| !dir.is_empty()).collect();
    directories.reverse();

    let mut rules = Vec::new();
    for directory in directories {
        // First matching instruction file in this directory wins, matching the
        // priority order used for the worktree root rules file in `agent.rs`.
        let Some(entry) = RULES_FILE_NAMES.iter().find_map(|name| {
            let name = RelPath::unix(name).ok()?;
            let candidate = directory.join(&name);
            worktree
                .entry_for_path(&candidate)
                .filter(|entry| entry.is_file())
        }) else {
            continue;
        };

        let abs_path: Arc<Path> = worktree.absolutize(&entry.path).into();
        if !already_loaded.insert(abs_path.clone()) {
            continue;
        }

        rules.push(NestedRule {
            display_path: format!(
                "{}/{}",
                worktree.root_name().as_unix_str(),
                entry.path.as_unix_str(),
            ),
            abs_path,
        });
    }

    rules
}

/// Renders discovered nested instruction files into a single `<nested_instructions>`
/// block suitable for prepending to a tool result. `rules` pairs each file's
/// display path with its (already-loaded) contents.
pub fn render_nested_rules<'a>(
    rules: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut output = String::new();
    output.push_str(
        "<nested_instructions>\n\
         The following instruction files apply to the directory containing the file that was just \
         read. Follow them when working in this area of the project.\n",
    );
    for (display_path, contents) in rules {
        let _ = write!(output, "\n`{display_path}`:\n");
        let _ = write!(
            output,
            "{}",
            MarkdownCodeBlock {
                tag: "",
                text: contents.trim(),
            }
        );
    }
    output.push_str("</nested_instructions>\n\n");
    output
}
