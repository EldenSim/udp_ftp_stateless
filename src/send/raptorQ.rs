use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use raptorq::{Encoder, ObjectTransmissionInformation};
use serde::de;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize};

use udp_ftp_stateless::PacketQ;

// -- Encoding Configuration Variables
pub struct EncoderConfigRQ {
    pub mtu: u16,
    pub number_of_repair_symbols: u32,
}

// Following section details the packet structure of RaptorQ implementation

pub fn raptorQ_main(udp_service: &UdpSocket) -> Result<(), Box<dyn Error>> {
    let encoder_config = init_raptorQ()?;
    // region:    --- Loop through sending folder
    let SENDING_DIRECTORY =
        env::var("SENDING_DIRECTORY").expect("SENDING_DIRECTORY env var not set");
    let DELAY_PER_FILE = env::var("DELAY_PER_FILE")
        .expect("DELAY_PER_FILE env var not set")
        .parse::<u64>()?;
    for entry in fs::read_dir(SENDING_DIRECTORY)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            unimplemented!()
        } else {
            send_file_with_raptorQ(&path, udp_service, &encoder_config)?;
        }
        thread::sleep(Duration::from_millis(DELAY_PER_FILE))
    }
    Ok(())
}

fn init_raptorQ() -> Result<EncoderConfigRQ, Box<dyn Error>> {
    let NUMBER_OF_REPAIR_SYMBOLS = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<u32>()?;
    let MTU = env::var("NUMBER_OF_REPAIR_SYMBOLS")
        .expect("NUMBER_OF_REPAIR_SYMBOLS env var not set")
        .parse::<u16>()?;
    let encoder_config = EncoderConfigRQ {
        number_of_repair_symbols: NUMBER_OF_REPAIR_SYMBOLS,
        mtu: MTU,
    };
    Ok(encoder_config)
}

fn send_file_with_raptorQ(
    path: &PathBuf,
    udp_service: &UdpSocket,
    encoder_config: &EncoderConfigRQ,
) -> Result<(), Box<dyn Error>> {
    // -- Obtain Filename and file data
    let filename = path.file_name().unwrap().to_str().unwrap().to_string();
    println!("->> Sending file: {}", filename);
    let file = fs::read(path)?;
    let filesize = file.len();

    // Create the Encoder, with an MTU of 1390 (max mtu 1400 - 4 bytes for header)
    let encoder = Encoder::with_defaults(&file, encoder_config.mtu);
    let init_encoder_config: raptorq::ObjectTransmissionInformation = encoder.get_config();
    // Perform the encoding, and serialize to Vec<u8> for transmission
    let packets: Vec<Vec<u8>> = encoder
        .get_encoded_packets(encoder_config.number_of_repair_symbols)
        .iter()
        .map(|packet| packet.serialize())
        .collect();

    for (i, packet_data) in packets.iter().enumerate() {
        let packet = PacketQ {
            filename: filename.clone(),
            filesize: filesize as u64,
            chunk_id: i as u64,
            encoder_config: init_encoder_config.clone().serialize(),
            data: packet_data.to_vec(),
        };
        let packet_bytes = bincode::serialize(&packet)?;
        udp_service.send(&packet_bytes).expect("Send packet error");
    }
    Ok(())
}
