mod jsonc;

/// Validate transform spec names without applying them.
pub(crate) fn validate(specs: &[String]) -> anyhow::Result<()> {
    for spec in specs {
        if !matches!(spec.as_str(), "jsonc-to-json") {
            anyhow::bail!("unknown transform {spec:?} (known: jsonc-to-json)");
        }
    }
    Ok(())
}

/// Apply a pipeline of transforms to `input`, returning the transformed bytes.
pub(crate) fn apply(specs: &[String], input: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut data = input.to_vec();
    for spec in specs {
        data = apply_one(spec, &data)?;
    }
    Ok(data)
}

fn apply_one(spec: &str, input: &[u8]) -> anyhow::Result<Vec<u8>> {
    match spec {
        "jsonc-to-json" => jsonc::apply(input),
        _ => anyhow::bail!("unknown transform: {spec:?}"),
    }
}
