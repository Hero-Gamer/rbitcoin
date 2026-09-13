#![no_main]

use libfuzzer_sys::fuzz_target;
use rbitcoin_net::AsMap;

fuzz_target!(|data: &[u8]| {
    // asmap bytecode, then leftover 16-byte IP (zeros when len <= 16).
    let (asmap, ip16) = if data.len() > 16 {
        let (asmap, ip) = data.split_at(data.len() - 16);
        let mut ip16 = [0u8; 16];
        ip16.copy_from_slice(ip);
        (asmap, ip16)
    } else {
        (data, [0u8; 16])
    };
    if let Some(m) = AsMap::from_bytes(asmap.to_vec()) {
        let _ = m.interpret_ip16(&ip16);
    }
});
