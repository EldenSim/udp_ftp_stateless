use std::collections::VecDeque;
use std::env;
use std::net::UdpSocket;

use async_std::sync::{Arc, Mutex};
use async_std::task;

use udp_ftp_stateless::{PacketQ, Result};
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

    let pointer_2_received_packets = Arc::clone(&received_packets);

    task::Builder::new()
        .name("Processing Task".to_string())
        .spawn(async move { processing_packets(pointer_2_received_packets).await })?;

    recv_packets(&udp_service, received_packets).await.unwrap();
    todo!();
    Ok(())
}

async fn processing_packets(received_packets: Arc<Mutex<Box<VecDeque<Vec<u8>>>>>) {
    loop {
        let mut packets_to_parses = received_packets.lock().await;
        let packet_bytes = match packets_to_parses.front() {
            Some(bytes) => bytes.clone(),
            None => continue,
        };
        packets_to_parses.pop_front();
        let packet: PacketQ = bincode::deserialize(&packet_bytes).unwrap();
    }
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
        let mut buffer = vec![0; MTU + 20];
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
