//! I2P SAM v3 STREAM CONNECT (system router, not SOCKS).

use crate::error::NetError;
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct I2pSam {
    sam_addr: SocketAddr,
    session_id: String,
    _control: TcpStream,
}

impl I2pSam {
    pub async fn connect(sam_addr: SocketAddr) -> Result<Self, NetError> {
        let mut control = TcpStream::connect(sam_addr)
            .await
            .map_err(|e| NetError::Encode(format!("i2p sam connect {sam_addr}: {e}")))?;
        hello(&mut control).await?;
        let session_id = fresh_session_id();
        write_line(
            &mut control,
            &format!("SESSION CREATE STYLE=STREAM ID={session_id} DESTINATION=TRANSIENT"),
        )
        .await?;
        let reply = read_line(&mut control).await?;
        if !reply.to_ascii_uppercase().contains("RESULT=OK") {
            return Err(NetError::Encode(format!("i2p sam session: {reply}")));
        }
        Ok(Self {
            sam_addr,
            session_id,
            _control: control,
        })
    }

    pub async fn stream_connect(&self, dest_b32: &str) -> Result<TcpStream, NetError> {
        let mut s = TcpStream::connect(self.sam_addr)
            .await
            .map_err(|e| NetError::Encode(format!("i2p sam stream connect: {e}")))?;
        hello(&mut s).await?;
        write_line(
            &mut s,
            &format!(
                "STREAM CONNECT ID={} DESTINATION={}",
                self.session_id, dest_b32
            ),
        )
        .await?;
        let reply = read_line(&mut s).await?;
        if !reply.to_ascii_uppercase().contains("RESULT=OK") {
            return Err(NetError::Encode(format!("i2p sam stream: {reply}")));
        }
        Ok(s)
    }
}

fn fresh_session_id() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("CSPRNG for SAM session id");
    let mut id = String::from("rbtc");
    for x in b {
        id.push_str(&format!("{x:02x}"));
    }
    id
}

async fn hello(s: &mut TcpStream) -> Result<(), NetError> {
    write_line(s, "HELLO VERSION MIN=3.1 MAX=3.3").await?;
    let reply = read_line(s).await?;
    let up = reply.to_ascii_uppercase();
    if !up.starts_with("HELLO REPLY") || !up.contains("RESULT=OK") {
        return Err(NetError::Encode(format!("i2p sam hello: {reply}")));
    }
    Ok(())
}

async fn write_line(s: &mut TcpStream, line: &str) -> Result<(), NetError> {
    s.write_all(line.as_bytes())
        .await
        .map_err(|e| NetError::Encode(format!("i2p sam write: {e}")))?;
    s.write_all(b"\n")
        .await
        .map_err(|e| NetError::Encode(format!("i2p sam write: {e}")))?;
    s.flush()
        .await
        .map_err(|e| NetError::Encode(format!("i2p sam write: {e}")))
}

async fn read_line(s: &mut TcpStream) -> Result<String, NetError> {
    let mut reader = BufReader::new(s);
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|e| NetError::Encode(format!("i2p sam read: {e}")))?;
    if n == 0 {
        return Err(NetError::Encode("i2p sam: connection closed".into()));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    async fn fake_sam(ok_hello: bool, dest_log: Arc<Mutex<Vec<String>>>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    break;
                };
                let log = Arc::clone(&dest_log);
                let ok = ok_hello;
                tokio::spawn(async move {
                    loop {
                        let line = match read_line(&mut s).await {
                            Ok(l) => l,
                            Err(_) => break,
                        };
                        let up = line.to_ascii_uppercase();
                        if up.starts_with("HELLO VERSION") {
                            if ok {
                                let _ =
                                    write_line(&mut s, "HELLO REPLY RESULT=OK VERSION=3.1").await;
                            } else {
                                let _ = write_line(&mut s, "HELLO REPLY RESULT=NOVERSION").await;
                                break;
                            }
                        } else if up.starts_with("SESSION CREATE") {
                            let _ = write_line(&mut s, "SESSION STATUS RESULT=OK DESTINATION=fake")
                                .await;
                        } else if let Some(rest) = line.strip_prefix("STREAM CONNECT ") {
                            log.lock().unwrap().push(rest.to_string());
                            let _ = write_line(&mut s, "STREAM STATUS RESULT=OK").await;
                            break;
                        } else {
                            let _ = write_line(&mut s, "PING").await;
                        }
                    }
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn i2p_sam_stream_connect_fake() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let addr = fake_sam(true, Arc::clone(&log)).await;
        let sam = I2pSam::connect(addr).await.unwrap();
        let dest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.b32.i2p";
        sam.stream_connect(dest).await.unwrap();
        sam.stream_connect(dest).await.unwrap();
        let got = log.lock().unwrap().clone();
        assert_eq!(got.len(), 2, "{got:?}");
        for g in &got {
            assert!(g.contains(dest), "{g}");
            assert!(g.contains("ID=rbtc"), "{g}");
        }

        let bad = fake_sam(false, Arc::new(Mutex::new(Vec::new()))).await;
        let err = match I2pSam::connect(bad).await {
            Err(e) => e,
            Ok(_) => panic!("bad HELLO must fail"),
        };
        let msg = format!("{err}");
        assert!(
            msg.contains("hello") || msg.contains("NOVERSION") || msg.contains("sam"),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn dial_i2p_uses_sam() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let addr = fake_sam(true, Arc::clone(&log)).await;
        let sam = I2pSam::connect(addr).await.unwrap();
        let peer = crate::NetAddr::I2p {
            dest: [0u8; 32],
            port: 8333,
        };
        sam.stream_connect(&peer.host_str()).await.unwrap();
        let got = log.lock().unwrap().clone();
        assert!(got.iter().any(|g| g.contains(&peer.host_str())), "{got:?}");
        let err = match crate::socks::Dialer::Direct.connect_net(peer).await {
            Err(e) => e,
            Ok(_) => panic!("Direct must not dial I2P"),
        };
        let msg = format!("{err}");
        assert!(msg.contains("SAM") || msg.contains("i2p"), "{msg}");
    }
}
