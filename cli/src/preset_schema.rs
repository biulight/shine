use crate::commands::{Cli, PresetReportFormat};
use anyhow::{Context, Result};
use clap::CommandFactory;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
struct PresetCommandHelpV1 {
    name: String,
    help: String,
}

#[derive(Serialize)]
struct PresetSchemaDocumentV1 {
    schema_version: u32,
    commands: Vec<PresetCommandHelpV1>,
    schemas: BTreeMap<String, Value>,
}

pub fn handle_schema(format: PresetReportFormat) -> Result<()> {
    let core = shine_core::runtime::preset_schema_reference_v1();
    let document = PresetSchemaDocumentV1 {
        schema_version: core.schema_version,
        commands: generated_command_help()?,
        schemas: core.schemas,
    };
    match format {
        PresetReportFormat::Json => println!("{}", serde_json::to_string_pretty(&document)?),
        PresetReportFormat::Text => {
            println!(
                "Preset authoring schema reference v{}",
                document.schema_version
            );
            println!("Commands:");
            for command in &document.commands {
                println!("  {}", command.name);
            }
            println!("Schemas:");
            for name in document.schemas.keys() {
                println!("  {name}");
            }
            println!("Use --format json for generated command help and JSON Schemas.");
        }
    }
    Ok(())
}

fn generated_command_help() -> Result<Vec<PresetCommandHelpV1>> {
    let root = Cli::command();
    let preset = root
        .find_subcommand("preset")
        .context("CLI does not contain the preset command")?;
    let mut output = Vec::new();
    for name in [
        "validate", "lint", "plan", "test", "pack", "migrate", "schema",
    ] {
        let mut command = preset
            .find_subcommand(name)
            .with_context(|| format!("preset command is missing {name}"))?
            .clone()
            .bin_name(format!("shine preset {name}"));
        let mut buffer = Vec::new();
        command.write_long_help(&mut buffer)?;
        output.push(PresetCommandHelpV1 {
            name: format!("shine preset {name}"),
            help: String::from_utf8(buffer).context("generated command help was not UTF-8")?,
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_reference_uses_live_clap_help() {
        let commands = generated_command_help().unwrap();
        assert_eq!(commands.len(), 7);
        assert!(commands[2].help.contains("--platform"));
        assert!(commands[3].help.contains("shine.test.toml"));
    }
}
