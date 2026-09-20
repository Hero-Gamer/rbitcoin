//! I2P SAM v3 STREAM CONNECT / FORWARD (system router, not SOCKS).

use crate::error::NetError;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct I2pSam {
    sam_addr: SocketAddr,
    session_id: String,
    _control: TcpStream,
    _forward: Option<TcpStream>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct I2pDialer {
    sam_addr: SocketAddr,
    session_id: String,
}

impl I2pSam {
    pub async fn connect(sam_addr: SocketAddr) -> Result<Self, NetError> {
        Self::connect_session(sam_addr, None).await
    }

    pub async fn connect_persistent(
        sam_addr: SocketAddr,
        dest_path: &Path,
    ) -> Result<Self, NetError> {
        let stored = match std::fs::read_to_string(dest_path) {
            Ok(s) => {
                let s = s.trim();
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(NetError::Encode(format!(
                    "i2p dest {}: {e}",
                    dest_path.display()
                )));
            }
        };
        let (sam, dest) = Self::connect_session_dest(sam_addr, stored.as_deref()).await?;
        if stored.is_none() {
            write_dest_file(dest_path, &dest)?;
        }
        Ok(sam)
    }

    async fn connect_session(sam_addr: SocketAddr, dest: Option<&str>) -> Result<Self, NetError> {
        let (sam, _) = Self::connect_session_dest(sam_addr, dest).await?;
        Ok(sam)
    }

    async fn connect_session_dest(
        sam_addr: SocketAddr,
        dest: Option<&str>,
    ) -> Result<(Self, String), NetError> {
        let mut control = TcpStream::connect(sam_addr)
            .await
            .map_err(|e| NetError::Encode(format!("i2p sam connect {sam_addr}: {e}")))?;
        hello(&mut control).await?;
        let session_id = fresh_session_id();
        let dest_arg = dest.unwrap_or("TRANSIENT");
        write_line(
            &mut control,
            &format!("SESSION CREATE STYLE=STREAM ID={session_id} DESTINATION={dest_arg}"),
        )
        .await?;
        let reply = read_line(&mut control).await?;
        if !reply.to_ascii_uppercase().contains("RESULT=OK") {
            return Err(NetError::Encode(format!("i2p sam session: {reply}")));
        }
        let destination = sam_kv(&reply, "DESTINATION")
            .ok_or_else(|| {
                NetError::Encode(format!("i2p sam session missing DESTINATION: {reply}"))
            })?
            .to_string();
        Ok((
            Self {
                sam_addr,
                session_id,
                _control: control,
                _forward: None,
            },
            destination,
        ))
    }

    pub async fn stream_connect(&self, dest_b32: &str) -> Result<TcpStream, NetError> {
        self.dialer().stream_connect(dest_b32).await
    }

    pub fn dialer(&self) -> I2pDialer {
        I2pDialer {
            sam_addr: self.sam_addr,
            session_id: self.session_id.clone(),
        }
    }

    pub async fn stream_forward(&mut self, port: u16) -> Result<(), NetError> {
        let mut s = TcpStream::connect(self.sam_addr)
            .await
            .map_err(|e| NetError::Encode(format!("i2p sam forward connect: {e}")))?;
        hello(&mut s).await?;
        write_line(
            &mut s,
            &format!("STREAM FORWARD ID={} PORT={port}", self.session_id),
        )
        .await?;
        let reply = read_line(&mut s).await?;
        if !reply.to_ascii_uppercase().contains("RESULT=OK") {
            return Err(NetError::Encode(format!("i2p sam forward: {reply}")));
        }
        self._forward = Some(s);
        Ok(())
    }
}

impl I2pDialer {
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

fn sam_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    line.split_whitespace().find_map(|tok| {
        if tok.len() >= prefix.len() && tok[..prefix.len()].eq_ignore_ascii_case(&prefix) {
            Some(&tok[prefix.len()..])
        } else {
            None
        }
    })
}

fn write_dest_file(path: &Path, dest: &str) -> Result<(), NetError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NetError::Encode(format!("i2p dest dir {}: {e}", parent.display())))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| NetError::Encode(format!("i2p dest {}: {e}", path.display())))?;
    writeln!(f, "{dest}")
        .map_err(|e| NetError::Encode(format!("i2p dest {}: {e}", path.display())))?;
    Ok(())
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
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::net::TcpListener;

    const FAKE_DEST: &str = "fakeprivdest";

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
                            log.lock().unwrap().push(line.clone());
                            let dest = sam_kv(&line, "DESTINATION").unwrap_or("TRANSIENT");
                            let reply_dest = if dest.eq_ignore_ascii_case("TRANSIENT") {
                                FAKE_DEST
                            } else {
                                dest
                            };
                            let _ = write_line(
                                &mut s,
                                &format!("SESSION STATUS RESULT=OK DESTINATION={reply_dest}"),
                            )
                            .await;
                        } else if up.starts_with("STREAM FORWARD") {
                            log.lock().unwrap().push(line.clone());
                            let _ = write_line(&mut s, "STREAM STATUS RESULT=OK").await;
                        } else if up.starts_with("STREAM CONNECT") {
                            log.lock().unwrap().push(line.clone());
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

    fn stream_lines(log: &Arc<Mutex<Vec<String>>>, prefix: &str) -> Vec<String> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|g| g.to_ascii_uppercase().starts_with(prefix))
            .cloned()
            .collect()
    }

    #[tokio::test]
    async fn i2p_sam_stream_connect_fake() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let addr = fake_sam(true, Arc::clone(&log)).await;
        let sam = I2pSam::connect(addr).await.unwrap();
        let dest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.b32.i2p";
        sam.stream_connect(dest).await.unwrap();
        sam.stream_connect(dest).await.unwrap();
        let got = stream_lines(&log, "STREAM CONNECT");
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
        let got = stream_lines(&log, "STREAM CONNECT");
        assert!(got.iter().any(|g| g.contains(&peer.host_str())), "{got:?}");
        let err = match crate::socks::Dialer::Direct.connect_net(peer).await {
            Err(e) => e,
            Ok(_) => panic!("Direct must not dial I2P"),
        };
        let msg = format!("{err}");
        assert!(msg.contains("SAM") || msg.contains("i2p"), "{msg}");
    }

    #[tokio::test]
    async fn i2p_accept_incoming_forwards_to_loopback() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let addr = fake_sam(true, Arc::clone(&log)).await;
        let dir = std::env::temp_dir().join(format!(
            "rbtc-i2p-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dest_path = dir.join("i2p").join("p2p.priv");
        let mut sam = I2pSam::connect_persistent(addr, &dest_path).await.unwrap();
        sam.stream_forward(18444).await.unwrap();
        let stored = std::fs::read_to_string(&dest_path).unwrap();
        assert_eq!(stored.trim(), FAKE_DEST);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dest_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let fw = stream_lines(&log, "STREAM FORWARD");
        assert_eq!(fw.len(), 1, "{fw:?}");
        assert!(fw[0].contains("PORT=18444"), "{}", fw[0]);
        assert!(fw[0].contains("ID=rbtc"), "{}", fw[0]);
        let creates = stream_lines(&log, "SESSION CREATE");
        assert!(
            creates.iter().any(|c| c.contains("DESTINATION=TRANSIENT")),
            "{creates:?}"
        );

        let mut sam2 = I2pSam::connect_persistent(addr, &dest_path).await.unwrap();
        sam2.stream_forward(18444).await.unwrap();
        let creates = stream_lines(&log, "SESSION CREATE");
        assert!(
            creates
                .iter()
                .any(|c| c.contains(&format!("DESTINATION={FAKE_DEST}"))),
            "{creates:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
