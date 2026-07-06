//! Secret storage backends for `shine env encrypt`/`decrypt`.
//!
//! Today GPG is the only backend. The [`SecretBackend`] trait exists so a
//! future Touch ID / keychain backend can be added alongside it without
//! reshaping call sites: ciphertext would carry a backend tag (e.g.
//! `keychain:<item-ref>`), with untagged base64 continuing to route to GPG
//! for backward compatibility with existing `config.toml` secrets.

mod gpg;

pub use gpg::{GpgBackend, decrypt_base64_gpg_secret, encrypt_gpg_secret_to_base64};

use anyhow::Result;
use std::future::Future;

/// A pluggable secret encryption/decryption backend.
pub trait SecretBackend {
    fn encrypt(&self, plaintext: &[u8]) -> impl Future<Output = Result<String>> + Send;
    fn decrypt(&self, ciphertext: &str) -> impl Future<Output = Result<String>> + Send;
}
