//! `Host` (the old trait) on the new hosts.
use super::*;

/// What a hook was handed by the new API: the server's host or the client's.
pub enum Adapter<'a> {
    Server(&'a mut dyn ServerHost),
    Client(&'a mut dyn ClientHost),
}

impl Host for Adapter<'_> {
    fn new_host(&self) -> &dyn mod_api::Host {
        match self {
            Adapter::Server(host) => &**host,
            Adapter::Client(host) => &**host,
        }
    }
    fn new_server(&mut self) -> &mut dyn ServerHost {
        match self {
            Adapter::Server(host) => &mut **host,
            Adapter::Client(_) => panic!("compat: a server-only call on the client"),
        }
    }
}
