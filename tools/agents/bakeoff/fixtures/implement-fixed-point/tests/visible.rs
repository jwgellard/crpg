use fx::{Fx, ParseFxError};

#[test]
fn mul_simple() {
    // 1.5 * 2.0 = 3.0
    assert_eq!(Fx(98304).mul_round(Fx(131072)), Fx(196608));
}

#[test]
fn div_by_zero_is_none() {
    assert_eq!(Fx::ONE.checked_div(Fx(0)), None);
}

#[test]
fn div_one_third_truncates() {
    assert_eq!(Fx::ONE.checked_div(Fx(196608)), Some(Fx(21845)));
}

#[test]
fn parse_basic() {
    assert_eq!(Fx::parse("1"), Ok(Fx::ONE));
    assert_eq!(Fx::parse("-1.5"), Ok(Fx(-98304)));
    assert_eq!(Fx::parse(".5"), Err(ParseFxError::Invalid));
    assert_eq!(Fx::parse("32768"), Err(ParseFxError::OutOfRange));
}
