mod jsonc;
mod template;

use std::collections::BTreeMap;

/// Validate transform spec names without applying them.
pub(crate) fn validate(specs: &[String]) -> anyhow::Result<()> {
    for spec in specs {
        if !matches!(spec.as_str(), "jsonc-to-json" | "template") {
            anyhow::bail!("unknown transform {spec:?} (known: jsonc-to-json, template)");
        }
    }
    Ok(())
}

/// Apply a pipeline of transforms to `input`, returning the transformed bytes.
///
/// `env` is passed to the `template` transform; other transforms ignore it.
pub(crate) fn apply(
    specs: &[String],
    input: &[u8],
    env: &BTreeMap<String, String>,
) -> anyhow::Result<Vec<u8>> {
    let mut data = input.to_vec();
    for spec in specs {
        data = apply_one(spec, &data, env)?;
    }
    Ok(data)
}

fn apply_one(spec: &str, input: &[u8], env: &BTreeMap<String, String>) -> anyhow::Result<Vec<u8>> {
    match spec {
        "jsonc-to-json" => jsonc::apply(input),
        "template" => template::apply(input, env),
        _ => anyhow::bail!("unknown transform: {spec:?}"),
    }
}
