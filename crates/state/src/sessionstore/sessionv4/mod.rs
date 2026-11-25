//! Read-only 1.x sessions-v4 format.

mod content;
mod error;
mod frames;
pub(crate) mod fs;
mod message;
mod session;
mod timestamp;

pub use content::{ContentPool, ContentRef, MAX_OBJECT_BYTES};
pub use error::{Damage, Limit, Unsupported, V4Error};
pub use frames::read_frame;
pub use session::{
    CATALOG_VERSION, Listing, ListingCache, Manifest, Session, Transcript, list, open, open_in,
};

pub const CODEC: &str = "reasonix.session.linear/v4";
pub const SCHEMA_VERSION: i64 = 4;
pub const MAX_FRAME_BYTES: usize = 8 << 20;
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
