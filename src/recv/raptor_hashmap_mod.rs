pub async fn old_main(udp_service: &UdpSocket) -> Result<()> {
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

    let file_segment_hashmap: Arc<Mutex<Box<HashMap<String, HashMap<String, Vec<u8>>>>>> =
        Arc::new(Mutex::new(Box::new(HashMap::new())));

    let mut storage = Box::new(Vec::new());
    let decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let merging_status_hashmap: Arc<Mutex<HashMap<String, bool>>> =
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
        // 512 need 2 send runs (with padding packets)
        // 64 need 2 send runs (padding packet error)
        // 32 need 2 send runs
        if storage.len() < 64 {
            continue;
        }
        // -- Obtain the packets in the temp storage and move them as will be used in spawed task
        let processing_storage: Vec<_> = storage.drain(..storage.len()).collect();

        // -- Clone
        let pointer_f_c_s_counter: Arc<Mutex<HashMap<String, Vec<u64>>>> =
            Arc::clone(&file_chunk_segment_counter);
        let pointer_file_segment_hashmap: Arc<
            Mutex<Box<HashMap<String, HashMap<String, Vec<u8>>>>>,
        > = Arc::clone(&file_segment_hashmap);
        let pointer_decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>> =
            Arc::clone(&decoding_status_hashmap);
        let pointer_merging_status_hashmap: Arc<Mutex<HashMap<String, bool>>> =
            Arc::clone(&merging_status_hashmap);
        // let pointer_2_decoding_status_hashmap = Arc::clone(&decoding_status_hashmap);
        task::spawn(async move {
            old_parsing_packets(
                processing_storage,
                pointer_f_c_s_counter,
                pointer_file_segment_hashmap,
                pointer_decoding_status_hashmap,
                pointer_merging_status_hashmap,
                NUMBER_OF_REPAIR_SYMBOLS as u64,
                MAX_SOURCE_SYMBOL_SIZE,
            )
            .await;
        });

        let pointer_2_decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>> =
            Arc::clone(&decoding_status_hashmap);
        let pointer_2_f_c_s_counter: Arc<Mutex<HashMap<String, Vec<u64>>>> =
            Arc::clone(&file_chunk_segment_counter);
        let pointer_2_file_segment_hashmap: Arc<
            Mutex<Box<HashMap<String, HashMap<String, Vec<u8>>>>>,
        > = Arc::clone(&file_segment_hashmap);
        let pointer_2_merging_status_hashmap = Arc::clone(&merging_status_hashmap);
        task::spawn(async move {
            // -- Try to join files if all chunks are decoded, if not continue loop.
            let (completed, completed_filename) = old_merge_temp_files(
                random_packet,
                pointer_2_decoding_status_hashmap.clone(),
                pointer_2_merging_status_hashmap.clone(),
            )
            .await;
            // // -- After generating main file, need to drop the data in the Hashmaps to free up memory

            if completed {
                let filename = completed_filename.unwrap();
                println!("->> Completed file: {}", filename);

                // -- Clearing memory may not yield expected result as of 15/1/2024
                // -- See: https://github.com/rust-lang/rust/issues/73307
                pointer_2_decoding_status_hashmap
                    .lock()
                    .await
                    .remove(&filename);
                pointer_2_f_c_s_counter.lock().await.remove(&filename);
                pointer_2_file_segment_hashmap
                    .lock()
                    .await
                    .remove(&filename);
                // pointer_2_merging_status_hashmap
                //     .lock()
                //     .await
                //     .remove(&filename);
            }
        });
    }

    Ok(())
}

async fn old_parsing_packets(
    processing_storage: Vec<Vec<u8>>,
    pointer_f_c_s_counter: Arc<Mutex<HashMap<String, Vec<u64>>>>,
    pointer_file_segment_hashmap: Arc<Mutex<Box<HashMap<String, HashMap<String, Vec<u8>>>>>>,
    pointer_decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>>,
    pointer_merging_status_hashmap: Arc<Mutex<HashMap<String, bool>>>,
    NUMBER_OF_REPAIR_SYMBOLS: u64,
    MAX_SOURCE_SYMBOL_SIZE: usize,
) {
    // -- Loop not needed if min packets before processing is not set
    for packet in processing_storage.iter() {
        let packet: Packet = bincode::deserialize(&packet).expect("Unable to deserialise packet.");
        let padding_check = packet.pre_padding;
        if padding_check == 0 {
            continue;
        }
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
            >= (number_of_segments_expected - NUMBER_OF_REPAIR_SYMBOLS as u64)
            && *temp_dec_stat_hashmap
                .get(&filename)
                .unwrap()
                .get(&chunk_id)
                .unwrap()
                == "Undecoded".to_string()
        {
            // println!("->> Enough segment to decode");
            let (mut segments_name_to_decode, mut segments_to_decode) = (Vec::new(), Vec::new());
            // let mut temp_f_s_hashmap_2 = pointer_file_segment_hashmap.lock().await;
            // let temp_segment_hashmap = temp_f_s_hashmap_2.get_mut(&filename).unwrap();
            // let temp_segment_hashmap = temp_f_s_hashmap.get_mut(&filename).unwrap();
            // let segments_received: Vec<_> =
            //     temp_f_s_hashmap.get(&filename).unwrap().keys().collect();
            // for segment_name in segments_received {
            //     segments_to_decode.push(
            //         temp_f_s_hashmap
            //             .get(&filename)
            //             .unwrap()
            //             .get(segment_name)
            //             .unwrap()
            //             .to_owned(),
            //     );
            //     segments_name_to_decode.push(segment_name.to_owned())
            // }
            // println!("->> num seg rece: {}", number_of_segments_received);
            for i in 0..number_of_segments_received {
                // let mut enough_segment = false;
                let chunk_segment_name = format!("c_{}_s_{}", chunk_id, i);
                let segment_option = temp_f_s_hashmap
                    .get(&filename)
                    .unwrap()
                    .get(&chunk_segment_name);
                match segment_option {
                    Some(segment) => {
                        segments_to_decode.push(segment.to_owned());
                        segments_name_to_decode.push(chunk_segment_name);
                        // enough_segment = true;
                    }
                    None => {
                        continue;
                    }
                }
            }
            *temp_dec_stat_hashmap
                .get_mut(&filename)
                .unwrap()
                .get_mut(&chunk_id)
                .unwrap() = "Decoding".to_string();
            // let mut temp_dec_stat_hashmap = pointer_2_decoding_status_hashmap.lock().await;
            let pointer_2_decoding_status_hashmap = Arc::clone(&pointer_decoding_status_hashmap);
            let pointer_2_merging_status_hashmap = Arc::clone(&pointer_merging_status_hashmap);
            task::spawn(async move {
                let chunk_decoded = decode_segments(
                    segments_to_decode,
                    segments_name_to_decode,
                    MAX_SOURCE_SYMBOL_SIZE,
                    chunksize as usize,
                    filename.clone(),
                    chunk_id as usize,
                )
                .await;

                let mut temp_2_dec_stat_hashmap = pointer_2_decoding_status_hashmap.lock().await;
                if !chunk_decoded {
                    *temp_2_dec_stat_hashmap
                        .get_mut(&filename)
                        .unwrap()
                        .get_mut(&chunk_id)
                        .unwrap() = "Undecoded".to_string();
                } else {
                    *temp_2_dec_stat_hashmap
                        .get_mut(&filename)
                        .unwrap()
                        .get_mut(&chunk_id)
                        .unwrap() = "Decoded".to_string();
                    pointer_2_merging_status_hashmap
                        .lock()
                        .await
                        .insert(filename.clone(), false);
                }
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
}

async fn old_merge_temp_files(
    packet: Vec<u8>,
    decoding_status_hashmap: Arc<Mutex<HashMap<String, HashMap<u64, String>>>>,
    merging_status_hashmap: Arc<Mutex<HashMap<String, bool>>>,
) -> (bool, Option<String>) {
    let packet: Packet = bincode::deserialize(&packet).unwrap();
    let chunksize = packet.preamble.chunksize;
    let filename = packet.preamble.filename;
    match merging_status_hashmap.lock().await.get(&filename) {
        Some(status) => {
            if *status {
                return (false, None);
            }
        }
        None => {
            return (false, None);
        }
    }
    // if *merging_status_hashmap.lock().await.get(&filename).unwrap() == true {
    //     return (false, None);
    // }
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
    // println!(
    //     "->> received and decoded chunks: {}",
    //     received_and_decoded_chunks
    // );
    if received_and_decoded_chunks == expected_chunks as usize {
        *merging_status_hashmap
            .lock()
            .await
            .get_mut(&filename)
            .unwrap() = true;

        // Check if total file size is greater than RAM and handle file write differently
        // let final_file_path = format!("./receiving_dir/{}", filename);
        // let mut final_file = OpenOptions::new()
        //     .create(true)
        //     .append(true)
        //     .open(final_file_path)
        //     .await
        //     .unwrap();
        // for i in 0..received_and_decoded_chunks {
        //     let file_path = format!("./temp/{}_{}.txt", filename, i);
        //     match fs::read(file_path).await {
        //         Ok(partial_file_bytes) => final_file.write_all(buf),
        //     }
        // }

        // NOTE: Depending on storage size before parsing, if too low merged file will run before last chunk decoded (WIP)
        if chunksize as usize * received_and_decoded_chunks > 1_000_000_000 {
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
                let _ = fs::remove_file(&file_2_path).await;
            }
        } else {
            let mut final_file_bytes = Vec::new();

            for i in 0..received_and_decoded_chunks {
                let file_path = format!("./temp/{}_{}.txt", filename, i);

                match fs::read(&file_path).await {
                    Ok(mut partial_file_bytes) => {
                        final_file_bytes.append(&mut partial_file_bytes);
                    }
                    Err(_) => {
                        *merging_status_hashmap
                            .lock()
                            .await
                            .get_mut(&filename)
                            .unwrap() = false;
                        return (false, None);
                    }
                }
            }
            let final_file_path = format!("./receiving_dir/{}", filename);
            fs::write(final_file_path, final_file_bytes).await.unwrap();
            for i in 0..received_and_decoded_chunks {
                let file_path = format!("./temp/{}_{}.txt", filename, i);
                let _ = fs::remove_file(file_path).await;
            }
            return (true, Some(filename));
        }
    };
    (false, None)
}
