/// Recursively syncs `doc` (a formatted TOML document) to match `target`:
/// removes keys absent from `target`, inserts/updates keys that differ, recurses
/// into nested tables, and leaves unchanged values (and their formatting/comments) untouched.
pub fn sync_table(doc: &mut toml_edit::Table, target: &toml::Table) {
    let to_remove = doc
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !target.contains_key(k))
        .collect::<Vec<_>>();

    for k in &to_remove {
        doc.remove(k);
    }
    for (key, target_value) in target {
        match target_value {
            toml::Value::Table(sub_target) => {
                let entry = doc
                    .entry(key)
                    .or_insert(toml_edit::Item::Table(Default::default()));
                if let Some(sub_doc) = entry.as_table_mut() {
                    sync_table(sub_doc, sub_target);
                }
            }
            _ => {
                if let (Some(inline), toml::Value::String(value)) = (
                    doc.get_mut(key).and_then(|item| item.as_inline_table_mut()),
                    target_value,
                ) && inline.contains_key("value")
                {
                    let unchanged = inline
                        .get("value")
                        .and_then(toml_edit::Value::as_str)
                        .is_some_and(|existing| existing == value);
                    if !unchanged {
                        inline.insert("value", toml_edit::Value::from(value.as_str()));
                    }
                    continue;
                }
                if let Some(existing) = doc.get(key).and_then(|t| t.as_value())
                    && values_equal(existing, target_value)
                {
                    continue;
                }
                doc.insert(
                    key,
                    toml_edit::value(convert_to_edit_toml_value(target_value)),
                );
            }
        }
    }
}

fn values_equal(edit: &toml_edit::Value, target: &toml::Value) -> bool {
    match (edit, target) {
        (toml_edit::Value::String(a), toml::Value::String(b)) => a.value() == b,
        (toml_edit::Value::Integer(a), toml::Value::Integer(b)) => a.value() == b,
        (toml_edit::Value::Float(a), toml::Value::Float(b)) => a.value() == b,
        (toml_edit::Value::Boolean(a), toml::Value::Boolean(b)) => a.value() == b,
        (toml_edit::Value::Array(a), toml::Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| values_equal(a, b))
        }
        _ => false,
    }
}

fn convert_to_edit_toml_value(v: &toml::Value) -> toml_edit::Value {
    match v {
        toml::Value::String(s) => toml_edit::Value::from(s.as_str()),
        toml::Value::Integer(i) => toml_edit::Value::from(*i),
        toml::Value::Float(f) => toml_edit::Value::from(*f),
        toml::Value::Boolean(b) => toml_edit::Value::from(*b),
        toml::Value::Datetime(d) => toml_edit::Value::from(*d),
        toml::Value::Array(arr) => {
            toml_edit::Value::Array(arr.iter().map(convert_to_edit_toml_value).collect())
        }
        toml::Value::Table(table) => {
            let mut inline = toml_edit::InlineTable::new();
            for (k, v) in table {
                inline.insert(k, convert_to_edit_toml_value(v));
            }
            toml_edit::Value::InlineTable(inline)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_preserves_toml_types_and_contents() {
        for literal in [
            "'hello'",
            "42",
            "2.5",
            "true",
            "false",
            "1979-05-27T07:32:00Z",
            "['a', 1, false]",
            "[[1, 2], [3]]",
            "{ key1 = 'val', key2 = 99 }",
            "{ inner = { x = true } }",
        ] {
            let expected = literal.parse::<toml::Value>().unwrap();
            let converted = convert_to_edit_toml_value(&expected);
            let actual = converted.to_string().parse::<toml::Value>().unwrap();
            assert_eq!(actual, expected, "{literal}");
        }
    }

    fn make_doc(toml: &str) -> toml_edit::DocumentMut {
        toml.parse().unwrap()
    }

    fn parse_table(toml: &str) -> toml::Table {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn sync_table_matches_target_keys_and_values_at_each_depth() {
        for (original, desired) in [
            ("", "name = \"shine\"\nversion = 1\n"),
            ("old = \"gone\"\nkeep = true\n", "keep = true"),
            ("version = 1", "version = 2"),
            (
                "[db]\nhost = \"old\"\nport = 5432\n",
                "[db]\nhost = \"new\"\nport = 5432\n",
            ),
            ("[section]\nkeep = 1\nstale = 2\n", "[section]\nkeep = 1\n"),
            ("a = 1\nb = 2\n", ""),
            ("", "x = true\ny = 3.14\n"),
        ] {
            let mut doc = make_doc(original);
            let target = parse_table(desired);
            sync_table(doc.as_table_mut(), &target);
            assert_eq!(
                parse_table(&doc.to_string()),
                target,
                "{original:?} -> {desired:?}"
            );
        }
    }

    #[test]
    fn sync_table_preserves_unchanged_value() {
        let mut doc = make_doc(r#"name = "shine" # important comment"#);
        let target = parse_table(r#"name = "shine""#);
        let before = doc.to_string();
        sync_table(doc.as_table_mut(), &target);
        assert_eq!(doc.to_string(), before);
    }

    #[test]
    fn sync_table_updates_inline_env_value_and_preserves_description() {
        let mut doc = make_doc(
            r#"[env]
TOKEN = { value = "old", description = "API token" }
"#,
        );
        let target = parse_table("[env]\nTOKEN = \"new\"\n");

        sync_table(doc.as_table_mut(), &target);

        let inline = doc["env"]["TOKEN"].as_inline_table().unwrap();
        assert_eq!(inline["value"].as_str(), Some("new"));
        assert_eq!(inline["description"].as_str(), Some("API token"));
    }

    #[test]
    fn values_equal_compares_types_and_nested_contents() {
        for literal in [
            "'hello'",
            "42",
            "2.5",
            "true",
            "['a', 1, false]",
            "[['x'], false]",
        ] {
            assert!(
                values_equal(&literal.parse().unwrap(), &literal.parse().unwrap()),
                "{literal}"
            );
        }
        for (edit, target) in [
            ("'hello'", "'world'"),
            ("42", "7"),
            ("'42'", "42"),
            ("['a', 1, false]", "['a', 1]"),
            ("[['x'], false]", "[['y'], false]"),
        ] {
            assert!(
                !values_equal(&edit.parse().unwrap(), &target.parse().unwrap()),
                "{edit} vs {target}"
            );
        }
    }
}
