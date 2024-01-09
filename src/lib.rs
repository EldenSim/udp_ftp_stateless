// region:    --- Modules

pub type Result<T> = core::result::Result<T, Error>;

pub type Error = Box<dyn std::error::Error>; // For early dev.

use uuid::Uuid;

// endregion: --- Modules

/*
Packet structure:
    Preamble (Limit to 64 bytes):
        - 0xAA | Filname | filesize | chunk_id | number_of_chunks_expected | segment_id | number of segments_expected |
    Data (Limit to MTU - IP/UDP header (28 bytes) - Preamble) -- target (9000 - 30 - 64 = 8506 ~ 8500):
        - 0xAA | data | 0xAA
*/

// -- Template for preamble info (64 bytes)
pub struct Preamble {
    // -- Max filename size limit to 16 bytes for now
    // -- TODO: Change to uuid format
    pub filename: Uuid,
    // -- 8 bytes
    pub filesize: u64,
    // -- 8 bytes
    pub chunk_id: u64,
    // -- 8 bytes
    pub number_of_chunks_expected: u64,
    // -- 8 bytes
    pub segment_id: u64,
    // -- 8 bytes
    pub number_of_segments_expected: u64,
    // -- 8 bytes
}

// -- Template for Packet (Maximum 9000)
pub struct Packet {
    pub pre_padding: u8,
    pub preamble: Preamble,
    pub mid_padding: u8,
    pub data: Vec<u8>,
    pub post_padding: u8,
}
