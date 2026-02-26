// SPDX-License-Identifier: MPL-2.0

mod fs;

use crate::{fs::vfs::file_system::FileSystem, prelude::*};

pub(super) fn init() {
    crate::fs::vfs::registry::register(&fs::VirtioFsType).unwrap();
}

pub(in crate::fs) fn new(tag: &str) -> Result<Arc<dyn FileSystem>> {
    fs::new(tag)
}
