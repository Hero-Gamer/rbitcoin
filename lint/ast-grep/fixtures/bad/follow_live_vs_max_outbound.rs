fn stale_follow_needs_room(follow_live: usize, max_outbound: usize) -> bool {
    false // cap deleted - unbounded follow_live
}
