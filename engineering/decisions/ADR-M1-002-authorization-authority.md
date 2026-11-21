# ADR-M1-002 — One authorization authority (resolve the importable risk-token boundary)

- **Status:** Proposed (M1 design only — not implementation authority)
- **Owner group:** Architecture Design Group, M1 Platform Constitution
- **Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9`, read via `git show 01eb382e:<path>`
- **Governing inputs:** M0 baseline §8 + Amendment A5 (remedy gates) and A9.4; A5 F-A5-02, F-A5-03 (CLM-010 partially falsified), F-A5-09; R1 Q4/Q5; R2 §A (F-A5-03 affirmed); A1 F-A1-07; `ADR-0004-live-disabled.md`; `target-architecture.md:55-57`
- **Challenge addressed:** **Owner challenge 1 — resolve the importable risk-token boundary without assuming a remote service is the answer.**
- **Amendment 1 (2026-11-17):** Challenger A-1 and B-1 disposition recorded in §3/§5/§9. Per the **coordinator's ruling on challenger A-1**, the M1 design posture is a **single in-process authority**; separate-process isolation is an **explicitly unimplemented deployment precondition**, not a current control and not built by M1. No remote service is the answer in this design. Invariant-ID collisions (A-0) RESOLVED: `M1-ARCH-INV-*` namespace and authoritative `INVARIANT-REGISTRY.md` applied; cross-references updated.

---

## 1. Problem

M0 found **three unconnected authorization owners** and one partially falsified claim (baseline §4/§8; A5 F-A5-02/F-A5-03):

- **Python `live_auth.verify_live_permit`** (`research/python/src/quantlab/live_auth.py:112-358`) is the richest verifier: root-signed trust registry, eight distinct control-observation roles (`:239-240`), distinct principals (`:349-353`), runtime-evidence signature check (`:265-268`) and one-time `AuthorizationUseLedger.consume()` (`:416-417`). But its verdict is an **optional argument** to `oms.submit()` and is ignored for routing because live submission still raises `LiveRoutingDisabled` (`oms.py:242-247`).
- **Rust `execution-policy`** (`crates/execution-policy/src/lib.rs`) verifies issuer+approver envelopes and mode, but `OrderRequest` has only `{broker, account, strategy, instrument, notional}` (`:71-78`) — **no side, venue, order_type, product or quantity**; its own doc says those are "NOT enforced" (`:28-30`, `:786-789`). It is used **only in its own tests**; `ExecutionPolicy::live()` (`:635-640`) is a public constructor never composed into an OMS.
- **Rust `risk-engine`** (`crates/risk-engine/src/lib.rs:97-124`) is a pure `evaluate_risk` over a **caller-supplied** `RiskContext`; the sole caller (`paper-engine::engine.rs:532`) derives `feed_health` from its own replayed projection (R2 §4 refinement), but any future caller can fabricate `FeedHealth::Healthy{age_ns:0}`.

**CLM-010 is partially falsified** (A5 F-A5-03, affirmed by R2 §A): `evaluate_and_sign` correctly always raises (`risk_service.py:113-123`), but `_GATE_SIGNING_CAPABILITY = object()` at `risk_service.py:24-26` is a **module-level importable token**, and `_sign_after_verified_sources(facts, *, capability)` at `:192-204` signs **any caller-supplied `RiskFacts`** gated only by `capability is _GATE_SIGNING_CAPABILITY`. `risk_evidence.py:29` imports that token. Any in-process code can therefore mint signed `RuntimeEvidence` for arbitrary facts with no eight-source verification and no replay-ledger consumption. The source itself states "It is not a security boundary; production must isolate this service in its own process" (`risk_service.py:24-25`).

The problem is to designate **one** authorization authority and fix the token boundary **without** presuming a remote KMS/mTLS/collector service is the answer — which R1 Q4 and Amendment A5 identify as a speculative coupling that would introduce new SPOFs into a system whose only safety property is a local, hard-disabled gate.

---

## 2. Alternatives considered (with rejection reasons)

| # | Alternative | Rejection reason |
|---|---|---|
| **a** | **Closure-defined / unimportable capability token** as the fix | Insufficient alone: it raises the bar for accidental import but does not change *which* component is authoritative, and any code in the same process can still reach the closure through the gate API. Adopted as **mandatory defence-in-depth**, not as the authority decision. |
| **b** | **In-process Rust `execution-policy` as the authority** | Rejected as authority. `OrderRequest` cannot express side/venue/order_type/product/quantity (`execution-policy/src/lib.rs:71-78`), so a permit cannot constrain the dimensions that matter; no root signature, eight observations, replay ledger or one-time consumption exists in that crate; no non-test caller. Making it authoritative would require new scope fields + wiring = implementation forbidden in M1 and unproven. Demoted to non-authoritative permit-schema reference. |
| **c** | **Python `live_auth` verified-sources gate as authority, with an explicit deployment precondition** | **Chosen** (see §3). It is the only implementation that verifies all eight signed observations, distinct principals, freshness, replay ledger and one-time permit use. Its drawback — remote coupling — is recorded as a **precondition**, not built now. |
| **d** | **Process isolation** as the answer | Not adoptable as a built solution: no deployed separate process, IAM role, KMS key policy, network isolation or CloudTrail exists (`F-A5-03`, `F-A5-09`; `aws-kms-risk-service.md:55` says collectors are missing). It is a **declared precondition for any future live use**, not an M1 implementation. R1 Q4/Amendment A5 forbid introducing it now without a measured requirement. |
| **e** | Assume a remote risk service / KMS / mTLS daemon is the authority | Rejected. R1 Q4 and Amendment A5: coupling a hard-disabled local system to KMS + daemon + 8 missing collectors creates new SPOFs and violates ADR-0001's measured-requirement rule. This ADR explicitly does **not** prescribe building that infrastructure. |

---

## 3. Decision

**The single authorization authority is the Python verified-sources authorization chain** — `SignedEvidenceRiskGate` / `risk_evidence.py` verification of eight signed control observations → `risk_service` signed `RuntimeEvidence` → `live_auth.verify_live_permit` → one-time `AuthorizationUseLedger.consume()`. This is the *logical* authority for the design: the only component whose verdict is derived from independently verified inputs rather than caller assertions.

Concrete decisions:

1. **Authority selection (option c).** `live_auth`'s verified-sources path is designated the one authoritative authorization verdict. No other component may produce an authorization decision. Because live routing is hard-disabled (`ADR-0004`; `oms.py:242-247`, `durable_oms.py:608-609`), the verdict is **currently non-operative for routing**; this ADR makes it the sole admissible verdict *if and when* routing is ever authorized, and forbids a second verdict source.

2. **Capability boundary hardening (option a, mandatory).** The importable module-level `_GATE_SIGNING_CAPABILITY` (`risk_service.py:24-26`) must be replaced by a **closure-defined capability** created inside the gate's construction path, reachable only by the verified-sources gate, so that arbitrary in-process code cannot obtain it. The public `evaluate_and_sign` fail-closed surface (`risk_service.py:113-123`) remains. This is **defence in depth, not a security boundary** — the source's own warning stands.

3. **Demotion (option b rejected).** Rust `execution-policy` is **not** an authorization authority. It is demoted to a *permit-schema/capability-surface reference* used only in tests; it must not be wired to any OMS as an authority until either (a) its `OrderRequest`/`PermitBody` are extended with side/venue/order_type/product/quantity, bound and verified, or (b) it is formally retired — each requiring a new ADR. Rust `risk-engine` remains a pure evaluation helper for the simulator, not an authority.

4. **M1 design posture — single in-process authority (coordinator ruling on challenger A-1).** For M1 the authoritative verdict is **one logical, in-process authority**: the Python verified-sources chain designated in item 1. The M1 execution profile posture is "single in-process authority with no implicit remote dependency"; this ADR and the Runtime profile ADR are aligned by that ruling, not by silence. **The M1 design does not deploy, mandate, or require a separate process for the authority, and no remote service is the answer here.** Live routing remains hard-disabled; the verdict is currently non-operative for routing.

4a. **Separate-process isolation — explicitly UNIMPLEMENTED DEPLOYMENT PRECONDITION (option d, not a control, not built by M1).** CLM-010's "independent risk authority" guarantee is a **deployment property**: at `01eb382e` no separate workload/process, IAM role, KMS key policy, network isolation, audit retention or eight collectors exist (`F-A5-03`, `F-A5-09`; `aws-kms-risk-service.md:55`). Under the coordinator's A-1 ruling, separate-process isolation is **not a current control and is not implemented, deployed, or authorized by M1**; it is recorded only as an unimplemented precondition for any *future* live deployment, to be decided under a measured-requirement ADR and owner ratification. **It is not a prerequisite that the M1 design must satisfy, and it must not be read as "the answer" to challenge 1.** M1 does not authorize building KMS, mTLS, collectors or any service. No local or remote deployment is claimed.

5. **Fail-closed rule.** Any authorization input that is unverified, stale, from an unapproved principal, replayed, or produced through the importable-token path is **denied**, never assumed valid. `reconciliation_blocked`, `kill_switch_engaged` and missing permits deny (A5 FM-12).

6. **Single-owner-of-truth for side/venue/type.** The authoritative permit must bind the exact order terms (side, venue, order_type, product, quantity, notional) as the Python `RiskFacts`/`RuntimeEvidence` path already does (`risk_service.py:_validate_facts` scopes include `side`, `order_type`, `product`, `venue`, `quantity`, `notional_paise`). The Rust permit's inability to express these (F-A5-02) is why it cannot be the authority.

This resolves the "two divergent systems" by **demoting one and hardening the other**, without merging languages and without building remote infrastructure (consistent with A5 §7 non-proposal and Amendment A5).

### 3.7 Residual bypass — explicit statement (challenger B-1)

- **The closure token does NOT close the forgery bypass.** It is defence in depth against accidental import only. **Any code running in the same process can still forge:** it can import/reach the gate's own verified-sources signing method and call `_sign_after_verified_sources(arbitrary_facts, capability=<any obtainable object>)` (`risk_service.py:192-204`), producing signed `RuntimeEvidence` with no eight-source verification and no ledger consumption — exactly the F-A5-03 reproduction. The demoted Rust `ExecutionPolicy::live()` (`execution-policy/src/lib.rs:635-640`) likewise remains a public constructor; "must not be wired to any OMS" is a design rule, not a technical barrier.
- **Enforcement requires the unimplemented deployment precondition (§3.4a).** No mechanism delivered by M1 changes this. CLM-010 remains **partially falsified / conditionally true only under undeployed isolation**, and this ADR does not claim otherwise (M1-ARCH-INV-011).
- **"One authorization authority" is a logical designation, not an enforced property** at baseline. The fail-closed live-disable gate (`oms.py:242-247`, `durable_oms.py:608-609`) is the only enforced control in this build.

---

## 4. Ownership

- **Decision owner:** Architecture Design Group (M1 Platform Constitution), ratifiable by the repository owner at the M1 exit gate.

**Owner role for every state this ADR creates (challenger Section F.8):**

| State created | Owner role | Person |
|---|---|---|
| Designation of one authorization authority; trust registry, policy, one-time permit ledger, verification verdict, closure-scoped signing-capability state (D3) | *Risk & Authorization owner* (D3 role; new, closes F-A1-07) | **Not named — owner-ratification condition at M1 exit gate** |
| Non-authoritative labeling of `execution-policy` / `risk-engine` | *Risk & Authorization owner* (D3) | **Not named — owner-ratification condition at M1 exit gate** |
| Deployment isolation precondition (§3.4a) — no state created in M1; future anchor/key owner | *Risk & Authorization owner* (D3) + repository-owner ratification of a future workload owner | **Not named — no such owner exists at M1 (owner-ratification condition)** |

- **State created by this decision:** the designation of one authority (a design record in this workspace), not a new permit, key, ledger or service. No repository file is modified.
- **Ratification condition:** naming the natural person for the D3 role is a **repository-owner action at the M1 exit gate**, not a design output and not a person this group may invent. Absent ratification, this decision is conditionally binding and §8.6 rejects it.
- **Demoted components:** `execution-policy` and `risk-engine` owners (D3) must label them non-authoritative in documentation at the implementation milestone — not in M1.

---

## 5. Invariants

These are group-scoped IDs within ADR-M1-001..004 (`M1-ARCH-INV-*`). Challenger A-0 is RESOLVED: the coordinator's global registry (`INVARIANT-REGISTRY.md`) assigns unique namespaces across all 11 ADRs with zero duplicate definitions; all citations here resolve unambiguously.

- **M1-ARCH-INV-009 — One authorization authority (logical designation, not enforced at baseline).** Exactly one component may produce an authorization verdict: the Python verified-sources chain. No second verdict source may be introduced. M1 posture is a single in-process authority (coordinator ruling on A-1).
- **M1-ARCH-INV-010 — No importable signing capability (future gate, not delivered).** The signing capability must not be a module-level importable object. This is a **requirement for a future milestone**, not a property delivered by M1: at baseline the token is importable (`risk_service.py:24-26`). See §3.7 residual bypass.
- **M1-ARCH-INV-011 — Capability ≠ security boundary; residual bypass remains.** Closure hardening is defence in depth only; **any code in the same process can still forge signed `RuntimeEvidence`**. Enforcement requires the unimplemented deployment precondition (§3.4a). CLM-010 stays partially falsified under undeployed isolation.
- **M1-ARCH-INV-012 — Exact-terms binding.** An authorization verdict binds side, venue, order_type, product, quantity, instrument and notional; a verdict that cannot express these is non-authoritative.
- **M1-ARCH-INV-013 — Verdict ignored today, sole verdict tomorrow.** Live routing stays hard-disabled; if ever enabled, the only admissible verdict is this authority's, consumed one-time.
- **M1-ARCH-INV-014 — Fail closed on missing evidence.** Unverified/stale/replayed/caller-forged authorization inputs deny (challenge 6 alignment).

---

## 6. Failure cases (concrete, including adversarial)

1. **Importable-token forgery (F-A5-03, reproduced at B/C).** In-process code executes `from quantlab.risk_service import _GATE_SIGNING_CAPABILITY` and calls `_sign_after_verified_sources(arbitrary_facts, capability=token)` (A5 §F-A5-03 snippet), minting signed `RuntimeEvidence` with no eight observations or ledger consume. **This bypass is NOT closed by M1** (§3.7). *Response:* M1-ARCH-INV-010 is a future-milestone requirement and AC-002-1 a future gate spec; actual enforcement needs the unimplemented isolation precondition (§3.4a).
2. **Divergent verdicts (F-A5-02).** A future live path accepts a Rust `execution-policy` `Authorization` for a buy while the permit intended a sell, because `OrderRequest` has no side field (A5 reproduction at `:168`). *Response:* M1-ARCH-INV-009/012; Rust crate demoted; acceptance AC-002-3.
3. **Callersupplied context fabrication.** A live adapter builds `RiskContext` with `kill_switch_engaged=false`, `feed_health=Healthy{age_ns:0}` while the real feed is stale (R2 §4 future-wiring). *Response:* this authority requires independently signed observations; Rust `risk-engine` context is not authoritative.
4. **Adversarial — remote-coupling SPOF (R1 Q4).** An implementer "solves" F-A5-03 by routing every authorization through AWS KMS + mTLS daemon + 8 collectors, which are missing/unavailable; the hard-disabled local gate now depends on remote availability. *Response:* this ADR forbids building that infrastructure in M1; deployment precondition is recorded, not implemented.
5. **Permit replay.** A consumed permit is reused. *Response:* one-time `AuthorizationUseLedger.consume()` is the only valid consumption (A5 FM-11); M1-ARCH-INV-013.
6. **Stale/tampered control observation.** A signed observation is stale or from a duplicate principal. *Response:* freshness/scope/distinct-principal checks at `risk_evidence.py:420-459` deny (A5 FM-9); fail closed.

---

## 7. Dependencies

- **Other ADRs:** ADR-M1-001 (D3 owner role; F-A1-07 closure); ADR-M1-003 (OMS consumes the verdict only as data; never self-authorizes); ADR-M1-004 (no new port/service to "fix" auth now); Governance ADR-M1-011 (challenge 6 — reword CLM-010 and separate maturity from verification).
- **Evidence (`01eb382e`):** `risk_service.py:24-26,113-123,192-204`; `risk_evidence.py:29,374-488`; `live_auth.py:112-358,416-417`; `execution-policy/src/lib.rs:28-30,71-78,635-640,669-751,786-789`; `risk-engine/src/lib.rs:97-124`; `oms.py:242-247`; A5 F-A5-02/03/09; R1 Q4/Q5; R2 §A.
- **External capabilities (preconditions only, not authorized):** separate workload/process isolation, signer key policy, network identity, audit retention, eight fresh signed observation collectors. **None exists at baseline; none is built by M1.**
- **Governing docs:** `ADR-0004-live-disabled.md`; `target-architecture.md:55-57`; `aws-kms-risk-service.md` (documents the gaps, does not authorize them).

---

## 8. Rejection conditions

This decision is overturned or revised if new evidence shows:

1. The Python verified-sources path is itself demonstrably bypassable at its trust root (e.g., root key controllable in-process) with no fix short of a different authority — then option (c) fails and (d) becomes mandatory with a measured requirement.
2. A measured requirement is approved for a genuinely independent remote authority (with collector deployment and key isolation) — then the deployment precondition is promoted to a build decision via new ADR; this ADR's "not built now" stance is revisited.
3. Rust `execution-policy` is extended, verified and composed with exact-terms binding under a new ADR — then the demotion in §3.3 is lifted and the "one authority" choice re-adjudicated (competing options (b) vs (c)).
4. Executed (E/F) evidence shows the importable-token path cannot be closed in-process and only process isolation suffices — M1-ARCH-INV-010 is insufficient and challenge 1 resolves to (d).
5. Source at `01eb382e` is shown to have a different, functioning authority (e.g., a wired live permit consumer) — the inventory is wrong.
6. Owner ratification of D3 is refused with authority reassigned elsewhere — §4 is void.
7. **The owner reverses the coordinator's A-1 ruling** and mandates separate-process isolation for the M1 design rather than treating it as an unimplemented deployment precondition — §3.4/§3.4a and the single-in-process posture are void and re-adjudicated.
8. Executed evidence shows the residual bypass (§3.7 / B-1) is closed by some in-process mechanism without the isolation precondition — then §3.7 and M1-ARCH-INV-011 are revised.

---

## 9. Measurable acceptance requirement (design only — not executed in M1)

These are **future gate specifications, not delivered properties and not executed in M1**. Every named artifact has an owner role and target milestone (challenger Section C); requirements depending on nonexistent external capabilities are labelled **PRECONDITION — not an M1 gate**.

| ID | Named test / command (future) | Threshold | Owner (role) | Target milestone | Status |
|---|---|---|---|---|---|
| AC-002-1 | `python -m pytest -q research/python/tests/test_risk_service_and_kms.py -k "arbitrary_facts or capability"` — **FUTURE gate spec**: the test file/case does not exist at baseline; require it to exist, assert ≥1 test executed (`-k` must not deselect all), then attempt `_sign_after_verified_sources(arbitrary_facts, capability=<any obtainable object>)` | **FUTURE gate spec — not a delivered property**: when implemented, 0 arbitrary-`RiskFacts` signatures and the F-A5-03 reproduction must fail. At baseline the bypass is OPEN (§3.7) | *Risk & Authorization owner* (D3) | M3 (after closure-scoping + test creation) | Future gate spec; residual bypass remains until §3.4a precondition is met |
| AC-002-2 | Static scan: `grep -rn "_GATE_SIGNING_CAPABILITY" research/python/src` | 0 module-level importable capability tokens; capability closure-scoped to verified-sources gate (future state) | *Risk & Authorization owner* (D3) + *Verification owner* (D9) | M3 | Future gate spec |
| AC-002-3 | Rust permit scope test: two `Authorization` results differing only in an absent side/venue dimension (A5 F-A5-02 reproduction) | Either crate demoted/non-authoritative in docs, or `Ok` authorizations distinguishable by exact terms; 0 indistinguishable authorizations if authoritative | *Risk & Authorization owner* (D3) | M2 | Future gate spec |
| AC-002-4 | `cargo test -p execution-policy -p risk-engine --all-targets --locked` + `grep -rn "authorize_order" --include=*.rs .`; strengthen with assertion that no OMS crate can reach `ExecutionPolicy::live()` / `authorize_order` | 0 non-test **reachable** callers from any OMS/profile crate; crates non-authoritative | *Risk & Authorization owner* (D3) + *Verification owner* (D9) | M2 | Future gate spec; vacuously true today, hence the reachability assertion |
| AC-002-5 | Verdict-source assertion: if any live-routing path is ever introduced, assert exactly one verdict source and one-time ledger consumption | 1 verdict source; 0 permit replays accepted | *Risk & Authorization owner* (D3) | **PRECONDITION — not an M1 gate** (requires a live-routing capability, currently hard-disabled) | **PRECONDITION — not an M1 gate** |

### Amendment-1 challenger disposition (ADR-M1-002)

- **A-1 (single vs separate process) — ACCEPTED, coordinator ruling recorded.** Exact text changes: §3 item 4 rewritten as "M1 design posture — single in-process authority" with no remote dependency; new §3.4a labels separate-process isolation an "explicitly UNIMPLEMENTED DEPLOYMENT PRECONDITION, not a current control and not built by M1", to be decided under a future measured-requirement ADR; §8.7 records the owner-reversal condition. No remote service is presented as the answer. The ruling is cited, not the other group's file.
- **B-1 (residual bypass) — ACCEPTED.** Exact text changes: new §3.7 "Residual bypass — explicit statement" says the closure token does not close the bypass and any same-process code can still forge; §5 M1-ARCH-INV-010/011 now say "future gate, not delivered" / "residual bypass remains"; §6 case 1 states the bypass is not closed by M1; §9 AC-002-1 threshold reworded as a FUTURE gate spec with a ≥1-test-executed assertion. The original honest CLM-010 disclosure is retained, not weakened.
- **Section C (untestable §9) — ACCEPTED.** §9 table gained `Owner (role)`, `Target milestone`, `Status` columns; AC-002-5 labelled PRECONDITION.
- **CONTEST (narrow):** challenger B-1 says the enforcement claim is false, not the disclosure. This
  position agrees with that distinction and keeps the disclosure intact; it contests only any
  reading that ADR-002 ever *claimed* the bypass was closed — §3.2 of the original ADR already
  called closure hardening "defence in depth, not a security boundary", and §3.4 already recorded
  CLM-010 as a deployment property. No text is retracted; the change is an explicit restatement
  (§3.7), not a correction of a false claim.

*End of ADR-M1-002.*
