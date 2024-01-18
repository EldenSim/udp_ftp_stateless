#![allow(non_snake_case)]
// region:    --- Modules

use std::collections::HashMap;
use std::env;
use std::net::UdpSocket;
use std::os::unix::fs::MetadataExt;

use async_std::fs::OpenOptions;
use async_std::io::{self};
use async_std::path::Path;
use async_std::sync::{Arc, Mutex};
use async_std::{fs, task};

use udp_ftp_stateless::Packet;
use udp_ftp_stateless::Result;
mod models;
use models::{ChunkSegmentData, DecodingStatus, FileChunkSegment, FileData, MergingStatus};

// endregion: --- Modules

/// This function handles the receiving and parsing of incomming packets
pub async fn main(udp_service: &UdpSocket) -> Result<()> {
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

    // -- Initialise the storage vectors
    // Change from HashMap to Vec as Hashmap requires hashing (slow)
    // -- See: https://github.com/rust-lang/rust/issues/73307
    // file_chunk_segment_storage: Keeps count of the number segments received for each chunk per file
    let file_chunk_segment_storage: Arc<Mutex<Box<Vec<FileChunkSegment>>>> =
        Arc::new(Mutex::new(Box::default()));

    // file_data_storage: Keeps the segment data only, received for each chunk and segment per file
    let file_data_storage: Arc<Mutex<Box<Vec<FileData>>>> = Arc::new(Mutex::new(Box::default()));

    // decoding_status_storage: Keeps track of the decoding status of each chunk per file (Undecoded, Decoding, Decoded)
    let decoding_status_storage: Arc<Mutex<Box<Vec<DecodingStatus>>>> =
        Arc::new(Mutex::new(Box::default()));

    // merging_status_storage: Keeps tracks of the merging status of each file
    let merging_status_storage: Arc<Mutex<Box<Vec<MergingStatus>>>> =
        Arc::new(Mutex::new(Box::default()));

    let files_to_be_merged: Arc<Mutex<Box<Vec<String>>>> = Arc::new(Mutex::new(Box::default()));

    let mut storage = Vec::new();
    loop {
        // -- Receive packets and store them first
        let mut buffer = vec![0; MTU];
        match udp_service.recv_from(&mut buffer) {
            Ok((bytes, _)) => {
                let recv_data = &buffer[..bytes];

                storage.push(recv_data.to_vec());
            }
            Err(e) => return Err(e.into()),
        }

        // -- Can implement a minimum amount of packets to receive before processing, amount will impact performace
        // -- 64 and 256 tested to work well
        if storage.len() < 64 {
            continue;
        }
        // -- Obtain the packets in the temp storage and move them as will be used in spawed task
        let processing_storage: Vec<_> = storage.drain(..storage.len()).collect();

        // -- Clone the storages as will be sent to different task which require ownership.
        // -- By using Arc this cloning is just cloning the pointer to where the data is on the heap
        // -- https://doc.rust-lang.org/std/sync/struct.Arc.html
        let pointer_f_c_s_storage = Arc::clone(&file_chunk_segment_storage);
        let pointer_file_data_storage = Arc::clone(&file_data_storage);
        let pointer_decoding_status_storage = Arc::clone(&decoding_status_storage);
        let pointer_merging_status_storage = Arc::clone(&merging_status_storage);
        let pointer_files_to_be_merged_vec = Arc::clone(&files_to_be_merged);

        // -- Spawn a task to handle this batch of received packets
        // -- Task definition from async_std:
        //      An executing asynchronous Rust program consists of a collection of native OS threads,
        //      on top of which multiple stackless coroutines are multiplexed.
        task::spawn(async move {
            parsing_packets(
                processing_storage,
                pointer_f_c_s_storage,
                pointer_file_data_storage,
                pointer_decoding_status_storage,
                pointer_merging_status_storage,
                pointer_files_to_be_merged_vec,
                NUMBER_OF_REPAIR_SYMBOLS as u64,
                MAX_SOURCE_SYMBOL_SIZE,
            )
            .await;
        });

        // -- Clone the storage again to support the next spawned task
        let pointer_2_f_c_s_storage = Arc::clone(&file_chunk_segment_storage);
        let pointer_2_file_data_storage = Arc::clone(&file_data_storage);
        let pointer_2_decoding_status_storage = Arc::clone(&decoding_status_storage);
        let pointer_2_merging_status_storage = Arc::clone(&merging_status_storage);
        let pointer_2_files_to_be_merged_vec = Arc::clone(&files_to_be_merged);

        // -- Spawn a task to check if merging a file is possible (i.e all chunks have been decoded)
        task::spawn(async move {
            // -- Try to join files if all chunks are decoded, if not continue loop.
            let (completed, completed_filename) = merge_temp_files(
                pointer_2_files_to_be_merged_vec.clone(),
                pointer_2_decoding_status_storage.clone(),
                pointer_2_merging_status_storage.clone(),
            )
            .await;

            // -- After generating main file, need to drop the data in the Hashmaps to free up memory
            if completed {
                let filename = completed_filename.unwrap();
                println!("->> Completed file: {}", filename);

                pointer_2_decoding_status_storage
                    .lock()
                    .await
                    .retain(|ds| ds.filename != filename);

                pointer_2_f_c_s_storage
                    .lock()
                    .await
                    .retain(|ds| ds.filename != filename);

                pointer_2_file_data_storage
                    .lock()
                    .await
                    .retain(|ds| ds.filename != filename);

                pointer_2_merging_status_storage
                    .lock()
                    .await
                    .retain(|ds| ds.filename != filename);

                pointer_2_files_to_be_merged_vec
                    .lock()
                    .await
                    .retain(|name| *name != filename);
            }
        });
    }

    Ok(())
}

// This function parses the packets in the processing storage
// and decodes them once enough segments are received
async fn parsing_packets(
    processing_storage: Vec<Vec<u8>>,
    pointer_f_c_s_storage: Arc<Mutex<Box<Vec<FileChunkSegment>>>>,
    pointer_file_data_storage: Arc<Mutex<Box<Vec<FileData>>>>,
    pointer_decoding_status_storage: Arc<Mutex<Box<Vec<DecodingStatus>>>>,
    pointer_merging_status_storage: Arc<Mutex<Box<Vec<MergingStatus>>>>,
    pointer_files_to_be_merged_vec: Arc<Mutex<Box<Vec<String>>>>,
    NUMBER_OF_REPAIR_SYMBOLS: u64,
    MAX_SOURCE_SYMBOL_SIZE: usize,
) {
    // -- Loop not needed if min packets before processing is not set
    for packet in processing_storage.iter() {
        // -- Deserialise the packet
        let packet: Packet = bincode::deserialize(&packet).expect("Unable to deserialise packet.");
        let padding_check = packet.pre_padding;
        // -- Check if packets is an empty packet
        // (empty packet contains 0 in pre_padding, used to fill min packet so remainding packets can be parsed)
        if padding_check == 0 {
            continue;
        }
        // -- Obtain the necessary info
        let preamble = packet.preamble;
        let filename = preamble.filename;
        let filesize = preamble.filesize;
        let chunk_id = preamble.chunk_id;
        let chunksize = preamble.chunksize;
        let number_of_chunks_expected = preamble.number_of_chunks_expected;
        let segment_id = preamble.segment_id;
        let number_of_segments_expected = preamble.number_of_segments_expected;
        // -- Check if file has already been written and skips the packet
        let received_path = format!("./receiving_dir/{}", filename);
        if Path::new(&received_path).exists().await
            && fs::metadata(&received_path).await.unwrap().size() == filesize
        {
            // break;
            continue;
        }
        // -- Add filename to a vec to verify if all files have been merged
        let mut files_to_be_merged_vec = pointer_files_to_be_merged_vec.lock().await;
        if !files_to_be_merged_vec.contains(&filename) {
            files_to_be_merged_vec.push(filename.clone());
        }

        // -- Check if filename for this segment exist, if not add a new FileData struct with filename and empty vec
        let chunk_segment_name = format!("c_{}_s_{}", chunk_id, segment_id);
        let mut temp_file_data_storage = pointer_file_data_storage.lock().await;
        if temp_file_data_storage
            .iter()
            .find(|fd| fd.filename == filename)
            .is_none()
        {
            temp_file_data_storage.push(FileData {
                filename: filename.clone(),
                chunk_segment_data: Vec::new(),
            })
        }
        // -- Add packet data into chunk_segment_data vec of FileData struct
        let chunk_segment_data_vec = &mut temp_file_data_storage
            .iter_mut()
            .find(|fcs| fcs.filename == filename)
            .unwrap()
            .chunk_segment_data;
        chunk_segment_data_vec.push(ChunkSegmentData {
            chunk_segment_name: chunk_segment_name,
            data: packet.data,
        });

        // -- Check if counter has filename then initialise vec of num chunks and num segment received
        let mut temp_f_c_s_storage = pointer_f_c_s_storage.lock().await;
        if temp_f_c_s_storage
            .iter()
            .find(|fcs| fcs.filename == filename)
            .is_none()
        {
            temp_f_c_s_storage.push(FileChunkSegment {
                filename: filename.clone(),
                chunks_segments: vec![0; number_of_chunks_expected as usize],
            });
        }
        // Increment count for segment received
        let chunk_segments = temp_f_c_s_storage
            .iter_mut()
            .find(|fcs| fcs.filename == filename)
            .unwrap();
        *chunk_segments
            .chunks_segments
            .get_mut(chunk_id as usize)
            .unwrap() += 1;

        // -- Obtain number of segment received
        let number_of_segments_received = *chunk_segments
            .chunks_segments
            .get(chunk_id as usize)
            .unwrap();

        // -- Check decoding status storage and add filename, to prevent programme decoding multiple times
        let mut temp_decoding_status_hashmap = pointer_decoding_status_storage.lock().await;
        if temp_decoding_status_hashmap
            .iter()
            .find(|ds| ds.filename == filename)
            .is_none()
        {
            temp_decoding_status_hashmap.push(DecodingStatus {
                filename: filename.clone(),
                chunks_status: vec!["Undecoded".to_string(); number_of_chunks_expected as usize],
            });
        }
        // -- If number of segments received is enough of the decoder to decode and chunk has not been decoded yet
        if number_of_segments_received
            >= (number_of_segments_expected - NUMBER_OF_REPAIR_SYMBOLS as u64)
            && *temp_decoding_status_hashmap
                .iter()
                .find(|ds| ds.filename == filename)
                .unwrap()
                .chunks_status
                .get(chunk_id as usize)
                .unwrap()
                == "Undecoded".to_string()
        {
            // -- Add all the necessary segments need to decode
            let (mut segments_name_to_decode, mut segments_to_decode) = (Vec::new(), Vec::new());

            for i in 0..number_of_segments_received {
                let chunk_segment_name = format!("c_{}_s_{}", chunk_id, i);
                let segment_option = temp_file_data_storage
                    .iter()
                    .find(|fd| fd.filename == filename)
                    .unwrap()
                    .chunk_segment_data
                    .iter()
                    .find(|csd| csd.chunk_segment_name == chunk_segment_name);

                match segment_option {
                    Some(segment) => {
                        segments_to_decode.push(segment.data.to_owned());
                        segments_name_to_decode.push(chunk_segment_name);
                        // enough_segment = true;
                    }
                    None => {
                        continue;
                    }
                }
            }
            // Change chunk status to Decoding
            let chunk_status = temp_decoding_status_hashmap
                .iter_mut()
                .find(|ds| ds.filename == filename)
                .unwrap()
                .chunks_status
                .get_mut(chunk_id as usize)
                .unwrap();
            *chunk_status = "Decoding".to_string();
            // Clone pointers as task needs ownership of them
            let pointer_2_decoding_status_storage = Arc::clone(&pointer_decoding_status_storage);
            let pointer_2_merging_status_storage = Arc::clone(&pointer_merging_status_storage);
            // Spawn a task to decode the segments
            task::spawn(async move {
                // Returns true if chunk has been successfully decoded and data is written in the temp folder
                let chunk_decoded = decode_segments(
                    segments_to_decode,
                    segments_name_to_decode,
                    MAX_SOURCE_SYMBOL_SIZE,
                    chunksize as usize,
                    filename.clone(),
                    chunk_id as usize,
                )
                .await;
                // If chunk has not been decoded change back decoding status to Undecoded
                let mut temp_2_decoding_status_storage =
                    pointer_2_decoding_status_storage.lock().await;
                if !chunk_decoded {
                    let chunk_status = temp_2_decoding_status_storage
                        .iter_mut()
                        .find(|ds| ds.filename == filename)
                        .unwrap()
                        .chunks_status
                        .get_mut(chunk_id as usize)
                        .unwrap();
                    *chunk_status = "Undecoded".to_string();
                } else {
                    // Change decoding status to Decoded and add merging status
                    *temp_2_decoding_status_storage
                        .iter_mut()
                        .find(|ds| ds.filename == filename)
                        .unwrap()
                        .chunks_status
                        .get_mut(chunk_id as usize)
                        .unwrap() = "Decoded".to_string();
                    pointer_2_merging_status_storage
                        .lock()
                        .await
                        .push(MergingStatus {
                            filename: filename.clone(),
                            chunksize: chunksize,
                            expected_chunks: number_of_chunks_expected,
                            status: false,
                        })
                }
            });
        }
    }
}

/// This function tries to merge all chunks written in the temp folder into the final file
async fn merge_temp_files(
    pointer_files_to_be_merged_vec: Arc<Mutex<Box<Vec<String>>>>,
    pointer_decoding_status_storage: Arc<Mutex<Box<Vec<DecodingStatus>>>>,
    pointer_merging_status_storage: Arc<Mutex<Box<Vec<MergingStatus>>>>,
) -> (bool, Option<String>) {
    // Obtain filename
    let mut files_to_be_merged_vec = pointer_files_to_be_merged_vec.lock().await;
    let filename = match files_to_be_merged_vec.last() {
        Some(filename) => filename.to_owned(),
        None => return (false, None),
    };
    // Check if chunks have already been merged, if so and return
    let mut temp_merging_status_storage = pointer_merging_status_storage.lock().await;
    match temp_merging_status_storage
        .iter()
        .find(|ms| ms.filename == filename)
    {
        Some(ms) => {
            if ms.status {
                return (false, None);
            }
        }
        None => {
            return (false, None);
        }
    }
    // Obtain chunksize to determine size of total file.
    // Used for changing write to final file method
    let chunksize = temp_merging_status_storage
        .iter()
        .find(|ms| ms.filename == filename)
        .unwrap()
        .chunksize;
    // Obtain numbder of chunks expected to determine if all chunks have been received and decoded
    let expected_chunks = temp_merging_status_storage
        .iter()
        .find(|ms| ms.filename == filename)
        .unwrap()
        .expected_chunks;
    // Check if chunks have been decoded (may not be needed)
    let temp_decoding_status_storage = pointer_decoding_status_storage.lock().await;
    if temp_decoding_status_storage
        .iter()
        .find(|ds| ds.filename == filename)
        .is_none()
    {
        return (false, None);
    };
    // Obtain the number of chunk received and decoded
    let received_and_decoded_chunks = temp_decoding_status_storage
        .iter()
        .find(|ds| ds.filename == filename)
        .unwrap()
        .chunks_status
        .iter()
        .filter(|string| **string == "Decoded".to_string())
        .count();

    // Check if all chunks have been decoded and proceed to merge the files
    if received_and_decoded_chunks == expected_chunks as usize {
        // Update merging status to true, so concurent task will not try to merge files
        temp_merging_status_storage
            .iter_mut()
            .find(|ms| ms.filename == filename)
            .unwrap()
            .status = true;

        // Write to final file using io::copy method, used for big files as size to big to store in memory before write (method 2)
        // NOTE: Depending on storage size before parsing, if too low merged file will run before last chunk decoded (WIP)
        if chunksize as usize * received_and_decoded_chunks > 3_000_000_000 {
            let temp_final_file_path = format!("./receiving_dir/{}", filename);
            for i in 0..received_and_decoded_chunks {
                let file_2_path = format!("./temp/{}_{}.txt", filename, i);
                let mut file_1 = OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&temp_final_file_path)
                    .await
                    .unwrap();
                let mut file_2 = match OpenOptions::new().read(true).open(&file_2_path).await {
                    Ok(file) => file,
                    Err(_) => return (false, None),
                };

                io::copy(&mut file_2, &mut file_1).await.unwrap();
                // Remove all temp files used to clean up
                let _ = fs::remove_file(&file_2_path).await;
            }

            return (true, Some(filename));
        } else {
            // Method of merging for smaller files
            let mut final_file_bytes = Vec::new();
            for i in 0..received_and_decoded_chunks {
                let file_path = format!("./temp/{}_{}.txt", filename, i);
                match fs::read(&file_path).await {
                    Ok(mut partial_file_bytes) => {
                        final_file_bytes.append(&mut partial_file_bytes);
                    }
                    Err(_) => {
                        temp_merging_status_storage
                            .iter_mut()
                            .find(|ms| ms.filename == filename)
                            .unwrap()
                            .status = false;
                        return (false, None);
                    }
                }
            }
            let final_file_path = format!("./receiving_dir/{}", filename);
            fs::write(final_file_path, final_file_bytes).await.unwrap();
            // Remove all temp files used to clean up
            for i in 0..received_and_decoded_chunks {
                let file_path = format!("./temp/{}_{}.txt", filename, i);
                let _ = fs::remove_file(file_path).await;
            }
            return (true, Some(filename));
        }
    };
    (false, None)
}

/// This function decodes the segments given using raptor_codes
async fn decode_segments(
    segments_to_decode: Vec<Vec<u8>>,
    segments_name_to_decode: Vec<String>,
    max_source_symbol_size: usize,
    chunksize: usize,
    filename: String,
    chunk_id: usize,
) -> bool {
    // Initialise decoded configuration
    let mut decoder = raptor_code::SourceBlockDecoder::new(max_source_symbol_size);
    // Loop through each segment and push to decoder
    for (i, segment) in segments_to_decode.iter().enumerate() {
        let esi = segments_name_to_decode[i]
            .split("_")
            .collect::<Vec<_>>()
            .get(3)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        decoder.push_encoding_symbol(segment, esi);
    }
    // If decoder not fully_specified, not enough segments received
    if !decoder.fully_specified() {
        return false;
    }
    // Reconstruct the chunk
    let reconstructed_chunk = decoder
        .decode(chunksize)
        .ok_or("Unable to decode message")
        .unwrap();

    // Write to file in temp folder
    let chunk_file_path = format!("./temp/{}_{}.txt", filename, chunk_id);
    // Check if path already exist
    if Path::new(&chunk_file_path).exists().await
        && fs::metadata(&chunk_file_path).await.unwrap().size() == reconstructed_chunk.len() as u64
    {
        return true;
    }
    fs::write(chunk_file_path, reconstructed_chunk)
        .await
        .unwrap();
    true
}
