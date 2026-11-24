# Independent release verifier threat model — 2026-11-24

<!-- SPDX-License-Identifier: Apache-2.0 -->

Status: implementation record for `crates/release-verifier` on branch
`feat/ndc-v2-independent-verifier`. This record describes the boundaries of the
independent Nirnay verifier. It creates no release, no key, no right and no
dataset acceptance.

## Assets and trust boundaries

| Asset | Boundary | Verifier stance |
|---|---|---|
| Release root | caller-owned path | trusted config, untrusted contents |
| Manifest, sidecar | published release directory | untrusted until signature checks pass |
| Artifact bytes | published artifacts directory | untrusted until stage five passes |
| Publisher registry | injected out-of-band input | trusted, revision-bound |
| Rights and coverage decisions | injected evaluators | trusted inputs, separate from signature |
| Private staging directory | caller-provided, verifier-exclusive | trusted once sealed |

The release supplies no key, no registry entry and no rights verdict. A
publisher signature establishes publisher-key control and integrity of the
exact signed bytes. It establishes no source authenticity, coverage, right or
dataset entitlement.

## Adversary capabilities

The adversary may write release files, choose artifact names and record bytes,
replace files with symbolic links or hard links, publish extra files, reorder
records, mutate a file during a read, alter declared sizes and digests, and
present a sidecar with an unknown or revoked key identity. The adversary cannot
alter the configured root path, the injected registry, the injected evaluators
or the caller resource policy.

## Defenses and stage mapping

1. Path safety rejects absolute prefixes, drive prefixes, backslashes, NUL
   bytes, traversal components, characters outside the component alphabet and
   duplicate normalized artifact paths before any artifact open.
2. Directory descent uses descriptor-relative `openat` with `O_NOFOLLOW`.
   The root and staging directories are opened through safe `OpenOptionsExt`
   with `O_NOFOLLOW | O_DIRECTORY`. No crate code contains `unsafe`.
3. Each published file is opened with `O_NOFOLLOW`, rejected when its `fstat`
   type is not a regular file and rejected when its link count exceeds one.
   `O_NONBLOCK` keeps a named pipe from stalling the verification thread.
4. Bytes are hashed from the opened descriptor. A post-read `fstat` compares
   device, inode, link count and size with the pre-read values. Any mismatch
   refuses the release. No second string-path open of a published file occurs.
5. The exact expected file set comes from `artifacts[]`, never from directory
   contents. A missing, extra, duplicate or undeclared entry refuses.
6. Stored bytes, decoded bytes and the ordered dataset digest are recomputed.
   Decoding is bounded by declared expanded size and the caller decompression
   budget. No buffer is sized from an untrusted declaration before a bound
   check and an actual-byte-count check.
7. Verified descriptor bytes are copied into private staging, rehashed from
   the copy, synced to disk and sealed. Later stages read only the sealed
   snapshot. A refusal removes staged files before any receipt is issued.

## Platform notes and residual risk

Descriptor-relative descent relies on the platform `openat` implementation
exposed by `rustix`; the configured root path itself is resolved once at open
time. Rename of a published entry between the exact-set enumeration and the
descriptor open is detected by descriptor identity, not by a path comparison.
Filesystem guarantees for `fsync` on directories remain a platform property.
Retention of private staging after an abrupt process stop is a caller cleanup
responsibility; the verifier refuses to consume an unsealed staging directory.

## Non-goals

No transport, publication journal, key issuance, provider contact, market-data
collection, synthetic observation, experiment, India implementation, live
route or rights grant belongs to this crate. The receipt records checks
performed; it is not a right.
