#[path = "sample_payment.rs"]
mod payment;

#[test]
fn cumulative_refunds_cannot_exceed_payment() {
    let first = payment::refund(1000, 1000);
    let second = payment::refund(1000, 1000);
    assert_eq!(first + second, 1000, "two refunds must not exceed the original payment");
}
