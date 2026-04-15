// SPDX-License-Identifier: MPL-2.0

//! Decompresses common payload formats in `no_std` environments.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::vec::Vec;

const GZIP_ID1: u8 = 0x1F;
const GZIP_ID2: u8 = 0x8B;
const GZIP_DEFLATE_METHOD: u8 = 8;
const GZIP_FIXED_HEADER_LEN: usize = 10;
const GZIP_TRAILER_LEN: usize = 8;
const GZIP_FLAG_FHCRC: u8 = 0x02;
const GZIP_FLAG_FEXTRA: u8 = 0x04;
const GZIP_FLAG_FNAME: u8 = 0x08;
const GZIP_FLAG_FCOMMENT: u8 = 0x10;
const GZIP_FLAG_RESERVED: u8 = 0xE0;

/// Describes a decompression failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// The gzip header is malformed.
    InvalidGzipHeader,
    /// The gzip stream uses an unsupported compression method.
    UnsupportedGzipMethod,
    /// The gzip flags contain reserved bits.
    InvalidGzipFlags,
    /// The gzip stream ends before a complete header or trailer.
    TruncatedGzip,
    /// The gzip header checksum does not match the header.
    InvalidGzipHeaderChecksum,
    /// The gzip trailer checksum or size does not match the decompressed data.
    InvalidGzipTrailer,
    /// The DEFLATE payload is malformed.
    InvalidDeflatePayload,
    /// The zlib payload is malformed.
    InvalidZlibPayload,
}

/// Decompresses a gzip-encoded buffer.
pub fn decompress_gzip(input: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let gzip_member = parse_gzip_member(input)?;
    let output = miniz_oxide::inflate::decompress_to_vec(gzip_member.deflate_payload)
        .map_err(|_| DecodeError::InvalidDeflatePayload)?;

    if crc32fast::hash(&output) != gzip_member.crc32 {
        return Err(DecodeError::InvalidGzipTrailer);
    }
    if (output.len() as u32) != gzip_member.input_size {
        return Err(DecodeError::InvalidGzipTrailer);
    }

    Ok(output)
}

/// Decompresses a zlib-encoded buffer.
pub fn decompress_zlib(input: &[u8]) -> Result<Vec<u8>, DecodeError> {
    miniz_oxide::inflate::decompress_to_vec_zlib(input).map_err(|_| DecodeError::InvalidZlibPayload)
}

#[derive(Clone, Copy, Debug)]
struct GzipMember<'a> {
    deflate_payload: &'a [u8],
    crc32: u32,
    input_size: u32,
}

fn parse_gzip_member(input: &[u8]) -> Result<GzipMember<'_>, DecodeError> {
    if input.len() < GZIP_FIXED_HEADER_LEN + GZIP_TRAILER_LEN {
        return Err(DecodeError::TruncatedGzip);
    }
    if input[0] != GZIP_ID1 || input[1] != GZIP_ID2 {
        return Err(DecodeError::InvalidGzipHeader);
    }
    if input[2] != GZIP_DEFLATE_METHOD {
        return Err(DecodeError::UnsupportedGzipMethod);
    }

    let flags = input[3];
    if flags & GZIP_FLAG_RESERVED != 0 {
        return Err(DecodeError::InvalidGzipFlags);
    }

    let trailer_offset = input.len() - GZIP_TRAILER_LEN;
    let mut deflate_offset = GZIP_FIXED_HEADER_LEN;

    if flags & GZIP_FLAG_FEXTRA != 0 {
        let extra_len_bytes = input
            .get(deflate_offset..deflate_offset + 2)
            .ok_or(DecodeError::TruncatedGzip)?;
        let extra_len = u16::from_le_bytes([extra_len_bytes[0], extra_len_bytes[1]]) as usize;
        deflate_offset = deflate_offset
            .checked_add(2)
            .and_then(|offset| offset.checked_add(extra_len))
            .ok_or(DecodeError::TruncatedGzip)?;
        if deflate_offset > trailer_offset {
            return Err(DecodeError::TruncatedGzip);
        }
    }

    if flags & GZIP_FLAG_FNAME != 0 {
        deflate_offset = skip_nul_terminated_field(input, deflate_offset, trailer_offset)?;
    }
    if flags & GZIP_FLAG_FCOMMENT != 0 {
        deflate_offset = skip_nul_terminated_field(input, deflate_offset, trailer_offset)?;
    }

    if flags & GZIP_FLAG_FHCRC != 0 {
        let checksum_bytes = input
            .get(deflate_offset..deflate_offset + 2)
            .ok_or(DecodeError::TruncatedGzip)?;
        let expected_checksum = u16::from_le_bytes([checksum_bytes[0], checksum_bytes[1]]);
        let actual_checksum = crc32fast::hash(&input[..deflate_offset]) as u16;
        if actual_checksum != expected_checksum {
            return Err(DecodeError::InvalidGzipHeaderChecksum);
        }
        deflate_offset += 2;
    }

    if deflate_offset > trailer_offset {
        return Err(DecodeError::TruncatedGzip);
    }

    let trailer = &input[trailer_offset..];
    Ok(GzipMember {
        deflate_payload: &input[deflate_offset..trailer_offset],
        crc32: u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]),
        input_size: u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]),
    })
}

fn skip_nul_terminated_field(
    input: &[u8],
    offset: usize,
    trailer_offset: usize,
) -> Result<usize, DecodeError> {
    let field = input
        .get(offset..trailer_offset)
        .ok_or(DecodeError::TruncatedGzip)?;
    let nul_offset = field
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(DecodeError::TruncatedGzip)?;
    offset
        .checked_add(nul_offset + 1)
        .ok_or(DecodeError::TruncatedGzip)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use std::io::Write;

    use flate2::{
        Compression,
        write::{GzEncoder, ZlibEncoder},
    };

    use super::{DecodeError, decompress_gzip, decompress_zlib};

    #[test]
    fn decompresses_gzip_payload() {
        let payload = b"hello gzip";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();

        assert_eq!(decompress_gzip(&compressed).unwrap(), payload);
    }

    #[test]
    fn rejects_gzip_with_bad_trailer() {
        let payload = b"hello gzip";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let mut compressed = encoder.finish().unwrap();
        let trailer_byte = compressed.last_mut().unwrap();
        *trailer_byte = trailer_byte.wrapping_add(1);

        assert_eq!(
            decompress_gzip(&compressed),
            Err(DecodeError::InvalidGzipTrailer)
        );
    }

    #[test]
    fn decompresses_zlib_payload() {
        let payload = b"hello zlib";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();

        assert_eq!(decompress_zlib(&compressed).unwrap(), payload);
    }
}
