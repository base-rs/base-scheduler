// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// An Explicit Congestion Notification codepoint, the two ECN bits of a datagram's IP TOS byte or IPv6 traffic class ([RFC 3168 §5](https://www.rfc-editor.org/rfc/rfc3168.html#section-5)).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Ecn {
    /// Not ECN-capable transport, `00`.
    NotEct,
    /// ECN-capable transport ECT(1), `01`.
    Ect1,
    /// ECN-capable transport ECT(0), `10`.
    Ect0,
    /// Congestion experienced, `11`.
    Ce,
}

impl Ecn {
    /// The codepoint in the two low bits of `byte`, an IP TOS byte or IPv6 traffic class. The DSCP bits above them are ignored.
    pub const fn from_bits(byte: u8) -> Self {
        match byte & 0b11 {
            0b00 => Self::NotEct,
            0b01 => Self::Ect1,
            0b10 => Self::Ect0,
            _ => Self::Ce,
        }
    }

    /// The codepoint's two bits, in the low bits of the returned byte.
    pub const fn bits(self) -> u8 {
        match self {
            Self::NotEct => 0b00,
            Self::Ect1 => 0b01,
            Self::Ect0 => 0b10,
            Self::Ce => 0b11,
        }
    }
}
