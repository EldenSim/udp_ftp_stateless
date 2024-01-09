// region:    --- modules

use std::env;

use dotenv::dotenv;
use udp_ftp_stateless::Result;

mod recv;
mod send;

// endregion: --- modules

fn main() -> Result<()> {
    // -- Loading dev env variables
    dotenv().ok();
    let FTP_MODE = env::var("FTP_MODE").expect("FTP_MODE env var not set");
    let LOCAL_ADDRESS = env::var("LOCAL_ADDRESS").expect("LOCAL_ADDRESS env var not set");

    match FTP_MODE.to_lowercase().as_str() {
        // -- Send operation mode
        "send" => {
            send::main()?;
        }
        // -- Recv operation mode
        "recv" => {
            recv::main()?;
        }
        _ => {
            return Err("Invalid FTP_MODE variable, FTP_MODE only operable in RECV or SEND".into());
        }
    }

    Ok(())
}
