// region:    --- Modules

use std::net::UdpSocket;
use std::path::PathBuf;
use std::str::FromStr;
use std::{env, fs};

use udp_ftp_stateless::Result;
use udp_ftp_stateless::{EncoderConfig, Packet, Preamble};

use uuid::{uuid, Uuid};

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
            send_file_with_raptor(&path, udp_service, chunksize, &encoder_config)?;
        }
    }

    // endregion: --- Loop through sending folder
    Ok(())
}

fn send_file_with_raptor(
    path: &PathBuf,
    udp_service: &UdpSocket,
    chunksize: usize,
    encoder_config: &EncoderConfig,
) -> Result<()> {
    // -- Obtain Filename and Filesize for preamble
    let filename = path.file_name().unwrap().to_str().unwrap().to_string();
    let file = fs::read(path)?;
    let filesize = file.len();
    // -- Obtain chunksize and max data size variable from MTU and max chunksize limits

    let chunks: Vec<_> = file.chunks(chunksize).collect();
    let total_number_of_chunks = chunks.len();
    for (chunk_id, chunk) in chunks.iter().enumerate() {
        // println!("->> Chunk id: {}", chunk_id);

        let mut encoder =
            raptor_code::SourceBlockEncoder::new(chunk, encoder_config.max_source_symbol_size);
        let total_num_of_symbols =
            encoder.nb_source_symbols() as usize + encoder_config.number_of_repair_symbols;

        for segment_id in 0..total_num_of_symbols {
            // println!("\t->> Segmnet id: {}", segment_id);
            // -- Obtain encoding_symbols
            let encoding_symbol = encoder.fountain(segment_id as u32);
            // -- Create Preamble with info
            let preamble = Preamble {
                filename: filename.clone(),
                filesize: filesize as u64,
                chunk_id: chunk_id as u64,
                number_of_chunks_expected: total_number_of_chunks as u64,
                segment_id: segment_id as u64,
                number_of_segments_expected: total_num_of_symbols as u64,
            };
            // println!("->> Preamble: {:#?}", preamble);
            // -- Create Packet with 170 Padding bytes
            let packet = Packet {
                pre_padding: 170,
                preamble,
                mid_padding: 170,
                data: encoding_symbol,
                post_padding: 170,
            };
            // println!("->> Packet: {:?}", packet);
            // -- Serialise Struct to send in bytes
            let packet_bytes = bincode::serialize(&packet)?;
            // -- Send Packet
            udp_service.send(&packet_bytes).expect("Send packet error");
        }
    }

    Ok(())
}

fn calculate_chunksize_and_data_size(max_source_symbol_size: usize) -> Result<usize> {
    // -- MTU limit will determine the size of packet
    let mtu_limit = env::var("MTU")
        .expect("MTU env var not set")
        .parse::<usize>()?;

    // -- Max data size is calculate by deducting preamble (64 bytes), IP/ UDP Headers (28 bytes) and buffer (10 bytes).
    let max_data_size = mtu_limit - 64 - 28 - 10;
    let MAX_CHUNKSIZE = env::var("MAX_CHUNKSIZE")
        .expect("MAX_CHUNKSIZE env var not set")
        .parse::<usize>()?;
    let chunksize = if (mtu_limit * max_source_symbol_size) > MAX_CHUNKSIZE {
        MAX_CHUNKSIZE
    } else {
        mtu_limit * max_source_symbol_size
    };
    // -- Note usize conversion may have error in future
    Ok(chunksize)
}
