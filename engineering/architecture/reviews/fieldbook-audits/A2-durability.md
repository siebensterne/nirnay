# A2 — READ-ONLY DURABILITY AUDIT

**Repo:** `/private/var/folders/7c/m_f7hs9n445174c0x3gsy5kw0000gn/T/opencode/fieldbook-review` (fresh clone, HEAD `aee91de`)
**Scope:** docs/durability-contract.md, docs/raw-frame-journal.md, docs/segmented-durability-design.md, docs/event-accounting-contract.md, docs/adr/0002-*.md, `src/integrity/journal.rs`, `src/integrity/**`, `src/storage/application/durable.rs`, `src/storage/application/durable_segmented.rs`, `src/storage/capacity.rs`, `src/storage/writer.rs`, `src/storage/catalog.rs`, `src/storage/persistence.rs`, `src/storage/report.rs`, relevant call sites and `tests/`.
**Method:** static source inspection only. No files in the repo were modified; no commands that collect, write to, or contact external services were run; no credentials touched; `/Users/tourist/Documents/fieldbook` untouched.
**Decision: CORRECT** (local durability claims are substantiated with narrow caveats below). **DEFER** for cross-failure-domain backup/restore, which is out of implementation scope, not a defect in what is claimed.

---

## 0. Fault model actually supported — and explicit rejection of a zero-loss claim

**I explicitly reject any unconditional “zero data loss” claim.** Neither the code nor the normative docs support one. `docs/durability-contract.md:148-149` itself says “No absolute zero-loss claim is made”; `docs/raw-frame-journal.md:314-318` states that live-feed soak with rotation and a storage-exhaustion run on an isolated volume “remain open operational evidence”; `docs/operating-envelope.md:136` records the 24-hour zero-loss soak as **Not executed**. There is no certification here; this is a code audit, not an assurance attestation.

The precise supported fault model is:

| Domain | Supported | Not supported |
|---|---|---|
| Market-data journal (`src/integrity/journal.rs`) | Single process/host, local filesystem. Crash safety for *finalized* runs; `sync_mode=always` fsyncs each record (`:1569-1578`), `interval` fsyncs on tick/finalization (`:1641-1656`, `:1409-1442`). Checksums detect accidental corruption/truncation. | Crash survival of a merely `synced` record *as a complete run* (§2). Disk/host failure, OS crash tolerance beyond `sync_all` semantics, replication, multi-site, authenticity (signed/tamper-evident) guarantees. |
| P6 legacy commit store | Local host, single writer, one fsynced+dir-synced file per commit (`src/storage/report.rs:285-346`, `durable.rs:297-341`). | Replication, DB transaction, multi-host. |
| P6 segmented store | Local host, `SyncPolicy::Full` fsyncs header/frames/footer/rename/dir (`durable_segmented.rs:194-202`, `:541-543`, `:589-591`, `:647-666`). `SyncPolicy::None` skips all explicit syncs (`:41-44`, `:541-543`, `:589-591`). | Power-loss durability under `SyncPolicy::None`; replicated/quorum acknowledgement; cross-host restore into a live store. |
| SBE writer / catalog | Local host, free-space watermark (`writer.rs:149-167`, `:286-305`), atomic single-file sidecars (`report.rs:285-346`). | Cross-file atomicity, durability of admitted-but-unwritten SBE messages. |
| Live PostgreSQL | Opt-in, operator-supplied server; `synchronous_commit=on` prerequisite stated in `docs/durability-contract.md:151-164`; CI does not run it (`:139-144`). | Any claim of a measured RPO/RTO; the repo holds no server transcript or evidence manifest. |

---

## 1. What constitutes a committed record

**Verified implementation facts**
- Market journal: “committed” requires the full `SegmentWriter::finalize` chain — flush (`journal.rs:1409`), `sync_all` of record bytes (`:1410-1413`), footer write + checksum (`:1414-1436`), file sync (`:1437-1442`), `fs::rename` of `.pmj.partial` → `.pmj` (`:1443-1450`), and directory `sync_all` (`:1452-1455`). The manifest entry is published *separately* afterwards (`:1725-1727` on rotation, `:1755-1759` at shutdown). Verification requires manifest state `Finalized` (`:1927-1929`) and no partial/orphan segments (`:1967-1985`). Ack levels themselves stop at `Synced` (`:330-339`, `:564-618`).
- P6 legacy: one commit = one never-rewritten `commit-<12 digits>.json` written by `write_json_atomic` (`durable.rs:125-127`, `:297-341`, `:315`), which fsyncs the temp file, renames, and fsyncs the parent directory (`report.rs:332-340`). Reopen verifies schema, file-position/sequence, checksum, and binding (`durable.rs:179-247`).
- P6 segmented: a commit frame is appended + (Full) fsynced (`:563-595`), then in-memory state advances (`:600-607`); the *segment* becomes committed at footer/rename/dir-sync (`:625-677`), and a recovery snapshot is written atomically every N commits (`:619-621`, `:740-754`). Snapshot frontier is only set for finalized segments (`:667-672`).

**Documented intent vs code** — `docs/durability-contract.md:8-17` calls `finalized` “evidence of a complete run” and `docs/raw-frame-journal.md:62-66` says the manifest is updated only after rename+dir-sync; both match the code. `docs/segmented-durability-design.md:31-45` matches `finalize_segment` for the segment, but its claim that “any `.tmp-*` file produces `IncompleteSegment`” (`:42-43`) does **not** match the code (see §4).

**Risks** — the segment rename and the manifest publication are two durable operations with no transaction spanning them; the manifest is the sole authority for completeness.

---

## 2. Does an acknowledged record survive a process crash?

**Answer: No, not as a verified complete run** for the market journal; **conditional** for P6.

- `JournalAckLevel::Synced` waits only for `counters.synced >= fence` (`journal.rs:586-598`); `sync_only` fsyncs the *open `.partial` file* and sets `synced = written` (`:1679-1697`), without footer, rename, manifest, or directory entry for a new segment. `SyncMode::Always` likewise only flushes+synces per record (`:1569-1578`). Verification rejects any `.pmj.partial` (`:1974-1975`) and any `.pmj` not listed in the manifest (`:1976-1979`). `recover_incomplete_run` only rewrites `Starting|Open` manifests to `Incomplete` (`:1898-1905`); it never promotes partial or orphan segments (`:1865-1875`). So the bytes can physically remain, but the run is *incomplete*, and no acknowledged record is recoverable as a committed run.
- Crash subprocess evidence: `src/integrity/journal/tests.rs::crash_recovery_is_conservative_and_idempotent` (`:1979-2014`) — crash after record write, after segment rename, and after manifest update all yield `Open` + `!complete` and recover to `Incomplete`. Notably, a crash *after rename but before manifest update* leaves an orphan `.pmj` that verification rejects (`:1976-1979`). Only `crash_after_final_manifest_update_keeps_committed_run_finalized` (`:2022-2038`) verifies complete. `post_commit_failure_does_not_downgrade_finalized_run` (`:2040-2065`) confirms a post-commit hook cannot downgrade a finalized run.
- P6 legacy: a successful `commit` is fsynced file+dir, so it survives process and ordinary OS crash on the local host (`durable.rs:315`, `report.rs:336-339`).
- P6 segmented with `Full`: frames in an unfinalized `.tmp-` file are re-read and validated on reopen (`:356-380`, `parse_temp` `:956-1034`), so acknowledged data survives a process crash *at the data level* — subject to the naming defect in §4/§7. With `SyncPolicy::None`, fsyncs are skipped (`:541-543`, `:589-591`, `:647-649`, `:664-666`), so acknowledged commits may be lost on power loss.

**Limits of the tests** — crash tests exercise `process::exit` (no destructors) at injected hooks, not real SIGKILL/power loss, and assert *fail-closed incompleteness*, not post-crash recovery of acknowledged data. `crash_child_process` is a subprocess helper, not a standalone guarantee.

---

## 3. What happens when disk fills

- Market journal: write/flush/sync/rotation/finalization errors all call `fail` (`journal.rs:1563-1577`, `:1601-1604`, `:1644-1655`, `:1748-1754`), which persists manifest `Failed` (`:1818-1823`), stops the writer, and notifies ack waiters. A full queue latches failure after `enqueue_timeout` (`:649-650`, `:682-694`). `mark_failed` is terminal (`:736-747`), and finalization refuses a failed run (`:1744-1747`). Upstream stops: `journal_data`/`journal_observations` convert `record`/`observe` errors into `WorkerEvent::Fatal` (`src/acquisition/session.rs:1638-1649`, `:1676-1688`, `:1693-1708`). No silent drop path exists by construction.
- SBE writer: `FileSink`/`RotatingSink` `check_watermark` refuses to continue below `output.min_free_bytes` (`:149-167`, `:286-305`; default `512 << 20` at `src/governance/config.rs:570`; zero disables). Sink write failure sets a fatal string and terminates the writer thread (`writer.rs:761-769`); `send`/`flush` then error (`:879-888`, `:903-918`) and `emit_outputs` propagates (`coordinator.rs:1142-1150`).
- P6 segmented: `SegmentFault::DiskFull` is a *synthetic early return* (`:509-511`) with test `disk_full_fault_leaves_the_store_unchanged` (`tests/segmented_durability.rs:262-278`) — it proves nothing about a real ENOSPC. There is no free-space watermark in the segmented store. `src/storage/capacity.rs` is startup capacity *planning* (`coordinator.rs:412`), not runtime disk protection.

**Limits** — `src/integrity/journal/tests.rs::storage_write_failure_marks_run_failed` (`:879-921`), `storage_sync_failure_marks_run_failed` (`:1025-1049`), `storage_flush_failure_marks_run_failed` (`:1051+`), `queue_saturation_then_shutdown_stays_failed` (`:1088+`) use injected faults. The real ENOSPC test `assert_real_filesystem_enospc` (`:923-1023`) is gated on `JOURNAL_ENOSPC_TEST_ROOT` and is invoked by an external script — **not part of the ordinary test suite**. `writer.rs::watermark_failure_stops_the_writer_before_the_disk_fills` (`:1767-1780`) sets `min_free_bytes=u64::MAX`, so it proves the check fires, not real filesystem exhaustion.

---

## 4. Are partial writes detected and recovered

**Detected: yes. Recovered: narrowly, and only in the P6 segmented store.**

- Market journal: every record carries a SHA-256 trailer (`journal.rs:1372-1384`), the footer has a checksum over all frames (`:1414-1436`), and verification recomputes every checksum, counts, byte ranges, sequence domains, and rejects bytes after a footer and unlisted/partial files (`:2127+`, `:1967-1985`). Truncation/corruption are fail-closed (tests `truncated_final_record_is_detected` `:767-804`, `checksum_mismatch_and_corrupt_manifest_fail_verification` `:805-852`). There is **no repair**: a partial segment is never rewritten or salvaged (`:1871-1875`).
- P6 legacy: `load_state` rejects truncated JSON, checksum mismatch, sequence gaps, version drift, and binding mismatch (`durable.rs:208-243`); deterministic faults `FailBeforeWrite`, `TruncatedAppend`, `CorruptChecksum` (`:67-76`, `:303-338`) with tests `partial_or_corrupt_commit_record_fails_closed` (`durable.rs:1045-1070`), `fail_before_write_leaves_the_store_unchanged` (`:1071-1081`), `sequence_gap_is_rejected_on_reopen` (`:1095+`).
- P6 segmented: `parse_temp` validates frame sequence and per-frame checksum (`:1000-1014`) and footer record count/checksum (`:1024-1032`). On reopen a temp **with** a valid footer is renamed into the committed namespace (`:296-309`); a temp **without** footer is recovered as an open segment and its frames are replayed (`:355-380`). An unparseable/garbage tail is a hard `CorruptSegment` (`:1017-1022`) — i.e. a partially written *last frame* is not tolerated and can reject otherwise-valid earlier frames in the same temp.

**Documented intent vs code** — `docs/segmented-durability-design.md:40-45` says “any `.tmp-*` file produces `StoreError::IncompleteSegment`” and “it is never silently discarded”. Code does two different things: validates-and-promotes a closed temp (`:302-309`) and validates-and-replays an open temp (`:356-380`); `IncompleteSegment` is returned only for multiple open temps (`:310-314`) or a garbage temp. `tests/segmented_durability.rs::incomplete_temp_segment_is_never_discarded` (`:119-140`) writes the literal bytes `"partial"` and accepts `IncompleteSegment` **or** `CorruptSegment`; it does **not** prove the documented “any temp” behavior and does not test recovery of a valid partially-written temp.

---

## 5. Does a failed writer stop upstream acceptance

- **Market journal: yes.** Admission is closed and latched (`journal.rs:698-711`, `:736-747`), enqueue rechecks `failure_reason` after a successful send (`:666-680`), failed enqueue latches (`:682-694`), and finalization refuses a failed run (`:1744-1747`). The session worker emits `Fatal` and stops before forwarding (`session.rs:1638-1649`, `:1676-1688`, `:1693-1708`). `tests/session_recovery.rs::failed_journal_append_stops_worker_before_event_forwarding` (`:428-554`) proves a rejected record does not reach `Events`.
- **P6 runner: yes.** `flush_batch` halts on unresolved receipts and failed local commits (`durable.rs:761-763`, `:794-804`); `quarantine_observation` halts if the quarantine append fails (`:666-675`). Tests: `streaming_runner_stops_before_a_later_record_can_overtake_uncertain_delivery` and `streaming_runner_stops_when_local_commit_fails_after_target_commit` (`tests/durable_pipeline.rs:202-298`). These prove the *simulator* runner, not a live database.
- **SBE writer: yes, but with an accounting hole.** The writer thread exits on first write failure (`writer.rs:761-769`); messages already admitted into the mpsc queue are never written and receive no per-message receipt. `WriterStats` reconciles `delta_admissions` vs `delta_messages` (`event-accounting-contract.md:74-77`) but the collector does not retroactively reject those admitted outputs.
- **Gap:** `PersistenceHandle::submit` is fire-and-forget (`persistence.rs:160-166`, `:191-224`); the worker records errors in `status.last_error` and survives (`persistence.rs:298-320`; test `write_failure_is_reported_and_the_worker_survives` `:575`). A metadata/catalog publication failure therefore does **not** stop upstream acceptance. Only `submit_final` waits for an ack (`:169-189`). This matters for §6.
- **Gap:** replicated profiles D4/D5 are refused at construction (`src/acquisition/evidence.rs:600-604`) because `locally_achievable()` covers only D1–D3 (`:132-136`), even though the evidence pipeline maps D4/D5 to `JournalAckLevel::Synced` (`:698-703`). No replicated acknowledgement exists.

---

## 6. Is index and manifest publication atomic

**Single-file atomicity: yes. Cross-file atomicity: no.**

- `write_json_atomic`/`write_bytes_atomic` write a sibling temp, fsync it, rename, and fsync the parent directory (`report.rs:264-267`, `:285-346`, `:348-354`). Used for the journal manifest (`journal.rs:1826-1829`), P6 commit files (`durable.rs:315`), segmented snapshots (`durable_segmented.rs:746-750`) and archive manifest (`:726-727`), and catalog/metadata (`catalog.rs:100-104`, `persistence.rs:332-361`).
- Market journal: `SegmentWriter::finalize` renames and dir-syncs the segment (`:1443-1455`), then `rotate_segment` writes the manifest (`:1725-1727`). A crash between them leaves a finalized `.pmj` absent from the manifest, which verification rejects (`:1976-1979`). Fail-closed, but *not atomic*, and acknowledged records in that segment are reported incomplete.
- SBE `RotatingSink::finalize_segment`: `writer.finish()/sync_all()` then `rename` (`:363-384`), then `write_manifest(false)` (`:399`). The manifest write fsyncs the same parent directory (report.rs:339), which happens to cover the earlier rename, but there is no explicit directory sync immediately after the rename and the two operations are not one transaction.
- P6 segmented: `rotate_archive` copies+fsyncs to `archive/.tmp-*`, renames, dir-syncs, writes the archive manifest atomically, dir-syncs again, and **only then** removes the hot source (`:688-736`). This ordering is safe for evidence, but a crash after archive rename and before manifest write leaves an unlisted archived segment; `load_archive` only validates *listed* entries (`:1189-1235`), so the orphan is ignored (the hot copy still exists). `write_snapshot` replaces `current.json` atomically and then truncates in-memory state (`:740-754`); snapshot replacement is decoupled from commits as designed.
- `persistence.rs::write_job` writes metadata and catalog via **two separate** atomic writes (`:332-361`); a crash between them can leave a newer metadata sidecar with an older catalog. Combined with the fire-and-forget `submit`, this is a real publication-atomicity gap.

---

## 7. Segment rotation during failure

- Market journal: `rotate_segment` opens the **replacement** segment before finalizing the previous one (`journal.rs:1717-1720`), so a finalize failure leaves both a `.partial` previous segment and an empty `.partial` replacement. Failure is latched (`:1719-1720`, `:1584-1596`) and the run cannot finalize. Tests: `rotation_rename_failure_is_failed_and_never_reported_complete` (`:1826-1863`), `directory_sync_failure_leaves_an_uncommitted_segment` (`:1866-1902`), `manifest_update_failure_does_not_commit_a_renamed_segment` (`:1905-1943`), `final_manifest_write_failure_is_failed_and_not_complete` (`:1946-1976`). Each asserts `Failed` + `!complete` — none asserts recovery of the rotated data.
- P6 segmented: `finalize_segment` (`:625-677`) sets `snapshot.frontier` and pushes `hot`, then calls `rotate_archive` if the hot cap is exceeded (`:673-675`). **Defect:** `commit` advances in-memory state/sequence at `:600-607` *before* `finalize_segment`/`write_snapshot`; if finalization or archival fails, `commit` returns `Err` although the record is durably in a finalized segment. A retry with the same logical record would append a *new* sequence, so deduplication depends on the downstream target key, not the local store.
- **Defect (segment naming after temp recovery):** `open_with` recovers a footerless temp with `frontier.segment = segment_name(first, first)` (`durable_segmented.rs:365`) while `last_sequence` may be greater (`:367`). `finalize_segment` later renames the temp to that stale `<first>-<first>` path (`:657`), but `load_segments` requires `footer.last_sequence == declared_last` from the name (`:1141`). A crash that leaves an open temp with more than one frame, followed by continued commits and finalization, therefore produces a store that fails to reopen with `CorruptSegment`.
- `SegmentFault::FailAfterFrames` (`:569-573`) fires **before** the frame is appended (the append is at `:574-595`), so `tests/segmented_durability.rs::every_finalization_boundary_fails_without_a_false_commit` (`:161-189`) never tests a crash *after* a frame was written. The archive faults `FailBeforeArchiveCopy`/`FailAfterArchiveCopyBeforeManifest` (`:683-687`, `:707-711`) are never used by any test.

---

## 8. Backup and restoration across failure domains

**There is no cross-failure-domain backup/restore.** `docs/durability-contract.md:63-69` lists replication, multi-site, RPO/RTO, and cross-run retention/archival policy as *not implemented*, and `:166-182` scopes disk failure and network partition out. The market journal's `recover_incomplete_run` is a manual CLI (`src/main.rs:205-220`) that only relabels state; it is not a restore.

`SegmentedStore::restore_archive` (`durable_segmented.rs:768-795`) rebuilds an archive manifest from **in-memory** `self.archives` (not the on-disk manifest), verifies each source segment's digest against the stored entry (`:783-787`), and copies segments into a separate `destination` directory (`:788-793`). It does **not** restore into a live store, does not restore hot segments or snapshots, and has **no test at all** (grep finds no call outside its definition). `docs/segmented-durability-design.md:94-102` says recovery from a missing acknowledged segment “requires restoring the segment from archive or backup” — that step is an operator procedure, not implemented code. `migrate_legacy` (`:1240-1343`) is a format migration, not a backup, and is tested by `legacy_store_migrates_without_rewriting_history` (`tests/segmented_durability.rs:280-305`). The live PostgreSQL path is opt-in and CI-unexecuted (`docs/durability-contract.md:139-144`).

---

## Consolidated defects / risks (ranked)

1. **High** — Segmented temp recovery writes a finalized segment name using only `first_sequence` (`durable_segmented.rs:365`) but finalizes to that name (`:657`), conflicting with the name/range check at `:1141`. Multi-frame open-temp recovery can make the store unopenable after the next finalization. Not covered by `fast_limits` (1 frame/segment) tests.
2. **High** — Ambiguous acknowledgement in the segmented store: state/sequence advance before finalization and archival (`:600-607` vs `:616-621`, `:673-675`), so `commit` can return `Err` for a durably finalized record, inviting duplicate logical commits.
3. **Medium** — Documented intent “any `.tmp-*` fails closed” (`docs/segmented-durability-design.md:42-43`) diverges from promotion/replay behavior (`:302-309`, `:356-380`). Security/audit reasoning based on the doc would be wrong.
4. **Medium** — `SyncPolicy::None` still returns success from `commit` with no `fsync` (`:541-543`, `:589-591`, `:647-649`, `:664-666`); a caller can read “acknowledged” as crash-safe when it is not.
5. **Medium** — `PersistenceHandle::submit` async write failures do not stop acceptance (`persistence.rs:160-166`, `:298-320`), and metadata/catalog are two independent atomic writes (`:332-361`); index publication is not atomic as a pair.
6. **Medium** — Segment rename and manifest publication are separate durable operations in both the market journal and the SBE writer; a crash between them orphans evidence and makes a run with acknowledged records report incomplete.
7. **Low** — `CorruptFrame` fault writes invalid JSON into the temp but still returns `Ok` and advances state (`durable_segmented.rs:582-587`, `:600-607`); fault path is untested.
8. **Low** — `open_with` auto-promotion uses `fs::rename` onto the final path without checking for an existing final segment (`:302-306`).
9. **Evidence gaps** — real ENOSPC and real power-loss tests are opt-in/absent; `restore_archive` and archive-fault paths are untested; D4/D5 cannot be exercised locally.

## Required changes

1. Fix the recovered-open-segment naming: derive `frontier.segment` from the actually recovered first/last sequence (or defer naming until `finalize_segment`), and add a test that recovers a footerless temp with ≥2 frames, continues commits, finalizes, and reopens.
2. Make the segmented `commit` acknowledgement unambiguous: either finalize before advancing in-memory state, or return a distinct “durable but unconfirmed” receipt and document retry semantics; cover archive faults in tests.
3. Reconcile `docs/segmented-durability-design.md:40-45`, `:94-102` with `durable_segmented.rs:296-317`, `:356-380`, `:768-795`; state exactly which tmp states are promoted/replayed/rejected and that restore is an export, not a live recovery.
4. Gate acknowledgements on `SyncPolicy`: never report a `Full`-equivalent ack under `None`, or document `None` as test-only. Add a real-ENOSPC and a real-crash/power-loss evidence run (the docs already name these as open).
5. Make metadata+catalog publication transactional or ordered-and-recoverable, and propagate `PersistenceHandle::submit` write failures to the collector's fatal path.
6. Add a cross-run backup/restore procedure and tests, or keep “not implemented” explicit in every operator-facing document. Do not relabel the archive export as restoration.

**Decision: CORRECT** for the scoped, local-durability implementation subject to changes 1–5; **DEFER** for item 6 (backup across failure domains is explicitly out of scope, not a false claim). No unconditional zero-loss claim is permitted. This audit is not a certification.
