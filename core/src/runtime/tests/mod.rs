//! Runtime regression tests that span domain modules.

mod harness;
mod lifecycle_concurrency;
mod refactoring_contracts;
mod shell_inspection;

pub(crate) mod privileged_contracts;

use super::{FileMetadata, FileSystemObservationHost, HostError, InMemoryHost};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

// The consumer must compile with filesystem observation as its only host capability.
pub(super) struct ObservationOnlyHost(pub(super) InMemoryHost);

type ReadFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HostError>> + Send + 'a>>;

impl FileSystemObservationHost for ObservationOnlyHost {
    fn canonicalize<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.0.canonicalize(path)
    }

    fn read<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<u8>> {
        self.0.read(path)
    }

    fn metadata<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, FileMetadata> {
        self.0.metadata(path)
    }

    fn read_dir<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<PathBuf>> {
        self.0.read_dir(path)
    }

    fn read_link<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.0.read_link(path)
    }
}
