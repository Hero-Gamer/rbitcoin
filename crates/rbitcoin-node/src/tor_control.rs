//! Tor control-port AUTH (cookie or password) for hidden-service setup.

use crate::error::NodeError;
use bitcoin::hex::DisplayHex;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

#[derive(Clone, Debug)]
pub enum TorAuth {
    Cookie(PathBuf),
    Password(String),
}

pub struct TorControl {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl TorControl {
    pub async fn connect_and_auth(addr: SocketAddr, auth: TorAuth) -> Result<Self, NodeError> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| NodeError::Init(format!("tor control connect {addr}: {e}")))?;
        let (r, w) = stream.into_split();
        let mut ctl = Self {
            reader: BufReader::new(r),
            writer: w,
        };
        ctl.authenticate(&auth).await?;
        ctl.command("GETINFO version").await?;
        Ok(ctl)
    }

    async fn authenticate(&mut self, auth: &TorAuth) -> Result<String, NodeError> {
        let line = match auth {
            TorAuth::Cookie(path) => {
                let bytes = std::fs::read(path).map_err(|e| {
                    NodeError::Init(format!("tor control cookie {}: {e}", path.display()))
                })?;
                format!("AUTHENTICATE {}", bytes.to_lower_hex_string())
            }
            TorAuth::Password(p) => format!("AUTHENTICATE \"{}\"", escape_quoted(p)),
        };
        self.command(&line).await
    }

    pub async fn command(&mut self, cmd: &str) -> Result<String, NodeError> {
        self.writer
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| NodeError::Init(format!("tor control write: {e}")))?;
        self.writer
            .write_all(b"\r\n")
            .await
            .map_err(|e| NodeError::Init(format!("tor control write: {e}")))?;
        self.writer
            .flush()
            .await
            .map_err(|e| NodeError::Init(format!("tor control write: {e}")))?;
        read_reply(&mut self.reader).await
    }
}

fn escape_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

async fn read_reply(reader: &mut BufReader<OwnedReadHalf>) -> Result<String, NodeError> {
    let mut body = String::new();
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| NodeError::Init(format!("tor control read: {e}")))?;
        if n == 0 {
            return Err(NodeError::Init("tor control: connection closed".into()));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.len() < 4 {
            return Err(NodeError::Init(format!(
                "tor control: short reply `{line}`"
            )));
        }
        let code = &line[..3];
        let sep = line.as_bytes()[3];
        let rest = &line[4..];
        if code.as_bytes()[0] == b'5' {
            return Err(NodeError::Init(format!("tor control: {line}")));
        }
        match sep {
            b'-' => {
                body.push_str(rest);
                body.push('\n');
            }
            b' ' => {
                if !rest.is_empty() {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(rest);
                }
                return Ok(body);
            }
            _ => {
                return Err(NodeError::Init(format!(
                    "tor control: unexpected reply `{line}`"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hex::DisplayHex;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn fake_control(cookie: Option<Vec<u8>>, password: Option<String>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let (r, mut w) = s.split();
            let mut reader = BufReader::new(r);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let line = line.trim_end_matches(['\r', '\n']).to_string();
                if let Some(rest) = line.strip_prefix("AUTHENTICATE ") {
                    let ok = if let Some(ref want) = cookie {
                        rest.eq_ignore_ascii_case(&want.to_lower_hex_string())
                    } else if let Some(ref want) = password {
                        rest == format!("\"{}\"", escape_quoted(want))
                    } else {
                        false
                    };
                    if ok {
                        w.write_all(b"250 OK\r\n").await.unwrap();
                    } else {
                        w.write_all(b"515 Authentication failed\r\n").await.unwrap();
                    }
                } else if line.eq_ignore_ascii_case("GETINFO version") {
                    w.write_all(b"250-version=0.4.8.10\r\n250 OK\r\n")
                        .await
                        .unwrap();
                } else {
                    w.write_all(b"510 Unrecognized command\r\n").await.unwrap();
                }
            }
        });
        addr
    }

    fn tmp_cookie(bytes: &[u8]) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rbtc-tor-cookie-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[tokio::test]
    async fn tor_control_auth_cookie_and_password() {
        let cookie = vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x23];
        let addr = fake_control(Some(cookie.clone()), None).await;
        let path = tmp_cookie(&cookie);
        TorControl::connect_and_auth(addr, TorAuth::Cookie(path.clone()))
            .await
            .unwrap();
        let _ = std::fs::remove_file(&path);

        let addr = fake_control(None, Some("s3cret".into())).await;
        TorControl::connect_and_auth(addr, TorAuth::Password("s3cret".into()))
            .await
            .unwrap();

        let addr = fake_control(Some(cookie.clone()), None).await;
        let bad = tmp_cookie(&[0x00, 0x01]);
        let err = match TorControl::connect_and_auth(addr, TorAuth::Cookie(bad.clone())).await {
            Err(e) => e,
            Ok(_) => panic!("wrong cookie must not authenticate"),
        };
        let msg = format!("{err}");
        assert!(
            msg.contains("515") || msg.contains("Authentication failed"),
            "{msg}"
        );
        let _ = std::fs::remove_file(&bad);
    }
}
