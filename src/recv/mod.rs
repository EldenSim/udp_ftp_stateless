use std::collections::VecDeque;

use std::fs;
use std::net::UdpSocket;
use std::time::Duration;
use std::{env, str, thread, time};

use async_std::sync::{Arc, Mutex};
use async_std::task;
use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation};

use udp_ftp_stateless::{PacketQ, Result};

use crate::recv::models::FileDetailsQ;
mod models;

pub async fn main(udp_service: UdpSocket) -> Result<()> {
    // -- Obtain MTU and raptor decoding settings, must be the same as sender settings
    let NUMBER_OF_REPAIR_SYMBOLS = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<u64>()?;
    let MAX_SOURCE_SYMBOL_SIZE = env::var("MAX_SOURCE_SYMBOL_SIZE")
        .expect("MAX_SOURCE_SYMBOL_SIZE env var not set")
        .parse::<usize>()?;

    let received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>> =
        Arc::new(Mutex::new(Box::new(VecDeque::new())));

    let file_details_storage: Arc<Mutex<Box<Vec<FileDetailsQ>>>> =
        Arc::new(Mutex::new(Box::new(Vec::new())));

    let pointer_2_received_packets = Arc::clone(&received_packets);
    let pointer_2_file_details_storage = Arc::clone(&file_details_storage);

    task::Builder::new()
        .name("Processing Task".to_string())
        .spawn(async move {
            processing_packets(
                pointer_2_received_packets,
                pointer_2_file_details_storage,
                NUMBER_OF_REPAIR_SYMBOLS,
            )
            .await
        })?;

    // task::Builder::new()
    //     .name("Receiving Task".to_string())
    //     .blocking(async move {
    //         recv_packets(&udp_service, received_packets).await.unwrap();
    //     });
    recv_packets(&udp_service, received_packets).await.unwrap();

    Ok(())
}

async fn processing_packets(
    received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>>,
    file_details_storage: Arc<Mutex<Box<Vec<FileDetailsQ>>>>,
    NUMBER_OF_REPAIR_SYMBOLS: u64,
) {
    // let mut files_to_ignore: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut files_to_ignore: Vec<String> = Vec::new();
    loop {
        let mut packets_to_parses = received_packets.lock().await;
        let packet_bytes = match packets_to_parses.front() {
            Some(bytes) => bytes.clone(),
            None => continue,
        };
        packets_to_parses.pop_front();
        let packet: PacketQ = bincode::deserialize(&packet_bytes).unwrap();
        let filename = packet.filename;
        // let files_to_ignore_lock = files_to_ignore.lock().await;
        if files_to_ignore.contains(&filename) {
            continue;
        }
        let number_of_chunks_expected = packet.number_of_chunks_expected;
        let encoder_config = packet.encoder_config;
        let mut file_details_storage_lock = file_details_storage.lock().await;
        // Check if instance of file details have already been created
        if file_details_storage_lock
            .iter()
            .find(|fdQ| fdQ.filename == filename)
            .is_none()
        {
            file_details_storage_lock.push(FileDetailsQ {
                filename: filename.clone(),
                number_of_chunks_expected,
                file_decoding_status: false,
                file_merging_status: false,
                data: Vec::new(),
                encoder_config,
            })
        }
        // Add data into file details
        let file_details = file_details_storage_lock
            .iter_mut()
            .find(|fdQ| fdQ.filename == filename)
            .unwrap();

        file_details.data.push(packet.data);

        let received_chunks = file_details.data.len();
        // println!("recv chunks: {}", received_chunks);

        if received_chunks >= (number_of_chunks_expected - NUMBER_OF_REPAIR_SYMBOLS) as usize
            && !file_details.file_decoding_status
        {
            // let pointer_2_file_details_storage = Arc::clone(&file_details_storage);
            // let pointer_2_files_to_ignore = Arc::clone(&files_to_ignore);
            // let filename_2 = filename.clone();

            println!("Decoding file: {}", filename);
            file_details.file_decoding_status = true;
            // Try to decode file
            let file_data = file_details.data.clone();
            let encoder_config = file_details.encoder_config;
            let (completed, reconstructed_data) = decode_packets(file_data, encoder_config).await;
            if completed {
                let data = reconstructed_data.unwrap();
                let file_path = format!("./receiving_dir/{}", filename);
                thread::spawn(move || {
                    fs::write(file_path, data).unwrap();
                });
                file_details.file_merging_status = true;
                let file_detail_index = file_details_storage_lock
                    .iter()
                    .position(|fdQ| fdQ.filename == filename)
                    .unwrap();
                file_details_storage_lock.remove(file_detail_index);
                files_to_ignore.push(filename);
            }
        }
        // let pointer_2_file_details_storage = Arc::clone(&file_details_storage);
        // let mut file_details_storage_lock_2 = file_details_storage.lock().await;
        // let file_detail_index = file_details_storage_lock_2
        //     .iter()
        //     .position(|fdQ| fdQ.filename == filename)
        //     .unwrap();
        // if file_details_storage_lock_2[file_detail_index].file_merging_status {
        //     file_details_storage_lock_2.remove(file_detail_index);
        // }
    }
}

async fn decode_packets(
    mut file_data: Vec<Vec<u8>>,
    encoder_config: [u8; 12],
) -> (bool, Option<Vec<u8>>) {
    println!("Length of packets: {:?}", &file_data.len());
    let mut decoder = Decoder::new(ObjectTransmissionInformation::deserialize(&encoder_config));

    // Perform the decoding
    let mut result = None;
    while !file_data.is_empty() {
        let packet = file_data.pop().unwrap();
        let encoded_packet = EncodingPacket::deserialize(&packet);
        result = decoder.decode(encoded_packet);
        if result.is_some() {
            return (true, result);
        }
    }
    return (false, None);
}

async fn recv_packets(
    udp_service: &UdpSocket,
    received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>>,
) -> Result<()> {
    let MTU = env::var("MTU")
        .expect("MTU env var not set")
        .parse::<usize>()?;
    // let mut no_hit_count = 0;
    let mut start = time::SystemTime::now();
    // udp_service.set_nonblocking(true)?;
    loop {
        let mut packets_storage = received_packets.lock().await;
        // -- Receive packets and store them first
        // Buffer depends on MTU limit set
        let mut buffer = vec![0; MTU + 100];
        match udp_service.recv_from(&mut buffer) {
            Ok((bytes, _)) => {
                let recv_data = &buffer[..bytes];
                packets_storage.push_back(recv_data.to_vec());
            }
            Err(_) => {
                continue;
                // if start.elapsed()? > Duration::from_secs(5) {
                //     println!("Setting to blocking");
                //     udp_service.set_nonblocking(false)?;
                //     start = time::SystemTime::now();
                //     continue;
                // } else {
                //     continue;
                // }
                // println!("no hit count: {}", no_hit_count);
                // if no_hit_count > 50 {
                //     udp_service.set_nonblocking(false)?;
                //     no_hit_count = 0;
                // } else {
                //     no_hit_count += 1;
                //     udp_service.set_nonblocking(true)?;
                //     continue;
                // }
            }
        }
        // if packets_storage.len() > 0 {
        //     udp_service.set_nonblocking(true)?;
        //     match udp_service.recv_from(&mut buffer) {
        //         Ok((bytes, _)) => {
        //             let recv_data = &buffer[..bytes];
        //             packets_storage.push_back(recv_data.to_vec());
        //         }
        //         Err(_) => continue,
        //     }
        // } else {
        //     udp_service.set_nonblocking(false)?;
        //     println!("Processing storage: {}", packets_storage.len());
        //     match udp_service.recv_from(&mut buffer) {
        //         Ok((bytes, _)) => {
        //             let recv_data = &buffer[..bytes];
        //             packets_storage.push_back(recv_data.to_vec());
        //         }
        //         Err(e) => return Err(e.into()),
        //     }
        // }
    }
}
