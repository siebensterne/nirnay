# Broker-neutral OMS contract and independent live-risk authorization

**Status:** broker-neutral order-state reference implementation and cryptographic permit verifier implemented; live broker adapters are not implemented or certified. Live routing is disabled by design.

## 1. Current OMS scope

The active OMS implementation has two layers:

- `quantlab.oms` provides the broker-neutral order-domain state machine and immutable order intent contract.
- `quantlab.durable_oms.DurableOMSStore` provides a single-node SQLite WAL/FULL synchronous store. It atomically commits intent acceptance, hash-chained event history and outbox command creation in one transaction. It uses restrictive database/WAL/SHM permissions, leases, stable client order IDs, unique broker event/execution IDs, monotonic broker update sequence, partial fill accounting, average fill price and explicit cancel/unknown-outcome states.

`PaperOutboxDispatcher` is the only admitted dispatcher. It requires `mode="paper"` and refuses `mode="live"` before any adapter call. It also rejects a broker/adapter identity mismatch before dispatch. A dispatcher command moves through `PENDING -> CLAIMED -> DISPATCHING`; a lease expiring before dispatch may be reclaimed, but a lease expiring after dispatch has begun becomes `UNKNOWN` and is never auto-requeued. A broker timeout/transport exception after a side effect may have started is ambiguous and requires authoritative reconciliation. A definitive rejection is distinct from an unknown outcome. Cancel request acknowledgement is not terminal cancellation; a later broker event must establish `CANCELLED`, `WORKING`, `PARTIALLY_FILLED` or `FILLED` after the cancel/fill race.

The SQLite event hash chain detects accidental or unauthorized edits only relative to a trusted copy of the final hash. It is not externally anchored by itself. SQLite WAL/FULL is a single-node durability design, not replicated high availability, cross-host locking, or distributed exactly-once delivery. Backup/restore, disk-full, process-kill and platform-specific durability drills remain deployment acceptance requirements.

`BrokerAdapter` and `OutboxBroker` are contracts, not live implementations. No active Upstox, FYERS, Kite or other broker execution adapter is connected to the durable OMS. Kite is currently a separate macOS Keychain-backed OAuth/read-only account operations app; its order submission path is hard-blocked. Therefore this repository has a durable paper OMS core, not a certified production broker-neutral OMS.

## 1.1 Durable OMS conformance and recovery rules

- Accepting an intent is idempotent for the identical `client_order_id`,
  broker and canonical intent bytes. Reusing an ID with changed contract data
  is a conflict.
- Intent, `OUTBOX_CREATED`, and the submit command are committed in one SQLite transaction.
- A command claimed but not yet dispatched can be reclaimed after a lease expires. Once `DISPATCHING` is committed, no automatic resend is allowed after timeout or process loss.
- Reconciliation must compare broker order book, order updates, executions/trades and local durable state. It must distinguish `not found` from a stale/incomplete broker snapshot.
- Broker event IDs and execution IDs are unique per broker. The per-order broker sequence must be strictly increasing. Out-of-order, duplicate-with-different-content, sequence gaps and overfills are not silently repaired.
- A cancel command does not imply cancellation. Partial fill/cancel races must be represented by subsequent execution and order-status events.
- `verify_event_chain()` verifies the internal append-only event digest chain. A production audit must anchor signed chain heads to an independently controlled off-host store.
- Do not put credentials, full account identifiers or proprietary order payloads into logs or source control.

## 2. Signed permit verifier

`quantlab.live_auth` verifies, but does not mint, an authorization. The private signing keys must remain outside this repository in separately governed HSM/KMS or equivalent offline signer processes.

The verifier requires:

1. an externally anchored root signature over a time-bounded trust registry;
2. a trusted issuer signature over an exact canonical permit body;
3. at least two approvals from distinct trusted principals, with distinct roles and required roles satisfied;
4. distinct Ed25519 public-key material for root, issuer, independent risk service and each approver;
5. a separate risk-service signature over exact runtime evidence, including build commit, broker/account/strategy/instrument/venue/side/order type/product, quantity/notional, projected exposure/daily loss, kill-switch, mode, feed health, reconciliation, entitlement, session, clock skew, risk snapshot age and policy/strategy artifact hashes;
6. strict scope, positive bounded integer amounts, trusted timestamps, maximum permit/runtime-evidence TTL, current registry snapshot and non-revoked keys;
7. one-time consumption in a durable hash-chain ledger bound to client order ID and canonical order-contract SHA-256.

The cryptographic verifier is an additional precondition. The runtime evidence path is implemented in `quantlab.risk_evidence` and `quantlab.risk_service`: `SignedEvidenceRiskGate` verifies the offline root-signed trust registry and requires eight fresh signed control observations (account risk, kill switch, market-data health/gaps, broker reconciliation, broker entitlement, venue session, clock synchronization and deployed artifact/commit). The service then builds `RiskFacts` from those signed payloads and signs exact scoped `RuntimeEvidence`. The public `IndependentRiskService.evaluate_and_sign` rejects arbitrary caller-assembled `RiskFacts`; only the signed-observation path is admitted by the application API. The test suite covers invalid, stale, tampered, missing, out-of-scope and revoked-source cases.

`AwsKmsEd25519Signer` implements AWS KMS Ed25519 key metadata validation, public-key retrieval, `Sign` with `MessageType=RAW`, signature length checks and local verification. No AWS KMS key, IAM role/policy, separate risk service process, control collectors or CloudTrail configuration is provisioned here. The Python module boundary is not an OS/cloud security boundary: production risk service deployment must use separate process, workload identity, network, KMS key policy, operators and source data authorities. The verifier is not an approval UI, deployed HSM, broker certification, regulatory registration or entitlement oracle. Tests create ephemeral keys and fake KMS clients only to prove the protocol; those are not deployment keys.

## 3. Why live routing remains impossible

`OrderManagementSystem(mode="live")` always raises `LiveRoutingDisabled`, even if passed a verified permit. The current code has no certified broker adapter, no complete independent risk runtime, no broker reconciliation service, no exchange test evidence, and no production key-management deployment. No config switch, environment variable, test signature or feature flag can change that policy in this version.

The active Kite operations package now offers a macOS Keychain-backed local OAuth login and read-only profile/orders/positions/instrument operations. It is not silently promoted into the durable OMS, and its `submit_manual` method is unconditionally blocked in this build. Its broker order ID is an acknowledgement identifier, not proof of exchange receipt or fill.

## 4. Future production admission checklist

Before any live adapter may be built or enabled, require an ADR and independent evidence for:

- broker-specific order capabilities and lifecycle mapping, including partial fills, reject reasons, cancel/replace, order modifications and trade corrections;
- account-level auth/entitlement and exchange/broker API policy for the exact product and order type;
- pre-trade risk at a separate authority boundary (max notional, order/position/sector/concentration limits, gross/net exposure, margin, daily loss, kill switch, stale-data, duplicate order, price collars, rate limits);
- durable intent and outbox/idempotency protocol before side effects;
- unknown outcome reconciliation before retry, with no blind order resubmission;
- signed, scoped, short-lived authorization per exact order or tightly defined batch;
- replay-proof permit consumption and revocation/rotation procedure;
- broker stream gap detection, account/order/trade reconciliation and restart recovery;
- independent security review and key separation, tamper-evident audit and least-privilege deployment;
- paper/mock exchange and broker certification evidence, controlled canary, rollback and incident drills;
- applicable exchange/broker/SEBI permissions and registrations determined by qualified compliance counsel.

A small order value does not bypass the above controls. This repository must keep live trading off until all of them are demonstrably in place.
