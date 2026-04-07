// SPDX-License-Identifier: MPL-2.0

use aster_time::read_monotonic_time;
use core2::io::{Cursor, Read};
use cpio_decoder::{CpioDecoder, CpioEntry, FileMetadata, FileType};
use device_id::{DeviceId, MajorId, MinorId};
use lending_iterator::LendingIterator;
use ostd::boot::boot_info;
use zune_inflate::{DeflateDecoder, DeflateOptions};

use super::{
    file::{InodeMode, InodeType},
    vfs::path::{FsPath, Path, PathResolver, is_dot},
};
use crate::{fs::vfs::inode::MknodType, prelude::*};

struct BoxedReader<'a>(Box<dyn Read + 'a>);

impl<'a> BoxedReader<'a> {
    pub fn new(reader: Box<dyn Read + 'a>) -> Self {
        BoxedReader(reader)
    }
}

impl Read for BoxedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> core2::io::Result<usize> {
        self.0.read(buf)
    }
}

struct RootfsPopulator<'a> {
    path_resolver: &'a PathResolver,
    parent_cache: BTreeMap<String, Path>,
}

impl<'a> RootfsPopulator<'a> {
    fn new(path_resolver: &'a PathResolver) -> Self {
        Self {
            path_resolver,
            parent_cache: BTreeMap::new(),
        }
    }

    fn append_entry(&mut self, entry: &mut CpioEntry<BoxedReader>) -> Result<()> {
        // Make sure the name is a relative path, and is not end with "/".
        let entry_name = entry
            .name()
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string();
        if entry_name.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "invalid entry name");
        }
        if is_dot(&entry_name) {
            return Ok(());
        }

        // Here we assume that the directory referred by "prefix" must has been created.
        // The basis of this assumption is：
        // The mkinitramfs script uses `find` command to ensure that the entries are
        // sorted that a directory always appears before its child directories and files.
        let (parent, name) = if let Some((prefix, last)) = entry_name.rsplit_once('/') {
            (self.lookup_parent(prefix)?, last.to_string())
        } else {
            (self.path_resolver.root().clone(), entry_name.clone())
        };

        let metadata = entry.metadata();
        let mode = InodeMode::from_bits_truncate(metadata.permission_mode());
        match metadata.file_type() {
            FileType::File => {
                self.append_regular_file(entry, &parent, &name, mode, metadata.size() as usize)?;
            }
            FileType::Dir => {
                let path = parent.new_fs_child(&name, InodeType::Dir, mode)?;
                self.parent_cache.insert(entry_name, path);
            }
            FileType::Link => {
                let path = parent.new_fs_child(&name, InodeType::SymLink, mode)?;
                let link_content = {
                    let mut link_data = Vec::with_capacity(metadata.size() as usize);
                    entry.read_all(&mut link_data)?;
                    core::str::from_utf8(&link_data)?.to_string()
                };
                path.inode().write_link(&link_content)?;
            }
            FileType::Char => {
                let device_id = try_device_id_from_metadata(metadata)?;
                parent.mknod(&name, mode, MknodType::CharDevice(device_id))?;
            }
            FileType::Block => {
                let device_id = try_device_id_from_metadata(metadata)?;
                parent.mknod(&name, mode, MknodType::BlockDevice(device_id))?;
            }
            FileType::FiFo => {
                parent.mknod(&name, mode, MknodType::NamedPipe)?;
            }
            FileType::Socket => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "socket files are not supported in initramfs"
                )
            }
        }

        Ok(())
    }

    fn lookup_parent(&mut self, prefix: &str) -> Result<Path> {
        if let Some(parent) = self.parent_cache.get(prefix) {
            return Ok(parent.clone());
        }

        let parent = self.path_resolver.lookup(&FsPath::try_from(prefix)?)?;
        self.parent_cache.insert(prefix.to_string(), parent.clone());
        Ok(parent)
    }

    fn append_regular_file(
        &mut self,
        entry: &mut CpioEntry<BoxedReader>,
        parent: &Path,
        name: &str,
        mode: InodeMode,
        file_size: usize,
    ) -> Result<()> {
        let path = parent.new_fs_child(name, InodeType::File, mode)?;
        if let Err(error) = self.write_regular_file(entry, &path, file_size) {
            let _ = parent.unlink(name);
            return Err(error);
        }

        Ok(())
    }

    fn write_regular_file(
        &mut self,
        entry: &mut CpioEntry<BoxedReader>,
        path: &Path,
        file_size: usize,
    ) -> Result<()> {
        if file_size > 0 {
            path.resize(file_size)?;
        }
        entry.read_all(path.inode().writer(0))?;
        Ok(())
    }
}

/// Unpack and prepare the rootfs from the initramfs CPIO buffer.
pub fn init_in_first_kthread(path_resolver: &PathResolver) -> Result<()> {
    let initramfs_buf = boot_info()
        .initramfs
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "no initramfs found"))?;

    let is_gzip = matches!(&initramfs_buf[..4], &[0x1F, 0x8B, _, _]);
    let suffix = if is_gzip { ".gz" } else { "" };

    println!("[kernel] unpacking initramfs.cpio{} to rootfs ...", suffix);
    let start_time = read_monotonic_time();

    let reader = match &initramfs_buf[..4] {
        &[0x1F, 0x8B, _, _] => {
            let decoder_options = DeflateOptions::default()
                .set_size_hint(estimate_gzip_uncompressed_size(initramfs_buf))
                .set_confirm_checksum(false);
            let mut gzip_decoder = DeflateDecoder::new_with_options(initramfs_buf, decoder_options);
            let decompressed = gzip_decoder.decode_gzip().map_err(|_| {
                Error::with_message(Errno::EINVAL, "failed to decompress initramfs")
            })?;
            BoxedReader::new(Box::new(Cursor::new(decompressed)))
        }
        _ => BoxedReader::new(Box::new(Cursor::new(initramfs_buf))),
    };

    let mut decoder = CpioDecoder::new(reader);
    let mut populator = RootfsPopulator::new(path_resolver);
    let mut entry_count = 0usize;

    while let Some(entry_result) = decoder.next() {
        let mut entry = entry_result?;
        entry_count += 1;
        if let Err(e) = populator.append_entry(&mut entry) {
            warn!(
                "[kernel] failed to add entry {} to rootfs: {:?}",
                entry.name(),
                e
            );
        }
    }

    let elapsed = read_monotonic_time() - start_time;
    println!(
        "[kernel] rootfs is ready ({} entries, {}.{:03} ms)",
        entry_count,
        elapsed.as_millis(),
        elapsed.as_micros() % 1_000
    );
    Ok(())
}

fn try_device_id_from_metadata(metadata: &FileMetadata) -> Result<u64> {
    let major = {
        let dev_maj = u16::try_from(metadata.rdev_maj())?;
        MajorId::try_from(dev_maj).map_err(|msg| Error::with_message(Errno::EINVAL, msg))?
    };
    let minor = MinorId::try_from(metadata.rdev_min())
        .map_err(|msg| Error::with_message(Errno::EINVAL, msg))?;
    Ok(DeviceId::new(major, minor).as_encoded_u64())
}

fn estimate_gzip_uncompressed_size(gzip_buf: &[u8]) -> usize {
    const GZIP_ISIZE_LEN: usize = 4;

    if gzip_buf.len() < GZIP_ISIZE_LEN {
        return 0;
    }

    let footer = &gzip_buf[gzip_buf.len() - GZIP_ISIZE_LEN..];
    let mut isize_bytes = [0u8; GZIP_ISIZE_LEN];
    isize_bytes.copy_from_slice(footer);
    u32::from_le_bytes(isize_bytes) as usize
}
