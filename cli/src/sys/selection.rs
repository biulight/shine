pub(super) fn format_item_ids(item_ids: &[String]) -> String {
    if item_ids.is_empty() {
        "(none)".to_string()
    } else {
        item_ids.join(", ")
    }
}
