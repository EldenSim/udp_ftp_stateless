// region:    --- Modules

use std::net::UdpSocket;

use udp_ftp_stateless::Result;

// endregion: --- Modules

pub fn init_udp_service(LOCAL_ADDRESS: &str, port: &str) -> Result<UdpSocket> {
    // -- Initialise UDP port
    // let port = 8000;
    let udp_addr = format!("{}:{}", LOCAL_ADDRESS, port);
    let udp_service = UdpSocket::bind(&udp_addr)?;
    println!("UDP Socket initialised at {}", udp_addr);
    Ok(udp_service)
}

pub fn connect_to_foreign_addr(
    udp_service: &UdpSocket,
    FOREIGN_ADDRESS: &str,
    foreign_port: &str,
) -> Result<()> {
    // -- Connect to sending address
    // let foreign_port = 8001;
    let foreign_addr = format!("{}:{}", FOREIGN_ADDRESS, foreign_port);
    udp_service.connect(&foreign_addr)?;
    println!("UDP Socket connected to {}", foreign_addr);
    Ok(())
}
