// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// A numeric port or a service name interpreted by the resolver's service database. Named services can resolve differently for different transports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressService<'a> {
    /// A numeric port.
    Port(u16),
    /// A service name, such as `"http"`, looked up in the resolver's service database.
    Name(&'a str),
}
