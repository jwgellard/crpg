//! T129 audit D6: a backwards timestamp must not rewind the bucket watermark.
//!
//! `TokenBucket` keeps its watermark (`last_ms`) at the highest timestamp it
//! has credited for. A backwards (or equal) call accrues nothing and leaves
//! that watermark where it was, so every later credit is measured from the
//! highest timestamp seen. These tests fail against the old unconditional
//! `self.last_ms = now_ms`, which let a backwards call lower the watermark and
//! double-credit the gap afterwards.
//!
//! Units: `TokenBucket::new(2, 2)` is a 2-token burst refilling 2 tokens/s,
//! and `refill_mt_per_ms` is the per-second rate, so 2 tokens/s = 2
//! milli-tokens per millisecond. `available()` floors to whole tokens.

use crpg_net::sim::TokenBucket;

/// Regression: a backwards zero-amount advance between two spends must not
/// rewind the watermark, so the final spend cannot double-credit the gap.
#[test]
fn backwards_zero_amount_advance_never_double_credits_the_gap() {
    let mut bucket = TokenBucket::new(2, 2);
    assert!(bucket.consume(0, 2), "drains the full burst");
    assert_eq!(bucket.available(), 0);
    // 600 ms at 2 mt/ms credits 1200 mt = one whole token.
    assert!(bucket.consume(600, 0));
    assert_eq!(bucket.available(), 1);
    // Spending one token leaves 1200 - 1000 = 200 mt.
    assert!(bucket.consume(600, 1));
    assert_eq!(bucket.available(), 0);
    // Backwards to 200: accrues nothing and must not move the watermark back.
    assert!(bucket.consume(200, 0));
    assert_eq!(bucket.available(), 0);
    // Forward to 800: only (800 - 600) * 2 = 400 mt may accrue, giving 600 mt
    // (still under a whole token). Under the rewind bug the watermark sat at
    // 200 and this credited (800 - 200) * 2 = 1200 mt, a second 1000 mt.
    assert!(bucket.consume(800, 0));
    assert_eq!(bucket.available(), 0);
    // 600 mt < 1000 mt needed, so the spend must fail; the bug would have left
    // 1400 mt and succeeded.
    assert!(
        !bucket.consume(800, 1),
        "backwards step must not double-credit the gap"
    );
}

/// Regression: a backwards *spend* must not rewind the watermark either.
#[test]
fn backwards_consume_never_double_credits_the_gap() {
    let mut bucket = TokenBucket::new(2, 2);
    assert!(bucket.consume(0, 2), "drains the full burst");
    // 500 ms at 2 mt/ms credits exactly 1000 mt = one whole token.
    assert!(bucket.consume(500, 0));
    assert_eq!(bucket.available(), 1);
    // Backwards spend from 500 to 400: accrues nothing, empties the balance,
    // and must leave the watermark at 500.
    assert!(bucket.consume(400, 1));
    assert_eq!(bucket.available(), 0);
    // Forward to 998: only (998 - 500) * 2 = 996 mt may accrue.
    assert!(bucket.consume(998, 0));
    assert_eq!(bucket.available(), 0);
    // 996 mt < 1000 mt: must fail. Under the rewind bug the watermark sat at
    // 400 and this credited (998 - 400) * 2 = 1196 mt and succeeded.
    assert!(
        !bucket.consume(998, 1),
        "backwards spend must not double-credit the gap"
    );
}

/// Forward-only control: the test-1 sequence with the backwards step removed
/// still credits exactly what the contract says, proving the fix did not stop
/// forward accrual.
#[test]
fn forward_only_advance_still_credits_exactly_one_jump() {
    let mut bucket = TokenBucket::new(2, 2);
    assert!(bucket.consume(0, 2), "drains the full burst");
    // One forward jump 0 -> 600 credits exactly 1200 mt.
    assert!(bucket.consume(600, 0));
    assert_eq!(bucket.available(), 1);
    assert!(bucket.consume(600, 1));
    assert_eq!(bucket.available(), 0);
    // One forward jump 600 -> 800 credits exactly 400 mt, giving 600 mt.
    assert!(bucket.consume(800, 0));
    assert_eq!(bucket.available(), 0);
    // Same contract-correct refusal as the backwards path, with no rewind.
    assert!(!bucket.consume(800, 1));
}
