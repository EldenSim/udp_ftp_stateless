#![allow(non_snake_case)]

use std::future::IntoFuture;
use std::net::UdpSocket;
// use std::sync::Mutex;
use std::os::unix::fs::MetadataExt;
use std::thread;
use std::time::Duration;
use std::{env, sync::Arc};

// use async_std::fs;
use async_std::fs::OpenOptions;
use async_std::io;
use async_std::path::Path;
use async_std::sync::Mutex;
// use std::sync::Mutex as stdMutex;
use async_std::task;
use std::fs;
use udp_ftp_stateless::{Packet, Result};
mod models;
use models::FileDetails;

use self::models::SegmentData;

pub async fn main(udp_service: UdpSocket) -> Result<()> {
    // -- Obtain MTU and raptor decoding settings, must be the same as sender settings
    let NUMBER_OF_REPAIR_SYMBOLS = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<u64>()?;
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
            processing_packets(
                pointer_2_received_packets,
                pointer_file_details_storage,
                NUMBER_OF_REPAIR_SYMBOLS,
                MAX_SOURCE_SYMBOL_SIZE,
            )
            .await
        })?;

    task::Builder::new()
        .name("Receiving Task".to_string())
        .blocking(async move {
            recv_packets(&udp_service, pointer_received_packets)
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
    NUMBER_OF_REPAIR_SYMBOLS: u64,
    MAX_SOURCE_SYMBOL_SIZE: usize,
) {
    while true {
        let mut packets_to_parses = received_packets.lock().await;
        let mut file_details_storage_lock = file_details_storage.lock().await;
        // let mut filename = String::new();
        // let mut chunk_id = 0;
        let mut decode_flag = false;

        let mut segments_to_decode = Vec::new();

        let mut chunksize = 0;

        let packet_bytes = match packets_to_parses.first() {
            Some(bytes) => bytes,
            None => continue,
        };

        let packet: Packet = bincode::deserialize(&packet_bytes).unwrap();
        // Packet details
        let filename = packet.preamble.filename;
        let chunk_id = packet.preamble.chunk_id as usize;
        let number_of_chunks_expected = packet.preamble.number_of_chunks_expected as usize;
        let segment_id = packet.preamble.segment_id as usize;
        let number_of_segments_expected = packet.preamble.number_of_segments_expected;
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
        println!("->> Chunk ID: {}", chunk_id);
        let segment_data_recv_per_chunk_vec = &mut file_details.segment_data_recv_per_chunk;
        segment_data_recv_per_chunk_vec.push(SegmentData {
            chunk_id: chunk_id,
            segments: vec![vec![0]; number_of_segments_expected as usize],
        });
        segment_data_recv_per_chunk_vec
            .iter_mut()
            .find(|sd| sd.chunk_id == chunk_id)
            .unwrap()
            .segments
            .insert(segment_id, packet.data);
        packets_to_parses.swap_remove(0);
        println!(
            "num segment check: {}",
            num_segments_recv_per_chunk_vec[chunk_id]
        );
        if num_segments_recv_per_chunk_vec[chunk_id]
            > (number_of_segments_expected - NUMBER_OF_REPAIR_SYMBOLS)
        {
            if file_details
                .chunk_decoding_status
                .get(chunk_id)
                .is_some_and(|x| *x == "Undecoded".to_string())
            {
                *file_details
                    .chunk_decoding_status
                    .get_mut(chunk_id)
                    .unwrap() = "Decoding".to_string();
                decode_flag = true;
                chunksize = packet.preamble.chunksize as usize;
                segments_to_decode = file_details
                    .segment_data_recv_per_chunk
                    .iter()
                    .find(|sd| sd.chunk_id == chunk_id)
                    .unwrap()
                    .segments
                    .clone();
                // for i in 0..num_segments_recv_per_chunk_vec[chunk_id] {
                //     let chunk_segment_name = format!("c_{}_s_{}", chunk_id, i);
                //     let segment_option = file_details
                //         .segment_data_recv_per_chunk
                //         .iter()
                //         .find(|csd| csd.chunk_segment_name == chunk_segment_name);
                //     match segment_option {
                //         Some(segment) => {
                //             segments_to_decode.push(segment.data.to_owned());
                //             segment_names_to_decode.push(chunk_segment_name);
                //             // enough_segment = true;
                //         }
                //         None => {
                //             continue;
                //         }
                //     }
                // }
            }
        }
        println!(
            "->> File details check: {:?}",
            file_details_storage_lock[0].num_segments_recv_per_chunk
        );

        if decode_flag {
            println!("Decoding: {}", chunk_id);
            let pointer_file_details_storage = Arc::clone(&file_details_storage);
            let filename_2 = filename.clone();
            task::spawn(async move {
                let (completed, decoded_chunk) = decode_chunks(
                    // segment_names_to_decode,
                    // segments_to_decode,
                    segments_to_decode,
                    chunksize,
                    MAX_SOURCE_SYMBOL_SIZE,
                );
                if completed {
                    let mut pointer_file_details_storage_lock =
                        pointer_file_details_storage.lock().await;
                    let file_details = pointer_file_details_storage_lock
                        .iter_mut()
                        .find(|fd| fd.filename == filename)
                        .unwrap();
                    let chunk_decoding_status = &mut file_details.chunk_decoding_status;
                    *chunk_decoding_status.get_mut(chunk_id).unwrap() = "Decoded".to_string();
                    // Write to file in temp folder
                    let chunk_file_path = format!("./temp/{}_{}.txt", filename.clone(), chunk_id);
                    // Check if path already exist
                    // if !Path::new(&chunk_file_path).exists().await
                    //     || fs::metadata(&chunk_file_path).await.unwrap().size()
                    //         == decoded_chunk.as_ref().unwrap().len() as u64
                    // {}
                    thread::spawn(move || {
                        fs::write(chunk_file_path, decoded_chunk.unwrap()).unwrap();
                    })
                    .join()
                    .unwrap();
                    // fs::write(chunk_file_path, decoded_chunk.unwrap()).unwrap();
                }
                // let pointer_2_file_details_storage = Arc::clone(&file_details_storage);
                let mut pointer_2_file_details_storage_lock =
                    pointer_file_details_storage.lock().await;
                let file_details = pointer_2_file_details_storage_lock
                    .iter_mut()
                    .find(|fd| fd.filename == filename_2)
                    .unwrap();
                if file_details
                    .chunk_decoding_status
                    .iter()
                    .filter(|x| **x == "Decoded".to_string())
                    .count()
                    == file_details.chunk_decoding_status.len()
                    && file_details.file_merging_status == false
                {
                    file_details.file_merging_status = true;
                    // let pointer_3_file_details_storage = Arc::clone(&file_details_storage);
                    let num_of_chunks = file_details.chunk_decoding_status.len();
                    task::spawn(async move {
                        merge_temp_files(filename_2, num_of_chunks).await;
                    });
                }
            });
        }
    }

    // let pointer_file_details_storage = Arc::clone(&file_details_storage);
}
// }

async fn merge_temp_files(filename: String, number_of_chunks: usize) {
    let temp_path = "./temp";
    let temp_final_file_path = format!("./receiving_dir/{}", filename);
    for i in 0..number_of_chunks {
        let file_2_path = format!("./temp/{}_{}.txt", filename, i);
        let mut file_1 = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&temp_final_file_path)
            .await
            .unwrap();
        let mut file_2 = match OpenOptions::new().read(true).open(&file_2_path).await {
            Ok(file) => file,
            Err(_) => return,
        };

        io::copy(&mut file_2, &mut file_1).await.unwrap();
        // Remove all temp files used to clean up
        // let _ = fs::remove_file(&file_2_path);
    }
}

fn decode_chunks(
    // segment_names: Vec<String>,
    segments: Vec<Vec<u8>>,
    // segments: Vec<ChunkSegmentData>,
    chunksize: usize,
    max_source_symbol_size: usize,
) -> (bool, Option<Vec<u8>>) {
    let mut decoder = raptor_code::SourceBlockDecoder::new(max_source_symbol_size);
    // Loop through each segment and push to decoder
    // for (i, segment) in segments.iter().enumerate() {
    println!("->> number of segments: {}", segments.len());
    for i in 0..segments.len() {
        // let esi = segment_names[i]
        //     .split("_")
        //     .collect::<Vec<_>>()
        //     .get(3)
        //     .unwrap()
        //     .parse::<u32>()
        //     .unwrap();
        // println!("->> csname check: {}", segment.chunk_segment_name);

        decoder.push_encoding_symbol(&segments[i], i as u32);
        if decoder.fully_specified() {
            break;
        }
    }
    // If decoder not fully_specified, not enough segments received
    if !decoder.fully_specified() {
        return (false, None);
    }
    // Reconstruct the chunk
    let reconstructed_chunk = decoder
        .decode(chunksize)
        .ok_or("Unable to decode message")
        .unwrap();

    (true, Some(reconstructed_chunk))
}

async fn recv_packets(
    udp_service: &UdpSocket,
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
