fn stale_follow_needs_room(follow_live: usize, max_outbound: usize) -> bool {
    follow_live >= max_outbound.max(1)
}
