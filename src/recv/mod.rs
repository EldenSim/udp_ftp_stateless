use async_std::sync::{Arc, Mutex};
use async_std::task;
use std::collections::VecDeque;
use std::env;
use std::net::UdpSocket;
use udp_ftp_stateless::Result;
mod models;
use models::FileDetails;

pub async fn main(udp_service: UdpSocket) -> Result<()> {
    // -- Obtain MTU and raptor decoding settings, must be the same as sender settings
    let NUMBER_OF_REPAIR_SYMBOLS = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<u64>()?;
    let MAX_SOURCE_SYMBOL_SIZE = env::var("MAX_SOURCE_SYMBOL_SIZE")
        .expect("MAX_SOURCE_SYMBOL_SIZE env var not set")
        .parse::<usize>()?;
    // let MTU = env::var("MTU")
    //     .expect("MTU env var not set")
    //     .parse::<usize>()?;
    // let PROCESSING_STORAGE = env::var("PROCESSING_STORAGE")
    //     .expect("PROCESSING_STORAGE env var not set")
    //     .parse::<usize>()?;

    let file_details_storage: Arc<Mutex<Box<Vec<FileDetails>>>> =
        Arc::new(Mutex::new(Box::new(Vec::new())));

    let received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>> =
        Arc::new(Mutex::new(Box::new(VecDeque::new())));

    // let pointer_received_packets = Arc::clone(&received_packets);
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

    recv_packets(&udp_service, received_packets).await.unwrap();
    todo!();
    Ok(())
}

async fn processing_packets(
    received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>>,
    file_details_storage: Arc<Mutex<Box<Vec<FileDetails>>>>,
    NUMBER_OF_REPAIR_SYMBOLS: u64,
    MAX_SOURCE_SYMBOL_SIZE: usize,
) {
    todo!()
}

async fn recv_packets(
    udp_service: &UdpSocket,
    received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>>,
) -> Result<()> {
    todo!()
}
