use rbitcoin_ibd::ibd::PeerBookSession;
use rbitcoin_net::seeds::AddrMan;
use std::net::SocketAddr;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::Mutex;

fn sa(o: u8) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, o)), 18444)
}

#[test]
fn peer_book_session_injects_seeds_and_flushes_on_drop() {
    let shared = Arc::new(Mutex::new(AddrMan::new()));
    {
        let mut sess = PeerBookSession::new(
            Some(Arc::clone(&shared)),
            &[
                rbitcoin_net::NetAddr::Ip(sa(1)),
                rbitcoin_net::NetAddr::Ip(sa(2)),
            ],
        );
        assert!(sess
            .book()
            .entry_of(&rbitcoin_net::NetAddr::from_socket(sa(1)))
            .is_some());
        assert!(sess
            .book()
            .entry_of(&rbitcoin_net::NetAddr::from_socket(sa(2)))
            .is_some());
        sess.book_mut().add(sa(3));
        assert!(sess
            .book()
            .entry_of(&rbitcoin_net::NetAddr::from_socket(sa(3)))
            .is_some());
        // Shared not yet flushed until drop/flush.
        assert!(shared
            .lock()
            .unwrap()
            .entry_of(&rbitcoin_net::NetAddr::from_socket(sa(3)))
            .is_none());
        sess.flush();
        assert!(shared
            .lock()
            .unwrap()
            .entry_of(&rbitcoin_net::NetAddr::from_socket(sa(3)))
            .is_some());
    }
    // Drop flushes again (idempotent).
    assert!(shared
        .lock()
        .unwrap()
        .entry_of(&rbitcoin_net::NetAddr::from_socket(sa(1)))
        .is_some());

    // No shared book — seeds only, flush is a no-op.
    let sess2 = PeerBookSession::new(None, &[rbitcoin_net::NetAddr::Ip(sa(9))]);
    assert!(sess2
        .book()
        .entry_of(&rbitcoin_net::NetAddr::from_socket(sa(9)))
        .is_some());
    sess2.flush();
}

/// Rows another writer adds to the shared book during catch-up (the
/// deferred DNS seeds) reach this session and survive its flush. A row
/// the session dropped is not pulled back in.
#[test]
fn peer_book_session_pulls_late_shared_rows() {
    let net = |o| rbitcoin_net::NetAddr::from_socket(sa(o));
    let shared = Arc::new(Mutex::new(AddrMan::new()));
    shared.lock().unwrap().add(sa(1));
    shared.lock().unwrap().add(sa(2));
    let mut sess = PeerBookSession::new(Some(Arc::clone(&shared)), &[]);
    sess.book_mut().evict_one(net(2));
    shared.lock().unwrap().add(sa(3));
    assert!(sess.book().entry_of(&net(3)).is_none());
    sess.pull_shared();
    assert!(sess.book().entry_of(&net(3)).is_some(), "late row pulled");
    assert!(
        sess.book().entry_of(&net(2)).is_none(),
        "evicted row stays out"
    );
    shared.lock().unwrap().add(sa(4));
    drop(sess);
    let book = shared.lock().unwrap();
    for o in [1, 3, 4] {
        assert!(book.entry_of(&net(o)).is_some(), "row {o} after flush");
    }
    assert!(book.entry_of(&net(2)).is_none());
}

/// A row copied between the shared book and the session keeps the source
/// group that taught it, so the group's share of a full book still holds.
/// A row both books hold keeps the destination's source.
#[test]
fn peer_book_session_keeps_each_rows_source() {
    use rbitcoin_net::seeds::{MAX_ADDR_MAN, MAX_ROWS_PER_SOURCE_GROUP};
    let row = |i: u32| {
        let [_, b, c, d] = i.to_be_bytes();
        SocketAddr::from(([11, b, c, d], 8333))
    };
    let src = |b: u8| SocketAddr::from(([44, b, 0, 1], 8333));
    let shared = Arc::new(Mutex::new(AddrMan::new()));
    let mut sess = PeerBookSession::new(Some(Arc::clone(&shared)), &[]);
    let share = MAX_ROWS_PER_SOURCE_GROUP as u32;
    for i in 0..share {
        assert!(shared
            .lock()
            .unwrap()
            .add_learned(row(i), src(1), MAX_ADDR_MAN));
    }
    let both = row(share);
    assert!(sess.book_mut().add_learned(both, src(2), MAX_ADDR_MAN));
    assert!(shared
        .lock()
        .unwrap()
        .add_learned(both, src(3), MAX_ADDR_MAN));
    let both_net = rbitcoin_net::NetAddr::from_socket(both);
    let session_source = sess.book().source_group_of(both_net);
    let shared_source = shared.lock().unwrap().source_group_of(both_net);
    assert!(session_source.is_some() && session_source != shared_source);
    sess.pull_shared();
    assert_eq!(
        sess.book().source_tally(),
        (MAX_ROWS_PER_SOURCE_GROUP + 1, 2),
        "pulled rows keep their source; the shared row keeps the session's"
    );
    assert_eq!(sess.book().source_group_of(both_net), session_source);
    let pulled = rbitcoin_net::NetAddr::from_socket(row(0));
    assert_eq!(
        sess.book().source_group_of(pulled),
        shared.lock().unwrap().source_group_of(pulled)
    );
    let late = row(share + 1);
    assert!(shared
        .lock()
        .unwrap()
        .add_learned(late, src(4), MAX_ADDR_MAN));
    sess.flush();
    assert_eq!(
        shared.lock().unwrap().source_tally(),
        (MAX_ROWS_PER_SOURCE_GROUP + 2, 3),
        "a flush keeps the session's sources and the shared row it never took"
    );
    assert_eq!(
        shared.lock().unwrap().source_group_of(both_net),
        session_source
    );
}

/// The session and the shared book are both live during catch-up. A group
/// that filled each with its share does not hold both shares after the
/// flush, the book is not over its cap, and another group still learns. A
/// row both books hold still takes the other book's flags.
#[test]
fn a_flush_does_not_double_a_source_groups_share() {
    use rbitcoin_net::seeds::{MAX_ADDR_MAN, MAX_ROWS_PER_SOURCE_GROUP};
    let row = |i: u32| {
        let [_, b, c, d] = i.to_be_bytes();
        SocketAddr::from(([11, b, c, d], 8333))
    };
    let src = |b: u8| SocketAddr::from(([44, b, 0, 1], 8333));
    let mut start = AddrMan::new();
    for i in 0..(MAX_ADDR_MAN - MAX_ROWS_PER_SOURCE_GROUP) as u32 {
        start.add(row(i));
    }
    let shared = Arc::new(Mutex::new(start));
    let mut sess = PeerBookSession::new(Some(Arc::clone(&shared)), &[]);
    let both = row(100_000);
    for i in 0..MAX_ROWS_PER_SOURCE_GROUP as u32 {
        assert!(sess
            .book_mut()
            .add_learned(row(100_000 + i), src(1), MAX_ADDR_MAN));
        let other = if i == 0 { both } else { row(200_000 + i) };
        assert!(shared
            .lock()
            .unwrap()
            .add_learned(other, src(1), MAX_ADDR_MAN));
    }
    let both = rbitcoin_net::NetAddr::from_socket(both);
    shared.lock().unwrap().note_connected_addr(both);
    sess.flush();
    let mut book = shared.lock().unwrap();
    assert_eq!(book.len(), MAX_ADDR_MAN);
    assert_eq!(book.source_tally(), (MAX_ROWS_PER_SOURCE_GROUP, 1));
    assert!(book.flags_of(&both).has_connected());
    assert!(book.add_learned(row(300_000), src(2), MAX_ADDR_MAN));
}

/// The session book still charges a group for rows the shared book has
/// since evicted. A shared row the node connected to is kept by the flush
/// whatever its teacher's share.
#[test]
fn a_flush_keeps_a_shared_row_the_node_connected_to() {
    use rbitcoin_net::seeds::{MAX_ADDR_MAN, MAX_ROWS_PER_SOURCE_GROUP};
    let row = |i: u32| {
        let [_, b, c, d] = i.to_be_bytes();
        SocketAddr::from(([11, b, c, d], 8333))
    };
    let src = |b: u8| SocketAddr::from(([44, b, 0, 1], 8333));
    let mut start = AddrMan::new();
    for i in 0..MAX_ADDR_MAN as u32 {
        if (i as usize) < MAX_ROWS_PER_SOURCE_GROUP {
            assert!(start.add_learned(row(i), src(1), MAX_ADDR_MAN));
        } else {
            start.add(row(i));
        }
    }
    let shared = Arc::new(Mutex::new(start));
    let sess = PeerBookSession::new(Some(Arc::clone(&shared)), &[]);
    let connected = rbitcoin_net::NetAddr::from_socket(row(100_001));
    {
        let mut book = shared.lock().unwrap();
        assert!(book.add_learned(row(100_000), src(2), MAX_ADDR_MAN));
        assert!(book.add_learned(row(100_001), src(1), MAX_ADDR_MAN));
        book.note_connected_addr(connected);
    }
    sess.flush();
    assert!(shared.lock().unwrap().flags_of(&connected).has_connected());
}
