# Refund contract
An order can be refunded more than once, but cumulative refunds must never exceed
the amount originally paid. For a $10 order, two requests for $10 must issue
$10 and $0 (or reject the second request), not $10 twice.

The bundled regression tests the current implementation; failure is expected.
The review agent may read and test, but must not modify the source or claim a fix.
