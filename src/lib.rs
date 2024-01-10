// region:    --- Modules

pub type Result<T> = core::result::Result<T, Error>;

pub type Error = Box<dyn std::error::Error>; // For early dev.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// endregion: --- Modules

/*
Packet structure:
    Preamble (Limit to 72 bytes):
        - 0xAA | filename | filesize | chunk_id | chunksize | number_of_chunks_expected | segment_id | number of segments_expected |
    Data (Limit to MTU - IP/UDP header (28 bytes) - Preamble) -- target (9000 - 30 - 72 = 8898 ~ 8500):
        - 0xAA | data | 0xAA
*/

// -- Template for preamble info (72 bytes)
#[derive(Debug, Deserialize, Serialize)]
pub struct Preamble {
    // -- Max filename size limit to 16 bytes for now
    // -- TODO: Change to uuid format
    pub filename: String,
    // -- 8 bytes
    pub filesize: u64,
    // -- 8 bytes
    pub chunk_id: u64,
    // -- 8 bytes
    pub chunksize: u64,
    // -- 8 bytes
    pub number_of_chunks_expected: u64,
    // -- 8 bytes
    pub segment_id: u64,
    // -- 8 bytes
    pub number_of_segments_expected: u64,
    // -- 8 bytes
}

// -- Template for Packet (Maximum 9000)
#[derive(Debug, Deserialize, Serialize)]
pub struct Packet {
    pub pre_padding: u8,
    pub preamble: Preamble,
    pub mid_padding: u8,
    pub data: Vec<u8>,
    pub post_padding: u8,
}

// -- Encoding Configuration Variables
pub struct EncoderConfig {
    pub max_source_symbol_size: usize,
    pub number_of_repair_symbols: usize,
}
