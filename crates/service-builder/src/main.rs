//! Command-line generation and byte-for-byte drift checking.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use service_builder::build_service;
use service_builder::ess::EssSources;
use service_builder::tree::Drift;
use service_definition::ServiceDefinition;

#[derive(Debug, Parser)]
#[command(about = "Generate an ESS-backed standalone service artifact tree")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile and rewrite the exclusively owned generated output tree.
    Generate(Inputs),
    /// Compile and refuse any byte-level drift in the generated output tree.
    Check(Inputs),
}

#[derive(Debug, clap::Args)]
struct Inputs {
    /// Directory containing the ESS `system.yaml` and its YAML sources.
    #[arg(long)]
    ess: PathBuf,
    /// Strict `service-definition/1` YAML or JSON document.
    #[arg(long)]
    definition: PathBuf,
    /// Exclusively builder-owned generated output root.
    #[arg(long)]
    output: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Generate(inputs) => {
            let build = compile(&inputs)?;
            build.artifacts.write(&inputs.output)?;
            println!("generated {}", inputs.output.display());
        }
        Command::Check(inputs) => {
            let build = compile(&inputs)?;
            let drift = build.artifacts.check(&inputs.output)?;
            if !drift.is_empty() {
                let details = drift
                    .iter()
                    .map(render_drift)
                    .collect::<Vec<_>>()
                    .join("\n");
                bail!(
                    "generated output drift in {}:\n{details}",
                    inputs.output.display()
                );
            }
            println!("generated output is current: {}", inputs.output.display());
        }
    }
    Ok(())
}

fn compile(inputs: &Inputs) -> Result<service_builder::ServiceBuild> {
    let sources = EssSources::read(&inputs.ess)?;
    let definition = read_definition(&inputs.definition)?;
    build_service(&sources, &definition)
}

fn read_definition(path: &Path) -> Result<ServiceDefinition> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading service definition {}", path.display()))?;
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("yaml" | "yml") => ServiceDefinition::from_yaml(&text),
        Some("json") => ServiceDefinition::from_json(&text),
        extension => bail!(
            "service definition {} must have a .yaml, .yml, or .json extension, found {:?}",
            path.display(),
            extension
        ),
    }
    .with_context(|| format!("validating service definition {}", path.display()))
}

fn render_drift(drift: &Drift) -> String {
    match drift {
        Drift::Missing(path) => format!("missing {path}"),
        Drift::Unexpected(path) => format!("unexpected {path}"),
        Drift::Changed(path) => format!("changed {path}"),
    }
}
