use super::*;

#[test]
fn managed_json_subset_bytes_and_receipt_hash_stay_stable() {
    let keys = vec!["zeta".into(), "alpha".into()];
    let source = br#"{"zeta":null,"ignored":"secret","alpha":1}"#;
    let golden = b"{\n  \"alpha\": 1,\n  \"zeta\": null\n}\n";
    assert_eq!(
        merge_managed_json_bytes(None, source, &keys).unwrap(),
        golden
    );
    assert_eq!(
        managed_json_hash(source, &keys).unwrap(),
        5755204479466105163
    );
    assert_eq!(
        installed_json_hash(br#"{"other":true,"alpha":1,"zeta":null}"#, &keys).unwrap(),
        Some(5755204479466105163)
    );
}

#[test]
fn managed_json_keeps_absent_partial_and_null_states_distinct() {
    let keys = vec!["alpha".into(), "zeta".into()];
    assert_eq!(installed_json_hash(br#"{"other":1}"#, &keys).unwrap(), None);
    assert!(managed_json_keys_absent(br#"{"other":1}"#, &keys).unwrap());
    assert!(!managed_json_keys_absent(br#"{"zeta":null}"#, &keys).unwrap());
    assert_eq!(
        installed_json_hash(br#"{"alpha":1}"#, &keys).unwrap(),
        Some(managed_json_hash(br#"{"alpha":1}"#, &["alpha".into()]).unwrap())
    );
    assert!(!managed_json_keys_match(br#"{}"#, br#"{"zeta":null}"#, &keys).unwrap());
    assert_eq!(
        managed_json_payload(br#"{"alpha":1}"#, &keys)
            .unwrap_err()
            .to_string(),
        "json-merge: source missing managed key `zeta`"
    );
}

#[test]
fn managed_json_merge_remove_and_restore_preserve_current_unrelated_values() {
    let keys = vec!["owned".into(), "removed".into()];
    let current = br#"{"owned":{"old":1},"removed":2,"user":{"live":true}}"#;
    let merged = merge_managed_json_bytes(
        Some(current),
        br#"{"owned":{"next":3},"removed":null,"ignored":false}"#,
        &keys,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<JsonValue>(&merged).unwrap(),
        serde_json::json!({"owned":{"next":3},"removed":null,"user":{"live":true}})
    );
    let restored =
        restore_managed_json_bytes(&merged, br#"{"owned":null,"user":"old"}"#, &keys).unwrap();
    assert_eq!(
        serde_json::from_slice::<JsonValue>(&restored).unwrap(),
        serde_json::json!({"owned":null,"user":{"live":true}})
    );
    let removed = remove_managed_json_bytes(&restored, &keys).unwrap();
    assert_eq!(
        serde_json::from_slice::<JsonValue>(&removed).unwrap(),
        serde_json::json!({"user":{"live":true}})
    );
}

#[test]
fn managed_json_errors_retain_context_and_validation_order() {
    let keys = vec!["owned".into()];
    assert_eq!(
        merge_managed_json_bytes(Some(b"[]"), b"null", &keys)
            .unwrap_err()
            .to_string(),
        "json-merge: source must be a JSON object"
    );
    assert_eq!(
        merge_managed_json_bytes(Some(b"[]"), br#"{"owned":1}"#, &keys)
            .unwrap_err()
            .to_string(),
        "json-merge: destination must be a JSON object"
    );
    assert_eq!(
        restore_managed_json_bytes(b"[]", b"null", &keys)
            .unwrap_err()
            .to_string(),
        "json-merge: destination must be a JSON object"
    );
    assert_eq!(
        restore_managed_json_bytes(b"{}", b"null", &keys)
            .unwrap_err()
            .to_string(),
        "json-merge: rollback must be a JSON object"
    );
    assert_eq!(
        managed_json_keys_match(b"{}", b"[]", &keys)
            .unwrap_err()
            .to_string(),
        "json-merge: comparison must be a JSON object"
    );
    assert_eq!(
        installed_json_hash(b"{", &keys).unwrap_err().to_string(),
        "json-merge: destination must be a JSON object"
    );
}
