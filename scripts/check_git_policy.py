#!/usr/bin/env python3
"""Validate the working copy against engineering/git-policy.json.

The policy can only be enforced where it is observable: branch names, commit
subjects, tracked generated paths, and the inventory of branches, worktrees and
stashes. Server-side rules are not observable from a clone and are not pretended
to be checked here.

    python scripts/check_git_policy.py
    python scripts/check_git_policy.py --strict   # also require a clean tree
    python scripts/check_git_policy.py --base main

Exit status is non-zero when any check fails, and every failure is named.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

POLICY = Path(__file__).resolve().parent.parent / "engineering" / "git-policy.json"


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {result.stderr.strip()}")
    return result.stdout


def fail(messages: list[str]) -> int:
    for message in messages:
        print(f"FAIL: {message}", file=sys.stderr)
    print(f"{len(messages)} git policy check(s) failed", file=sys.stderr)
    return 1


def check_branch(policy: dict, branch: str) -> list[str]:
    rule = policy["branch"]
    if branch == rule["base"]:
        return []
    errors: list[str] = []
    prefix, _, topic = branch.partition("/")
    if not topic:
        errors.append(f"branch `{branch}` is not `<prefix>/<topic>`")
        return errors
    if prefix not in rule["allowed_topic_prefixes"]:
        allowed = ", ".join(rule["allowed_topic_prefixes"])
        errors.append(f"branch `{branch}`: prefix `{prefix}` is not one of {allowed}")
    if not re.match(rule["topic_pattern"], topic):
        errors.append(f"branch `{branch}`: topic `{topic}` does not match {rule['topic_pattern']}")
    return errors


def check_subjects(policy: dict, base: str) -> list[str]:
    rule = policy["commit_subject"]
    pattern = re.compile(rule["pattern"])
    errors: list[str] = []

    try:
        subjects = [line for line in git("log", "--format=%s", f"{base}..HEAD").splitlines() if line]
    except RuntimeError:
        subjects = []
    if not subjects:
        subjects = [line for line in git("log", "-1", "--format=%s").splitlines() if line]

    # GitHub generates merge-commit subjects; they are not topic commits and
    # are exempt from the topic pattern, matching fieldbook's policy.
    subjects = [
        subject
        for subject in subjects
        if not subject.startswith(("Merge pull request ", "Merge branch "))
    ]

    for subject in subjects:
        if len(subject) > rule["maximum_length"]:
            errors.append(f"subject is {len(subject)} characters (max {rule['maximum_length']}): `{subject}`")
        if subject.strip().lower() in rule["forbidden_exact_subjects"]:
            errors.append(f"subject says nothing: `{subject}`")
        elif not pattern.match(subject):
            errors.append(f"subject does not match `type(scope): description`: `{subject}`")
    return errors


def check_authors(policy: dict, base: str) -> list[str]:
    """Require the configured author email on all unmerged commits."""
    expected = policy["commit_subject"].get("required_author_email")
    if not expected:
        return []
    try:
        authors = [
            line
            for line in git("log", "--format=%ae", f"{base}..HEAD").splitlines()
            if line
        ]
    except RuntimeError:
        authors = []
    if not authors:
        authors = [git("log", "-1", "--format=%ae").strip()]
    return [
        f"commit author `{author}` is not `{expected}`"
        for author in authors
        if author != expected
    ]


def check_generated(policy: dict) -> list[str]:
    rule = policy["generated_paths"]
    components = set(rule["forbidden_components"])
    prefixes = tuple(rule["forbidden_component_prefixes"])
    suffixes = tuple(rule["forbidden_suffixes"])
    # Exceptions are explicit and reasoned in the policy file; anything not
    # listed here is a failure even if it happens to look like the exception.
    exceptions = {entry["path"] for entry in rule.get("exceptions", [])}

    errors: list[str] = []
    for path in git("ls-files", "-z").split("\0"):
        if not path or path in exceptions:
            continue
        parts = path.split("/")
        for part in parts:
            if part in components:
                errors.append(f"tracked generated path: `{path}` (component `{part}`)")
                break
            if prefixes and part.startswith(prefixes):
                errors.append(f"tracked generated path: `{path}` (component `{part}`)")
                break
        else:
            if suffixes and path.endswith(suffixes):
                errors.append(f"tracked generated path: `{path}` (forbidden suffix)")
    return errors


def check_inventory(policy: dict, branch: str) -> list[str]:
    """Check inventory; the finished branch layout is required only on base."""
    rule = policy["inventory"]
    errors: list[str] = []

    branches = sorted(
        line for line in git("for-each-ref", "--format=%(refname:short)", "refs/heads").splitlines() if line
    )
    # The policy inventory describes the state after a change is merged. An
    # active topic branch is expected during work, so equality is enforced
    # only when checked out on the base branch (same rule as fieldbook).
    if branch == policy["branch"]["base"] and branches != sorted(
        rule["target_local_branches"]
    ):
        errors.append(
            f"local branches are {branches}, target is {sorted(rule['target_local_branches'])}"
        )

    remotes = sorted(line for line in git("remote").splitlines() if line)
    if remotes != sorted(rule["target_remote_names"]):
        errors.append(f"remotes are {remotes}, target is {sorted(rule['target_remote_names'])}")

    # `%(refname:short)` collapses `refs/remotes/origin/HEAD` to `origin`, which
    # looks like a fourth target; filter the symbolic ref by its full name.
    tracking = sorted(
        ref.removeprefix("refs/remotes/")
        for ref in git("for-each-ref", "--format=%(refname)", "refs/remotes").splitlines()
        if ref and not ref.endswith("/HEAD")
    )
    if tracking != sorted(rule["target_remote_tracking_branches"]):
        errors.append(
            f"remote-tracking branches are {tracking}, target is {sorted(rule['target_remote_tracking_branches'])}"
        )

    worktrees = git("worktree", "list", "--porcelain").count("worktree ")
    if worktrees != rule["target_worktree_count"]:
        errors.append(f"{worktrees} worktree(s), target is {rule['target_worktree_count']}")

    stashes = len([line for line in git("stash", "list").splitlines() if line])
    if stashes != rule["target_stash_count"]:
        errors.append(f"{stashes} stash(es), target is {rule['target_stash_count']}")

    return errors


def check_clean(policy: dict) -> list[str]:
    if not policy["working_tree"]["must_be_clean_after_finish"]:
        return []
    dirty = [line for line in git("status", "--porcelain").splitlines() if line]
    if dirty:
        preview = ", ".join(line.strip() for line in dirty[:5])
        more = "" if len(dirty) <= 5 else f" (+{len(dirty) - 5} more)"
        return [f"working tree is not clean: {preview}{more}"]
    return []


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Validate the working copy against the git policy")
    parser.add_argument("--policy", type=Path, default=POLICY)
    parser.add_argument("--base", default=None, help="defaults to the policy's base branch")
    parser.add_argument("--strict", action="store_true", help="also require a clean working tree")
    args = parser.parse_args(argv)

    if not args.policy.is_file():
        return fail([f"{args.policy} is missing"])
    policy = json.loads(args.policy.read_text(encoding="utf-8"))
    base = args.base or policy["branch"]["base"]

    try:
        branch = git("rev-parse", "--abbrev-ref", "HEAD").strip()
    except RuntimeError as error:
        return fail([str(error)])

    errors: list[str] = []
    errors.extend(check_branch(policy, branch))
    errors.extend(check_subjects(policy, base))
    errors.extend(check_authors(policy, base))
    errors.extend(check_generated(policy))
    errors.extend(check_inventory(policy, branch))
    if args.strict:
        errors.extend(check_clean(policy))

    if errors:
        return fail(errors)

    print(f"PASS: branch `{branch}`, commit subjects, tracked paths and inventory match the git policy")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
