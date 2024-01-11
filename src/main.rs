// region:    --- modules

use std::env;
use std::net::UdpSocket;

use dotenv::dotenv;
use udp_ftp_stateless::Result;

mod recv;
mod send;
mod udp;

// endregion: --- modules

#[async_std::main]
async fn main() -> Result<()> {
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

            send::main(&udp_service)?;
        }
        // -- Recv operation mode
        "recv" => {
            let port = "8001";
            let udp_service = udp::init_udp_service(&LOCAL_ADDRESS, port)?;
            recv::main(&udp_service).await?;
        }
        _ => {
            return Err("Invalid FTP_MODE variable, FTP_MODE only operable in RECV or SEND".into());
        }
    }

    Ok(())
}
