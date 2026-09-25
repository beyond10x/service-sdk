//! Shared exact ESS fixture loader for `/4` integration tests.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use service_builder::ess::EssSources;

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../service-host/tests/fixtures/er-v4")
        .join(name)
}

pub fn sources(name: &str) -> EssSources {
    let base = fixture(name).join("ess");
    let mut pending = vec![base.clone()];
    let mut paths = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "yaml")
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    EssSources::new(
        paths
            .into_iter()
            .map(|path| {
                let label = path
                    .strip_prefix(&base)
                    .expect("fixture child")
                    .display()
                    .to_string();
                let text = fs::read_to_string(path).expect("fixture source");
                (label, text)
            })
            .collect::<BTreeMap<_, _>>(),
    )
    .expect("explicit source inventory")
}
