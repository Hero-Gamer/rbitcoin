use crate::config::{ConfApply, NodeConfig};
use crate::inhibit::SuspendInhibit;
use crate::run::{run_node, run_p2p};
use rbitcoin_consensus::default_milestone_height;
use rbitcoin_log::{self, error, info, warn, Level};
use rbitcoin_store::HeadScale;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

#[allow(clippy::large_enum_variant)] // uring vs pool vs iocp backends
/// CLI parse result before log init / datadir open / run.
#[derive(Debug)]
pub(crate) enum OperatorArgs {
    Help,
    Version,
    Ready {
        config: NodeConfig,
        log_level_cli: Option<Option<Level>>,
    },
}

/// Assemble [`NodeConfig`] from argv (conf then CLI `apply_kv`). Does not open the store.
pub(crate) fn operator_config_from_args<I, T>(args: I) -> Result<OperatorArgs, ExitCode>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    match parse_operator_flags(&args)? {
        OperatorFlagEnd::Help => Ok(OperatorArgs::Help),
        OperatorFlagEnd::Version => Ok(OperatorArgs::Version),
        OperatorFlagEnd::Flags(parsed) => {
            let mut config = NodeConfig::default();
            if let Some(ref cp) = parsed.conf_path {
                if let Err(e) = config.merge_conf_file(cp) {
                    eprintln!("error: {e}");
                    return Err(ExitCode::from(2));
                }
                config.conf_path = Some(cp.clone());
            }
            apply_operator_kvs(&mut config, parsed.kvs)?;
            finish_operator_config(config, parsed.smoke, parsed.log_level_cli)
        }
    }
}

enum OperatorFlagEnd {
    Help,
    Version,
    Flags(ParsedOperatorFlags),
}

struct ParsedOperatorFlags {
    smoke: bool,
    conf_path: Option<PathBuf>,
    log_level_cli: Option<Option<Level>>,
    kvs: Vec<(String, String)>,
}

fn parse_operator_flags(args: &[OsString]) -> Result<OperatorFlagEnd, ExitCode> {
    let mut i = 1usize;
    let mut smoke = false;
    let mut conf_path: Option<PathBuf> = None;
    let mut log_level_cli: Option<Option<Level>> = None;
    let mut kvs: Vec<(String, String)> = Vec::new();

    while i < args.len() {
        let a = args[i].to_string_lossy();
        match a.as_ref() {
            "--help" | "-h" => {
                eprintln!("{}", operator_usage());
                return Ok(OperatorFlagEnd::Help);
            }
            "--version" | "-V" => {
                eprintln!("rbitcoin-node {}", env!("CARGO_PKG_VERSION"));
                return Ok(OperatorFlagEnd::Version);
            }
            "--smoke" => {
                smoke = true;
                i += 1;
            }
            "--conf" => match take_arg(args, &mut i, "--conf") {
                Ok(v) => conf_path = Some(PathBuf::from(v)),
                Err(c) => return Err(c),
            },
            other if other.starts_with("--conf=") => {
                let v = &other["--conf=".len()..];
                if v.is_empty() {
                    eprintln!("error: --conf requires a path");
                    return Err(ExitCode::from(2));
                }
                conf_path = Some(PathBuf::from(v));
                i += 1;
            }
            "--log-level" => match take_arg(args, &mut i, "--log-level") {
                Ok(raw) => match parse_log_level(&raw) {
                    Ok(v) => log_level_cli = Some(v),
                    Err(c) => return Err(c),
                },
                Err(c) => return Err(c),
            },
            other if other.starts_with("--log-level=") => {
                match parse_log_level(&other["--log-level=".len()..]) {
                    Ok(v) => log_level_cli = Some(v),
                    Err(c) => return Err(c),
                }
                i += 1;
            }
            other => match parse_cli_flag(args, &mut i, other) {
                Ok(Some(kv)) => kvs.push(kv),
                Ok(None) => {}
                Err(c) => return Err(c),
            },
        }
    }
    Ok(OperatorFlagEnd::Flags(ParsedOperatorFlags {
        smoke,
        conf_path,
        log_level_cli,
        kvs,
    }))
}

fn apply_operator_kvs(config: &mut NodeConfig, kvs: Vec<(String, String)>) -> Result<(), ExitCode> {
    let mut saw_listen = false;
    let mut saw_connect = false;
    let mut saw_seednode = false;
    for (key, val) in kvs {
        if key == "listen" && !saw_listen {
            config.listen.p2p = crate::config::P2pListen::Auto;
            config.listen.p2p_extra.clear();
            saw_listen = true;
        }
        if key == "no_listen" && !saw_listen {
            config.listen.p2p = crate::config::P2pListen::Auto;
            config.listen.p2p_extra.clear();
            saw_listen = true;
        }
        if key == "connect" && !saw_connect {
            config.listen.connect.clear();
            saw_connect = true;
        }
        if key == "seed_node" && !saw_seednode {
            config.listen.seednodes.clear();
            saw_seednode = true;
        }
        match config.apply_kv(&key, &val) {
            Ok(ConfApply::Applied) => {}
            Ok(ConfApply::Unknown(k)) => {
                eprintln!("error: unknown argument `--{k}`");
                return Err(ExitCode::from(2));
            }
            Err(e) => return Err(cli_apply_err(e)),
        }
    }
    Ok(())
}

fn finish_operator_config(
    mut config: NodeConfig,
    smoke: bool,
    log_level_cli: Option<Option<Level>>,
) -> Result<OperatorArgs, ExitCode> {
    if let Err(e) = config.subversion() {
        eprintln!("{e}");
        return Err(ExitCode::from(1));
    }

    if !config.milestone_explicit {
        config.milestone_height = default_milestone_height(config.network);
    }
    config.smoke = smoke;
    config.absorb_inbound_env();
    config.resolve_listen_defaults();
    Ok(OperatorArgs::Ready {
        config,
        log_level_cli,
    })
}

/// Process entry used by `main` and high-level scenarios.
pub fn cli_main<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let (mut config, log_level_cli) = match operator_config_from_args(args) {
        Ok(OperatorArgs::Help | OperatorArgs::Version) => return ExitCode::SUCCESS,
        Ok(OperatorArgs::Ready {
            config,
            log_level_cli,
        }) => (config, log_level_cli),
        Err(c) => return c,
    };

    match log_level_cli {
        Some(Some(level)) => rbitcoin_log::init(level),
        Some(None) => rbitcoin_log::init_off(),
        None => {
            if let Some(ref raw) = config.conf_log_level {
                match parse_log_level(raw) {
                    Ok(Some(l)) => rbitcoin_log::init(l),
                    Ok(None) => rbitcoin_log::init_off(),
                    Err(c) => return c,
                }
            } else if !rbitcoin_log::init_from_env() {
                rbitcoin_log::init(Level::Info);
            }
        }
    }

    if let Some(ref p) = config.api_log {
        if let Err(e) = rbitcoin_log::init_api_log(p) {
            eprintln!("error: --api-log {}: {e}", p.display());
            return ExitCode::from(2);
        }
        rbitcoin_log::info!("api-log: {}", p.display());
    }

    let (soft, hard) = rbitcoin_store::ensure_nofile_budget();
    if soft > 0 {
        rbitcoin_log::debug!("node: RLIMIT_NOFILE soft={soft} hard={hard}");
    }

    let _suspend_inhibit = if config.inhibit_suspend {
        match SuspendInhibit::try_start("rbitcoin-node running (IBD / tip follow)") {
            Some(g) => Some(g),
            None => {
                warn!(
                    "node: --inhibit-suspend requested but systemd-inhibit unavailable; continuing without inhibit"
                );
                None
            }
        }
    } else {
        None
    };

    if let Err(e) = config.ensure_datadir() {
        error!("{e}");
        return ExitCode::FAILURE;
    }

    if config.smoke {
        config.head_scale = HeadScale::Tiny;
        match run_node(config) {
            Ok(handle) => {
                info!(
                    "rbitcoin-node {} on {} datadir={}",
                    env!("CARGO_PKG_VERSION"),
                    handle.network_name(),
                    handle.config.datadir.path().display()
                );
                if std::env::var_os("RBITCOIN_TEST_DROP_STORE").is_some() {
                    let _ = std::fs::remove_dir_all(handle.config.store_path());
                }
                match handle.shutdown() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        error!("shutdown error: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(e) => {
                print_run_err(&e);
                ExitCode::FAILURE
            }
        }
    } else {
        let rt = match node_tokio_runtime() {
            Ok(rt) => rt,
            Err(e) => {
                error!("runtime: {e}");
                return ExitCode::FAILURE;
            }
        };
        let code = match rt.block_on(run_p2p(config)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                print_run_err(&e);
                ExitCode::FAILURE
            }
        };
        rt.shutdown_timeout(std::time::Duration::from_secs(2));
        code
    }
}

fn operator_usage() -> String {
    format!(
        "rbitcoin-node {} — usage:\n\
  rbitcoin-node [--conf FILE] [--datadir PATH] [--datadir-cold PATH] [--network NET] \\\n\
    [--signet-challenge HEX] [--signet-block-time SECS] \\\n\
    [--listen ADDR] [--no-listen] [--connect ADDR]... [--seed-node HOST]... [--proxy HOST:PORT] [--onion HOST:PORT] [--proxy-randomize[=0|1]] [--only-net NET]... \\\n\
    [--tor-control [HOST:PORT]] [--tor-control-cookie PATH] [--tor-control-password PASS] \\\n\
    [--i2p-sam [HOST:PORT]] [--i2p-accept-incoming] \\\n\
    [--electrum-listen ADDR] [--esplora-listen ADDR] [--esplora-onion[=0|1]] \\\n\
    [--sh-index] [--prune-inwit] [--prune-inwit-ram-threshold-bytes N] [--sp-tweaks] [--sp-tweaks-dust SATS] [--max-sh-creates N] [--esplora-block-template] \\\n\
    [--rpc] [--rpc-listen [ADDR]] [--rpc-token-file PATH] [--rpc-work-queue N] \\\n\
    [--milestone HEIGHT] \\\n\
    [--max-outbound N] [--max-inbound N] \\\n\
    [--mempool-size-mb N] [--mempool-expiry HOURS] \\\n\
    [--test-activation-height name@HEIGHT] [--persist-mempool[=0|1]] [--trusted] [--always-relay] [--relay] \\\n\
    [--net-permission SPEC] [--net-permission-bind SPEC] [--net-permission-relay[=0|1]] [--net-permission-force-relay[=0|1]] \\\n\
    [--blocks-only] [--prefill-compact[=0|1]] [--min-relay-tx-fee BTC] \\\n\
    [--limit-cluster-count N] [--limit-cluster-size KVB] [--peer-timeout SECS] \\\n\
    [--external-ip IP] [--ua-comment STR] \\\n\
    [--min-chain-work HEX] [--max-tip-age SECS] [--check-blocks N] [--mock-time UNIX] \\\n\
    [--block-version N] [--block-min-tx-fee BTC] [--alert-notify CMD] [--startup-notify CMD] \\\n\
    [--max-run-secs N] [--log-level LEVEL] [--api-log PATH] [--asmap PATH] \\\n\
    [--no-seeds] [--no-listen] [--no-discover] [--listen-onion] [--cjdns-reachable] [--smoke] [--inhibit-suspend]\n\n\
Networks: mainnet|testnet|signet|regtest.\n\
Custom Signet: --signet-challenge HEX [--signet-block-time SECS].\n\
Log level: error|warn|info|debug|trace|off (CLI > conf log_level > RBITCOIN_LOG / RUST_LOG).\n\
API log: --api-log PATH writes one JSON line per Electrum/Esplora/RPC call (also TRACE `api:`).\n\
Asmap: --asmap PATH loads a Core ip_asn.dat (relative to datadir). Unset tries {{datadir}}/ip_asn.dat.\n\
Milestone: skip script/sig checks at/below HEIGHT.\n\
  Defaults: mainnet 840000, signet 2000000, testnet 2500000, regtest 0. Use 0 for full scripts.\n\
Check-blocks: --check-blocks N revalidates the last N confirmed heights on open (default 6; 0 = all).\n\
Mempool: --mempool-size-mb (default ~300 MiB weight budget).\n\
Peers: --max-outbound (default 16 live download), --max-inbound (default 125).\n\
  --proxy HOST:PORT SOCKS5 for all P2P outbound; --onion HOST:PORT SOCKS for onion (02).\n\
  --proxy-randomize (default on) uses a fresh SOCKS username per peer (Tor circuit isolation).\n\
  --tor-control [HOST:PORT] talks to system tor (default 127.0.0.1:9051). Cookie or password AUTH;\n\
  failed AUTH is a start error. Unset: no control connection.\n\
  --tor-control-cookie PATH (default /run/tor/control.authcookie). --tor-control-password PASS.\n\
  --i2p-sam [HOST:PORT] SAM v3 to system i2pd (default 127.0.0.1:7656). --only-net=i2p requires it.\n\
  --i2p-accept-incoming persist {{datadir}}/i2p/p2p.priv and STREAM FORWARD to the P2P bind. Needs --listen.\n\
  --listen-onion ADD_ONION the P2P port (loopback bind even with --no-listen). Needs --tor-control and --max-inbound > 0.\n\
  --cjdns-reachable treat fc00::/8 as CJDNS (dial and advertise). --only-net=cjdns requires it.\n\
  --trusted / --always-relay / --relay are inbound permission knobs.\n\
  --net-permission / --net-permission-bind are CIDR or bind grants (noban, relay, …; IPv4 and IPv6).\n\
  --net-permission-relay (default on) / --net-permission-force-relay (default off) are implicit bits on a bare CIDR grant.\n\
Scripthash: --sh-index (default off) builds Class B for Electrum/Esplora address history.\n\
  Electrum/Esplora start without it; scripthash/address methods fail closed.\n\
  --prune-inwit refuse inwit reconstruct below tip-288 heights; advertise NETWORK_LIMITED.\n\
    Kept heights are store/inwit.window/{{height}}.bin plus a RAM cache. Unpruned nodes read inwit.body.\n\
  --prune-inwit-ram-threshold-bytes N RAM cap for that cache (default 268435456; 0 keeps nothing in RAM).\n\
  --max-sh-creates N refuses Electrum/Esplora joins with more than N creates (0 = unlimited).\n\
  --esplora-block-template enables GET /block-template (GBT template JSON; default off).\n\
  --esplora-onion (default on) ADD_ONION for --esplora-listen when --tor-control is set.\n\
Silent payments: --sp-tweaks (default off) writes/serves the thin BIP-352 tweak index.\n\
  --sp-tweaks-dust SATS omits served P2TR outs with value <= SATS (default 1000; 0 = all; 546 = Cake electrs).\n\
RPC: --rpc unix socket {{datadir}}/rpc.sock; --rpc-listen [ADDR] adds TCP (default 127.0.0.1 and Core-matching port). Token {{datadir}}/rpc.token (Bearer). No --rpcuser.\n\
Cold files: --datadir-cold PATH puts Class A inwit.body/idx under PATH/store (HDD).\n\
  Default (flag omitted): hot and cold files both live under --datadir.\n\
Conf: --conf FILE (snake_case key=value; CLI kebab overrides conf). See OPERATOR.md and docs/rpc.md.\n\
Advanced debug/IO knobs remain RBITCOIN_* env (not required for normal sync; preserved if CLI omits).\n\
IBD densify: up to 1024 concurrent getdata, max 16 in transit per peer.\n\
  Relay / RPC initialblockdownload after catch-up: --min-chain-work + --max-tip-age (24h).",
        env!("CARGO_PKG_VERSION")
    )
}

fn take_arg(args: &[OsString], i: &mut usize, flag: &str) -> Result<String, ExitCode> {
    *i += 1;
    if *i >= args.len() {
        eprintln!("error: {flag} requires a value");
        return Err(ExitCode::from(2));
    }
    let v = args[*i].to_string_lossy().into_owned();
    *i += 1;
    Ok(v)
}

fn parse_log_level(raw: &str) -> Result<Option<Level>, ExitCode> {
    if raw.eq_ignore_ascii_case("off") || raw.eq_ignore_ascii_case("none") {
        Ok(None)
    } else if let Some(l) = Level::parse(raw) {
        Ok(Some(l))
    } else {
        eprintln!("error: bad --log-level `{raw}` (use error|warn|info|debug|trace|off)");
        Err(ExitCode::from(2))
    }
}

fn is_bool_key(key: &str) -> bool {
    matches!(
        key,
        "sh_index"
            | "prune_inwit"
            | "sp_tweaks"
            | "esplora_block_template"
            | "esplora_onion"
            | "blocks_only"
            | "prefill_compact"
            | "persist_mempool"
            | "net_permission_relay"
            | "net_permission_force_relay"
            | "no_seeds"
            | "no_listen"
            | "no_discover"
            | "listen_onion"
            | "cjdns_reachable"
            | "proxy_randomize"
            | "i2p_accept_incoming"
            | "inhibit_suspend"
            | "trusted"
            | "always_relay"
            | "relay"
            | "rpc"
    )
}

fn is_optional_addr_key(key: &str) -> bool {
    matches!(
        key,
        "rpc_listen" | "electrum_listen" | "esplora_listen" | "tor_control" | "i2p_sam"
    )
}

fn looks_like_flag(s: &str) -> bool {
    s.starts_with("--") || matches!(s, "-h" | "-V")
}

fn parse_cli_flag(
    args: &[OsString],
    i: &mut usize,
    flag: &str,
) -> Result<Option<(String, String)>, ExitCode> {
    let rest = if let Some(r) = flag.strip_prefix("--") {
        r
    } else {
        eprintln!("error: unknown argument `{flag}`");
        return Err(ExitCode::from(2));
    };
    if rest.is_empty() {
        eprintln!("error: unknown argument `{flag}`");
        return Err(ExitCode::from(2));
    }
    let (name, eq_val) = match rest.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (rest, None),
    };
    let key = name.replace('-', "_");
    if key == "smoke" || key == "help" || key == "version" || key == "conf" || key == "log_level" {
        eprintln!("error: unknown argument `{flag}`");
        return Err(ExitCode::from(2));
    }
    let val = if let Some(v) = eq_val {
        *i += 1;
        v.to_string()
    } else if is_bool_key(&key) {
        *i += 1;
        "1".to_string()
    } else if is_optional_addr_key(&key) {
        *i += 1;
        if *i >= args.len() {
            String::new()
        } else {
            let next = args[*i].to_string_lossy().into_owned();
            if looks_like_flag(&next) {
                String::new()
            } else {
                *i += 1;
                next
            }
        }
    } else {
        *i += 1;
        if *i >= args.len() {
            eprintln!("error: --{name} requires a value");
            return Err(ExitCode::from(2));
        }
        let next = args[*i].to_string_lossy().into_owned();
        if looks_like_flag(&next) {
            eprintln!("error: --{name} requires a value");
            return Err(ExitCode::from(2));
        }
        *i += 1;
        next
    };
    Ok(Some((key, val)))
}

fn print_run_err(e: &crate::error::NodeError) {
    match e {
        crate::error::NodeError::FutureTip | crate::error::NodeError::Init(_) => eprintln!("{e}"),
        crate::error::NodeError::Locked(_) => eprintln!("Error: {e}"),
        _ => error!("{e}"),
    }
}

fn cli_apply_err(e: crate::error::NodeError) -> ExitCode {
    match e {
        crate::error::NodeError::Init(_) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
        other => {
            eprintln!("error: {other}");
            ExitCode::from(2)
        }
    }
}

fn blocking_pool_size() -> usize {
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4)
        .max(4)
}

fn node_tokio_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(blocking_pool_size())
        .thread_name("tokio-rt-worker")
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OPERATOR_ENV_TEST_LOCK;
    use rbitcoin_primitives::Network;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_datadir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("rbitcoin-cli-{n}"))
    }

    /// `ExitCode` is not `PartialEq`; compare via `Debug` (stable, sufficient for tests).
    fn assert_exit(got: ExitCode, want: ExitCode) {
        assert_eq!(
            format!("{got:?}"),
            format!("{want:?}"),
            "exit code mismatch"
        );
    }

    #[test]
    fn blocking_pool_is_capped_not_tokio_default() {
        assert!(blocking_pool_size() >= 4);
        assert!(blocking_pool_size() <= 512);
    }

    fn ready_config<I, T>(args: I) -> NodeConfig
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        match operator_config_from_args(args) {
            Ok(OperatorArgs::Ready { config, .. }) => config,
            other => panic!("expected assembled config, got {other:?}"),
        }
    }

    #[test]
    fn help_advertises_kebab_not_concatenated_core_names() {
        let h = operator_usage();
        for flag in [
            "--prefill-compact",
            "--limit-cluster-count",
            "--limit-cluster-size",
            "--min-relay-tx-fee",
            "--mempool-expiry",
            "--external-ip",
            "--seed-node",
            "--mock-time",
            "--block-version",
            "--block-min-tx-fee",
            "--alert-notify",
            "--startup-notify",
            "--test-activation-height",
            "--rpc-work-queue",
            "--peer-timeout",
            "--blocks-only",
            "--ua-comment",
            "--min-chain-work",
            "--max-tip-age",
            "--check-blocks",
            "--net-permission",
            "--net-permission-bind",
            "--net-permission-relay",
            "--net-permission-force-relay",
            "--signet-block-time",
            "--sh-index",
            "--prune-inwit",
            "--sp-tweaks",
            "--sp-tweaks-dust",
            "--esplora-block-template",
            "--esplora-onion",
            "--rpc",
            "--rpc-listen",
            "--rpc-token-file",
            "--proxy",
            "--onion",
            "--proxy-randomize",
            "--no-listen",
            "--no-discover",
            "--listen-onion",
            "--cjdns-reachable",
            "--tor-control",
            "--tor-control-cookie",
            "--tor-control-password",
            "--i2p-sam",
            "--i2p-accept-incoming",
        ] {
            assert!(h.contains(flag), "help must list {flag}");
        }
        for concat in ["--shindex", "--sptweaks", "-shindex", "-sptweaks"] {
            assert!(
                !h.contains(concat),
                "help must not advertise concatenated or one-dash long {concat}"
            );
        }
        for concat in [
            "--prefillcompact",
            "--limitclustercount",
            "--limitclustersize",
            "--minrelaytxfee",
            "--mempoolexpiry",
            "--externalip",
            "--seednode",
            "--mocktime",
            "--blockversion",
            "--blockmintxfee",
            "--alertnotify",
            "--startupnotify",
            "--testactivationheight",
            "--rpcworkqueue",
            "--checkblocks",
            "--blocksdir",
            "--blocks-dir",
            "--whitelist-relay",
            "--whitelist-forcerelay",
            "--nolisten",
            "--nodiscover",
            "--listenonion",
            "--cjdnsreachable",
            "--torcontrol",
            "--i2psam",
        ] {
            assert!(!h.contains(concat), "help must not advertise {concat}");
        }
        assert!(
            h.contains("Networks: mainnet|testnet|signet|regtest."),
            "network list must end with a period"
        );
        assert!(
            h.contains("[--signet-block-time SECS]"),
            "duration placeholder must be SECS"
        );
    }

    #[test]
    fn sh_index_and_sp_tweaks_are_kebab_not_concat() {
        let on = ready_config(["rbitcoin-node", "--sh-index", "--sp-tweaks"]);
        assert!(on.shindex);
        assert!(on.sptweaks);
        let eq = ready_config(["rbitcoin-node", "--sh-index=1", "--sp-tweaks-dust=546"]);
        assert!(eq.shindex);
        assert_eq!(eq.sptweaks_dust, 546);
        assert_exit(cli_main(["rbitcoin-node", "--shindex"]), ExitCode::from(2));
        assert_exit(cli_main(["rbitcoin-node", "-shindex"]), ExitCode::from(2));
        assert_exit(cli_main(["rbitcoin-node", "--sptweaks"]), ExitCode::from(2));
        assert_exit(
            cli_main(["rbitcoin-node", "--sptweaks-dust=1"]),
            ExitCode::from(2),
        );
    }

    #[test]
    fn proxy_conf_and_cli() {
        let _g = OPERATOR_ENV_TEST_LOCK.lock().unwrap();
        let cfg = ready_config(["rbitcoin-node", "--proxy", "127.0.0.1:9050"]);
        assert_eq!(cfg.listen.proxy, Some("127.0.0.1:9050".parse().unwrap()));
        assert!(cfg.listen.onion.is_none());
        match cfg.listen.dialer() {
            rbitcoin_net::Dialer::Socks {
                proxy, randomize, ..
            } => {
                assert_eq!(proxy, Some("127.0.0.1:9050".parse().unwrap()));
                assert!(randomize);
            }
            other => panic!("expected socks dialer, got {other:?}"),
        }
        assert_eq!(
            NodeConfig::default().listen.dialer(),
            rbitcoin_net::Dialer::Direct
        );

        let mut from_conf = NodeConfig::default();
        assert_eq!(
            from_conf.apply_kv("proxy", "127.0.0.1:9050").unwrap(),
            ConfApply::Applied
        );
        assert_eq!(
            from_conf.listen.proxy,
            Some("127.0.0.1:9050".parse().unwrap())
        );

        let empty = NodeConfig::default().apply_kv("proxy", "").unwrap_err();
        let empty_msg = format!("{empty}");
        assert!(
            empty_msg.contains("proxy"),
            "empty proxy must be a start error: {empty_msg}"
        );

        let bad = NodeConfig::default()
            .apply_kv("proxy", "not-an-addr")
            .unwrap_err();
        let bad_msg = format!("{bad}");
        assert!(
            bad_msg.contains("proxy"),
            "invalid proxy must be a start error: {bad_msg}"
        );

        let split = ready_config([
            "rbitcoin-node",
            "--proxy",
            "127.0.0.1:9050",
            "--onion",
            "127.0.0.1:9051",
        ]);
        assert_eq!(split.listen.proxy, Some("127.0.0.1:9050".parse().unwrap()));
        assert_eq!(split.listen.onion, Some("127.0.0.1:9051".parse().unwrap()));
        assert!(split.listen.proxy.is_some());
        assert!(split.listen.onion.is_some());
        assert_ne!(split.listen.proxy, split.listen.onion);

        let h = operator_usage();
        assert!(h.contains("--proxy"), "help must list kebab --proxy");
        assert!(h.contains("--onion"), "help must list kebab --onion");
        assert!(
            h.contains("--proxy-randomize"),
            "help must list kebab --proxy-randomize"
        );
    }

    #[test]
    fn onion_only_dialer_does_not_socks_clearnet() {
        let _g = OPERATOR_ENV_TEST_LOCK.lock().unwrap();
        let split = ready_config([
            "rbitcoin-node",
            "--proxy",
            "127.0.0.1:9050",
            "--onion",
            "127.0.0.1:9051",
        ]);
        match split.listen.dialer() {
            rbitcoin_net::Dialer::Socks { proxy, onion, .. } => {
                assert_eq!(proxy, split.listen.proxy);
                assert_eq!(onion, split.listen.onion);
            }
            other => panic!("expected split socks dialer, got {other:?}"),
        }
        let onion_only = ready_config(["rbitcoin-node", "--onion", "127.0.0.1:9051"]);
        match onion_only.listen.dialer() {
            rbitcoin_net::Dialer::Socks { proxy, onion, .. } => {
                assert!(proxy.is_none(), "onion-only must not SOCKS clearnet");
                assert_eq!(onion, Some("127.0.0.1:9051".parse().unwrap()));
            }
            other => panic!("expected onion-only socks dialer, got {other:?}"),
        }
        match onion_only.listen.isolated_dialer() {
            rbitcoin_net::Dialer::Socks {
                proxy,
                onion,
                randomize,
                ..
            } => {
                assert!(proxy.is_none());
                assert_eq!(onion, Some("127.0.0.1:9051".parse().unwrap()));
                assert!(
                    randomize,
                    "isolated broadcast always randomizes SOCKS creds"
                );
            }
            other => panic!("expected onion-only isolated dialer, got {other:?}"),
        }
    }

    #[test]
    fn proxy_randomize_defaults_on() {
        let _g = OPERATOR_ENV_TEST_LOCK.lock().unwrap();
        assert!(NodeConfig::default().listen.proxy_randomize);
        let off = ready_config([
            "rbitcoin-node",
            "--proxy",
            "127.0.0.1:9050",
            "--proxy-randomize=0",
        ]);
        assert!(!off.listen.proxy_randomize);
        match off.listen.dialer() {
            rbitcoin_net::Dialer::Socks { randomize, .. } => assert!(!randomize),
            other => panic!("expected socks dialer, got {other:?}"),
        }
        let on = ready_config(["rbitcoin-node", "--proxy", "127.0.0.1:9050"]);
        assert!(on.listen.proxy_randomize);
        match on.listen.dialer() {
            rbitcoin_net::Dialer::Socks { randomize, .. } => assert!(randomize),
            other => panic!("expected socks dialer, got {other:?}"),
        }
    }

    #[test]
    fn max_inbound_zero_is_allowed() {
        let _g = OPERATOR_ENV_TEST_LOCK.lock().unwrap();
        let cfg = ready_config(["rbitcoin-node", "--max-inbound", "0"]);
        assert_eq!(cfg.listen.max_inbound, 0);
        assert!(cfg.listen.max_inbound_explicit);
        cfg.validate()
            .expect("CLI --max-inbound 0 must assemble and validate");
        let out = NodeConfig::default()
            .apply_kv("max_outbound", "0")
            .unwrap_err();
        assert!(format!("{out}").contains("max_outbound"));
    }

    #[test]
    fn listen_zero_does_not_default_loopback() {
        let _g = OPERATOR_ENV_TEST_LOCK.lock().unwrap();
        let mut c = NodeConfig::default();
        assert_eq!(c.apply_kv("listen", "0").unwrap(), ConfApply::Applied);
        assert_eq!(c.listen.p2p, crate::config::P2pListen::Off);
        assert!(c.listen.p2p_bind_addr(Network::Regtest).is_none());

        let n = ready_config(["rbitcoin-node", "--no-listen"]);
        assert_eq!(n.listen.p2p, crate::config::P2pListen::Off);
        assert!(n.listen.p2p_bind_addr(Network::Regtest).is_none());

        let eq = ready_config(["rbitcoin-node", "--listen=0"]);
        assert_eq!(eq.listen.p2p, crate::config::P2pListen::Off);

        let bound = ready_config(["rbitcoin-node", "--listen", "127.0.0.1:18444"]);
        assert_eq!(
            bound.listen.p2p,
            crate::config::P2pListen::Socket("127.0.0.1:18444".parse().unwrap())
        );
        assert_eq!(
            bound.listen.p2p_bind_addr(Network::Regtest),
            Some("127.0.0.1:18444".parse().unwrap())
        );

        let auto = NodeConfig::default();
        assert_eq!(auto.listen.p2p, crate::config::P2pListen::Auto);
        assert_eq!(
            auto.listen.p2p_bind_addr(Network::Regtest),
            Some("127.0.0.1:18444".parse().unwrap())
        );

        let h = operator_usage();
        assert!(
            h.contains("--no-listen"),
            "help must list kebab --no-listen"
        );
        assert!(
            !h.contains("--nolisten"),
            "help must not advertise concatenated --nolisten"
        );
    }

    #[test]
    fn listen_onion_binds_loopback_when_nolisten() {
        let c = ready_config(["rbitcoin-node", "--no-listen", "--listen-onion"]);
        assert!(c.listen.listen_onion);
        assert_eq!(c.listen.p2p, crate::config::P2pListen::Off);
        assert!(c.listen.p2p_bind_addr(Network::Regtest).is_none());
        assert_eq!(
            c.listen.start_p2p_bind(Network::Regtest),
            Some("127.0.0.1:0".parse().unwrap())
        );
        let mut conf = NodeConfig::default();
        conf.apply_kv("listen_onion", "1").unwrap();
        assert!(conf.listen.listen_onion);
    }

    #[test]
    fn listen_onion_refused_when_max_inbound_zero() {
        let c = ready_config([
            "rbitcoin-node",
            "--listen-onion",
            "--max-inbound",
            "0",
            "--tor-control",
        ]);
        let err = c.validate().unwrap_err().to_string();
        assert!(
            err.contains("listen-onion") && err.contains("max-inbound"),
            "{err}"
        );
        let no_tor = ready_config(["rbitcoin-node", "--listen-onion"]);
        let err = no_tor.validate().unwrap_err().to_string();
        assert!(err.contains("tor-control"), "{err}");
    }

    #[test]
    fn listen_cjdns_addr_parses() {
        let mut c = NodeConfig::default();
        c.apply_kv("listen", "[fc00:1:2:3:4:5:6:7]:8333").unwrap();
        match c.listen.p2p {
            crate::config::P2pListen::Socket(a) => {
                assert!(a.is_ipv6(), "{a}");
                assert_eq!(a.port(), 8333);
                let ip = match a.ip() {
                    std::net::IpAddr::V6(v) => v,
                    other => panic!("expected v6, got {other}"),
                };
                assert!(rbitcoin_net::is_cjdns_ip(ip), "{ip}");
            }
            other => panic!("expected socket listen, got {other:?}"),
        }
        let off = ready_config(["rbitcoin-node", "--no-listen"]);
        assert_eq!(off.listen.p2p, crate::config::P2pListen::Off);
        assert!(off.listen.p2p_bind_addr(Network::Regtest).is_none());
    }

    #[test]
    fn connect_onion_and_ipv4() {
        let mut c = NodeConfig::default();
        c.apply_kv("connect", "1.2.3.4:8333").unwrap();
        c.apply_kv(
            "connect",
            "pg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscryd.onion:8333",
        )
        .unwrap();
        assert_eq!(c.listen.connect.len(), 2);
        assert_eq!(c.listen.connect[0], "1.2.3.4:8333".parse().unwrap());
        assert!(matches!(
            c.listen.connect[1],
            rbitcoin_net::NetAddr::Onion { port: 8333, .. }
        ));
        let err = NodeConfig::default()
            .apply_kv("connect", "short.onion:8333")
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("connect") || msg.contains("onion") || msg.contains("bad"),
            "{msg}"
        );
        let mut s = NodeConfig::default();
        s.apply_kv(
            "seed_node",
            "pg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscryd.onion:8333",
        )
        .unwrap();
        assert_eq!(
            s.listen.seednodes,
            vec!["pg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscryd.onion:8333".to_string()]
        );
        assert!(NodeConfig::default()
            .apply_kv("seed_node", "short.onion:8333")
            .is_err());
        let n = ready_config([
            "rbitcoin-node",
            "--connect",
            "pg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscryd.onion:8333",
        ]);
        assert!(matches!(
            n.listen.connect[0],
            rbitcoin_net::NetAddr::Onion { port: 8333, .. }
        ));
    }

    #[test]
    fn only_net_onion_without_proxy_is_config_error() {
        let mut c = NodeConfig::default();
        c.apply_kv("only_net", "onion").unwrap();
        let err = c.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("SOCKS") && msg.contains("only-net"), "{msg}");
        c.apply_kv("proxy", "127.0.0.1:9050").unwrap();
        c.validate().unwrap();
        let mut i2p_ok = NodeConfig::default();
        i2p_ok.apply_kv("only_net", "i2p").unwrap();
        assert_eq!(i2p_ok.listen.only_net, vec![rbitcoin_net::OnlyNet::I2p]);
        let mut cjdns = NodeConfig::default();
        cjdns.apply_kv("only_net", "cjdns").unwrap();
        assert_eq!(cjdns.listen.only_net, vec![rbitcoin_net::OnlyNet::Cjdns]);
        let err = cjdns.validate().unwrap_err().to_string();
        assert!(err.contains("cjdns-reachable"), "{err}");
        cjdns.apply_kv("cjdns_reachable", "1").unwrap();
        cjdns.validate().unwrap();
        let ok = ready_config([
            "rbitcoin-node",
            "--only-net",
            "onion",
            "--proxy",
            "127.0.0.1:9050",
        ]);
        assert_eq!(ok.listen.only_net, vec![rbitcoin_net::OnlyNet::Onion]);
        let h = operator_usage();
        assert!(h.contains("--only-net"), "help must list kebab --only-net");
        assert!(
            !h.contains("--onlynet"),
            "help must not advertise concatenated --onlynet"
        );
    }

    #[test]
    fn prune_inwit_is_kebab() {
        let on = ready_config([
            "rbitcoin-node",
            "--prune-inwit",
            "--prune-inwit-ram-threshold-bytes=4096",
        ]);
        assert!(on.prune_inwit);
        assert_eq!(on.prune_inwit_ram_threshold_bytes, 4096);
        let mut conf = NodeConfig::default();
        conf.apply_kv("prune_inwit", "1").unwrap();
        conf.apply_kv("prune_inwit_ram_threshold_bytes", "8192")
            .unwrap();
        assert!(conf.prune_inwit);
        assert_eq!(conf.prune_inwit_ram_threshold_bytes, 8192);
        conf.apply_kv("prune_inwit_ram_threshold_bytes", "0")
            .unwrap();
        assert_eq!(conf.prune_inwit_ram_threshold_bytes, 0);
        let h = operator_usage();
        assert!(h.contains("--prune-inwit"));
        assert!(h.contains("--prune-inwit-ram-threshold-bytes"));
        assert!(!h.contains("--pruneinwit"));
        assert_exit(
            cli_main(["rbitcoin-node", "--pruneinwit"]),
            ExitCode::from(2),
        );
    }

    #[test]
    fn tor_control_cli_defaults() {
        let omitted = ready_config(["rbitcoin-node", "--tor-control"]);
        assert_eq!(omitted.tor.control, Some("127.0.0.1:9051".parse().unwrap()));
        assert!(omitted.tor.cookie.is_none());
        assert!(omitted.tor.password.is_none());
        let explicit = ready_config(["rbitcoin-node", "--tor-control", "10.0.0.5:9151"]);
        assert_eq!(explicit.tor.control, Some("10.0.0.5:9151".parse().unwrap()));
        let cookie = ready_config([
            "rbitcoin-node",
            "--tor-control",
            "--tor-control-cookie",
            "/tmp/rbtc-tor-cookie",
        ]);
        assert_eq!(
            cookie.tor.cookie.as_deref(),
            Some(std::path::Path::new("/tmp/rbtc-tor-cookie"))
        );
        let mut conf = NodeConfig::default();
        conf.apply_kv("tor_control", "").unwrap();
        conf.apply_kv("tor_control_password", "pw").unwrap();
        assert_eq!(conf.tor.control, Some("127.0.0.1:9051".parse().unwrap()));
        assert_eq!(conf.tor.password.as_deref(), Some("pw"));
        let h = operator_usage();
        assert!(h.contains("--tor-control"));
        assert!(h.contains("--tor-control-cookie"));
        assert!(h.contains("--tor-control-password"));
        assert!(!h.contains("--torcontrol"));
    }

    #[test]
    fn i2p_sam_cli() {
        let omitted = ready_config(["rbitcoin-node", "--i2p-sam"]);
        assert_eq!(
            omitted.listen.i2p_sam,
            Some("127.0.0.1:7656".parse().unwrap())
        );
        let explicit = ready_config(["rbitcoin-node", "--i2p-sam", "127.0.0.1:7656"]);
        assert_eq!(
            explicit.listen.i2p_sam,
            Some("127.0.0.1:7656".parse().unwrap())
        );
        let h = operator_usage();
        assert!(h.contains("--i2p-sam"));
        assert!(!h.contains("--i2psam"));
        assert!(h.contains("--i2p-accept-incoming"));
        assert!(!h.contains("--i2pacceptincoming"));
    }

    #[test]
    fn i2p_accept_incoming_cli() {
        let c = ready_config(["rbitcoin-node", "--i2p-sam", "--i2p-accept-incoming"]);
        assert!(c.listen.i2p_accept_incoming);
        assert_eq!(c.listen.i2p_sam, Some("127.0.0.1:7656".parse().unwrap()));
        let mut conf = NodeConfig::default();
        conf.apply_kv("i2p_sam", "").unwrap();
        conf.apply_kv("i2p_accept_incoming", "1").unwrap();
        conf.validate().unwrap();
        assert!(conf.listen.i2p_accept_incoming);
    }

    #[test]
    fn i2p_accept_incoming_without_listen_is_config_error() {
        let c = ready_config([
            "rbitcoin-node",
            "--no-listen",
            "--i2p-sam",
            "--i2p-accept-incoming",
        ]);
        let err = c.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("--listen") && msg.contains("i2p"), "{msg}");
        let mut no_sam = NodeConfig::default();
        no_sam.apply_kv("i2p_accept_incoming", "1").unwrap();
        let err = no_sam.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("SAM") && msg.contains("i2p"), "{msg}");
    }

    #[tokio::test]
    async fn i2p_accept_incoming_forwards_to_loopback() {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::{TcpListener, TcpStream};

        async fn write_line(s: &mut TcpStream, line: &str) {
            s.write_all(line.as_bytes()).await.unwrap();
            s.write_all(b"\n").await.unwrap();
            s.flush().await.unwrap();
        }
        async fn read_line(s: &mut TcpStream) -> Option<String> {
            let mut reader = BufReader::new(s);
            let mut line = String::new();
            let n = reader.read_line(&mut line).await.ok()?;
            if n == 0 {
                return None;
            }
            Some(line.trim_end_matches(['\r', '\n']).to_string())
        }

        let log = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let log_acc = Arc::clone(&log);
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    break;
                };
                let log = Arc::clone(&log_acc);
                tokio::spawn(async move {
                    loop {
                        let Some(line) = read_line(&mut s).await else {
                            break;
                        };
                        let up = line.to_ascii_uppercase();
                        if up.starts_with("HELLO VERSION") {
                            write_line(&mut s, "HELLO REPLY RESULT=OK VERSION=3.1").await;
                        } else if up.starts_with("SESSION CREATE") {
                            write_line(&mut s, "SESSION STATUS RESULT=OK DESTINATION=fakeprivdest")
                                .await;
                        } else if up.starts_with("STREAM FORWARD") {
                            log.lock().unwrap().push(line);
                            write_line(&mut s, "STREAM STATUS RESULT=OK").await;
                        } else if up.starts_with("STREAM CONNECT") {
                            write_line(&mut s, "STREAM STATUS RESULT=OK").await;
                            break;
                        }
                    }
                });
            }
        });

        let dir = tmp_datadir();
        let dest_path = dir.join("i2p").join("p2p.priv");
        let mut sam = rbitcoin_net::I2pSam::connect_persistent(addr, &dest_path)
            .await
            .unwrap();
        sam.stream_forward(18444).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&dest_path).unwrap().trim(),
            "fakeprivdest"
        );
        let fw = log.lock().unwrap().clone();
        assert_eq!(fw.len(), 1, "{fw:?}");
        assert!(fw[0].contains("PORT=18444"), "{}", fw[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_net_i2p_without_sam_is_config_error() {
        let mut c = NodeConfig::default();
        c.apply_kv("only_net", "i2p").unwrap();
        let err = c.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("SAM") && msg.contains("i2p"), "{msg}");
        c.apply_kv("i2p_sam", "").unwrap();
        c.validate().unwrap();
        let ok = ready_config(["rbitcoin-node", "--only-net", "i2p", "--i2p-sam"]);
        assert_eq!(ok.listen.only_net, vec![rbitcoin_net::OnlyNet::I2p]);
        assert_eq!(ok.listen.i2p_sam, Some("127.0.0.1:7656".parse().unwrap()));
    }

    #[test]
    fn only_net_cjdns_without_reachable_is_config_error() {
        let mut c = NodeConfig::default();
        c.apply_kv("only_net", "cjdns").unwrap();
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("cjdns-reachable"), "{err}");
        c.apply_kv("cjdns_reachable", "1").unwrap();
        c.validate().unwrap();
        let ok = ready_config(["rbitcoin-node", "--only-net", "cjdns", "--cjdns-reachable"]);
        assert_eq!(ok.listen.only_net, vec![rbitcoin_net::OnlyNet::Cjdns]);
        assert!(ok.listen.cjdns_reachable);
        ok.validate().unwrap();
    }

    #[test]
    fn cjdns_connect_without_reachable_is_config_error() {
        let mut c = NodeConfig::default();
        c.apply_kv("connect", "[fc00:1:2:3:4:5:6:7]:8333").unwrap();
        assert!(matches!(
            c.listen.connect[0],
            rbitcoin_net::NetAddr::Cjdns { .. }
        ));
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("cjdns-reachable"), "{err}");
        c.apply_kv("cjdns_reachable", "1").unwrap();
        c.validate().unwrap();
    }

    #[test]
    fn no_discover_conf() {
        let _g = OPERATOR_ENV_TEST_LOCK.lock().unwrap();
        assert!(NodeConfig::default().listen.discover);
        let off = ready_config(["rbitcoin-node", "--no-discover"]);
        assert!(!off.listen.discover);
        let mut c = NodeConfig::default();
        assert_eq!(c.apply_kv("no_discover", "1").unwrap(), ConfApply::Applied);
        assert!(!c.listen.discover);
        c.apply_kv("no_discover", "0").unwrap();
        assert!(c.listen.discover);
    }

    #[test]
    fn kebab_seed_node_and_min_relay_tx_fee_parse() {
        let seeds = ready_config(["rbitcoin-node", "--seed-node", "127.0.0.1:8333"]);
        assert_eq!(seeds.listen.seednodes, vec!["127.0.0.1:8333".to_string()]);
        let fee = ready_config(["rbitcoin-node", "--min-relay-tx-fee", "0.00001000"]);
        assert_eq!(fee.mempool.min_relay_fee_btc.as_deref(), Some("0.00001000"));
        let compact = ready_config(["rbitcoin-node", "--prefill-compact=0"]);
        assert!(!compact.prefill_compact);
        let rpc = ready_config(["rbitcoin-node", "--network", "regtest", "--rpc-listen"]);
        assert!(rpc.rpc.socket);
        assert_eq!(rpc.rpc.listen.unwrap().port(), 18443);
        assert_eq!(rpc.rpc.listen.unwrap().ip().to_string(), "127.0.0.1");
        let sock = ready_config(["rbitcoin-node", "--rpc"]);
        assert!(sock.rpc.socket);
        assert!(sock.rpc.listen.is_none());
        let el = ready_config(["rbitcoin-node", "--sh-index", "--electrum-listen"]);
        assert_eq!(el.listen.electrum.unwrap().port(), 50001);
        let es = ready_config(["rbitcoin-node", "--sh-index", "--esplora-listen"]);
        match es.listen.esplora.unwrap() {
            rbitcoin_esplora::EsploraListen::Tcp(a) => assert_eq!(a.port(), 3000),
            #[cfg(unix)]
            rbitcoin_esplora::EsploraListen::Unix(_) => panic!("default esplora-listen is TCP"),
        }
        #[cfg(unix)]
        {
            let es_unix = ready_config([
                "rbitcoin-node",
                "--sh-index",
                "--esplora-listen",
                "/tmp/esplora.sock",
            ]);
            match es_unix.listen.esplora.unwrap() {
                rbitcoin_esplora::EsploraListen::Unix(p) => {
                    assert_eq!(p, std::path::PathBuf::from("/tmp/esplora.sock"))
                }
                rbitcoin_esplora::EsploraListen::Tcp(_) => panic!("path must be unix"),
            }
        }
    }

    #[test]
    fn check_blocks_cli_parses_zero_and_negative() {
        let omitted = ready_config(["rbitcoin-node"]);
        assert_eq!(omitted.check_blocks, None);
        assert_eq!(
            omitted.check_blocks_window(),
            rbitcoin_store::VERIFY_TIP_BLOCKS
        );
        let six = ready_config(["rbitcoin-node", "--check-blocks=6"]);
        assert_eq!(six.check_blocks, Some(6));
        assert_eq!(six.check_blocks_window(), 6);
        let all = ready_config(["rbitcoin-node", "--check-blocks", "0"]);
        assert_eq!(all.check_blocks, Some(0));
        assert_eq!(all.check_blocks_window(), 0);
        let neg = ready_config(["rbitcoin-node", "--check-blocks=-1"]);
        assert_eq!(neg.check_blocks, Some(-1));
        assert_eq!(neg.check_blocks_window(), 0);
    }

    #[test]
    fn prefillcompact_omitted_is_on_zero_disables() {
        let omitted = ready_config(["rbitcoin-node"]);
        assert!(omitted.prefill_compact);
        let off = ready_config(["rbitcoin-node", "--prefill-compact=0"]);
        assert!(!off.prefill_compact);
        let on = ready_config(["rbitcoin-node", "--prefill-compact"]);
        assert!(on.prefill_compact);
        let on_eq = ready_config(["rbitcoin-node", "--prefill-compact=1"]);
        assert!(on_eq.prefill_compact);
    }

    #[test]
    fn max_sh_creates_and_esplora_block_template_cli_hyphens() {
        let omitted = ready_config(["rbitcoin-node"]);
        assert_eq!(omitted.max_sh_creates, 0);
        assert!(!omitted.esplora_block_template);
        let n = ready_config(["rbitcoin-node", "--max-sh-creates", "42"]);
        assert_eq!(n.max_sh_creates, 42);
        let eq = ready_config(["rbitcoin-node", "--max-sh-creates=9"]);
        assert_eq!(eq.max_sh_creates, 9);
        let gbt = ready_config(["rbitcoin-node", "--esplora-block-template"]);
        assert!(gbt.esplora_block_template);
        let gbt_eq = ready_config(["rbitcoin-node", "--esplora-block-template=1"]);
        assert!(gbt_eq.esplora_block_template);
        let off = ready_config(["rbitcoin-node", "--esplora-block-template=0"]);
        assert!(!off.esplora_block_template);
        assert!(NodeConfig::default().esplora_onion);
        let onion_off = ready_config(["rbitcoin-node", "--esplora-onion=0"]);
        assert!(!onion_off.esplora_onion);
    }

    #[test]
    fn explicit_milestone_zero_sticks_on_mainnet() {
        use rbitcoin_consensus::{default_milestone_height, Milestone};

        let omitted = ready_config(["rbitcoin-node"]);
        assert_eq!(omitted.network, Network::Mainnet);
        assert_eq!(
            omitted.milestone_height,
            default_milestone_height(Network::Mainnet)
        );
        assert!(omitted.milestone().skips_scripts_at(1));

        let cli0 = ready_config(["rbitcoin-node", "--milestone", "0"]);
        assert_eq!(cli0.network, Network::Mainnet);
        assert_eq!(cli0.milestone_height, 0);
        assert_eq!(cli0.milestone(), Milestone::NONE);
        assert!(!cli0.milestone().skips_scripts_at(1));

        assert!(operator_config_from_args(["rbitcoin-node", "--assumevalid-height=0"]).is_err());

        assert!(
            matches!(
                operator_config_from_args(["rbitcoin-node", "-V"]),
                Ok(OperatorArgs::Version)
            ),
            "-V must assemble Version before run"
        );
        assert!(
            operator_config_from_args(["rbitcoin-node", "--conf="]).is_err(),
            "empty --conf= must fail"
        );
        match operator_config_from_args(["rbitcoin-node", "--log-level=off"]) {
            Ok(OperatorArgs::Ready {
                log_level_cli: Some(None),
                ..
            }) => {}
            other => panic!("--log-level=off must be Ready with log off, got {other:?}"),
        }
        assert!(operator_config_from_args(["rbitcoin-node", "--log-level"]).is_err());

        let dir = tmp_datadir();
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("m.conf");
        std::fs::write(&conf, "milestone=0\n").unwrap();
        let from_conf = ready_config(["rbitcoin-node", "--conf", conf.to_str().unwrap()]);
        assert_eq!(from_conf.network, Network::Mainnet);
        assert_eq!(from_conf.milestone_height, 0);
        assert_eq!(from_conf.milestone(), Milestone::NONE);
        let eq = format!("--conf={}", conf.to_str().unwrap());
        let from_eq = ready_config(["rbitcoin-node", eq.as_str()]);
        assert_eq!(from_eq.milestone_height, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flag_matrix_cli_equals_conf_apply_kv() {
        let _g = OPERATOR_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut cfg = crate::config::NodeConfig::default();
        assert_eq!(
            cfg.apply_kv("network", "regtest").unwrap(),
            crate::config::ConfApply::Applied
        );
        assert_eq!(cfg.network, Network::Regtest);
        assert_eq!(
            cfg.apply_kv("chain", "signet").unwrap(),
            crate::config::ConfApply::Unknown("chain".into())
        );
        assert_eq!(cfg.network, Network::Regtest);

        let dir = tmp_datadir();
        assert_exit(
            cli_main([
                "rbitcoin-node",
                "--smoke",
                "--network=regtest",
                "--datadir",
                dir.to_str().unwrap(),
                "--no-seeds=1",
                "--log-level",
                "error",
                "--milestone",
                "0",
            ]),
            ExitCode::SUCCESS,
        );
        let _ = std::fs::remove_dir_all(&dir);

        assert_exit(
            cli_main(["rbitcoin-node", "--chain=regtest"]),
            ExitCode::from(2),
        );

        let dir = tmp_datadir();
        let conf = dir.join("node.conf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&conf, "network=testnet\n").unwrap();
        let smoke = dir.join("smoke");
        assert_exit(
            cli_main([
                "rbitcoin-node",
                "--smoke",
                "--conf",
                conf.to_str().unwrap(),
                "--network=regtest",
                "--datadir",
                smoke.to_str().unwrap(),
                "--no-seeds",
                "--log-level",
                "error",
                "--milestone",
                "0",
            ]),
            ExitCode::SUCCESS,
        );
        assert!(
            smoke.join("store").exists(),
            "CLI datadir must win over conf"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn testactivationheight_cli_smoke_regtest() {
        let _g = OPERATOR_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tmp_datadir();
        let code = cli_main([
            "rbitcoin-node",
            "--smoke",
            "--network",
            "regtest",
            "--datadir",
            dir.to_str().unwrap(),
            "--test-activation-height=csv@102",
            "--test-activation-height=dersig@50",
            "--trusted",
            "--limit-cluster-count=10",
            "--min-chain-work=0x65",
            "--no-seeds",
            "--log-level",
            "error",
            "--milestone",
            "0",
        ]);
        assert_exit(code, ExitCode::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn minimumchainwork_rejects_non_hex() {
        let dir = tmp_datadir();
        let code = cli_main([
            "rbitcoin-node",
            "--smoke",
            "--network",
            "regtest",
            "--datadir",
            dir.to_str().unwrap(),
            "--min-chain-work=test",
            "--log-level",
            "error",
        ]);
        assert_exit(code, ExitCode::from(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_and_missing_value_errors() {
        assert_exit(cli_main(["rbitcoin-node", "--nope"]), ExitCode::from(2));
        assert_exit(cli_main(["rbitcoin-node", "--network"]), ExitCode::from(2));
        assert_exit(
            cli_main(["rbitcoin-node", "--network", "bogus"]),
            ExitCode::from(2),
        );
        assert_exit(cli_main(["rbitcoin-node", "--datadir"]), ExitCode::from(2));
        assert_exit(
            cli_main(["rbitcoin-node", "--datadir-cold"]),
            ExitCode::from(2),
        );
        assert_exit(
            cli_main(["rbitcoin-node", "--listen", "not-an-addr"]),
            ExitCode::from(2),
        );
        assert_exit(
            cli_main(["rbitcoin-node", "--log-level", "wat"]),
            ExitCode::from(2),
        );
        assert_exit(cli_main(["rbitcoin-node", "--api-log"]), ExitCode::from(2));
        assert_exit(cli_main(["rbitcoin-node", "--asmap"]), ExitCode::from(2));
        assert_exit(
            cli_main(["rbitcoin-node", "--max-outbound", "0"]),
            ExitCode::from(2),
        );
        assert_exit(
            cli_main(["rbitcoin-node", "--mempool-size-mb", "0"]),
            ExitCode::from(2),
        );
        // Missing values / parse rejects for advanced knobs.
        assert_exit(cli_main(["rbitcoin-node", "--conf"]), ExitCode::from(2));
        assert_exit(
            cli_main(["rbitcoin-node", "--max-inbound"]),
            ExitCode::from(2),
        );
        assert_exit(
            cli_main(["rbitcoin-node", "--max-inbound", "nope"]),
            ExitCode::from(2),
        );
        assert_exit(
            cli_main(["rbitcoin-node", "--sp-tweaks-dust"]),
            ExitCode::from(2),
        );
        assert_exit(
            cli_main(["rbitcoin-node", "--sp-tweaks-dust", "nope"]),
            ExitCode::from(2),
        );
        // Bad conf path / invalid conf log_level.
        let dir = tmp_datadir();
        std::fs::create_dir_all(&dir).unwrap();
        assert_exit(
            cli_main([
                "rbitcoin-node",
                "--conf",
                dir.join("missing.conf").to_str().unwrap(),
                "--datadir",
                dir.join("d").to_str().unwrap(),
            ]),
            ExitCode::from(2),
        );
        let conf = dir.join("badlog.conf");
        std::fs::write(&conf, "log_level=notalevel\nnetwork=regtest\n").unwrap();
        assert_exit(
            cli_main([
                "rbitcoin-node",
                "--smoke",
                "--conf",
                conf.to_str().unwrap(),
                "--datadir",
                dir.join("d2").to_str().unwrap(),
                "--no-seeds",
            ]),
            ExitCode::from(2),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn smoke_datadir_cold_puts_inwit_on_cold_store() {
        let _g = OPERATOR_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tmp_datadir();
        let hot = dir.join("hot");
        let cold = dir.join("cold");
        let code = cli_main([
            "rbitcoin-node",
            "--smoke",
            "--network",
            "regtest",
            "--datadir",
            hot.to_str().unwrap(),
            "--datadir-cold",
            cold.to_str().unwrap(),
            "--no-seeds",
            "--log-level",
            "error",
            "--milestone",
            "0",
        ]);
        assert_exit(code, ExitCode::SUCCESS);
        assert!(hot.join("store").is_dir());
        assert!(hot.join("store/txout.body").is_file());
        assert!(!hot.join("store/inwit.body").exists());
        assert!(cold.join("store/inwit.body").is_file());
        assert!(cold.join("store/inwit.loc").is_file());
        assert!(hot.join("store").join("inwit.reloc").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_signet_cli_smoke() {
        let dir = tmp_datadir();
        let code = cli_main([
            "rbitcoin-node",
            "--smoke",
            "--network",
            "signet",
            "--datadir",
            dir.to_str().unwrap(),
            "--signet-challenge",
            "51",
            "--signet-block-time",
            "60",
            "--no-seeds",
            "--log-level",
            "error",
            "--milestone",
            "0",
        ]);
        assert_exit(code, ExitCode::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_cli_flags_reject_core_aliases() {
        let _g = OPERATOR_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tmp_datadir();
        let code = cli_main([
            "rbitcoin-node",
            "--smoke",
            "--network",
            "regtest",
            "--datadir",
            dir.to_str().unwrap(),
            "--milestone",
            "0",
            "--max-inbound",
            "5",
            "--mempool-size-mb",
            "8",
            "--log-level",
            "error",
            "--no-seeds",
        ]);
        assert_exit(code, ExitCode::SUCCESS);
        for flag in [
            "--chain=regtest",
            "--assumevalid-height=0",
            "--maxconnections=5",
            "--maxmempool=8",
            "--whitelist=noban@127.0.0.1",
            "--blocksonly",
            "--minimumchainwork=0x65",
            "--maxtipage=3600",
            "--uacomment=x",
            "--peertimeout=1",
            "--prefillcompact=0",
            "--limitclustercount=10",
            "--limitclustersize=10",
            "--minrelaytxfee=0.0001",
            "--mempoolexpiry=1",
            "--externalip=1.2.3.4",
            "--seednode=127.0.0.1:1",
            "--mocktime=1",
            "--blockversion=1",
            "--blockmintxfee=0.00000001",
            "--alertnotify=echo",
            "--startupnotify=echo",
            "--testactivationheight=csv@102",
            "--rpcworkqueue=1",
            "--datadircold=/tmp/x",
            "--electrumlisten=127.0.0.1:1",
            "--esploralisten=127.0.0.1:1",
            "--maxshcreates=1",
            "--esplorablocktemplate=1",
            "--apilog=/tmp/x",
            "--maxrunsecs=1",
            "--inhibitsuspend=1",
            "--rpclisten=127.0.0.1:1",
            "--rpc-user=u",
            "--rpcuser=u",
            "--rpcpassword=p",
            "--checkblocks=6",
            "--blocksdir=/tmp/x",
            "--blocks-dir=/tmp/x",
            "--whitelist-relay=0",
            "--whitelist-forcerelay=1",
            "--shindex",
            "--sptweaks",
            "-shindex",
            "-sptweaks",
            "-datadir=/tmp/x",
        ] {
            assert_exit(cli_main(["rbitcoin-node", flag]), ExitCode::from(2));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conf_file_then_cli_override() {
        let _g = OPERATOR_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tmp_datadir();
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("node.conf");
        std::fs::write(&conf, "network=signet\nmax_inbound=33\n").unwrap();
        let data = dir.join("data");
        let code = cli_main([
            "rbitcoin-node",
            "--smoke",
            "--conf",
            conf.to_str().unwrap(),
            "--datadir",
            data.to_str().unwrap(),
            "--network",
            "regtest", // CLI overrides conf network
            "--log-level",
            "error",
            "--no-seeds",
            "--milestone",
            "0",
        ]);
        assert_exit(code, ExitCode::SUCCESS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CLI omit of inbound must not clobber pre-set advanced envs.
    #[test]
    fn cli_omit_preserves_advanced_env() {
        let _g = OPERATOR_ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("RBITCOIN_P2P_MAX_INBOUND", "91");
        let dir = tmp_datadir();
        let code = cli_main([
            "rbitcoin-node",
            "--smoke",
            "--network",
            "regtest",
            "--datadir",
            dir.to_str().unwrap(),
            "--log-level",
            "error",
            "--no-seeds",
            "--milestone",
            "0",
            // no --max-inbound
        ]);
        assert_exit(code, ExitCode::SUCCESS);
        assert_eq!(
            std::env::var("RBITCOIN_P2P_MAX_INBOUND").as_deref(),
            Ok("91")
        );
        std::env::remove_var("RBITCOIN_P2P_MAX_INBOUND");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
