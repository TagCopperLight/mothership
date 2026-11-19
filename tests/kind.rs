use super::*;

#[test]
fn a_kind_reads_back_from_its_name() {
    assert_eq!(Kind::UserService.name(), "user-service");
    assert!("user-service".parse::<Kind>() == Ok(Kind::UserService));
    assert!("userservice".parse::<Kind>().is_err());
}
