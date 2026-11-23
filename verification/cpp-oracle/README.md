# C++ numeric oracle (verification only)

This target provides a separately built exact-numeric reference for Nirnay. It
is **not linked into the production Rust workspace**, is not a broker adapter,
and is not a source of truth for trading/accounting semantics. D11 retired the
market-state/order-book oracle sources and the order-book corpus; only pure
numeric code remains in this target.

The source promoted from the predecessor tree is:

- `numeric.h` and `numeric.cpp`
- `numeric_driver.cpp` (fixed-point conformance driver)
- `tests/numeric_orderbook_test.cpp` (pure numeric boundary assertions)

The C++ test covers fixed-point parse/format/rescale, integer overflow
boundaries, rational exactness, tick grids, quantity steps and lot sizes. It
uses compiler-supported `__int128` for checked intermediate arithmetic; this
verification target requires GCC or Clang and makes no claim of ISO C++
portability.

Build and run independently:

```bash
cmake -S verification/cpp-oracle -B /tmp/nirnay-cpp-oracle -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/nirnay-cpp-oracle --parallel
ctest --test-dir /tmp/nirnay-cpp-oracle --output-on-failure
```

A true differential oracle requires the Rust and C++ drivers to consume the same
versioned numeric corpus and compare canonical outputs. This target does **not**
assert byte-for-byte equivalence with every active Rust implementation. Do not
use a C++ result to overrule an accepted Rust/domain contract without a reviewed
ADR.
