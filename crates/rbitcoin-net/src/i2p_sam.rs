//! I2P SAM v3 STREAM CONNECT / FORWARD (system router, not SOCKS).

use crate::error::NetError;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

static INSTALLED: Mutex<Option<Installed>> = Mutex::new(None);

struct Installed {
    dialer: I2pDialer,
    _keepalive: Option<TcpStream>,
}

pub struct I2pSam {
    sam_addr: SocketAddr,
    session_id: String,
    destination: String,
    _control: TcpStream,
    _forward: Option<TcpStream>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct I2pDialer {
    sam_addr: SocketAddr,
    session_id: String,
    destination: String,
}

pub fn install(dialer: I2pDialer) {
    *INSTALLED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Installed {
        dialer,
        _keepalive: None,
    });
}

fn install_kept(dialer: I2pDialer, keepalive: TcpStream) {
    *INSTALLED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Installed {
        dialer,
        _keepalive: Some(keepalive),
    });
}

pub async fn stream_connect_installed(dest_b32: &str) -> Result<TcpStream, NetError> {
    let first = installed()?;
    match first.stream_connect(dest_b32).await {
        Ok(s) => Ok(s),
        Err(e) if session_dead(&e) => {
            let (fresh, keepalive) = first.recreate_session().await?;
            install_kept(fresh.clone(), keepalive);
            fresh.stream_connect(dest_b32).await
        }
        Err(e) => Err(e),
    }
}

fn installed() -> Result<I2pDialer, NetError> {
    INSTALLED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.dialer.clone())
        .ok_or_else(|| NetError::Encode("i2p dial requires SAM (--i2p-sam)".into()))
}

fn session_dead(err: &NetError) -> bool {
    match err {
        NetError::Io(_) | NetError::Disconnected => true,
        NetError::Encode(s) => {
            let up = s.to_ascii_uppercase();
            up.contains("INVALID_ID")
                || up.contains("CONNECTION CLOSED")
                || up.contains("STREAM CONNECT:")
        }
        _ => false,
    }
}

fn sam_retry(err: &NetError) -> bool {
    match err {
        NetError::Io(_) | NetError::Disconnected => true,
        NetError::Encode(s) => {
            let l = s.to_ascii_lowercase();
            l.contains("broken pipe")
                || l.contains("connection reset")
                || l.contains("connection closed")
                || l.contains("i2p sam connect")
                || l.contains("early eof")
                || l.contains("unexpected eof")
        }
        _ => false,
    }
}

fn stream_retry(err: &NetError) -> bool {
    match err {
        NetError::Io(_) | NetError::Disconnected => true,
        NetError::Encode(s) => {
            let l = s.to_ascii_lowercase();
            l.contains("early eof")
                || l.contains("unexpected eof")
                || l.contains("broken pipe")
                || l.contains("connection reset")
                || l.contains("connection closed")
                || l.contains("cant_reach")
                || l.contains("timeout")
        }
        _ => false,
    }
}

#[cfg(test)]
fn clear_installed() {
    *INSTALLED.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        let mut last = None;
        for _ in 0..32 {
            match Self::connect_session_dest_once(sam_addr, dest).await {
                Ok(v) => return Ok(v),
                Err(e) if sam_retry(&e) => {
                    last = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.expect("sam retry"))
    }

    async fn connect_session_dest_once(
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
            &format!(
                "SESSION CREATE STYLE=STREAM ID={session_id} DESTINATION={dest_arg} \
                 inbound.length=1 outbound.length=1 inbound.quantity=1 outbound.quantity=1"
            ),
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
                destination: destination.clone(),
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
            destination: self.destination.clone(),
        }
    }

    fn into_keepalive(self) -> (I2pDialer, TcpStream) {
        let dialer = self.dialer();
        (dialer, self._control)
    }

    pub async fn stream_forward(&mut self, port: u16) -> Result<(), NetError> {
        let mut last = None;
        for _ in 0..24 {
            match self.stream_forward_once(port).await {
                Ok(s) => {
                    self._forward = Some(s);
                    return Ok(());
                }
                Err(e) if stream_retry(&e) => {
                    last = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.expect("forward retry"))
    }

    async fn stream_forward_once(&self, port: u16) -> Result<TcpStream, NetError> {
        let mut s = TcpStream::connect(self.sam_addr)
            .await
            .map_err(|e| NetError::Encode(format!("i2p sam forward connect: {e}")))?;
        hello(&mut s).await?;
        write_line(
            &mut s,
            &format!(
                "STREAM FORWARD ID={} PORT={port} HOST=127.0.0.1 SILENT=true",
                self.session_id
            ),
        )
        .await?;
        let reply = read_line(&mut s).await?;
        if !reply.to_ascii_uppercase().contains("RESULT=OK") {
            return Err(NetError::Encode(format!("i2p sam forward: {reply}")));
        }
        Ok(s)
    }
}

impl I2pDialer {
    pub async fn stream_connect(&self, dest_b32: &str) -> Result<TcpStream, NetError> {
        let mut last = None;
        for _ in 0..24 {
            match self.stream_connect_once(dest_b32).await {
                Ok(s) => return Ok(s),
                Err(e) if stream_retry(&e) => {
                    last = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.expect("stream retry"))
    }

    async fn stream_connect_once(&self, dest_b32: &str) -> Result<TcpStream, NetError> {
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

    async fn recreate_session(&self) -> Result<(Self, TcpStream), NetError> {
        let (sam, _) = I2pSam::connect_session_dest(self.sam_addr, Some(&self.destination)).await?;
        Ok(sam.into_keepalive())
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
    let mut buf = Vec::new();
    loop {
        let mut b = [0u8; 1];
        s.read_exact(&mut b)
            .await
            .map_err(|e| NetError::Encode(format!("i2p sam read: {e}")))?;
        if b[0] == b'\n' {
            break;
        }
        if b[0] != b'\r' {
            buf.push(b[0]);
        }
        if buf.len() > 16 * 1024 {
            return Err(NetError::Encode("i2p sam read: line too long".into()));
        }
    }
    String::from_utf8(buf).map_err(|e| NetError::Encode(format!("i2p sam read: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::net::TcpListener;

    const FAKE_DEST: &str = "fakeprivdest";
    static INSTALL_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn fake_sam(
        ok_hello: bool,
        dest_log: Arc<Mutex<Vec<String>>>,
    ) -> (SocketAddr, Arc<Mutex<HashSet<String>>>) {
        fake_sam_opts(
            ok_hello,
            dest_log,
            Arc::new(Mutex::new(0)),
            Arc::new(Mutex::new(0)),
        )
        .await
    }

    async fn fake_sam_opts(
        ok_hello: bool,
        dest_log: Arc<Mutex<Vec<String>>>,
        stream_drops: Arc<Mutex<usize>>,
        forward_drops: Arc<Mutex<usize>>,
    ) -> (SocketAddr, Arc<Mutex<HashSet<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let live = Arc::new(Mutex::new(HashSet::new()));
        let live_accept = Arc::clone(&live);
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    break;
                };
                let log = Arc::clone(&dest_log);
                let live = Arc::clone(&live_accept);
                let drops = Arc::clone(&stream_drops);
                let fwd_drops = Arc::clone(&forward_drops);
                let ok = ok_hello;
                tokio::spawn(async move {
                    let mut created_id: Option<String> = None;
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
                            if let Some(id) = sam_kv(&line, "ID") {
                                live.lock().unwrap().insert(id.to_string());
                                created_id = Some(id.to_string());
                            }
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
                            let drop_n = {
                                let mut n = fwd_drops.lock().unwrap();
                                if *n > 0 {
                                    *n -= 1;
                                    true
                                } else {
                                    false
                                }
                            };
                            if drop_n {
                                break;
                            }
                            let _ = write_line(&mut s, "STREAM STATUS RESULT=OK").await;
                        } else if up.starts_with("STREAM CONNECT") {
                            log.lock().unwrap().push(line.clone());
                            let drop_n = {
                                let mut n = drops.lock().unwrap();
                                if *n > 0 {
                                    *n -= 1;
                                    true
                                } else {
                                    false
                                }
                            };
                            if drop_n {
                                break;
                            }
                            let id = sam_kv(&line, "ID").unwrap_or("");
                            let known = live.lock().unwrap().contains(id);
                            if known {
                                let _ = write_line(&mut s, "STREAM STATUS RESULT=OK").await;
                            } else {
                                let _ = write_line(&mut s, "STREAM STATUS RESULT=INVALID_ID").await;
                            }
                            break;
                        } else {
                            let _ = write_line(&mut s, "PING").await;
                        }
                    }
                    if let Some(id) = created_id {
                        live.lock().unwrap().remove(&id);
                    }
                });
            }
        });
        (addr, live)
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
    async fn i2p_sam_stream_connect_retries_early_eof() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(Mutex::new(2));
        let (addr, _live) =
            fake_sam_opts(true, Arc::clone(&log), drops, Arc::new(Mutex::new(0))).await;
        let sam = I2pSam::connect(addr).await.unwrap();
        let dest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.b32.i2p";
        sam.stream_connect(dest).await.unwrap();
        let got = stream_lines(&log, "STREAM CONNECT");
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[tokio::test]
    async fn i2p_sam_stream_forward_retries_early_eof() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(Mutex::new(2));
        let (addr, _live) =
            fake_sam_opts(true, Arc::clone(&log), Arc::new(Mutex::new(0)), drops).await;
        let mut sam = I2pSam::connect(addr).await.unwrap();
        sam.stream_forward(18444).await.unwrap();
        let got = stream_lines(&log, "STREAM FORWARD");
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[tokio::test]
    async fn i2p_sam_stream_connect_fake() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (addr, _live) = fake_sam(true, Arc::clone(&log)).await;
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

        let (bad, _live) = fake_sam(false, Arc::new(Mutex::new(Vec::new()))).await;
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
        let _gate = INSTALL_GATE.lock().await;
        clear_installed();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (addr, _live) = fake_sam(true, Arc::clone(&log)).await;
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
        let (addr, _live) = fake_sam(true, Arc::clone(&log)).await;
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
        assert!(fw[0].contains("HOST=127.0.0.1"), "{}", fw[0]);
        assert!(fw[0].contains("SILENT=true"), "{}", fw[0]);
        let creates = stream_lines(&log, "SESSION CREATE");
        assert!(
            creates
                .iter()
                .any(|c| c.contains("DESTINATION=TRANSIENT") && c.contains("inbound.length=1")),
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

    #[tokio::test]
    async fn i2p_sam_reconnects_after_invalid_id() {
        let _gate = INSTALL_GATE.lock().await;
        clear_installed();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (addr, live) = fake_sam(true, Arc::clone(&log)).await;
        let sam = I2pSam::connect(addr).await.unwrap();
        crate::socks::install_i2p_dialer(sam.dialer());
        let peer = crate::NetAddr::I2p {
            dest: [0u8; 32],
            port: 8333,
        };
        crate::socks::Dialer::Direct
            .connect_net(peer)
            .await
            .unwrap();
        assert_eq!(stream_lines(&log, "SESSION CREATE").len(), 1);

        live.lock().unwrap().clear();
        crate::socks::Dialer::Direct
            .connect_net(peer)
            .await
            .expect("SAM INVALID_ID must recreate the session and retry");
        let creates = stream_lines(&log, "SESSION CREATE");
        assert_eq!(creates.len(), 2, "{creates:?}");
        assert!(
            creates[1].contains(&format!("DESTINATION={FAKE_DEST}")),
            "{}",
            creates[1]
        );

        crate::socks::Dialer::Direct
            .connect_net(peer)
            .await
            .unwrap();
        assert_eq!(
            stream_lines(&log, "SESSION CREATE").len(),
            2,
            "published dialer must keep the new session id"
        );
        clear_installed();
    }
}
