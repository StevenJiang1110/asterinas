// SPDX-License-Identifier: MPL-2.0

use alloc::{ffi::CString, vec::Vec};

use ostd::mm::{Fallible, VmReader};

/// A trait providing the ability to read a C string.
pub trait ReadCString {
    /// Reads bytes until the first nul byte and creates a C string.
    ///
    /// This method reads up to `max_len` bytes. The kernel must limit `max_len`
    /// to prevent unbounded heap allocation (i.e. it cannot be a value
    /// specified arbitrarily by the user space). If no nul terminator is found
    /// after exhausting the reader or reading out `max_len` bytes, this method
    /// will return `None`.
    fn read_cstring_until_nul(&mut self, max_len: usize) -> ostd::Result<Option<CString>>;

    /// Reads bytes until the first nul byte or the reader end, and creates a
    /// C string.
    ///
    /// This method reads up to `max_len` bytes. The kernel must limit `max_len`
    /// to prevent unbounded heap allocation (i.e. it cannot be a value
    /// specified arbitrarily by the user space). If no nul terminator is found
    /// after exhausting the reader or reading `max_len` bytes, this method will
    /// construct a C string with a nul byte appended.
    ///
    /// Depending on whether the nul byte is found in the reader, the number of
    /// bytes read may equal the length of the C string or the length of the C
    /// string plus one (i.e., the nul byte). To distinguish the two cases, this
    /// method also returns an integer representing the number of bytes read.
    fn read_cstring_until_end(&mut self, max_len: usize) -> ostd::Result<(CString, usize)>;
}

/// The recommended size for initial allocation.
///
/// This is used to optimize the allocated space for the most common case, in
/// which the user provides a short string.
pub const INIT_ALLOC_SIZE: usize = 128;

impl ReadCString for VmReader<'_, Fallible> {
    fn read_cstring_until_nul(&mut self, max_len: usize) -> ostd::Result<Option<CString>> {
        // This implementation is inspired by
        // the `do_strncpy_from_user` function in Linux kernel.
        // The original Linux implementation can be found at:
        // <https://elixir.bootlin.com/linux/v6.0.9/source/lib/strncpy_from_user.c#L28>

        let mut buffer: Vec<u8> = Vec::with_capacity(INIT_ALLOC_SIZE.min(max_len));

        if read_until_nul_byte(self, &mut buffer, max_len)? {
            return Ok(Some(CString::from_vec_with_nul(buffer).unwrap()));
        }

        Ok(None)
    }

    fn read_cstring_until_end(&mut self, max_len: usize) -> ostd::Result<(CString, usize)> {
        let mut buffer: Vec<u8> = Vec::with_capacity(INIT_ALLOC_SIZE.min(max_len));

        if read_until_nul_byte(self, &mut buffer, max_len)? {
            let buffer_len = buffer.len();
            return Ok((CString::from_vec_with_nul(buffer).unwrap(), buffer_len));
        }

        let buffer_len = buffer.len();
        Ok((CString::new(buffer).unwrap(), buffer_len))
    }
}

/// Reads bytes from `reader` into `buffer` until a nul byte is found.
///
/// This method returns the following values:
/// 1. `Ok(true)`: If a nul byte is found in the reader;
/// 2. `Ok(false)`: If no nul byte is found and the `reader` is exhausted
///    or the `max_len` is reached;
/// 3. `Err(_)`: If an error occurs while reading from the `reader`.
pub fn read_until_nul_byte(
    reader: &mut VmReader,
    buffer: &mut Vec<u8>,
    max_len: usize,
) -> ostd::Result<bool> {
    macro_rules! read_one_byte_at_a_time_while {
        ($cond:expr) => {
            while $cond {
                let byte = reader.read_val::<u8>()?;
                buffer.push(byte);
                if byte == 0 {
                    return Ok(true);
                }
            }
        };
    }

    // Handle the first few bytes to make `cur_addr` aligned with `size_of::<usize>`
    read_one_byte_at_a_time_while!(
        !is_addr_aligned(reader.cursor() as usize) && buffer.len() < max_len && reader.has_remain()
    );

    // Handle the rest of the bytes in bulk
    let mut cloned_reader = reader.clone();
    while (buffer.len() + size_of::<usize>()) <= max_len {
        let Ok(word) = cloned_reader.read_val::<usize>() else {
            break;
        };

        if has_zero(word) {
            for byte in word.to_ne_bytes() {
                reader.skip(1);
                buffer.push(byte);
                if byte == 0 {
                    return Ok(true);
                }
            }
            unreachable!("The branch should never be reached unless `has_zero` has bugs.")
        }

        reader.skip(size_of::<usize>());
        buffer.extend_from_slice(&word.to_ne_bytes());
    }

    // Handle the last few bytes that are not enough for a word
    read_one_byte_at_a_time_while!(buffer.len() < max_len && reader.has_remain());

    Ok(false)
}

/// Determines whether the value contains a zero byte.
///
/// This magic algorithm is from the Linux `has_zero` function:
/// <https://elixir.bootlin.com/linux/v6.0.9/source/include/asm-generic/word-at-a-time.h#L93>
const fn has_zero(value: usize) -> bool {
    const ONE_BITS: usize = usize::from_le_bytes([0x01; size_of::<usize>()]);
    const HIGH_BITS: usize = usize::from_le_bytes([0x80; size_of::<usize>()]);

    value.wrapping_sub(ONE_BITS) & !value & HIGH_BITS != 0
}

/// Checks if the given address is aligned.
const fn is_addr_aligned(addr: usize) -> bool {
    (addr & (size_of::<usize>() - 1)) == 0
}
