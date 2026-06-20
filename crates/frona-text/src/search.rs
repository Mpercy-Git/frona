use std::path::{Path, PathBuf};

/// Iterator over the readable text files in `dir`, respecting `.gitignore`.
/// Matches the configuration that frona's storage layer uses for content
/// search.
pub fn walk_with_ignore(dir: &Path) -> impl Iterator<Item = PathBuf> {
    ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        // Honour .gitignore even when the walked tree isn't a git repo. Our
        // agent workspaces are not git-initialised, but we still want to
        // respect any .gitignore the user (or a skill) drops in.
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_visible_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "x").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "y").unwrap();
        let mut names: Vec<_> = walk_with_ignore(tmp.path())
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn skips_dotfiles() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".hidden"), "x").unwrap();
        std::fs::write(tmp.path().join("visible.txt"), "y").unwrap();
        let names: Vec<_> = walk_with_ignore(tmp.path())
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["visible.txt"]);
    }

    fn names_in(root: &std::path::Path) -> Vec<String> {
        let mut out: Vec<String> = walk_with_ignore(root)
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn respects_gitignore_in_root() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "secret.txt\n").unwrap();
        std::fs::write(tmp.path().join("visible.txt"), "y").unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "x").unwrap();
        let names = names_in(tmp.path());
        assert!(names.contains(&"visible.txt".to_string()));
        assert!(!names.contains(&"secret.txt".to_string()));
        // .gitignore itself is hidden(true), so it's excluded.
        assert!(!names.contains(&".gitignore".to_string()));
    }

    #[test]
    fn respects_gitignore_glob_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "*.log\nbuild/\n").unwrap();
        std::fs::write(tmp.path().join("a.log"), "x").unwrap();
        std::fs::write(tmp.path().join("b.log"), "y").unwrap();
        std::fs::write(tmp.path().join("keep.txt"), "z").unwrap();
        std::fs::create_dir(tmp.path().join("build")).unwrap();
        std::fs::write(tmp.path().join("build").join("artifact"), "w").unwrap();
        let names = names_in(tmp.path());
        assert!(names.contains(&"keep.txt".to_string()));
        assert!(!names.contains(&"a.log".to_string()));
        assert!(!names.contains(&"b.log".to_string()));
        // build/ excluded → no artifacts inside.
        assert!(!names.iter().any(|n| n.starts_with("build")));
    }

    #[test]
    fn recurses_into_subdirectories() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::create_dir(tmp.path().join("sub").join("nested")).unwrap();
        std::fs::write(tmp.path().join("top.txt"), "x").unwrap();
        std::fs::write(tmp.path().join("sub").join("a.txt"), "y").unwrap();
        std::fs::write(tmp.path().join("sub").join("nested").join("b.txt"), "z").unwrap();
        let names = names_in(tmp.path());
        // ignore::WalkBuilder uses platform path separator; normalise.
        let normalised: Vec<String> = names.iter().map(|n| n.replace('\\', "/")).collect();
        assert!(normalised.iter().any(|n| n == "top.txt"));
        assert!(normalised.iter().any(|n| n == "sub/a.txt"));
        assert!(normalised.iter().any(|n| n == "sub/nested/b.txt"));
    }

    #[test]
    fn empty_directory_returns_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let count = walk_with_ignore(tmp.path()).count();
        assert_eq!(count, 0);
    }

    #[test]
    fn non_existent_directory_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let bogus = tmp.path().join("does-not-exist");
        let count = walk_with_ignore(&bogus).count();
        assert_eq!(count, 0);
    }

    #[test]
    fn only_returns_files_not_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), "x").unwrap();
        for p in walk_with_ignore(tmp.path()) {
            assert!(p.is_file(), "non-file in results: {p:?}");
        }
    }

    #[test]
    fn nested_gitignore_applies_per_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("root.txt"), "x").unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub").join(".gitignore"), "secret.txt\n").unwrap();
        std::fs::write(tmp.path().join("sub").join("ok.txt"), "y").unwrap();
        std::fs::write(tmp.path().join("sub").join("secret.txt"), "z").unwrap();
        let names = names_in(tmp.path());
        let normalised: Vec<String> = names.iter().map(|n| n.replace('\\', "/")).collect();
        assert!(normalised.iter().any(|n| n == "root.txt"));
        assert!(normalised.iter().any(|n| n == "sub/ok.txt"));
        assert!(!normalised.iter().any(|n| n == "sub/secret.txt"));
    }
}
