//! Which peer addresses the host talks to at all. Shared with the phone,
//! which applies the same rule to pairing QR codes; see
//! [`lanlink_auth::netpolicy`].

pub use lanlink_auth::netpolicy::is_local_address;
