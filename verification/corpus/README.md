# Numeric and book differential corpus — version 1

These files are the shared input to the C++ oracle drivers and the Rust
counterparts. Both drivers read the identical bytes from one file; the
differential gate compares their stdout byte for byte and their exit-status
class (success versus failure).

## Book format

One operation per line:

- `abs <buy|sell> <raw_price> <raw_quantity>`
- `delta <buy|sell> <raw_price> <signed_delta>`
- `snapshot`

A snapshot prints one canonical JSON object with best bid first and best ask
first. Zero quantity removes a level. Invalid operations produce no stdout and
a non-zero exit status.

## Numeric format

One operation per line:

- `parse <decimal_text> <scale>`
- `format <raw> <scale>`
- `rescale <raw> <from_scale> <to_scale>`

Each operation prints one canonical result line. A rejected operation prints
`error <op>` and continues, so failure cases remain part of the stream.

The corpus is synthetic and contains no provider, account or venue data.
