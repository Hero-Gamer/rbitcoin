fn strong() {
    let r: Result<u32, ()> = Ok(1);
    assert_eq!(r.unwrap(), 1);
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        // Tier1: exact variant check inside test module
        match Ok::<u32, ()>(42) {
            Ok(v) => assert_eq!(v, 42),
            Err(e) => panic!("failed: {:?}", e),
        }
    }
}
