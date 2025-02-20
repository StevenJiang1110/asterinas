// SPDX-License-Identifier: MPL-2.0

pub struct UnboundNetlinkRoute {
    _private: (),
}

impl UnboundNetlinkRoute {
    pub const fn new() -> Self {
        Self { _private: () }
    }
}
