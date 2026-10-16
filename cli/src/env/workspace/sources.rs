//! Pure declaration precedence. Authorization never changes which source wins.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Binding {
    Plain(usize),
    Secret(usize),
}

pub(super) fn effective_bindings<'a>(
    sources: impl IntoIterator<Item = &'a SourceFile>,
) -> BTreeMap<String, Binding> {
    let mut bindings = BTreeMap::new();
    for (index, source) in sources.into_iter().enumerate() {
        for key in source.plain.keys() {
            bindings.insert(key.clone(), Binding::Plain(index));
        }
        for key in source.secret.keys() {
            bindings.insert(key.clone(), Binding::Secret(index));
        }
    }
    bindings
}

pub(super) fn plain_values(
    sources: &[SourceFile],
    bindings: &BTreeMap<String, Binding>,
) -> SensitiveValues {
    let mut values = SensitiveValues::default();
    for (key, binding) in bindings {
        if let Binding::Plain(index) = binding {
            values.insert(key.clone(), sources[*index].plain[key].clone());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declaration_order_is_independent_of_secret_release() {
        for kinds in [
            "p", "s", "pp", "ss", "ps", "sp", "psp", "sps", "psps", "spsp",
        ] {
            let sources = kinds
                .chars()
                .enumerate()
                .map(|(index, kind)| {
                    let text = if kind == 'p' {
                        format!("[plain]\nTOKEN = 'plain-{index}'")
                    } else {
                        "[secret]\nTOKEN = true".into()
                    };
                    parse_source(Path::new("source.toml"), &text).unwrap()
                })
                .collect::<Vec<_>>();
            let bindings = effective_bindings(&sources);
            let last = sources.len() - 1;
            let plain = kinds.ends_with('p');
            assert_eq!(
                bindings["TOKEN"],
                if plain {
                    Binding::Plain(last)
                } else {
                    Binding::Secret(last)
                }
            );
            assert_eq!(
                plain_values(&sources, &bindings).contains_key("TOKEN"),
                plain
            );
            assert_eq!(
                sources
                    .iter()
                    .map(|source| source.secret.len())
                    .sum::<usize>(),
                kinds.chars().filter(|kind| *kind == 's').count()
            );
        }
    }
}
