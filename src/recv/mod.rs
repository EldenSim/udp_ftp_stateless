// region:    --- Modules

use std::collections::HashMap;
use std::env;
use std::mem::size_of_val;
use std::net::UdpSocket;
use std::os::unix::fs::MetadataExt;

use async_std::fs::OpenOptions;
use async_std::io::WriteExt;
use async_std::path::Path;
use udp_ftp_stateless::Result;
use udp_ftp_stateless::{Packet, Preamble};

use async_std::sync::{Arc, Mutex};
use async_std::{fs, task};

// endregion: --- Modules

pub async fn main(udp_service: &UdpSocket) -> Result<()> {
    // -- Obtain necessary env var
    /*
        Sender and Receiver should have the same env var for the following:
            - NUMBER_OF_REPAIR_SYMBOLS: Used for determining when to start decoding (number of expected segment - NUMBER_OF_REPAIR_SYMBOLS)
            - MAX_SOURCE_SYMBOL_SIZE: Used for initialising raptor decoder
            - MTU: Used for initialising initial recv buffer size
            - MAX_CHUNKSIZE: Used for limiting the max chunksize to be received from sender
    */
    let NUMBER_OF_REPAIR_SYMBOLS = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<usize>()?;
    let MAX_SOURCE_SYMBOL_SIZE = env::var("MAX_SOURCE_SYMBOL_SIZE")
        .expect("MAX_SOURCE_SYMBOL_SIZE env var not set")
        .parse::<usize>()?;
    let MTU = env::var("MTU")
        .expect("MTU env var not set")
        .parse::<usize>()?;
    let MAX_CHUNKSIZE = env::var("MAX_CHUNKSIZE")
        .expect("MAX_CHUNKSIZE env var not set")
        .parse::<usize>()?;
    let chunksize = if (MTU * MAX_SOURCE_SYMBOL_SIZE) > MAX_CHUNKSIZE {
        MAX_CHUNKSIZE
    } else {
        MTU * MAX_SOURCE_SYMBOL_SIZE
    };

    // -- Setting up objects that need to be refereced and updated from spawned task
    /*
        - file_chunk_segment_counter: Used for keeping track of number of segments received from each chunk
        - segment_hashmap: Used for storing each segment's data
        - storage: Used for storing received packets, will be empty and moved to move_temp_storage used for spawned task
        - decoding_status_hashmap: Used for determining if a decoding of the chunk segments is in progress,
                                to avoid multiple instances of segments being decoded
    */
    let file_chunk_segment_counter: Arc<Mutex<HashMap<String, Vec<u64>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // let segment_hashmap: Arc<Mutex<Box<HashMap<String, Vec<u8>>>>> =
    //     Arc::new(Mutex::new(Box::new(HashMap::new())));

    let file_segment_hashmap: Arc<Mutex<Box<HashMap<String, HashMap<String, Vec<u8>>>>>> =
        Arc::new(Mutex::new(Box::new(HashMap::new())));

    let mut storage = Box::new(Vec::new());
    let decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // let mut files_complete = Box::new(Vec::new());
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

        let random_packet = storage[0].to_owned();

        // -- Can implement a minimum amount of packets to receive before processing (Not sure if helps in performance)
        // if storage.len() < 5 {
        //     continue;
        // }
        // -- Obtain the packets in the temp storage and move them as will be used in spawed task
        let processing_storage: Vec<_> = storage.drain(..storage.len()).collect();

        // -- Clone
        let pointer_f_c_s_counter = Arc::clone(&file_chunk_segment_counter);
        let pointer_file_segment_hashmap = Arc::clone(&file_segment_hashmap);
        let pointer_decoding_status_hashmap = Arc::clone(&decoding_status_hashmap);
        // let pointer_2_decoding_status_hashmap = Arc::clone(&decoding_status_hashmap);
        task::spawn(async move {
            // -- Loop not needed if min packets before processing is not set
            for packet in processing_storage.iter() {
                let packet: Packet =
                    bincode::deserialize(&packet).expect("Unable to deserialise packet.");
                let preamble = packet.preamble;
                let filename = preamble.filename;
                let filesize = preamble.filesize;
                let chunk_id = preamble.chunk_id;
                let chunksize = preamble.chunksize;
                let number_of_chunks_expected = preamble.number_of_chunks_expected;
                let segment_id = preamble.segment_id;
                let number_of_segments_expected = preamble.number_of_segments_expected;
                let received_path = format!("./receiving_dir/{}", filename);
                if Path::new(&received_path).exists().await
                    && fs::metadata(&received_path).await.unwrap().size() == filesize
                {
                    break;
                }

                // -- Update segment hashmap with received packet data
                let chunk_segment_name = format!("c_{}_s_{}", chunk_id, segment_id);
                let mut temp_f_s_hashmap = pointer_file_segment_hashmap.lock().await;
                if !temp_f_s_hashmap.contains_key(&filename) {
                    temp_f_s_hashmap.insert(filename.clone(), HashMap::new());
                    temp_f_s_hashmap
                        .get_mut(&filename)
                        .unwrap()
                        .insert(chunk_segment_name.clone(), packet.data);
                } else {
                    // println!("->> Inserting cs name: {}", chunk_segment_name);
                    temp_f_s_hashmap
                        .get_mut(&filename)
                        .unwrap()
                        .insert(chunk_segment_name.clone(), packet.data);
                    // println!(
                    //     "->> len segment hashmap: {}",
                    //     temp_f_s_hashmap.get(&filename).unwrap().len()
                    // );
                }

                // -----
                // -- Check if counter has filename then initialise vec of num chunks and num segment received
                let mut temp_f_c_s_counter = pointer_f_c_s_counter.lock().await;
                if !temp_f_c_s_counter.contains_key(&filename) {
                    temp_f_c_s_counter.insert(
                        filename.clone(),
                        vec![0; number_of_chunks_expected as usize],
                    );
                }
                *temp_f_c_s_counter
                    .get_mut(&filename)
                    .unwrap()
                    .get_mut(chunk_id as usize)
                    .unwrap() += 1;
                // -- Obtain number of segment received
                let number_of_segments_received = *temp_f_c_s_counter
                    .get(&filename)
                    .unwrap()
                    .get(chunk_id as usize)
                    .unwrap();
                // --  If num of segments recevied less then expected update counter else ignore
                // if number_of_segments_received < number_of_segments_expected {
                //     *temp_f_c_s_counter
                //         .get_mut(&filename)
                //         .unwrap()
                //         .get_mut(chunk_id as usize)
                //         .unwrap() += 1
                // }
                let mut temp_dec_stat_hashmap = pointer_decoding_status_hashmap.lock().await;
                if !temp_dec_stat_hashmap.contains_key(&filename) {
                    temp_dec_stat_hashmap.insert(filename.clone(), HashMap::new());
                    // .and_then(|mut hashmap| hashmap.insert(chunk_id, "Undecoded".to_string()));
                    temp_dec_stat_hashmap
                        .get_mut(&filename)
                        .unwrap()
                        .insert(chunk_id, "Undecoded".to_string());
                }
                if !temp_dec_stat_hashmap
                    .get(&filename)
                    .unwrap()
                    .contains_key(&chunk_id)
                {
                    temp_dec_stat_hashmap
                        .get_mut(&filename)
                        .unwrap()
                        .insert(chunk_id, "Undecoded".to_string());
                }

                if number_of_segments_received
                    > (number_of_segments_expected - NUMBER_OF_REPAIR_SYMBOLS as u64)
                    && *temp_dec_stat_hashmap
                        .get(&filename)
                        .unwrap()
                        .get(&chunk_id)
                        .unwrap()
                        == "Undecoded".to_string()
                {
                    // println!("->> Enough segment to decode");
                    let (mut segments_name_to_decode, mut segments_to_decode) =
                        (Vec::new(), Vec::new());
                    // let mut temp_f_s_hashmap_2 = pointer_file_segment_hashmap.lock().await;
                    // let temp_segment_hashmap = temp_f_s_hashmap_2.get_mut(&filename).unwrap();
                    // let temp_segment_hashmap = temp_f_s_hashmap.get_mut(&filename).unwrap();
                    let segments_received: Vec<_> =
                        temp_f_s_hashmap.get(&filename).unwrap().keys().collect();
                    // for i in 0..number_of_segments_received {
                    for segment_name in segments_received {
                        // let segment_name = format!("c_{}_s_{}", chunk_id, i);
                        // println!("->> segment name: {}", segment_name);
                        // let temp_2_segment_hashmap = move_temp_segment_template.lock().await;

                        let segment_bytes = temp_f_s_hashmap
                            .get(&filename)
                            .unwrap()
                            .get(segment_name)
                            .unwrap();
                        // println!("->> Segment details: {:?}", segment_details.0);
                        // segments_name_to_decode.push(segment_details.0.to_string());
                        // segments_to_decode.push(segment_details.1.to_owned());
                        segments_name_to_decode.push(segment_name.to_string());
                        segments_to_decode.push(segment_bytes.to_owned());
                        // temp_segment_hashmap.remove(&segment_name).unwrap();
                    }
                    // println!("->> segments length: {}", segments_to_decode.len());
                    // println!("->> segments name: {:?}", segments_name_to_decode);
                    *temp_dec_stat_hashmap
                        .get_mut(&filename)
                        .unwrap()
                        .get_mut(&chunk_id)
                        .unwrap() = "Decoding".to_string();
                    // let mut temp_dec_stat_hashmap = pointer_2_decoding_status_hashmap.lock().await;
                    let pointer_2_decoding_status_hashmap =
                        Arc::clone(&pointer_decoding_status_hashmap);

                    task::spawn(async move {
                        decode_segments(
                            segments_to_decode,
                            segments_name_to_decode,
                            MAX_SOURCE_SYMBOL_SIZE,
                            chunksize as usize,
                            filename.clone(),
                            chunk_id as usize,
                        )
                        .await;
                        let mut temp_2_dec_stat_hashmap =
                            pointer_2_decoding_status_hashmap.lock().await;
                        *temp_2_dec_stat_hashmap
                            .get_mut(&filename)
                            .unwrap()
                            .get_mut(&chunk_id)
                            .unwrap() = "Decoded".to_string();
                    });
                }

                // println!("->> chunk id: {}", chunk_id);
            }
            // println!(
            //     "->> segment hashmap: {:?}",
            //     move_temp_segment_template.lock().await.len()
            // );
            // println!(
            //     "->> Mem segment hashmap {}",
            //     size_of_val(&**move_temp_segment_template.lock().await)
            // );
            // println!(
            //     "->> counter Hashmap: {:?}",
            //     move_temp_f_c_s_counter.lock().await
            // );
        });
        let pointer_2_decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>> =
            Arc::clone(&decoding_status_hashmap);
        // task::spawn(async move {
        //     join_files(random_packet, move_2_temp_decoding_status_hashmap).await;
        // });
        // -- Try to join files if all chunks are decoded, if not continue loop.
        let (completed, completed_filename) =
            merge_temp_files(random_packet, pointer_2_decoding_status_hashmap).await;
        // // -- After generating main file, need to drop the data in the Hashmaps to free up memory

        if completed {
            // let drop_decoding_status_hashmap = Arc::clone(&decoding_status_hashmap);
            // let drop_segment_hashmap = Arc::clone(&file_segment_hashmap);
            // let drop_file_chunk_segment_counter = Arc::clone(&file_chunk_segment_counter);
            let filename = completed_filename.unwrap();
            println!("->> C filename: {}", filename);
            // println!("->> removing file: {}", filename);
            // drop_decoding_status_hashmap.lock().await.remove(&filename);
            // drop_file_chunk_segment_counter
            //     .lock()
            //     .await
            //     .remove(&filename);
            // drop_segment_hashmap.lock().await.remove(&filename);
            decoding_status_hashmap.lock().await.remove(&filename);
            file_chunk_segment_counter.lock().await.remove(&filename);
            file_segment_hashmap.lock().await.remove(&filename);
        }
    }

    Ok(())
}

async fn merge_temp_files(
    packet: Vec<u8>,
    decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>>,
) -> (bool, Option<String>) {
    let packet: Packet = bincode::deserialize(&packet).unwrap();
    let filename = packet.preamble.filename;
    let expected_chunks = packet.preamble.number_of_chunks_expected;
    if !decoding_status_hashmap.lock().await.contains_key(&filename) {
        return (false, None);
    }
    let received_and_decoded_chunks = decoding_status_hashmap
        .lock()
        .await
        .get(&filename)
        .unwrap()
        .iter()
        .map(|(_, v)| *v == "Decoded".to_string())
        .count();
    println!(
        "->> received and decoded chunks: {}",
        received_and_decoded_chunks
    );
    if received_and_decoded_chunks == expected_chunks as usize {
        let mut final_file_bytes = Vec::new();
        for i in 0..received_and_decoded_chunks {
            let file_path = format!("./temp/{}_{}.txt", filename, i);

            match fs::read(&file_path).await {
                Ok(mut partial_file_bytes) => {
                    final_file_bytes.append(&mut partial_file_bytes);
                }
                Err(_) => return (false, None),
            }

            if final_file_bytes.len() > 6_000_000_000 {
                // -- TODO: Write to file first if file is large (e.g 12 GB and up depends on RAM size)
                todo!()
            }
        }
        let final_file_path = format!("./receiving_dir/{}", filename);
        fs::write(final_file_path, final_file_bytes).await.unwrap();
        for i in 0..received_and_decoded_chunks {
            let file_path = format!("./temp/{}_{}.txt", filename, i);
            let _ = fs::remove_file(file_path).await;
        }
        return (true, Some(filename));
    };
    (false, None)
}

async fn decode_segments(
    segments_to_decode: Vec<Vec<u8>>,
    segments_name_to_decode: Vec<String>,
    max_source_symbol_size: usize,
    chunksize: usize,
    filename: String,
    chunk_id: usize,
) {
    let mut decoder = raptor_code::SourceBlockDecoder::new(max_source_symbol_size);
    let mut i = 0;
    while !decoder.fully_specified() {
        let esi = segments_name_to_decode[i].split("_").collect::<Vec<_>>()[3]
            .parse::<u32>()
            .unwrap();
        decoder.push_encoding_symbol(&segments_to_decode[i], esi);
        i += 1
    }

    let reconstructed_chunk = decoder
        .decode(chunksize)
        .ok_or("Unable to decode message")
        .unwrap();
    println!("->> Decoded DONE");
    let chunk_file_path = format!("./temp/{}_{}.txt", filename, chunk_id);
    fs::write(chunk_file_path, reconstructed_chunk)
        .await
        .unwrap()
}
