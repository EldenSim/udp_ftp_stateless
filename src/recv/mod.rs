#![allow(non_snake_case)]

use std::net::UdpSocket;
// use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::{env, sync::Arc};

use async_std::sync::Mutex;
use async_std::task;
use udp_ftp_stateless::{Packet, Result};
mod models;
use models::FileDetails;

use self::models::ChunkSegmentData;

pub async fn main(udp_service: UdpSocket) -> Result<()> {
    // -- Obtain MTU and raptor decoding settings, must be the same as sender settings
    let NUMBER_OF_REPAIR_SYMBOLS = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<usize>()?;
    let MAX_SOURCE_SYMBOL_SIZE = env::var("MAX_SOURCE_SYMBOL_SIZE")
        .expect("MAX_SOURCE_SYMBOL_SIZE env var not set")
        .parse::<usize>()?;
    let MTU = env::var("MTU")
        .expect("MTU env var not set")
        .parse::<usize>()?;
    let PROCESSING_STORAGE = env::var("PROCESSING_STORAGE")
        .expect("PROCESSING_STORAGE env var not set")
        .parse::<usize>()?;

    let file_details_storage: Arc<Mutex<Box<Vec<FileDetails>>>> =
        Arc::new(Mutex::new(Box::new(Vec::new())));

    let received_packets: Arc<Mutex<Box<Vec<Vec<u8>>>>> =
        Arc::new(Mutex::new(Box::new(Vec::new())));

    let pointer_received_packets = Arc::clone(&received_packets);
    // thread::Builder::new()
    //     .name("Receiving Thread".to_string())
    //     .spawn(move || {
    //         recv_packets(udp_service, pointer_received_packets).await;
    //     })?;
    let pointer_file_details_storage = Arc::clone(&file_details_storage);
    let pointer_2_received_packets = Arc::clone(&received_packets);
    task::Builder::new()
        .name("Processing Task".to_string())
        .spawn(async move {
            processing_packets(pointer_2_received_packets, pointer_file_details_storage).await
        })?;
    task::Builder::new()
        .name("Receiving Task".to_string())
        .blocking(async move {
            recv_packets(udp_service, pointer_received_packets)
                .await
                .unwrap()
        });
    // recv_packets(udp_service, pointer_received_packets).await?;

    // thread::Builder::new()
    //     .name("Processing Thread".to_string())
    //     .spawn(move || {
    //         processing_packets(pointer_2_received_packets, pointer_file_details_storage).await;
    //     })?;

    Ok(())
}

async fn processing_packets(
    received_packets: Arc<Mutex<Box<Vec<Vec<u8>>>>>,
    file_details_storage: Arc<Mutex<Box<Vec<FileDetails>>>>,
) {
    while true {
        let mut packets_to_parses = received_packets.lock().await;
        let mut file_details_storage_lock = file_details_storage.lock().await;

        while packets_to_parses.len() > 0 {
            let packet_bytes = &packets_to_parses[0];
            let packet: Packet = bincode::deserialize(&packet_bytes).unwrap();
            // Packet details
            let filename = packet.preamble.filename;
            let chunk_id = packet.preamble.chunk_id as usize;
            let number_of_chunks_expected = packet.preamble.number_of_chunks_expected as usize;
            let segment_id = packet.preamble.segment_id as usize;
            if file_details_storage_lock
                .iter()
                .find(|fd| fd.filename == filename)
                .is_none()
            {
                let file_details = FileDetails {
                    filename: filename.clone(),
                    chunksize: packet.preamble.chunksize,
                    num_segments_recv_per_chunk: vec![0; number_of_chunks_expected],
                    segment_data_recv_per_chunk: Vec::new(),
                    chunk_decoding_status: vec!["Undecoded".to_string(); number_of_chunks_expected],
                    file_merging_status: false,
                };
                file_details_storage_lock.push(file_details);
            }
            // Update file_details
            let file_details = file_details_storage_lock
                .iter_mut()
                .find(|fd| fd.filename == filename.clone())
                .unwrap();
            // Update segment counter in num_segment_recv_per_chunk
            let num_segments_recv_per_chunk_vec = &mut file_details.num_segments_recv_per_chunk;
            num_segments_recv_per_chunk_vec[chunk_id] += 1;

            // Update segment_data_recv_per_chunk
            let chunk_segment_name = format!("c_{}_s_{}", chunk_id, segment_id);
            let segment_data_recv_per_chunk_vec = &mut file_details.segment_data_recv_per_chunk;
            segment_data_recv_per_chunk_vec.push(ChunkSegmentData {
                chunk_segment_name,
                data: packet.data,
            });
            packets_to_parses.swap_remove(0);
        }
        println!(
            "->> File details check: {:?}",
            file_details_storage_lock[0].num_segments_recv_per_chunk
        );
        // let pointer_file_details_storage = Arc::clone(&file_details_storage);
        // task::spawn(async move { decode_chunks(file_details_storage_lock) });
    }
}

// fn decode_chunks(file_details_storage: Arc<Mutex<Box<Vec<FileDetails>>>>) {
//     let file_details_storage_lock = file_details_storage.lock().unwrap();
//     let segments_to_decode = file_details_storage_lock.iter().find(|fd| fd)
//     // Initialise decoded configuration
//     let mut decoder = raptor_code::SourceBlockDecoder::new(max_source_symbol_size);
//     // Loop through each segment and push to decoder
//     for (i, segment) in segments_to_decode.iter().enumerate() {
//         let esi = segments_name_to_decode[i]
//             .split("_")
//             .collect::<Vec<_>>()
//             .get(3)
//             .unwrap()
//             .parse::<u32>()
//             .unwrap();
//         decoder.push_encoding_symbol(segment, esi);
//     }
//     // If decoder not fully_specified, not enough segments received
//     if !decoder.fully_specified() {
//         return false;
//     }
//     // Reconstruct the chunk
//     let reconstructed_chunk = decoder
//         .decode(chunksize)
//         .ok_or("Unable to decode message")
//         .unwrap();

//     // Write to file in temp folder
//     let chunk_file_path = format!("./temp/{}_{}.txt", filename, chunk_id);
//     // Check if path already exist
//     if Path::new(&chunk_file_path).exists().await
//         && fs::metadata(&chunk_file_path).await.unwrap().size() == reconstructed_chunk.len() as u64
//     {
//         return true;
//     }
//     fs::write(chunk_file_path, reconstructed_chunk)
//         .await
//         .unwrap();
//     true
// }

async fn recv_packets(
    udp_service: UdpSocket,
    received_packets: Arc<Mutex<Box<Vec<Vec<u8>>>>>,
) -> Result<()> {
    loop {
        let mut packets_storage = received_packets.lock().await;
        // -- Receive packets and store them first
        let mut buffer = vec![0; 1522];
        match udp_service.recv_from(&mut buffer) {
            Ok((bytes, _)) => {
                let recv_data = &buffer[..bytes];

                packets_storage.push(recv_data.to_vec());
            }
            Err(e) => return Err(e.into()),
        }
    }
}
