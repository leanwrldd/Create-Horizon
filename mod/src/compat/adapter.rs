//! `Host` (the old trait) on the new hosts.
use super::*;

/// What a hook was handed by the new API: the server's host, the client's, or the shared base of both.
pub enum Adapter<'a> {
    Server(&'a mut dyn ServerHost),
    Client(&'a mut dyn ClientHost),
}

impl Host for Adapter<'_> {}
