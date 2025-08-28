// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/[pid]/oom_score_adj`.
pub struct OomScoreAdjFileOps;

impl OomScoreAdjFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl FileOps for OomScoreAdjFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let mut res = Vec::new();
        res.push(b'0');
        res.push(b'\n');
        Ok(res)
    }
}
