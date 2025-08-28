// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/gid_map`.
pub struct GidMapFileOps;

impl GidMapFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for GidMapFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let res = format!("{:>10}{:>10}{:>10}", 0, 0, 429467295);
        Ok(res.into_bytes())
    }
}
