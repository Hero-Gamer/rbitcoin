//! Tor control-port AUTH (cookie or password) for hidden-service setup.

use crate::error::NodeError;
use bitcoin::hex::DisplayHex;
use bitcoin_hashes::{hmac, sha256, Hash, HashEngine};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

const SAFECOOKIE_SERVER_KEY: &[u8] = b"Tor safe cookie authentication server-to-controller hash";
const SAFECOOKIE_CLIENT_KEY: &[u8] = b"Tor safe cookie authentication controller-to-server hash";

pub const DEFAULT_CONTROL_PORT: u16 = 9051;
pub const DEFAULT_COOKIE_PATH: &str = "/run/tor/control.authcookie";

pub fn default_control_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], DEFAULT_CONTROL_PORT))
}

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

    pub async fn connect_if_configured(
        addr: Option<SocketAddr>,
        cookie: Option<&Path>,
        password: Option<&str>,
    ) -> Result<Option<Self>, NodeError> {
        let Some(addr) = addr else {
            return Ok(None);
        };
        let auth = match password {
            Some(p) if !p.is_empty() => TorAuth::Password(p.to_string()),
            _ => TorAuth::Cookie(
                cookie
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_COOKIE_PATH)),
            ),
        };
        Ok(Some(Self::connect_and_auth(addr, auth).await?))
    }

    async fn authenticate(&mut self, auth: &TorAuth) -> Result<String, NodeError> {
        match auth {
            TorAuth::Password(p) => {
                let line = format!("AUTHENTICATE \"{}\"", escape_quoted(p));
                self.command(&line).await
            }
            TorAuth::Cookie(path) => {
                let info = self.protocol_auth_info().await?;
                if info.methods.iter().any(|m| m == "SAFECOOKIE") {
                    self.authenticate_safecookie(path).await
                } else {
                    Err(NodeError::Init(
                        "tor control: SAFECOOKIE is not advertised; refusing plain cookie AUTHENTICATE"
                            .into(),
                    ))
                }
            }
        }
    }

    async fn authenticate_safecookie(&mut self, path: &Path) -> Result<String, NodeError> {
        let cookie = std::fs::read(path)
            .map_err(|e| NodeError::Init(format!("tor control cookie {}: {e}", path.display())))?;
        if cookie.len() != 32 {
            return Err(NodeError::Init(
                "tor control: SAFECOOKIE cookie must be 32 bytes".into(),
            ));
        }
        let mut client_nonce = [0u8; 32];
        getrandom::fill(&mut client_nonce).map_err(|e| {
            NodeError::Init(format!("tor control SAFECOOKIE client nonce rng: {e}"))
        })?;
        let reply = self
            .command(&format!(
                "AUTHCHALLENGE SAFECOOKIE {}",
                client_nonce.to_lower_hex_string()
            ))
            .await?;
        let challenge = parse_authchallenge_reply(&reply)?;
        let mut mat = Vec::with_capacity(96);
        mat.extend_from_slice(&cookie);
        mat.extend_from_slice(&client_nonce);
        mat.extend_from_slice(&challenge.server_nonce);
        let want_server = hmac_sha256(SAFECOOKIE_SERVER_KEY, &mat);
        if want_server != challenge.server_hash {
            return Err(NodeError::Init(format!(
                "tor control SAFECOOKIE server hash mismatch want={} got={}",
                want_server.to_lower_hex_string(),
                challenge.server_hash.to_lower_hex_string()
            )));
        }
        let client = hmac_sha256(SAFECOOKIE_CLIENT_KEY, &mat);
        self.command(&format!("AUTHENTICATE {}", client.to_lower_hex_string()))
            .await
    }

    async fn protocol_auth_info(&mut self) -> Result<ProtocolAuthInfo, NodeError> {
        let body = self.command("PROTOCOLINFO 1").await?;
        parse_protocol_auth_info(&body)
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

    pub async fn add_named_onion(
        &mut self,
        datadir: &Path,
        name: &str,
        bound: SocketAddr,
    ) -> Result<HiddenService, NodeError> {
        let key_path = datadir.join("onion").join(format!("{name}.priv"));
        let virt = bound.port();
        let target = SocketAddr::from(([127, 0, 0, 1], virt));
        self.add_onion_persistent(&key_path, virt, target).await
    }

    pub async fn add_electrum_onion(
        &mut self,
        datadir: &Path,
        bound: SocketAddr,
    ) -> Result<HiddenService, NodeError> {
        self.add_named_onion(datadir, "electrum", bound).await
    }

    pub async fn add_esplora_onion(
        &mut self,
        datadir: &Path,
        bound: SocketAddr,
    ) -> Result<HiddenService, NodeError> {
        self.add_named_onion(datadir, "esplora", bound).await
    }

    pub async fn add_p2p_onion(
        &mut self,
        datadir: &Path,
        bound: SocketAddr,
        virt: u16,
    ) -> Result<HiddenService, NodeError> {
        let key_path = datadir.join("onion").join("p2p.priv");
        let target = SocketAddr::from(([127, 0, 0, 1], bound.port()));
        self.add_onion_persistent(&key_path, virt, target).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HiddenService {
    pub service_id: String,
    pub private_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolAuthInfo {
    methods: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SafeCookieChallenge {
    server_hash: [u8; 32],
    server_nonce: [u8; 32],
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

fn parse_protocol_auth_info(body: &str) -> Result<ProtocolAuthInfo, NodeError> {
    for line in body.lines() {
        if !line.starts_with("AUTH ") {
            continue;
        }
        let methods = kv_token(line, "METHODS")
            .ok_or_else(|| NodeError::Init(format!("tor control AUTH METHODS missing: {line}")))?
            .split(',')
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        return Ok(ProtocolAuthInfo { methods });
    }
    Err(NodeError::Init(
        "tor control PROTOCOLINFO missing AUTH line".into(),
    ))
}

fn parse_authchallenge_reply(body: &str) -> Result<SafeCookieChallenge, NodeError> {
    let mut server_hash = None;
    let mut server_nonce = None;
    for line in body.lines() {
        if let Some(v) = kv_token(line, "SERVERHASH") {
            server_hash = Some(hex32(v, "SERVERHASH")?);
        }
        if let Some(v) = kv_token(line, "SERVERNONCE") {
            server_nonce = Some(hex32(v, "SERVERNONCE")?);
        }
    }
    let Some(server_hash) = server_hash else {
        return Err(NodeError::Init(
            "tor control AUTHCHALLENGE missing SERVERHASH".into(),
        ));
    };
    let Some(server_nonce) = server_nonce else {
        return Err(NodeError::Init(
            "tor control AUTHCHALLENGE missing SERVERNONCE".into(),
        ));
    };
    Ok(SafeCookieChallenge {
        server_hash,
        server_nonce,
    })
}

fn kv_token<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|tok| tok.strip_prefix(&format!("{key}=")))
}

fn hex32(s: &str, field: &str) -> Result<[u8; 32], NodeError> {
    if s.len() != 64 {
        return Err(NodeError::Init(format!(
            "tor control {field} must be 64 hex chars"
        )));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_nybble(chunk[0])
            .ok_or_else(|| NodeError::Init(format!("tor control {field} has non-hex content")))?;
        let lo = hex_nybble(chunk[1])
            .ok_or_else(|| NodeError::Init(format!("tor control {field} has non-hex content")))?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nybble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut engine = hmac::HmacEngine::<sha256::Hash>::new(key);
    engine.input(data);
    hmac::Hmac::<sha256::Hash>::from_engine(engine).to_byte_array()
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
            let mut safecookie_expected: Option<[u8; 32]> = None;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let line = line.trim_end_matches(['\r', '\n']).to_string();
                log_task.lock().unwrap().push(line.clone());
                if line.eq_ignore_ascii_case("PROTOCOLINFO 1") {
                    if cookie.is_some() {
                        w.write_all(
                            format!(
                                "250-PROTOCOLINFO 1\r\n250-AUTH METHODS=SAFECOOKIE COOKIEFILE=\"{}\"\r\n250-VERSION Tor=\"0.4.8.10\"\r\n250 OK\r\n",
                                DEFAULT_COOKIE_PATH
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                    } else if password.is_some() {
                        w.write_all(
                            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=HASHEDPASSWORD\r\n250-VERSION Tor=\"0.4.8.10\"\r\n250 OK\r\n",
                        )
                        .await
                        .unwrap();
                    } else {
                        w.write_all(
                            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=NULL\r\n250-VERSION Tor=\"0.4.8.10\"\r\n250 OK\r\n",
                        )
                        .await
                        .unwrap();
                    }
                } else if let Some(rest) = line.strip_prefix("AUTHCHALLENGE SAFECOOKIE ") {
                    let Some(ref cookie_bytes) = cookie else {
                        w.write_all(b"515 Authentication failed\r\n").await.unwrap();
                        continue;
                    };
                    let client_nonce = match hex32(rest, "CLIENTNONCE") {
                        Ok(v) => v,
                        Err(_) => {
                            w.write_all(b"512 Bad client nonce\r\n").await.unwrap();
                            continue;
                        }
                    };
                    let server_nonce = [0x5au8; 32];
                    let mut mat = Vec::with_capacity(96);
                    mat.extend_from_slice(cookie_bytes);
                    mat.extend_from_slice(&client_nonce);
                    mat.extend_from_slice(&server_nonce);
                    let server_hash = hmac_sha256(SAFECOOKIE_SERVER_KEY, &mat);
                    safecookie_expected = Some(hmac_sha256(SAFECOOKIE_CLIENT_KEY, &mat));
                    w.write_all(
                        format!(
                            "250 AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n",
                            server_hash.to_lower_hex_string(),
                            server_nonce.to_lower_hex_string()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                } else if let Some(rest) = line.strip_prefix("AUTHENTICATE ") {
                    let ok = if let Some(ref want) = cookie {
                        rest.eq_ignore_ascii_case(&want.to_lower_hex_string())
                            || safecookie_expected.as_ref().is_some_and(|v| {
                                rest.eq_ignore_ascii_case(&v.to_lower_hex_string())
                            })
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

    async fn fake_control_bad_safecookie() -> SocketAddr {
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
                if line.eq_ignore_ascii_case("PROTOCOLINFO 1") {
                    w.write_all(
                        b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=SAFECOOKIE\r\n250-VERSION Tor=\"0.4.8.10\"\r\n250 OK\r\n",
                    )
                    .await
                    .unwrap();
                } else if line.starts_with("AUTHCHALLENGE SAFECOOKIE ") {
                    let bad_hash = [0u8; 32];
                    let nonce = [1u8; 32];
                    w.write_all(
                        format!(
                            "250 AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n",
                            bad_hash.to_lower_hex_string(),
                            nonce.to_lower_hex_string()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                } else if line.starts_with("AUTHENTICATE ") {
                    w.write_all(b"515 Authentication failed\r\n").await.unwrap();
                } else {
                    w.write_all(b"510 Unrecognized command\r\n").await.unwrap();
                }
            }
        });
        addr
    }

    async fn fake_control_cookie_only(cookie: Vec<u8>) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
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
                if line.eq_ignore_ascii_case("PROTOCOLINFO 1") {
                    w.write_all(
                        format!(
                            "250-PROTOCOLINFO 1\r\n250-AUTH METHODS=COOKIE COOKIEFILE=\"{}\"\r\n250 OK\r\n",
                            DEFAULT_COOKIE_PATH
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                } else if let Some(rest) = line.strip_prefix("AUTHENTICATE ") {
                    if rest.eq_ignore_ascii_case(&cookie.to_lower_hex_string()) {
                        w.write_all(b"250 OK\r\n").await.unwrap();
                    } else {
                        w.write_all(b"515 Authentication failed\r\n").await.unwrap();
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
        let cookie = vec![0x2a; 32];
        let (addr, log) = fake_control(Some(cookie.clone()), None).await;
        let path = tmp_cookie(&cookie);
        TorControl::connect_and_auth(addr, TorAuth::Cookie(path.clone()))
            .await
            .unwrap();
        let cmds = log.lock().unwrap().clone();
        assert!(cmds.iter().any(|c| c == "PROTOCOLINFO 1"), "{cmds:?}");
        let _ = std::fs::remove_file(&path);

        let (addr, _) = fake_control(None, Some("s3cret".into())).await;
        TorControl::connect_and_auth(addr, TorAuth::Password("s3cret".into()))
            .await
            .unwrap();

        let (addr, _) = fake_control(Some(cookie.clone()), None).await;
        let bad = tmp_cookie(&[0x00; 32]);
        let err = match TorControl::connect_and_auth(addr, TorAuth::Cookie(bad.clone())).await {
            Err(e) => e,
            Ok(_) => panic!("wrong cookie must not authenticate"),
        };
        let msg = format!("{err}");
        assert!(
            msg.contains("515")
                || msg.contains("Authentication failed")
                || msg.contains("server hash mismatch"),
            "{msg}"
        );
        let _ = std::fs::remove_file(&bad);
    }

    #[tokio::test]
    async fn tor_add_onion_new_persists_key() {
        let cookie = vec![0x11; 32];
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
        let cookie = vec![0x33; 32];
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

    #[tokio::test]
    async fn tor_control_auth_fail_is_start_error() {
        let cookie = vec![0xaa; 32];
        let (addr, _) = fake_control(Some(cookie.clone()), None).await;
        let bad = tmp_cookie(&[0x00; 32]);
        let err =
            match TorControl::connect_if_configured(Some(addr), Some(bad.as_path()), None).await {
                Err(e) => e,
                Ok(_) => panic!("bad cookie must fail start"),
            };
        let msg = format!("{err}");
        assert!(
            msg.contains("515")
                || msg.contains("Authentication failed")
                || msg.contains("tor control"),
            "{msg}"
        );
        let none = TorControl::connect_if_configured(None, None, None)
            .await
            .unwrap();
        assert!(none.is_none());
        let _ = std::fs::remove_file(&bad);
    }

    #[tokio::test]
    async fn tor_control_safecookie_serverhash_mismatch_fails() {
        let cookie = vec![0x11; 32];
        let addr = fake_control_bad_safecookie().await;
        let path = tmp_cookie(&cookie);
        let err = match TorControl::connect_and_auth(addr, TorAuth::Cookie(path.clone())).await {
            Ok(_) => panic!("bad SAFECOOKIE server hash must fail"),
            Err(e) => e,
        };
        let msg = format!("{err}");
        assert!(msg.contains("server hash mismatch"), "{msg}");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn electrum_hidden_service_add_onion_when_listening() {
        let cookie = vec![0x55; 32];
        let (addr, log) = fake_control(Some(cookie.clone()), None).await;
        let cookie_path = tmp_cookie(&cookie);
        let mut ctl = TorControl::connect_and_auth(addr, TorAuth::Cookie(cookie_path.clone()))
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "rbtc-tor-electrum-hs-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bound: SocketAddr = "127.0.0.1:50001".parse().unwrap();
        let hs = ctl.add_electrum_onion(&dir, bound).await.unwrap();
        assert_eq!(hs.service_id, FAKE_SID);
        let cmds = log.lock().unwrap().clone();
        let onion = cmds
            .iter()
            .find(|c| c.starts_with("ADD_ONION "))
            .cloned()
            .expect("ADD_ONION");
        assert!(onion.contains("Port=50001,127.0.0.1:50001"), "{onion}");
        assert!(std::path::Path::new(&dir.join("onion").join("electrum.priv")).is_file());
        let _ = std::fs::remove_file(&cookie_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn esplora_hidden_service_add_onion() {
        let cookie = vec![0x77; 32];
        let (addr, log) = fake_control(Some(cookie.clone()), None).await;
        let cookie_path = tmp_cookie(&cookie);
        let mut ctl = TorControl::connect_and_auth(addr, TorAuth::Cookie(cookie_path.clone()))
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "rbtc-tor-esplora-hs-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bound: SocketAddr = "127.0.0.1:3000".parse().unwrap();
        let hs = ctl.add_esplora_onion(&dir, bound).await.unwrap();
        assert_eq!(hs.service_id, FAKE_SID);
        let cmds = log.lock().unwrap().clone();
        let onion = cmds
            .iter()
            .find(|c| c.starts_with("ADD_ONION "))
            .cloned()
            .expect("ADD_ONION");
        assert!(onion.contains("Port=3000,127.0.0.1:3000"), "{onion}");
        assert!(std::path::Path::new(&dir.join("onion").join("esplora.priv")).is_file());
        let _ = std::fs::remove_file(&cookie_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn p2p_add_onion_persists_key() {
        let cookie = vec![0x21; 32];
        let (addr, log) = fake_control(Some(cookie.clone()), None).await;
        let cookie_path = tmp_cookie(&cookie);
        let mut ctl = TorControl::connect_and_auth(addr, TorAuth::Cookie(cookie_path.clone()))
            .await
            .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "rbtc-tor-p2p-hs-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bound: SocketAddr = "127.0.0.1:23456".parse().unwrap();
        let hs = ctl.add_p2p_onion(&dir, bound, 18444).await.unwrap();
        assert_eq!(hs.service_id, FAKE_SID);
        let cmds = log.lock().unwrap().clone();
        let onion = cmds
            .iter()
            .find(|c| c.starts_with("ADD_ONION "))
            .cloned()
            .expect("ADD_ONION");
        assert!(onion.contains("Port=18444,127.0.0.1:23456"), "{onion}");
        assert!(std::path::Path::new(&dir.join("onion").join("p2p.priv")).is_file());
        let hs2 = ctl.add_p2p_onion(&dir, bound, 18444).await.unwrap();
        assert_eq!(hs2.service_id, FAKE_SID);
        let _ = std::fs::remove_file(&cookie_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tor_plain_cookie_is_not_sent_when_safecookie_is_absent() {
        let cookie = vec![0xab; 32];
        let (addr, log) = fake_control_cookie_only(cookie.clone()).await;
        let path = tmp_cookie(&cookie);
        let err = match TorControl::connect_and_auth(addr, TorAuth::Cookie(path.clone())).await {
            Err(e) => e,
            Ok(_) => panic!("COOKIE without SAFECOOKIE must fail closed"),
        };
        let msg = format!("{err}");
        assert!(msg.contains("SAFECOOKIE"), "{msg}");
        let cmds = log.lock().unwrap().clone();
        assert!(
            cmds.iter().all(|c| !c.starts_with("AUTHENTICATE ")),
            "must not send the raw cookie: {cmds:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn tor_short_cookie_does_not_fall_back_to_raw_hex() {
        let cookie = vec![0x11, 0x22];
        let (addr, log) = fake_control(Some(cookie.clone()), None).await;
        let path = tmp_cookie(&cookie);
        let err = match TorControl::connect_and_auth(addr, TorAuth::Cookie(path.clone())).await {
            Err(e) => e,
            Ok(_) => panic!("a short SAFECOOKIE cookie must fail closed"),
        };
        let msg = format!("{err}");
        assert!(msg.contains("32 bytes"), "{msg}");
        let cmds = log.lock().unwrap().clone();
        assert!(
            cmds.iter().all(|c| !c.starts_with("AUTHENTICATE ")),
            "must not send the raw cookie: {cmds:?}"
        );
        let _ = std::fs::remove_file(&path);
    }
}
