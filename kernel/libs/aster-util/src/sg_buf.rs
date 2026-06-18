// SPDX-License-Identifier: MPL-2.0

//! Scatter-gather buffer traits.
//!
//! `MultiRead` represents one or more readable byte ranges, while
//! `MultiWrite` represents one or more writable byte ranges. These traits live
//! in `aster-util` so kernel components and utility crates can share
//! buffer-copying interfaces without depending on `aster-kernel`.

use ostd::{
    Error,
    mm::{FallibleVmRead, FallibleVmWrite, Infallible, VmReader, VmWriter},
};
use ostd_pod::Pod;

use crate::read_cstring::ReadCString;

/// Defines the read behavior for a scatter-gather buffer.
pub trait MultiRead: ReadCString {
    /// Reads the exact number of bytes required to exhaust `self` or fill `writer`,
    /// accumulating total bytes read.
    ///
    /// If the return value is `Ok(n)`,
    /// then `n` should be `min(self.sum_lens(), writer.avail())`.
    ///
    /// # Errors
    ///
    /// This method returns [`Error::PageFault`] if a page fault occurs, along with
    /// the number of bytes copied before the error occurs. When an error is returned,
    /// both `self` and `writer` are advanced by the returned byte count.
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (Error, usize)>;

    /// Calculates the total length of data remaining to read.
    fn sum_lens(&self) -> usize;

    /// Checks if the data remaining to read is empty.
    fn is_empty(&self) -> bool {
        self.sum_lens() == 0
    }

    /// Skips the first `nbytes` bytes of data, or skips to the end if the readers have
    /// insufficient bytes.
    fn skip_some(&mut self, nbytes: usize);
}

/// Defines the write behavior for a scatter-gather buffer.
pub trait MultiWrite {
    /// Writes the exact number of bytes required to exhaust `writer` or fill `self`,
    /// accumulating total bytes read.
    ///
    /// If the return value is `Ok(n)`,
    /// then `n` should be `min(self.sum_lens(), reader.remain())`.
    ///
    /// # Errors
    ///
    /// This method returns [`Error::PageFault`] if a page fault occurs, along with
    /// the number of bytes copied before the error occurs. When an error is returned,
    /// both `self` and `reader` are advanced by the returned byte count.
    fn write(&mut self, reader: &mut VmReader<'_, Infallible>) -> Result<usize, (Error, usize)>;

    /// Calculates the length of space available to write.
    fn sum_lens(&self) -> usize;

    /// Checks if the space available to write is empty.
    fn is_empty(&self) -> bool {
        self.sum_lens() == 0
    }

    /// Skips the first `nbytes` bytes of data, or skips to the end if the writers have
    /// insufficient bytes.
    fn skip_some(&mut self, nbytes: usize);
}

impl MultiRead for VmReader<'_> {
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (Error, usize)> {
        self.read_fallible(writer)
    }

    fn sum_lens(&self) -> usize {
        self.remain()
    }

    fn skip_some(&mut self, nbytes: usize) {
        self.skip(self.remain().min(nbytes));
    }
}

impl dyn MultiRead + '_ {
    /// Reads a `T` value, returning `None` if the reader has insufficient bytes.
    pub fn read_val_opt<T: Pod>(&mut self) -> ostd::Result<Option<T>> {
        let mut val = T::new_zeroed();
        let nbytes = self
            .read(&mut VmWriter::from(val.as_mut_bytes()))
            .map_err(|(err, _copied_len)| err)?;

        if nbytes == size_of::<T>() {
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }
}

impl MultiWrite for VmWriter<'_> {
    fn write(&mut self, reader: &mut VmReader<'_, Infallible>) -> Result<usize, (Error, usize)> {
        self.write_fallible(reader)
    }

    fn sum_lens(&self) -> usize {
        self.avail()
    }

    fn skip_some(&mut self, nbytes: usize) {
        self.skip(self.avail().min(nbytes));
    }
}

impl dyn MultiWrite + '_ {
    /// Writes a `T` value, truncating the value if the writer has insufficient bytes.
    pub fn write_val_trunc<T: Pod>(&mut self, val: &T) -> ostd::Result<()> {
        let _nbytes = self
            .write(&mut VmReader::from(val.as_bytes()))
            .map_err(|(err, _copied_len)| err)?;
        // `_nbytes` may be smaller than the value size. We ignore it to truncate the value.

        Ok(())
    }
}
