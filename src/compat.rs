//! Legacy names are read only for migration and recovery of existing user data.
use std::path::{Path, PathBuf};
pub fn default_data(base: &Path) -> PathBuf {
    let current = base.join("CleanerUltra");
    let legacy = base.join("PhotoSort");
    if !current.exists() && legacy.is_dir() {
        legacy
    } else {
        current
    }
}
pub fn media_vault(root: &Path, operation_path: &Path) -> PathBuf {
    let legacy = root.join(".photosort-trash");
    if operation_path
        .parent()
        .is_some_and(|parent| same_path(parent, &legacy))
    {
        legacy
    } else {
        root.join(".cleaner_ultra-trash")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reuse_legacy_journals_without_copying_or_overwriting() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(default_data(t.path()), t.path().join("CleanerUltra"));
        std::fs::create_dir(t.path().join("PhotoSort")).unwrap();
        assert_eq!(default_data(t.path()), t.path().join("PhotoSort"));
        std::fs::create_dir(t.path().join("CleanerUltra")).unwrap();
        assert_eq!(default_data(t.path()), t.path().join("CleanerUltra"));
    }
    #[test]
    fn old_vault_is_only_accepted_directly_under_original_root() {
        let root = Path::new("library");
        assert_eq!(
            media_vault(root, &root.join(".photosort-trash/a.jpg")),
            root.join(".photosort-trash")
        );
        assert_eq!(
            media_vault(root, Path::new("elsewhere/.photosort-trash/a.jpg")),
            root.join(".cleaner_ultra-trash")
        );
    }
}

pub fn space_vault(parent: &Path, operation_path: &Path) -> PathBuf {
    let legacy = parent.join(".photosort-space-trash");
    if operation_path
        .parent()
        .is_some_and(|parent| same_path(parent, &legacy))
    {
        legacy
    } else {
        parent.join(".cleaner_ultra-space-trash")
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .eq_ignore_ascii_case(
                &b.to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .replace('/', "\\"),
            )
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
