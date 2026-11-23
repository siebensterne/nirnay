# Provider boundary and capture semantics

## Upstox Market Data Feed V3

- Authenticate using `UPSTOX_ACCESS_TOKEN` only through process environment.
- Obtain an authorised WebSocket redirect URI from the V3 authorize endpoint for each connection.
- Send the documented subscription control object as a binary WebSocket message.
- Archive each binary payload through `market-archive::JournalWriter` before protobuf diagnostics.
- Keep the checked-in protobuf schema and its source hash in `schemas/upstox/v3/`.
- Distinguish Analytics Token smoke access from the Market Data Feed access token.
- Preserve the actual subscription mode in the session manifest. Current documented normal limits are two connections/user; LTPC 5,000 individual / 2,000 combined instrument keys; Option Greeks 3,000 individual / 2,000 combined; Full 2,000 individual / 1,500 combined. Upstox Plus documents Full D30 at 50 individual / 1,500 combined. These are plan/mode-dependent API ceilings, not a promise that any account is entitled to them.
- Standard `full` includes five levels; `full_d30` is a separate Plus mode with 30 levels. The active recorder/normalizer is scoped to its existing supported mode and must not claim D30 parsing or subscription until explicitly implemented and verified.
- Preserve `market_info`, initial snapshot and subsequent live messages in the raw frame journal. Do not mistake indicative equilibrium price (IEP) / closing auction fields for an executed trade or continuous-book quote. The active canonical model does not yet provide a full auction-session state model; exclude those intervals from executable simulation unless separately classified.

Raw record `source=upstox`, `provider_instrument_id=__multi_instrument_frame__` means the raw payload can contain several instrument keys. The separate diagnostic stream lists the keys inside the feed response. `capture_sequence` is local receive order and is not a provider sequence. The envelope's `currentTs` remains diagnostic; it is not relabelled as each instrument's exchange timestamp.

## FYERS Data Socket

- Use the vendor SDK token format `appid:accesstoken` supplied by `FYERS_ACCESS_TOKEN`.
- Subscribe to `SymbolUpdate` and optionally `DepthUpdate`.
- The current recorder uses the Data Socket SDK callbacks (`SymbolUpdate` and optional `DepthUpdate`). Its five-level `DepthUpdate` path is distinct from FYERS' separate Tick-by-Tick (TBT) Protobuf socket.
- Documented Data Socket symbol limits are 100/user for Standard and 5,000/user for Prime across connections. The separate TBT socket documents 50-level depth for NSE equity/NFO and limits of 1 symbol for Standard or 15 for Prime (up to three connections of five). These account-tier limits are external entitlements; they are not inferred by the recorder.
- `fyers-recorder --max-symbols N` is an operator-declared cap, defaults conservatively to 100, and accepts up to 5,000. Set it only after checking the account plan/entitlement. The current recorder does not implement TBT, 50-level depth or broker-side entitlement verification.
- Persist SDK callback JSON, wall-clock and monotonic receive time, local sequence, connection lifecycle and local queue gaps.

The SDK has already decoded the transport message. These records are callback observations, not original binary frames. No provider sequence is claimed. The manifest marks the capture as not research-grade until feed continuity, entitlement and data semantics are verified.

## Zerodha Kite

- Use `KITE_API_KEY` and `KITE_ACCESS_TOKEN` for account/order API operations.
- Do not infer live or historical market-data entitlement from Personal order access.
- Pin the authenticated profile user ID in the private policy before an order plan can pass.
- Refresh and hash the current IST session's NSE instrument CSV before an order plan.
- Restrict manual writes to regular CNC BUY LIMIT DAY, NSE cash, configured allowlist, current tick/lot, ₹1,000 notional and 100 shares maximum.
- Require a 3% cash reserve, no existing same-day order, no existing net position in the target symbol, static-IP, daily-auth and market-session dates matching the current IST session.
- Write durable intent and submission-attempt records before POST. Do not retry a request with an uncertain outcome. Reconcile by order tag and acknowledged order ID; compare the full order contract against the durable intent before reading fill state. Reject mismatched trade instruments, overfills and BUY executions above the limit.

The app does not infer fill from an `order_id`. It uses order-history and trade queries to reconstruct the observed state. Full order-update streaming, multi-order OMS state, reservation accounting and independent signed risk permits are not delivered by this operator surface.

## Shared boundary

Provider identity, exchange instrument identity and internal instrument identity are separate. Do not key internal domain records solely by provider symbol/token. Every captured session carries a non-secret entitlement reference. Account credentials, authorised websocket URLs and market-data rights are excluded from archives and source control.

## Frozen legacy capture paths

The Upstox and FYERS capture, decode and provider-config paths described above are legacy
frozen under RRP-004. They are not the canonical source authority; Fieldbook owns acquisition,
raw storage, source decoding and normalization under ADR-0001. No new feature or provider
operation is permitted in those paths. See `evidence/SOURCE-INGESTION-FREEZE-2026-11-23.md`
and `engineering/source-ingestion-freeze.v1.json` for the frozen list, target owners and
migration order.
