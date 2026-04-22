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
    fn test_convert_to_edit_toml_value_string() {
        let v = toml::Value::String("hello".to_string());
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::String(_)));
        if let toml_edit::Value::String(s) = result {
            assert_eq!(s.value(), "hello");
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_integer() {
        let v = toml::Value::Integer(42);
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::Integer(_)));
        if let toml_edit::Value::Integer(i) = result {
            assert_eq!(*i.value(), 42);
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_float() {
        let v = toml::Value::Float(3.14);
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::Float(_)));
        if let toml_edit::Value::Float(f) = result {
            assert_eq!(*f.value(), 3.14);
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_boolean() {
        for b in [true, false] {
            let v = toml::Value::Boolean(b);
            let result = convert_to_edit_toml_value(&v);
            assert!(matches!(result, toml_edit::Value::Boolean(_)));
            if let toml_edit::Value::Boolean(val) = result {
                assert_eq!(*val.value(), b);
            }
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_datetime() {
        let dt: toml::value::Datetime = "1979-05-27T07:32:00Z".parse().unwrap();
        let v = toml::Value::Datetime(dt.clone());
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::Datetime(_)));
        if let toml_edit::Value::Datetime(d) = result {
            assert_eq!(d.value().to_string(), dt.to_string());
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_array() {
        let v = toml::Value::Array(vec![
            toml::Value::String("a".to_string()),
            toml::Value::Integer(1),
            toml::Value::Boolean(false),
        ]);
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::Array(_)));
        if let toml_edit::Value::Array(arr) = result {
            assert_eq!(arr.len(), 3);
            assert!(matches!(
                arr.iter().next().unwrap(),
                toml_edit::Value::String(_)
            ));
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_nested_array() {
        let v = toml::Value::Array(vec![
            toml::Value::Array(vec![toml::Value::Integer(1), toml::Value::Integer(2)]),
            toml::Value::Array(vec![toml::Value::Integer(3)]),
        ]);
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::Array(_)));
        if let toml_edit::Value::Array(outer) = result {
            assert_eq!(outer.len(), 2);
            assert!(matches!(
                outer.iter().next().unwrap(),
                toml_edit::Value::Array(_)
            ));
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_table() {
        let mut table = toml::value::Table::new();
        table.insert("key1".to_string(), toml::Value::String("val".to_string()));
        table.insert("key2".to_string(), toml::Value::Integer(99));
        let v = toml::Value::Table(table);
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::InlineTable(_)));
        if let toml_edit::Value::InlineTable(inline) = result {
            assert_eq!(inline.len(), 2);
            let v1 = inline.get("key1").unwrap();
            assert!(matches!(v1, toml_edit::Value::String(_)));
            if let toml_edit::Value::String(s) = v1 {
                assert_eq!(s.value(), "val");
            }
            let v2 = inline.get("key2").unwrap();
            assert!(matches!(v2, toml_edit::Value::Integer(_)));
            if let toml_edit::Value::Integer(i) = v2 {
                assert_eq!(*i.value(), 99);
            }
        }
    }

    #[test]
    fn test_convert_to_edit_toml_value_nested_table() {
        let mut inner = toml::value::Table::new();
        inner.insert("x".to_string(), toml::Value::Boolean(true));
        let mut outer = toml::value::Table::new();
        outer.insert("inner".to_string(), toml::Value::Table(inner));
        let v = toml::Value::Table(outer);
        let result = convert_to_edit_toml_value(&v);
        assert!(matches!(result, toml_edit::Value::InlineTable(_)));
        if let toml_edit::Value::InlineTable(inline) = result {
            let nested = inline.get("inner").unwrap();
            assert!(matches!(nested, toml_edit::Value::InlineTable(_)));
            if let toml_edit::Value::InlineTable(inner_inline) = nested {
                assert_eq!(inner_inline.len(), 1);
                let xv = inner_inline.get("x").unwrap();
                assert!(matches!(xv, toml_edit::Value::Boolean(_)));
                if let toml_edit::Value::Boolean(b) = xv {
                    assert!(*b.value());
                }
            }
        }
    }

    fn make_doc(toml: &str) -> toml_edit::DocumentMut {
        toml.parse().unwrap()
    }

    fn make_target(toml: &str) -> toml::Table {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn sync_table_adds_new_keys() {
        let mut doc = make_doc("");
        let target = make_target("name = \"shine\"\nversion = 1\n");
        sync_table(doc.as_table_mut(), &target);
        assert_eq!(doc["name"].as_str(), Some("shine"));
        assert_eq!(doc["version"].as_integer(), Some(1));
    }

    #[test]
    fn sync_table_removes_stale_keys() {
        let mut doc = make_doc("old = \"gone\"\nkeep = true\n");
        let target = make_target("keep = true");
        sync_table(doc.as_table_mut(), &target);
        assert!(doc.get("old").is_none());
        assert!(doc.get("keep").is_some());
    }

    #[test]
    fn sync_table_updates_changed_value() {
        let mut doc = make_doc("version = 1");
        let target = make_target("version = 2");
        sync_table(doc.as_table_mut(), &target);
        assert_eq!(doc["version"].as_integer(), Some(2));
    }

    #[test]
    fn sync_table_preserves_unchanged_value() {
        let mut doc = make_doc(r#"name = "shine" # important comment"#);
        let target = make_target(r#"name = "shine""#);
        let before = doc.to_string();
        sync_table(doc.as_table_mut(), &target);
        assert_eq!(doc.to_string(), before);
    }

    #[test]
    fn sync_table_recurses_into_nested_tables() {
        let mut doc = make_doc("[db]\nhost = \"old\"\nport = 5432\n");
        let target = make_target("[db]\nhost = \"new\"\nport = 5432\n");
        sync_table(doc.as_table_mut(), &target);
        assert_eq!(doc["db"]["host"].as_str(), Some("new"));
        assert_eq!(doc["db"]["port"].as_integer(), Some(5432));
    }

    #[test]
    fn sync_table_removes_nested_stale_keys() {
        let mut doc = make_doc("[section]\nkeep = 1\nstale = 2\n");
        let target = make_target("[section]\nkeep = 1\n");
        sync_table(doc.as_table_mut(), &target);
        assert!(doc["section"].get("stale").is_none());
        assert_eq!(doc["section"]["keep"].as_integer(), Some(1));
    }

    #[test]
    fn sync_table_empty_target_clears_doc() {
        let mut doc = make_doc("a = 1\nb = 2\n");
        let target = make_target("");
        sync_table(doc.as_table_mut(), &target);
        assert!(doc.get("a").is_none());
        assert!(doc.get("b").is_none());
    }

    #[test]
    fn sync_table_empty_doc_fills_from_target() {
        let mut doc = make_doc("");
        let target = make_target("x = true\ny = 3.14\n");
        sync_table(doc.as_table_mut(), &target);
        assert_eq!(doc["x"].as_bool(), Some(true));
    }

    #[test]
    fn test_values_equal() {
        use toml::Value as TargetValue;
        use toml_edit::Value as EditValue;

        assert!(values_equal(
            &EditValue::from("hello"),
            &TargetValue::from("hello")
        ));
        assert!(values_equal(
            &EditValue::from(42i64),
            &TargetValue::from(42i64)
        ));
        assert!(values_equal(
            &EditValue::from(3.14f64),
            &TargetValue::from(3.14f64)
        ));
        assert!(values_equal(
            &EditValue::from(true),
            &TargetValue::from(true)
        ));

        let edit_array = EditValue::Array(
            vec![
                EditValue::from("a"),
                EditValue::from(1i64),
                EditValue::from(false),
            ]
            .into_iter()
            .collect(),
        );
        let target_array = TargetValue::Array(vec![
            TargetValue::from("a"),
            TargetValue::from(1i64),
            TargetValue::from(false),
        ]);

        assert!(values_equal(&edit_array, &target_array));

        let different_string = TargetValue::from("world");
        assert!(!values_equal(&EditValue::from("hello"), &different_string));

        let different_int = TargetValue::from(7i64);
        assert!(!values_equal(&EditValue::from(42i64), &different_int));

        let different_type = TargetValue::from(42i64);
        assert!(!values_equal(&EditValue::from("42"), &different_type));

        let shorter_array =
            TargetValue::Array(vec![TargetValue::from("a"), TargetValue::from(1i64)]);
        assert!(!values_equal(&edit_array, &shorter_array));

        let nested_edit_array = EditValue::Array(
            vec![
                EditValue::Array(vec![EditValue::from("x")].into_iter().collect()),
                EditValue::from(false),
            ]
            .into_iter()
            .collect(),
        );
        let nested_target_array = TargetValue::Array(vec![
            TargetValue::Array(vec![TargetValue::from("x")]),
            TargetValue::from(false),
        ]);
        assert!(values_equal(&nested_edit_array, &nested_target_array));
    }
}
