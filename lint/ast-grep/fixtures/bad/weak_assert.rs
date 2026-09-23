fn weak() {
    let r: Result<(), ()> = Ok(());
    assert!(r.is_err());
    assert!(r.is_ok());
    assert!(matches!(r, Err(_)));
    assert!(matches!(r, Ok(_)));
    assert_eq!(r.is_err(), true);
    assert_eq!(true, r.is_ok());
    assert!(r.is_ok_and(|_| true));
}
