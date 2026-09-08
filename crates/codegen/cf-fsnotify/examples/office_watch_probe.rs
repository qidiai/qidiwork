//! Live probe: arm cf_fsnotify on the real office fixture dir and print
//! events while the manifest is rewritten externally.
//!
//! Run: cargo run -p cf-fsnotify --example office_watch_probe

use std::path::PathBuf;
use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let dir = PathBuf::from(r"C:\Users\ASUS\.qidi\office-workspaces\default");
    println!("watching {}", dir.display());
    match cf_fsnotify::shared(dir.clone(), cf_fsnotify::FsConfig::default()) {
        Ok(source) => {
            println!("armed ok");
            let mut rx = source.subscribe();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
            loop {
                tokio::select! {
                    ev = rx.recv() => {
                        match ev {
                            Ok(e) => println!("EVENT: {e:?}"),
                            Err(e) => {
                                println!("RX ERR: {e:?}");
                                break;
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        println!("TIMEOUT: no events in 90s");
                        break;
                    }
                }
            }
        }
        Err(e) => println!("ARM FAILED: {e:?}"),
    }
}
