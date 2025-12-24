use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::sleep;
use tracing::{debug, info, warn};

pub struct TcpProxy {
    local_port: u16,
    remote: String,
}

impl TcpProxy {
    pub fn new(local_port: u16, remote: String) -> Self {
        Self { local_port, remote }
    }

    pub async fn run_until_ctrl_c(&self, delay: Duration) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", self.local_port))
            .await
            .with_context(|| format!("bind to 127.0.0.1:{}", self.local_port))?;

        info!(port = self.local_port, "Proxy listener ready");
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("CTRL+C received; shutting proxy down");
                    break;
                }
                incoming = listener.accept() => {
                    let (socket, addr) = incoming?;
                    let remote = self.remote.clone();
                    tokio::spawn(async move {
                        if let Err(err) = handle(socket, addr, &remote).await {
                            warn!(%addr, %remote, "proxy session failed: {err}");
                        }
                    });
                }
            }

            sleep(delay).await;
        }

        Ok(())
    }
}

async fn handle(mut inbound: TcpStream, client: SocketAddr, remote: &str) -> Result<()> {
    debug!(%client, %remote, "proxy session starting");
    let mut outbound = TcpStream::connect(remote)
        .await
        .with_context(|| format!("connect to {remote}"))?;

    copy_bidirectional(&mut inbound, &mut outbound)
        .await
        .context("copy data between client and provider")?;

    Ok(())
}
