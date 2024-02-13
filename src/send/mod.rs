#![allow(non_snake_case)]

use std::net::UdpSocket;

use udp_ftp_stateless::Result;

mod raptor;
use raptor::raptor_main;
mod raptorQ;
use raptorQ::raptorQ_main;

pub fn main(udp_service: &UdpSocket) -> Result<()> {
    // raptor encoding
    // raptor_main(udp_service)?;

    // raptorQ encoding
    raptorQ_main(udp_service)?;

    Ok(())
}
