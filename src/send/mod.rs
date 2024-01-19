#![allow(non_snake_case)]
// region:    --- Modules

use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Duration;
use std::{env, fs, thread};

use udp_ftp_stateless::Result;
use udp_ftp_stateless::{EncoderConfig, Packet, Preamble};

// endregion: --- Modules

pub fn main(udp_service: &UdpSocket) -> Result<()> {
    // region:    --- Loop through sending folder
    let SENDING_DIRECTORY =
        env::var("SENDING_DIRECTORY").expect("SENDING_DIRECTORY env var not set");
    let NUMBER_OF_REPAIR_SYMBOLS = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<usize>()?;
    let MAX_SOURCE_SYMBOL_SIZE = env::var("MAX_SOURCE_SYMBOL_SIZE")
        .expect("MAX_SOURCE_SYMBOL_SIZE env var not set")
        .parse::<usize>()?;

    let DELAY_PER_CHUNK = env::var("DELAY_PER_CHUNK")
        .expect("DELAY_PER_CHUNK env var not set")
        .parse::<u64>()?;
    let DELAY_PER_FILE = env::var("DELAY_PER_FILE")
        .expect("DELAY_PER_FILE env var not set")
        .parse::<u64>()?;

    let PROCESSING_STORAGE = env::var("PROCESSING_STORAGE")
        .expect("PROCESSING_STORAGE env var not set")
        .parse::<usize>()?;

    let chunksize = calculate_chunksize_and_data_size(MAX_SOURCE_SYMBOL_SIZE)?;

    let encoder_config = EncoderConfig {
        max_source_symbol_size: MAX_SOURCE_SYMBOL_SIZE,
        number_of_repair_symbols: NUMBER_OF_REPAIR_SYMBOLS,
    };
    for entry in fs::read_dir(SENDING_DIRECTORY)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            unimplemented!()
        } else {
            send_file_with_raptor(
                &path,
                udp_service,
                chunksize,
                &encoder_config,
                DELAY_PER_CHUNK,
                PROCESSING_STORAGE,
            )?;
        }
        thread::sleep(Duration::from_millis(DELAY_PER_FILE))
    }

    // endregion: --- Loop through sending folder
    Ok(())
}

/// This function will breaks up the file into chunks to be encoded using Raptor Codes FEC
/// After each encoding, chunks will be split into segment which will be repackage into packets
/// with necessary info for receiver to decode.
/// After all data has been sent it will also send empty packets to fill up the min packet processing buffer
/// on the receiver side.
fn send_file_with_raptor(
    path: &PathBuf,
    udp_service: &UdpSocket,
    chunksize: usize,
    encoder_config: &EncoderConfig,
    DELAY_PER_CHUNK: u64,
    PROCESSING_STORAGE: usize,
) -> Result<()> {
    // -- Obtain Filename and Filesize for preamble
    let filename = path.file_name().unwrap().to_str().unwrap().to_string();
    println!("->> Sending file: {}", filename);
    let file = fs::read(path)?;
    let filesize = file.len();

    // -- Obtain chunksize and max data size variable from MTU and max chunksize limits
    let chunks: Vec<_> = file.chunks(chunksize).collect();
    let total_number_of_chunks = chunks.len();
    println!("->> Number of chunks: {}", total_number_of_chunks);
    let mut total_num_of_symbols = 0;
    for (chunk_id, chunk) in chunks.iter().enumerate() {
        let chunksize = chunk.len();
        let mut encoder =
            raptor_code::SourceBlockEncoder::new(chunk, encoder_config.max_source_symbol_size);
        total_num_of_symbols =
            encoder.nb_source_symbols() as usize + encoder_config.number_of_repair_symbols;
        for segment_id in 0..total_num_of_symbols {
            // -- Obtain encoding_symbols
            let encoding_symbol = encoder.fountain(segment_id as u32);
            // -- Create Preamble with info
            let preamble = Preamble {
                filename: filename.clone(),
                filesize: filesize as u64,
                chunk_id: chunk_id as u64,
                chunksize: chunksize as u64,
                number_of_chunks_expected: total_number_of_chunks as u64,
                segment_id: segment_id as u64,
                number_of_segments_expected: total_num_of_symbols as u64,
            };

            // -- Create Packet with 170 Padding bytes
            let packet = Packet {
                pre_padding: 170,
                preamble,
                mid_padding: 170,
                data: encoding_symbol,
                post_padding: 170,
            };

            // -- Serialise Struct to send in bytes
            let packet_bytes = bincode::serialize(&packet)?;
            // -- Send Packet
            udp_service.send(&packet_bytes).expect("Send packet error");
        }

        thread::sleep(Duration::from_millis(DELAY_PER_CHUNK))
    }
    // -- Can try changing to send empty only at the end of all files
    // -- Now is sending after every file
    let number_of_padding_packets =
        PROCESSING_STORAGE - (total_number_of_chunks * total_num_of_symbols) % PROCESSING_STORAGE;
    let empty_preamble = Preamble {
        filename: "".to_string(),
        filesize: 0,
        chunk_id: 0,
        chunksize: 0,
        number_of_chunks_expected: 0,
        segment_id: 0,
        number_of_segments_expected: 0,
    };
    let empty_packet = Packet {
        pre_padding: 0,
        preamble: empty_preamble,
        mid_padding: 0,
        data: vec![0],
        post_padding: 0,
    };
    let empty_packet_bytes = bincode::serialize(&empty_packet)?;
    for _ in 0..number_of_padding_packets + 1 {
        udp_service
            .send(&empty_packet_bytes)
            .expect("Send empty packet error");
    }

    Ok(())
}

/// This function calculates the chunksize to split the file
/// This is needed as size of packet is determine by encoded symbols
/// and packet size is ultimately limited by MTU.
fn calculate_chunksize_and_data_size(max_source_symbol_size: usize) -> Result<usize> {
    // -- MTU limit will determine the size of packet
    let mtu_limit = env::var("MTU")
        .expect("MTU env var not set")
        .parse::<usize>()?;

    // -- Max data size is calculate by deducting preamble (72 bytes), IP/ UDP Headers (28 bytes) and buffer (10 bytes).
    // let max_data_size = mtu_limit - 72 - 28 - 10;
    let MAX_CHUNKSIZE = env::var("MAX_CHUNKSIZE")
        .expect("MAX_CHUNKSIZE env var not set")
        .parse::<usize>()?;
    let chunksize = if (mtu_limit * max_source_symbol_size) > MAX_CHUNKSIZE {
        MAX_CHUNKSIZE
    } else {
        mtu_limit * max_source_symbol_size
    };
    Ok(chunksize)
}
