// region:    --- Optimise Struct test

#[derive(Debug)]
pub struct FileDetails {
    pub filename: String,
    pub chunksize: u64,
    pub num_segments_recv_per_chunk: Vec<u64>,
    pub segment_data_recv_per_chunk: Vec<SegmentData>,
    pub chunk_decoding_status: Vec<String>,
    pub file_merging_status: bool,
}

pub struct FileChunkSegment {
    pub filename: String,
    pub chunks_segments: Vec<u64>,
}

pub struct FileData {
    pub filename: String,
    pub chunk_segment_data: Vec<SegmentData>,
}

#[derive(Debug, Clone)]
pub struct SegmentData {
    pub chunk_id: usize,
    pub segments: Vec<Vec<u8>>,
}

pub struct DecodingStatus {
    pub filename: String,
    pub chunks_status: Vec<String>,
}

#[derive(Debug)]
pub struct MergingStatus {
    pub filename: String,
    pub chunksize: u64,
    pub expected_chunks: u64,
    pub status: bool,
}

// endregion: --- Optimise Struct test
