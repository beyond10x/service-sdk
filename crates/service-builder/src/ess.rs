//! Loading and compiling ESS source without bypassing its validation boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};
use ess_compiler::EssIr;
use ess_compiler::source::SourceMap;
use ess_domain::spec::{RawSpecFile, Specification};
use ess_domain::system::Source;

/// A deterministic set of ESS source files, keyed by slash-separated relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EssSources {
    files: BTreeMap<String, String>,
}

impl EssSources {
    /// Creates sources from already-read files.
    pub fn new(files: BTreeMap<String, String>) -> Result<Self> {
        if files.is_empty() {
            bail!("an ESS service definition must contain at least one YAML file");
        }
        if !files.contains_key("system.yaml") {
            bail!("an ESS service definition directory must contain `system.yaml`");
        }
        for path in files.keys() {
            validate_relative_path(path)?;
            if !matches!(
                Path::new(path)
                    .extension()
                    .and_then(std::ffi::OsStr::to_str),
                Some("yaml" | "yml")
            ) {
                bail!("ESS source `{path}` is not YAML");
            }
        }
        Ok(Self { files })
    }

    /// Reads every YAML file below a specification directory in stable path order.
    pub fn read(directory: &Path) -> Result<Self> {
        if !directory.join("system.yaml").is_file() {
            bail!(
                "{} is not an ESS specification directory: `system.yaml` is absent",
                directory.display()
            );
        }
        let root = directory
            .canonicalize()
            .with_context(|| format!("resolving {}", directory.display()))?;
        let mut pending = vec![root.clone()];
        let mut visited = BTreeSet::new();
        let mut files = BTreeMap::new();

        while let Some(directory) = pending.pop() {
            let identity = directory
                .canonicalize()
                .with_context(|| format!("resolving {}", directory.display()))?;
            if !visited.insert(identity) {
                continue;
            }
            let mut entries = fs::read_dir(&directory)
                .with_context(|| format!("reading {}", directory.display()))?
                .collect::<Result<Vec<_>, _>>()?;
            entries.sort_by_key(std::fs::DirEntry::path);
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path
                    .extension()
                    .is_some_and(|extension| extension == "yaml" || extension == "yml")
                {
                    let relative = path
                        .strip_prefix(&root)
                        .expect("a discovered file remains below its root");
                    let name = slash_path(relative)?;
                    let contents = fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    files.insert(name, contents);
                }
            }
        }
        Self::new(files)
    }

    /// Files in canonical path order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files
            .iter()
            .map(|(path, contents)| (path.as_str(), contents.as_str()))
    }

    /// Parses, assembles, validates, and resolves through the official ESS compiler.
    pub fn compile(&self) -> Result<EssIr> {
        let mut parsed = Vec::with_capacity(self.files.len());
        let mut texts = SourceMap::new();
        for (path, contents) in &self.files {
            let raw = RawSpecFile::parse(contents)
                .with_context(|| format!("parsing ESS source `{path}`"))?;
            let source = Source::new(path.clone());
            texts.insert(source.as_str(), contents);
            parsed.push((source, raw));
        }
        let specification = Specification::assemble(parsed).map_err(|errors| {
            anyhow!("ESS service definition was refused during validation:\n{errors}")
        })?;
        ess_compiler::compile(&specification, &texts).map_err(|diagnostics| {
            anyhow!("ESS service definition did not resolve:\n{diagnostics}")
        })
    }
}

fn validate_relative_path(path: &str) -> Result<()> {
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("generated/source path `{path}` escapes its declared root");
    }
    Ok(())
}

fn slash_path(path: &Path) -> Result<String> {
    let mut rendered = String::new();
    for (index, component) in path.components().enumerate() {
        let std::path::Component::Normal(component) = component else {
            bail!("ESS source path {} is not relative", path.display());
        };
        let component = component
            .to_str()
            .ok_or_else(|| anyhow!("ESS source path {} is not UTF-8", path.display()))?;
        if index > 0 {
            rendered.push('/');
        }
        rendered.push_str(component);
    }
    validate_relative_path(&rendered)?;
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_cannot_escape_the_definition_root() {
        let files = BTreeMap::from([
            ("system.yaml".into(), "irrelevant".into()),
            ("../secret.yaml".into(), "irrelevant".into()),
        ]);
        assert!(EssSources::new(files).is_err());
    }

    #[test]
    fn a_source_set_requires_the_ess_header() {
        let files = BTreeMap::from([("domains/list.yaml".into(), "irrelevant".into())]);
        assert!(EssSources::new(files).is_err());
    }
}
