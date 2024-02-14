use std::collections::VecDeque;
use std::net::UdpSocket;
use std::{env, fs, str};

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

    task::Builder::new()
        .name("Processing Task".to_string())
        .spawn(async move {
            processing_packets(
                pointer_2_received_packets,
                file_details_storage,
                NUMBER_OF_REPAIR_SYMBOLS,
            )
            .await
        })?;

    recv_packets(&udp_service, received_packets).await.unwrap();
    todo!();
    Ok(())
}

async fn processing_packets(
    received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>>,
    file_details_storage: Arc<Mutex<Box<Vec<FileDetailsQ>>>>,
    NUMBER_OF_REPAIR_SYMBOLS: u64,
) {
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
        println!("recv chunks: {}", received_chunks);

        if received_chunks >= (number_of_chunks_expected - NUMBER_OF_REPAIR_SYMBOLS) as usize
            && !file_details.file_decoding_status
        {
            println!("Decoding file: {}", filename);
            file_details.file_decoding_status = true;
            // Try to decode file
            let file_data = file_details.data.clone();
            let encoder_config = file_details.encoder_config;
            let (completed, reconstructed_data) = decode_packets(file_data, encoder_config).await;
            if completed {
                let data = reconstructed_data.unwrap();
                let file_path = format!("./receiving_dir/{}", filename);
                fs::write(file_path, data).unwrap();
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
    loop {
        let mut packets_storage = received_packets.lock().await;
        // -- Receive packets and store them first
        // Buffer depends on MTU limit set
        let mut buffer = vec![0; 1500];
        if packets_storage.len() > 0 {
            udp_service.set_nonblocking(true)?;
            match udp_service.recv_from(&mut buffer) {
                Ok((bytes, _)) => {
                    let recv_data = &buffer[..bytes];
                    packets_storage.push_back(recv_data.to_vec());
                }
                Err(_) => continue,
            }
        } else {
            udp_service.set_nonblocking(false)?;
            match udp_service.recv_from(&mut buffer) {
                Ok((bytes, _)) => {
                    let recv_data = &buffer[..bytes];
                    packets_storage.push_back(recv_data.to_vec());
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}
