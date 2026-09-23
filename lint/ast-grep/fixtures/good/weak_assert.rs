fn strong() {
    let r: Result<u32, ()> = Ok(1);
    assert_eq!(r.unwrap(), 1);
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        // Exact value, in a file this rule actually scans.
        match Ok::<u32, ()>(42) {
            Ok(v) => assert_eq!(v, 42),
            Err(e) => panic!("failed: {:?}", e),
        }
    }
}
