//! Deterministic, exclusively owned generated output trees.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Manifest written into every generated root.
pub const MANIFEST_PATH: &str = "service-builder.manifest.json";

const MANIFEST_FORMAT: &str = "service-builder-output/1";

/// A complete generated tree, keyed by slash-separated relative path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactTree {
    files: BTreeMap<String, String>,
}

/// One byte-level difference between an expected and committed generated tree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Drift {
    /// An expected file is absent.
    Missing(String),
    /// A committed file is not generated anymore.
    Unexpected(String),
    /// A file exists at the right path with different bytes.
    Changed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputManifest {
    format: String,
    files: BTreeMap<String, String>,
}

impl ArtifactTree {
    /// Creates an empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one UTF-8 artifact under a safe relative path.
    pub fn insert(&mut self, path: impl Into<String>, contents: impl Into<String>) -> Result<()> {
        let path = path.into();
        validate_path(&path)?;
        if path == MANIFEST_PATH {
            bail!("`{MANIFEST_PATH}` is owned by service-builder");
        }
        if self.files.insert(path.clone(), contents.into()).is_some() {
            bail!("two generated artifacts claim `{path}`");
        }
        Ok(())
    }

    /// Adds all ESS artifacts below one explicit ownership prefix.
    pub fn extend_ess(
        &mut self,
        prefix: &str,
        artifacts: &BTreeMap<String, ess_gen::Artifact>,
    ) -> Result<()> {
        validate_path(prefix)?;
        for (path, artifact) in artifacts {
            if artifact.path != *path {
                bail!(
                    "ESS artifact key `{path}` disagrees with its declared path `{}`",
                    artifact.path
                );
            }
            self.insert(format!("{prefix}/{path}"), artifact.contents.clone())?;
        }
        Ok(())
    }

    /// Files excluding the builder-owned manifest.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
    }

    /// Compares a committed root with this complete tree.
    pub fn check(&self, root: &Path) -> Result<Vec<Drift>> {
        let expected = self.rendered()?;
        let actual = read_tree(root)?;
        let mut drift = Vec::new();
        for (path, expected_contents) in &expected {
            match actual.get(path) {
                None => drift.push(Drift::Missing(path.clone())),
                Some(actual_contents) if actual_contents != expected_contents => {
                    drift.push(Drift::Changed(path.clone()));
                }
                Some(_) => {}
            }
        }
        for path in actual.keys() {
            if !expected.contains_key(path) {
                drift.push(Drift::Unexpected(path.clone()));
            }
        }
        drift.sort();
        Ok(drift)
    }

    /// Replaces files owned by the previous manifest and writes this tree.
    ///
    /// Files not named by a valid previous manifest are refused rather than deleted. Generated
    /// roots are exclusive: callers must move handwritten files outside them.
    pub fn write(&self, root: &Path) -> Result<()> {
        let rendered = self.rendered()?;
        let existing = read_tree(root)?;
        if !existing.is_empty() {
            let manifest_text = existing.get(MANIFEST_PATH).ok_or_else(|| {
                anyhow!(
                    "{} is not an owned generated root: `{MANIFEST_PATH}` is absent",
                    root.display()
                )
            })?;
            let manifest: OutputManifest = serde_json::from_str(manifest_text)
                .with_context(|| format!("reading {MANIFEST_PATH} in {}", root.display()))?;
            validate_manifest(&manifest)?;
            let owned: BTreeSet<_> = manifest
                .files
                .keys()
                .cloned()
                .chain(std::iter::once(MANIFEST_PATH.to_owned()))
                .collect();
            let unowned: Vec<_> = existing
                .keys()
                .filter(|path| !owned.contains(*path))
                .cloned()
                .collect();
            if !unowned.is_empty() {
                bail!(
                    "{} contains files not owned by its manifest: {}",
                    root.display(),
                    unowned.join(", ")
                );
            }
            for stale in owned.iter().filter(|path| !rendered.contains_key(*path)) {
                if stale != MANIFEST_PATH {
                    let expected_digest = manifest
                        .files
                        .get(stale)
                        .expect("every owned non-manifest path has a digest");
                    if let Some(contents) = existing.get(stale) {
                        let actual_digest = digest(contents.as_bytes());
                        if &actual_digest != expected_digest {
                            bail!(
                                "refusing to remove modified stale generated file `{stale}`; its bytes no longer match the ownership manifest"
                            );
                        }
                    }
                }
                let target = root.join(path_from_slashes(stale));
                if target.is_file() {
                    fs::remove_file(&target).with_context(|| {
                        format!("removing stale generated file {}", target.display())
                    })?;
                }
            }
        }

        for (path, contents) in rendered {
            let target = root.join(path_from_slashes(&path));
            let parent = target
                .parent()
                .expect("a validated relative file path has a parent");
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
            fs::write(&target, contents)
                .with_context(|| format!("writing generated file {}", target.display()))?;
        }
        Ok(())
    }

    fn rendered(&self) -> Result<BTreeMap<String, String>> {
        let mut digests = BTreeMap::new();
        for (path, contents) in &self.files {
            digests.insert(path.clone(), digest(contents.as_bytes()));
        }
        let manifest = OutputManifest {
            format: MANIFEST_FORMAT.to_owned(),
            files: digests,
        };
        let mut manifest_text = serde_json::to_string_pretty(&manifest)?;
        manifest_text.push('\n');
        let mut rendered = self.files.clone();
        rendered.insert(MANIFEST_PATH.to_owned(), manifest_text);
        Ok(rendered)
    }
}

fn validate_manifest(manifest: &OutputManifest) -> Result<()> {
    if manifest.format != MANIFEST_FORMAT {
        bail!(
            "unsupported generated-root manifest format {:?}",
            manifest.format
        );
    }
    for path in manifest.files.keys() {
        validate_path(path)?;
        if path == MANIFEST_PATH {
            bail!("generated-root manifest cannot list itself");
        }
    }
    Ok(())
}

fn read_tree(root: &Path) -> Result<BTreeMap<String, String>> {
    if !root.exists() {
        return Ok(BTreeMap::new());
    }
    if !root.is_dir() {
        bail!("generated root {} is not a directory", root.display());
    }
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("resolving generated root {}", root.display()))?;
    let mut pending = vec![canonical_root.clone()];
    let mut files = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .with_context(|| format!("reading generated directory {}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::path);
        for entry in entries {
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_symlink() {
                bail!("generated root contains symlink {}", path.display());
            }
            if file_type.is_dir() {
                pending.push(path);
                continue;
            }
            if !file_type.is_file() {
                bail!(
                    "generated root contains unsupported entry {}",
                    path.display()
                );
            }
            let relative = path
                .strip_prefix(&canonical_root)
                .expect("walked entries remain below the generated root");
            let relative = slash_path(relative)?;
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("reading generated file {} as UTF-8", path.display()))?;
            files.insert(relative, contents);
        }
    }
    Ok(files)
}

fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\\')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        bail!("generated path `{path}` is not a slash-separated file path");
    }
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("generated path `{path}` escapes or aliases its root");
    }
    Ok(())
}

fn slash_path(path: &Path) -> Result<String> {
    let mut rendered = String::new();
    for component in path.components() {
        let std::path::Component::Normal(component) = component else {
            bail!("generated path {} is not relative", path.display());
        };
        let component = component
            .to_str()
            .ok_or_else(|| anyhow!("generated path {} is not UTF-8", path.display()))?;
        if !rendered.is_empty() {
            rendered.push('/');
        }
        rendered.push_str(component);
    }
    validate_path(&rendered)?;
    Ok(rendered)
}

fn path_from_slashes(path: &str) -> PathBuf {
    path.split('/').collect()
}

fn digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "service-builder-tree-{}-{sequence}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).expect("remove stale test directory");
            }
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if self.0.exists() {
                fs::remove_dir_all(&self.0).expect("remove test directory");
            }
        }
    }

    #[test]
    fn writing_then_checking_is_byte_identical() {
        let root = TestDirectory::new();
        let mut tree = ArtifactTree::new();
        tree.insert("runtime/ir.json", "{}\n").unwrap();
        tree.insert("client/plan.json", "[]\n").unwrap();

        tree.write(&root.0).unwrap();
        assert_eq!(tree.check(&root.0).unwrap(), Vec::<Drift>::new());
    }

    #[test]
    fn stale_owned_files_are_removed_but_unowned_files_are_refused() {
        let root = TestDirectory::new();
        let mut first = ArtifactTree::new();
        first.insert("old.txt", "old").unwrap();
        first.write(&root.0).unwrap();

        let mut second = ArtifactTree::new();
        second.insert("new.txt", "new").unwrap();
        second.write(&root.0).unwrap();
        assert!(!root.0.join("old.txt").exists());

        fs::write(root.0.join("handwritten.txt"), "mine").unwrap();
        assert!(second.write(&root.0).is_err());
        assert_eq!(
            fs::read_to_string(root.0.join("handwritten.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn paths_cannot_escape_or_claim_the_manifest() {
        let mut tree = ArtifactTree::new();
        assert!(tree.insert("../outside", "no").is_err());
        assert!(tree.insert(MANIFEST_PATH, "no").is_err());
    }
}
