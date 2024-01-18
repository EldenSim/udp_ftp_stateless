// region:    --- modules

use std::net::UdpSocket;
use std::time::Duration;
use std::{env, thread};

use dotenv::dotenv;
use udp_ftp_stateless::Result;

mod recv;
mod send;
mod udp;

// endregion: --- modules

fn main() -> Result<()> {
    // -- Loading dev env variables
    dotenv().ok();
    let FTP_MODE = env::var("FTP_MODE").expect("FTP_MODE env var not set");
    let LOCAL_ADDRESS = env::var("LOCAL_ADDRESS").expect("LOCAL_ADDRESS env var not set");

    match FTP_MODE.to_lowercase().as_str() {
        // -- Send operation mode
        "send" => {
            let FOREIGN_ADDRESS =
                env::var("FOREIGN_ADDRESS").expect("FOREIGN_ADDRESS env var not set");
            let port = "8000";
            let udp_service = udp::init_udp_service(&LOCAL_ADDRESS, port)?;
            let foreign_port = "8001";
            udp::connect_to_foreign_addr(&udp_service, &FOREIGN_ADDRESS, foreign_port)?;
            for i in 0..1 {
                send::main(&udp_service)?;
                thread::sleep(Duration::from_millis(0));
            }
        }
        // -- Recv operation mode
        "recv" => {
            let port = "8000";
            let udp_service = udp::init_udp_service(&LOCAL_ADDRESS, port)?;
            recv::main(&udp_service)?;
        }
        _ => {
            return Err("Invalid FTP_MODE variable, FTP_MODE only operable in RECV or SEND".into());
        }
    }

    Ok(())
}
