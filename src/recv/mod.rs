// region:    --- Modules

use std::collections::HashMap;
use std::env;
use std::mem::size_of_val;
use std::net::UdpSocket;

use udp_ftp_stateless::Result;
use udp_ftp_stateless::{Packet, Preamble};

use async_std::sync::{Arc, Mutex};
use async_std::{fs, task};

// endregion: --- Modules

pub fn main(udp_service: &UdpSocket) -> Result<()> {
    // -- Hashmap to keep track of number of chunks received and number of segment received for each chunk
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

    let mut file_chunk_segment_counter: Arc<Mutex<HashMap<String, Vec<u64>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let mut segment_hashmap: Arc<Mutex<Box<HashMap<String, Vec<u8>>>>> =
        Arc::new(Mutex::new(Box::new(HashMap::new())));
    let mut temp_storage = Box::new(Vec::new());
    let decoding_status_hashmap: Arc<Mutex<HashMap<u64, bool>>> =
        Arc::new(Mutex::new(HashMap::new()));
    loop {
        let mut buffer = vec![0; MTU];
        match udp_service.recv_from(&mut buffer) {
            Ok((bytes, _)) => {
                let recv_data = &buffer[..bytes];
                temp_storage.push(recv_data.to_vec());
            }
            Err(e) => return Err(e.into()),
        }
        // -- Parse packet and name them
        let move_temp_f_c_s_counter = Arc::clone(&file_chunk_segment_counter);
        let move_temp_storage: Vec<_> = temp_storage.drain(..temp_storage.len()).collect();
        let move_temp_segment_template = Arc::clone(&segment_hashmap);
        let move_temp_decoding_status_hashmap = Arc::clone(&decoding_status_hashmap);
        task::spawn(async move {
            for packet in move_temp_storage.iter() {
                let packet: Packet =
                    bincode::deserialize(&packet).expect("Unable to deserialise packet.");
                let preamble = packet.preamble;
                let filename = preamble.filename;
                let chunk_id = preamble.chunk_id;
                let chunksize = preamble.chunksize;
                let number_of_chunks_expected = preamble.number_of_chunks_expected;
                let segment_id = preamble.segment_id;
                let number_of_segments_expected = preamble.number_of_segments_expected;
                let file_chunk_segment_name =
                    format!("{}_c_{}_s_{}", &filename, chunk_id, segment_id);
                // let file_chunk_name = format!("{}_c_{}", &filename, chunk_id);
                // -- Insert segment data into segment hashmap
                move_temp_segment_template
                    .lock()
                    .await
                    .insert(file_chunk_segment_name.clone(), packet.data);
                // -- Check if counter has filename then initialise vec of num chunks and num segment received
                if !move_temp_f_c_s_counter.lock().await.contains_key(&filename) {
                    move_temp_f_c_s_counter.lock().await.insert(
                        filename.clone(),
                        vec![0; number_of_chunks_expected as usize],
                    );
                }
                // -- Obtain number of segment received
                let number_of_segments_received =
                    move_temp_f_c_s_counter.lock().await.get(&filename).unwrap()[chunk_id as usize];
                // --  If num of segments recevied less then expected update counter else ignore
                if number_of_segments_received < number_of_segments_expected {
                    move_temp_f_c_s_counter
                        .lock()
                        .await
                        .get_mut(&filename)
                        .unwrap()[chunk_id as usize] += 1
                }

                if !move_temp_decoding_status_hashmap
                    .lock()
                    .await
                    .contains_key(&chunk_id)
                {
                    move_temp_decoding_status_hashmap
                        .lock()
                        .await
                        .insert(chunk_id, false);
                }

                if number_of_segments_received
                    > (number_of_segments_expected - NUMBER_OF_REPAIR_SYMBOLS as u64)
                    && move_temp_decoding_status_hashmap.lock().await[&chunk_id] == false
                {
                    println!("->> Enough segment to decode");
                    let (mut segments_name_to_decode, mut segments_to_decode) =
                        (Vec::new(), Vec::new());
                    for i in 0..number_of_segments_received {
                        let segment_name = format!("{}_c_{}_s_{}", &filename, chunk_id, i);
                        println!("->> segment name: {}", segment_name);
                        let mut segment_hashmap = move_temp_segment_template.lock().await;

                        let segment_details = segment_hashmap.get_key_value(&segment_name).unwrap();
                        segments_name_to_decode.push(segment_details.0.to_string());
                        segments_to_decode.push(segment_details.1.to_owned());
                        segment_hashmap.remove(&segment_name);
                    }
                    println!("->> segments length: {}", segments_to_decode.len());
                    // println!("->> segments name: {:?}", segments_name_to_decode);
                    *move_temp_decoding_status_hashmap
                        .lock()
                        .await
                        .get_mut(&chunk_id)
                        .unwrap() = true;

                    task::spawn(async move {
                        decode_segments(
                            segments_to_decode.to_owned(),
                            segments_name_to_decode,
                            MAX_SOURCE_SYMBOL_SIZE,
                            chunksize as usize,
                            filename,
                            chunk_id as usize,
                        )
                        .await
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
    }

    Ok(())
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
        let esi = segments_name_to_decode[i].split("_").collect::<Vec<_>>()[4]
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
