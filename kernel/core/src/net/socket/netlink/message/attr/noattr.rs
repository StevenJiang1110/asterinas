// SPDX-License-Identifier: MPL-2.0

use super::{Attribute, CAttrHeader, ParsedAttrs};
use crate::{net::socket::netlink::message::ContinueRead, prelude::*, util::MultiRead};

/// A special type indicates that a segment cannot have attributes.
#[derive(Debug)]
pub(crate) enum NoAttr {}

impl Attribute for NoAttr {
    type Type = ();

    fn type_from_raw(_type_: u16) -> Option<Self::Type> {
        None
    }

    fn type_(&self) -> u16 {
        match *self {}
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match *self {}
    }

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<ContinueRead<Self>>
    where
        Self: Sized,
    {
        let payload_len = header.payload_len();
        reader.skip_some(payload_len);

        Ok(ContinueRead::Skipped)
    }

    fn read_all_from(
        reader: &mut dyn MultiRead,
        total_len: usize,
        _strict_check: bool,
        _dump_all: bool,
    ) -> Result<ContinueRead<ParsedAttrs<Self>>>
    where
        Self: Sized,
    {
        reader.skip_some(total_len);

        Ok(ContinueRead::Parsed(ParsedAttrs {
            attrs: Vec::new(),
            seen_types: Vec::new(),
        }))
    }
}
