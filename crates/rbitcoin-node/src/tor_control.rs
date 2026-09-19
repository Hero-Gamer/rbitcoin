//! Tor control-port AUTH (cookie or password) for hidden-service setup.

use crate::error::NodeError;
use bitcoin::hex::DisplayHex;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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

    pub async fn add_onion_persistent(
        &mut self,
        key_path: &Path,
        virt: u16,
        target: SocketAddr,
    ) -> Result<HiddenService, NodeError> {
        let stored = match std::fs::read_to_string(key_path) {
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
                return Err(NodeError::Init(format!(
                    "tor control key {}: {e}",
                    key_path.display()
                )));
            }
        };
        let spec = match stored.as_deref() {
            Some(k) => k.to_string(),
            None => "NEW:ED25519-V3".to_string(),
        };
        let cmd = format!("ADD_ONION {spec} Port={virt},{target}");
        let reply = self.command(&cmd).await?;
        let hs = parse_add_onion_reply(&reply)?;
        if stored.is_none() {
            let Some(ref pk) = hs.private_key else {
                return Err(NodeError::Init(
                    "tor control ADD_ONION NEW missing PrivateKey".into(),
                ));
            };
            write_key_file(key_path, pk)?;
        }
        Ok(hs)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HiddenService {
    pub service_id: String,
    pub private_key: Option<String>,
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

fn parse_add_onion_reply(body: &str) -> Result<HiddenService, NodeError> {
    let mut service_id = None;
    let mut private_key = None;
    for line in body.lines() {
        if let Some(id) = line.strip_prefix("ServiceID=") {
            service_id = Some(id.trim().to_string());
        } else if let Some(pk) = line.strip_prefix("PrivateKey=") {
            private_key = Some(pk.trim().to_string());
        }
    }
    let Some(service_id) = service_id else {
        return Err(NodeError::Init(
            "tor control ADD_ONION missing ServiceID".into(),
        ));
    };
    Ok(HiddenService {
        service_id,
        private_key,
    })
}

fn write_key_file(path: &Path, key: &str) -> Result<(), NodeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            NodeError::Init(format!("tor control key dir {}: {e}", parent.display()))
        })?;
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
        .map_err(|e| NodeError::Init(format!("tor control key {}: {e}", path.display())))?;
    writeln!(f, "{key}")
        .map_err(|e| NodeError::Init(format!("tor control key {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hex::DisplayHex;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const FAKE_SID: &str = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd";
    const FAKE_PK: &str = "ED25519-V3:dGVzdGtleWJsb2I";

    async fn fake_control(
        cookie: Option<Vec<u8>>,
        password: Option<String>,
    ) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log_task = Arc::clone(&log);
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
                log_task.lock().unwrap().push(line.clone());
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
                } else if let Some(rest) = line.strip_prefix("ADD_ONION ") {
                    let spec = rest.split_once(" Port=").map(|(s, _)| s).unwrap_or(rest);
                    if spec == "NEW:ED25519-V3" {
                        w.write_all(
                            format!("250-ServiceID={FAKE_SID}\r\n250-PrivateKey={FAKE_PK}\r\n250 OK\r\n")
                                .as_bytes(),
                        )
                        .await
                        .unwrap();
                    } else if spec == FAKE_PK {
                        w.write_all(format!("250-ServiceID={FAKE_SID}\r\n250 OK\r\n").as_bytes())
                            .await
                            .unwrap();
                    } else {
                        w.write_all(b"512 Invalid onion key\r\n").await.unwrap();
                    }
                } else {
                    w.write_all(b"510 Unrecognized command\r\n").await.unwrap();
                }
            }
        });
        (addr, log)
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

    fn tmp_key_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "rbtc-tor-onion-{}-{}/electrum.priv",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn tor_control_auth_cookie_and_password() {
        let cookie = vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x23];
        let (addr, _) = fake_control(Some(cookie.clone()), None).await;
        let path = tmp_cookie(&cookie);
        TorControl::connect_and_auth(addr, TorAuth::Cookie(path.clone()))
            .await
            .unwrap();
        let _ = std::fs::remove_file(&path);

        let (addr, _) = fake_control(None, Some("s3cret".into())).await;
        TorControl::connect_and_auth(addr, TorAuth::Password("s3cret".into()))
            .await
            .unwrap();

        let (addr, _) = fake_control(Some(cookie.clone()), None).await;
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

    #[tokio::test]
    async fn tor_add_onion_new_persists_key() {
        let cookie = vec![0x11, 0x22];
        let (addr, _) = fake_control(Some(cookie.clone()), None).await;
        let cookie_path = tmp_cookie(&cookie);
        let mut ctl = TorControl::connect_and_auth(addr, TorAuth::Cookie(cookie_path.clone()))
            .await
            .unwrap();
        let key_path = tmp_key_path();
        let target: SocketAddr = "127.0.0.1:50001".parse().unwrap();
        let hs = ctl
            .add_onion_persistent(&key_path, 50001, target)
            .await
            .unwrap();
        assert_eq!(hs.service_id, FAKE_SID);
        let stored = std::fs::read_to_string(&key_path).unwrap();
        assert_eq!(stored.trim(), FAKE_PK);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_file(&cookie_path);
        let _ = std::fs::remove_dir_all(key_path.parent().unwrap());
    }

    #[tokio::test]
    async fn tor_add_onion_reuse_key_same_id() {
        let cookie = vec![0x33, 0x44];
        let (addr, log) = fake_control(Some(cookie.clone()), None).await;
        let cookie_path = tmp_cookie(&cookie);
        let mut ctl = TorControl::connect_and_auth(addr, TorAuth::Cookie(cookie_path.clone()))
            .await
            .unwrap();
        let key_path = tmp_key_path();
        let target: SocketAddr = "127.0.0.1:50001".parse().unwrap();
        let first = ctl
            .add_onion_persistent(&key_path, 50001, target)
            .await
            .unwrap();
        let second = ctl
            .add_onion_persistent(&key_path, 50001, target)
            .await
            .unwrap();
        assert_eq!(first.service_id, second.service_id);
        assert_eq!(second.service_id, FAKE_SID);
        let cmds = log.lock().unwrap().clone();
        let onions: Vec<_> = cmds
            .iter()
            .filter(|c| c.starts_with("ADD_ONION "))
            .cloned()
            .collect();
        assert_eq!(onions.len(), 2, "{onions:?}");
        assert!(
            onions[0].starts_with("ADD_ONION NEW:ED25519-V3 "),
            "{}",
            onions[0]
        );
        assert!(
            onions[1].starts_with(&format!("ADD_ONION {FAKE_PK} ")),
            "{}",
            onions[1]
        );
        let _ = std::fs::remove_file(&cookie_path);
        let _ = std::fs::remove_dir_all(key_path.parent().unwrap());
    }
}
