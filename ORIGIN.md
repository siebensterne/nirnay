# Origin, license and provenance policy

## Organization and scope

fieldbook and nirnay are complementary, independent projects under the same
GitHub organization (`siebensterne`). They share a neutral quality contract
(`quality-contract.json`), toolchain policy and Apache-2.0 license. They do
not share code, modules, paths or CI jobs. Cross-pollution checkers reject
foreign repository markers and absolute host paths in tooling files.

## License

Both repositories are licensed under Apache License 2.0 with the accompanying
`NOTICE` file. Apache-2.0 is permissive and non-copyleft: it imposes no
obligation to publish derivative source, includes an express patent grant,
and requires preservation of copyright, patent, trademark and attribution
notices. There is no GPL, LGPL, AGPL or other copyleft dependency in the
authored source of either repository.

Copyright 2026 Kartik Jangid <22kartikjangid@gmail.com>.

## Originality

All authored source must be original work produced for these projects. Do not
copy code, comments or documentation from another repository, answer, book or
website into an authored path. Third-party material is permitted only as
byte-exact, hash-pinned reference documents under `references/` (fieldbook) or
`engineering/references/` (nirnay), or as declared dependencies with their own
license metadata. Those references are excluded from authorship, formatting
and English-prose gates.

Every newly added authored source file must carry the SPDX identifier
`SPDX-License-Identifier: Apache-2.0` in its first ten lines. The
`check_license_provenance.py` gate enforces this on added files in local gates
and CI. A missing identifier, a missing NOTICE or an altered LICENSE fails
closed.

## Commit authorship

Commits in both repositories use the same author identity:
Kartik Jangid <22kartikjangid@gmail.com>. Commit subjects follow the
repository git policy; direct commits to `main` and history rewriting of
published commits are forbidden.
