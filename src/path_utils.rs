use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    if let Ok(path) = fs::canonicalize(path) {
        return path;
    }

    let mut missing = Vec::new();
    let mut ancestor = path;
    while let Some(parent) = ancestor.parent() {
        let Some(name) = ancestor.file_name() else {
            break;
        };
        missing.push(name.to_os_string());
        if let Ok(mut canonical_parent) = fs::canonicalize(parent) {
            missing
                .iter()
                .rev()
                .for_each(|component| canonical_parent.push(component));
            return normalize_path_lexically(&canonical_parent);
        }
        ancestor = parent;
    }

    normalize_path_lexically(path)
}

pub(crate) fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                if !normalized.pop() && !normalized.has_root() {
                    normalized.push("..");
                }
            }
            Component::Normal(value) => normalized.push(value),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::normalize_path_lexically;
    use std::path::Path;

    #[test]
    fn lexical_normalization_does_not_escape_above_root() {
        assert_eq!(normalize_path_lexically(Path::new("/..")), Path::new("/"));
        assert_eq!(
            normalize_path_lexically(Path::new("/tmp/../../x")),
            Path::new("/x")
        );
    }

    #[test]
    fn lexical_normalization_preserves_relative_parent_components() {
        assert_eq!(
            normalize_path_lexically(Path::new("a/../../b")),
            Path::new("../b")
        );
    }
}
