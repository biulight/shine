//! Owned environment values. Cleanup follows ownership, including early return/cancellation.
use super::*;
use std::ops::Deref;
use zeroize::ZeroizeOnDrop;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub(crate) struct SensitiveValues(BTreeMap<String, String>);

impl std::fmt::Debug for SensitiveValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SensitiveValues([redacted])")
    }
}
impl Deref for SensitiveValues {
    type Target = BTreeMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<BTreeMap<String, String>> for SensitiveValues {
    fn from(values: BTreeMap<String, String>) -> Self {
        Self(values)
    }
}
impl SensitiveValues {
    pub(super) fn insert(&mut self, key: String, value: String) {
        if let Some(mut value) = self.0.insert(key, value) {
            value.zeroize();
        }
    }
    pub(super) fn remove(&mut self, key: &str) -> Option<String> {
        self.0.remove(key)
    }
    /// Transfer ownership only at the existing external broker map boundary.
    pub(super) fn into_map(mut self) -> BTreeMap<String, String> {
        std::mem::take(&mut self.0)
    }
}
impl Zeroize for SensitiveValues {
    fn zeroize(&mut self) {
        for value in self.0.values_mut() {
            value.zeroize();
        }
    }
}
impl ZeroizeOnDrop for SensitiveValues {}
impl Drop for SensitiveValues {
    fn drop(&mut self) {
        self.zeroize();
        #[cfg(test)]
        {
            assert!(self.0.values().all(String::is_empty));
            WIPED.with(|events| events.borrow_mut().push(self.0.keys().cloned().collect()));
        }
    }
}

#[cfg(test)]
thread_local! {
    static WIPED: std::cell::RefCell<Vec<Vec<String>>> = const { std::cell::RefCell::new(Vec::new()) };
}
#[cfg(test)]
pub(super) fn take_wipe_events() -> Vec<Vec<String>> {
    WIPED.with(|events| std::mem::take(&mut *events.borrow_mut()))
}
/// Retain the guard while iterating, so an error also wipes unconsumed values.
pub(crate) struct ValuesIntoIter(SensitiveValues);
impl Iterator for ValuesIntoIter {
    type Item = (String, String);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.0.pop_first()
    }
}
impl IntoIterator for SensitiveValues {
    type Item = (String, String);
    type IntoIter = ValuesIntoIter;
    fn into_iter(self) -> Self::IntoIter {
        ValuesIntoIter(self)
    }
}

impl FromIterator<(String, String)> for SensitiveValues {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(iter: T) -> Self {
        let mut values = Self::default();
        for (key, value) in iter {
            values.insert(key, value);
        }
        values
    }
}
impl<'a> IntoIterator for &'a SensitiveValues {
    type Item = (&'a String, &'a String);
    type IntoIter = std::collections::btree_map::Iter<'a, String, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl PartialEq<BTreeMap<String, String>> for SensitiveValues {
    fn eq(&self, other: &BTreeMap<String, String>) -> bool {
        &self.0 == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_redact_debug_and_wipe_replaced_and_remaining_values() {
        let mut values = SensitiveValues::from(BTreeMap::from([("TOKEN".into(), "secret".into())]));
        assert!(!format!("{values:?}").contains("secret"));
        values.insert("TOKEN".into(), "replacement".into());
        assert_eq!(values["TOKEN"], "replacement");
        values.zeroize();
        assert_eq!(values["TOKEN"], "");
    }
    #[test]
    fn transparent_payload_encoding_preserves_the_existing_map_contract() {
        let map = BTreeMap::from([("TOKEN".into(), "secret".into())]);
        let values = SensitiveValues::from(map.clone());
        assert_eq!(
            toml::to_string(&values).unwrap(),
            toml::to_string(&map).unwrap()
        );
        let decoded: SensitiveValues = toml::from_str(&toml::to_string(&values).unwrap()).unwrap();
        assert_eq!(&*decoded, &map);
    }
}
