use rbitcoin_rpc::methods::mempool::*;

/// A session answers only once fee history or flow is warm (thousands
/// of blocks or a minute of relay), so the success shape is pinned here.
#[test]
fn an_estimate_is_feerate_and_blocks() {
    let r = smart_fee_json(2, 0.000_020_00);
    assert_eq!(r["feerate"], sat_btc_json(2_000), "{r}");
    assert_eq!(r["blocks"], 2, "{r}");
    assert_eq!(
        r.as_object().unwrap().len(),
        2,
        "only feerate and blocks: {r}"
    );
    let none = smart_fee_json(6, -1.0);
    assert!(none.get("feerate").is_none(), "{none}");
    assert_eq!(none["blocks"], 6, "{none}");
    let off = smart_fee_json(12, 0.000_010_50);
    assert_eq!(off["blocks"], 12, "{off}");
    assert!(off.get("feerate").is_some(), "{off}");
    assert!(off.get("errors").is_none(), "{off}");
    let far = smart_fee_json(100, 0.000_008_00);
    assert_eq!(far["blocks"], 100, "{far}");
    assert!(far.get("feerate").is_some(), "{far}");
}
