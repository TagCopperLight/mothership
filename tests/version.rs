use super::*;

#[test]
fn versions_parse_with_a_prefix_and_a_suffix() {
    let version = Version::parse("v16.4.3-alpine").unwrap();
    assert_eq!(version.major(), 16);
    assert!(Version::parse("16.4").unwrap() < Version::parse("16.10").unwrap());
    for bad in ["latest", "16.x", "", "16.", "16-", "alpine"] {
        assert!(Version::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn versions_of_another_shape_dont_compare() {
    let (short, long) = (
        Version::parse("16.4").unwrap(),
        Version::parse("16.4.1").unwrap(),
    );
    assert_eq!(short.partial_cmp(&long), None);
    let alpine = Version::parse("16.4-alpine").unwrap();
    assert!(!short.follows(&alpine));
}

#[test]
fn an_update_keeps_the_major_and_never_goes_back() {
    check_update("16.4.3", "16.5.0").unwrap();
    check_update("16.4.3", "16.4.3").unwrap();
    let err = |old, new| check_update(old, new).unwrap_err().to_string();
    assert_eq!(err("16.4.3", "17.0.0"), "17.0.0 leaves major version 16");
    assert_eq!(err("16.4.3", "16.4.2"), "16.4.2 is older than 16.4.3");
    assert_eq!(err("16.4", "16.4.1"), "16.4.1 isn't written like 16.4");
}
