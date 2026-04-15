#!/usr/bin/env python3
"""Benchmark ccc, rg, and hybrid search workflows on this repository."""

from __future__ import annotations

import argparse
import json
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable


REPO_ROOT = Path(__file__).resolve().parent.parent
STOPWORDS = {"rg", "ccc", "search", "n", "path", "lang", "sed", "p", "bash"}
IGNORED_HIT_PATHS = {
    "scripts/benchmark_ccc_search.py",
    "docs/ccc-benchmark.md",
}


@dataclass(frozen=True)
class SearchHit:
    path: str
    line_start: int | None
    line_end: int | None
    source: str

    def display(self) -> str:
        if self.line_start is None:
            return self.path
        if self.line_end and self.line_end != self.line_start:
            return f"{self.path}:{self.line_start}-{self.line_end}"
        return f"{self.path}:{self.line_start}"


@dataclass(frozen=True)
class Target:
    path: str
    start: int | None = None
    end: int | None = None

    def matches(self, hit: SearchHit) -> bool:
        if hit.path != self.path:
            return False
        if self.start is None:
            return True
        if hit.line_start is None:
            return False
        target_end = self.end if self.end is not None else self.start
        hit_end = hit.line_end if hit.line_end is not None else hit.line_start
        return hit.line_start <= target_end and self.start <= hit_end


@dataclass(frozen=True)
class CccQuery:
    query: str
    path: str | None = None
    langs: tuple[str, ...] = ()
    limit: int = 10

    def command(self) -> list[str]:
        cmd = ["ccc", "search"]
        for lang in self.langs:
            cmd.extend(["--lang", lang])
        if self.path:
            cmd.extend(["--path", self.path])
        cmd.extend(["--limit", str(self.limit)])
        cmd.append(self.query)
        return cmd

    def token_count(self) -> int:
        return _token_count([self.query, self.path, *self.langs])


@dataclass(frozen=True)
class RgQuery:
    patterns: tuple[str, ...]
    paths: tuple[str, ...]

    def command(self, override_paths: Iterable[str] | None = None) -> list[str]:
        cmd = ["rg", "-n", "--no-heading", "--color", "never"]
        for pattern in self.patterns:
            cmd.extend(["-e", pattern])
        cmd.extend(list(override_paths) if override_paths is not None else self.paths)
        return cmd

    def token_count(self, override_paths: Iterable[str] | None = None) -> int:
        return _token_count([*self.patterns, *(list(override_paths) if override_paths is not None else self.paths)])


@dataclass(frozen=True)
class HybridPlan:
    seed: CccQuery
    rerank: RgQuery
    candidate_limit: int = 8
    path_hints: tuple[str, ...] = ()


@dataclass(frozen=True)
class Task:
    task_id: str
    prompt: str
    targets: tuple[Target, ...]
    ccc_queries: tuple[CccQuery, ...]
    rg_queries: tuple[RgQuery, ...]
    hybrid: HybridPlan


@dataclass
class MethodResult:
    task_id: str
    method: str
    prompt: str
    commands: list[str] = field(default_factory=list)
    hits: list[SearchHit] = field(default_factory=list)
    first_result: SearchHit | None = None
    answer: SearchHit | None = None
    first_result_sufficient: bool = False
    completed: bool = False
    queries_used: int = 0
    token_count: int = 0
    notes: list[str] = field(default_factory=list)
    scores: dict[str, int] = field(default_factory=dict)


TASKS: tuple[Task, ...] = (
    Task(
        task_id="T1",
        prompt="Find the workflow step that validates a `v...` tag matches the `codex-enhanced` runtime version.",
        targets=(Target(".github/workflows/pypi-release.yml", 66, 71),),
        ccc_queries=(
            CccQuery("workflow step validates v tag matches codex-enhanced runtime version"),
            CccQuery(
                "codex-enhanced runtime version tag validation v prefix",
                path=".github/workflows/**",
            ),
            CccQuery("codex-enhanced tag version validation", langs=("yaml",)),
        ),
        rg_queries=(
            RgQuery(
                ("codex-enhanced|runtime version|refs/tags/v|validate.*tag|tag matches",),
                (".github", "codex-rs"),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery(
                "workflow step validates v tag matches codex-enhanced runtime version",
                path=".github/workflows/**",
            ),
            rerank=RgQuery(
                ("Validate tag matches codex-enhanced runtime version|runtime_ver|tag_ver",),
                (),
            ),
            path_hints=("pypi-release.yml",),
        ),
    ),
    Task(
        task_id="T2",
        prompt="Find where the current `codex-enhanced` package version is declared.",
        targets=(Target("sdk/python-runtime-enhanced/pyproject.toml", 7, 7),),
        ccc_queries=(
            CccQuery("package version declaration", path="sdk/python-runtime-enhanced/*"),
        ),
        rg_queries=(
            RgQuery(
                ('^version *= *"[^"]+"',),
                (
                    "sdk/python-runtime-enhanced/pyproject.toml",
                    "sdk/python-runtime-enhanced",
                ),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("package version declaration", path="sdk/python-runtime-enhanced/*"),
            rerank=RgQuery(('^version *= *"[^"]+"',), ()),
            path_hints=("pyproject.toml",),
        ),
    ),
    Task(
        task_id="T3",
        prompt="Find the implementation that deletes a Feishu reaction by explicit reaction id.",
        targets=(Target("codex-rs/clawbot/src/provider/feishu.rs", 244, 260),),
        ccc_queries=(
            CccQuery("Feishu delete reaction explicit reaction id"),
        ),
        rg_queries=(
            RgQuery(
                ("reaction_id|delete.*reaction|remove.*reaction|delete_reaction",),
                ("codex-rs",),
            ),
            RgQuery(
                ("pub async fn remove_reaction_by_id|ApiRequest::delete\\(|reactions/\\{\\}",),
                ("codex-rs/clawbot/src/provider/feishu.rs",),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("Feishu delete reaction explicit reaction id"),
            rerank=RgQuery(
                ("remove_reaction_by_id|ApiRequest::delete\\(|reaction_id",),
                (),
            ),
            path_hints=("provider/feishu.rs",),
        ),
    ),
    Task(
        task_id="T4",
        prompt="Find where interactive Codex exit respawns the current session.",
        targets=(Target("codex-rs/cli/src/main.rs", 465, 480),),
        ccc_queries=(
            CccQuery("interactive Codex exit respawns current session"),
        ),
        rg_queries=(
            RgQuery(
                ("respawn|re-spawn|spawn.*session|exit.*respawn|respawn.*session|resume.*session",),
                ("codex-rs",),
            ),
            RgQuery(
                (
                    "finish_interactive_exit|build_interactive_respawn_args|RespawnCurrentSession|respawn_requested|respawn this session",
                ),
                ("codex-rs/cli/src/main.rs", "codex-rs/tui/src"),
            ),
            RgQuery(
                ("respawn_current_codex_session\\(|ExitReason::RespawnRequested",),
                ("codex-rs/cli/src/main.rs",),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("interactive Codex exit respawns current session"),
            rerank=RgQuery(
                ("respawn_current_codex_session\\(|ExitReason::RespawnRequested",),
                (),
            ),
            path_hints=("cli/src/main.rs",),
        ),
    ),
    Task(
        task_id="T5",
        prompt="Find the actual PTY-backed process spawn implementation, not docs or re-exports.",
        targets=(Target("codex-rs/utils/pty/src/pty.rs", 140, 161),),
        ccc_queries=(
            CccQuery("PTY backed process spawn implementation portable_pty spawn_command"),
        ),
        rg_queries=(
            RgQuery(
                (
                    "portable_pty|openpty|forkpty|spawn\\(|spawn_command|tokio::process::Command|pty-backed|master.open_slave|spawn_process",
                ),
                ("codex-rs",),
            ),
            RgQuery(
                (
                    "spawn_process_with_inherited_fds|spawn_command\\(|portable_pty::native_pty_system|openpty|SlavePty|MasterPty",
                ),
                ("codex-rs/utils/pty/src", "codex-rs/exec-server/src"),
            ),
            RgQuery(
                ("pair\\.slave\\.spawn_command\\(|openpty\\(|spawn_process_portable\\(",),
                ("codex-rs/utils/pty/src/pty.rs",),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("PTY backed process spawn implementation portable_pty spawn_command"),
            rerank=RgQuery(
                ("spawn_process_portable\\(|pair\\.slave\\.spawn_command\\(|pub async fn spawn_process",),
                (),
            ),
            path_hints=("utils/pty/src/pty.rs",),
        ),
    ),
    Task(
        task_id="T6",
        prompt="Find the doc section that explains how Enter behaves during paste-burst buffering.",
        targets=(Target("docs/tui-chat-composer.md", 235, 239),),
        ccc_queries=(
            CccQuery("Enter behaves during paste burst buffering documentation"),
            CccQuery("Enter behaves during paste burst buffering documentation", path="docs/**"),
        ),
        rg_queries=(
            RgQuery(
                ("paste-burst|paste burst|buffering|Enter behaves|Enter key|paste.*Enter",),
                ("docs", "codex-rs", "README.md"),
            ),
            RgQuery(
                ("### Enter handling|When paste-burst buffering is active",),
                ("docs/tui-chat-composer.md",),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("Enter behaves during paste burst buffering documentation"),
            rerank=RgQuery(
                ("### Enter handling|When paste-burst buffering is active",),
                (),
            ),
            path_hints=("docs/tui-chat-composer.md",),
        ),
    ),
    Task(
        task_id="T7",
        prompt="Find where the app-server protocol declares the `thread/dream/start` RPC.",
        targets=(Target("codex-rs/app-server-protocol/src/protocol/common.rs", 295, 298),),
        ccc_queries=(
            CccQuery("thread/dream/start RPC declaration"),
            CccQuery("ThreadDreamStart thread/dream/start", path="codex-rs/app-server-protocol/src/protocol/**"),
        ),
        rg_queries=(
            RgQuery(
                ("thread/dream/start|dream/start|ThreadDreamStart|DreamStart",),
                ("codex-rs/app-server-protocol",),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery(
                "ThreadDreamStart thread/dream/start",
                path="codex-rs/app-server-protocol/src/protocol/**",
            ),
            rerank=RgQuery(
                ("ThreadDreamStart|thread/dream/start",),
                (),
            ),
            path_hints=("app-server-protocol/src/protocol/common.rs",),
        ),
    ),
    Task(
        task_id="T8",
        prompt="Find the local install script used in the `codex-rs` release/testing flow.",
        targets=(Target("codex-rs/install_local.sh"),),
        ccc_queries=(
            CccQuery("local install script release testing flow codex-rs"),
            CccQuery("install_local.sh"),
        ),
        rg_queries=(
            RgQuery(
                ("install_local\\.sh|bash install_local\\.sh|local install script",),
                ("codex-rs", ".github", "docs"),
            ),
            RgQuery(
                ("^",),
                ("codex-rs/install_local.sh",),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("install_local.sh"),
            rerank=RgQuery(("^",), ()),
            path_hints=("codex-rs/install_local.sh", "install_local.sh"),
        ),
    ),
    Task(
        task_id="T9",
        prompt="Find the TUI bridge code that removes the clawbot auto-ack reaction by calling the Feishu provider.",
        targets=(Target("codex-rs/tui/src/app/enhanced/clawbot.rs", 788, 812),),
        ccc_queries=(
            CccQuery("clawbot auto ack reaction remove Feishu provider bridge"),
        ),
        rg_queries=(
            RgQuery(
                (
                    "remove_clawbot_auto_ack_reaction|remove_reaction_by_id|provider\\.remove_reaction|auto_ack_reaction_id",
                ),
                (
                    "codex-rs/tui/src/app/enhanced/clawbot.rs",
                    "codex-rs/tui/src/app/enhanced",
                ),
            ),
            RgQuery(
                ("async fn remove_clawbot_auto_ack_reaction|provider\\.remove_reaction_by_id\\(",),
                ("codex-rs/tui/src/app/enhanced/clawbot.rs",),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("clawbot auto ack reaction remove Feishu provider bridge"),
            rerank=RgQuery(
                ("async fn remove_clawbot_auto_ack_reaction|provider\\.remove_reaction_by_id\\(",),
                (),
            ),
            path_hints=("tui/src/app/enhanced/clawbot.rs",),
        ),
    ),
    Task(
        task_id="T10",
        prompt="Find the user-facing docs that advertise fast session respawn support in `codex-enhanced`.",
        targets=(Target("sdk/python-runtime-enhanced/README.md", 18, 18),),
        ccc_queries=(
            CccQuery("fast session respawn support codex-enhanced"),
            CccQuery("Fast respawn support"),
            CccQuery("fast respawn support", path="sdk/python-runtime-enhanced/**"),
            CccQuery("Fast respawn support restart and resume current session"),
        ),
        rg_queries=(
            RgQuery(
                ("codex-enhanced|fast session respawn|session respawn|respawn support|respawn",),
                ("docs", "sdk", "README.md", "codex-rs"),
            ),
            RgQuery(
                ("Fast `respawn` support|restart and resume the current session",),
                ("sdk/python-runtime-enhanced/README.md", "README.md"),
            ),
        ),
        hybrid=HybridPlan(
            seed=CccQuery("fast respawn support", path="sdk/python-runtime-enhanced/**"),
            rerank=RgQuery(
                ("Fast `respawn` support|restart and resume the current session",),
                (),
            ),
            path_hints=("sdk/python-runtime-enhanced/README.md", "README.md"),
        ),
    ),
)


def _token_count(parts: Iterable[str | None]) -> int:
    total = 0
    for part in parts:
        if not part:
            continue
        tokens = [token.lower() for token in re.findall(r"[A-Za-z0-9]+", part)]
        total += sum(1 for token in tokens if token not in STOPWORDS)
    return total


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--method",
        action="append",
        choices=("ccc", "rg", "hybrid"),
        help="Restrict to specific method(s). Defaults to all.",
    )
    parser.add_argument(
        "--task",
        action="append",
        help="Restrict to specific task ids such as T1. May be provided multiple times.",
    )
    parser.add_argument(
        "--refresh-ccc",
        action="store_true",
        help="Run `ccc index` before executing the benchmark.",
    )
    parser.add_argument(
        "--json-output",
        type=Path,
        help="Optional path for machine-readable benchmark output.",
    )
    return parser.parse_args()


def run_command(cmd: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd,
        text=True,
        capture_output=True,
        check=False,
    )


def render_command(cmd: list[str]) -> str:
    return shlex.join(cmd)


def parse_ccc_hits(stdout: str) -> list[SearchHit]:
    hits: list[SearchHit] = []
    pattern = re.compile(r"^File: (?P<path>.+?):(?P<start>\d+)(?:-(?P<end>\d+))? \[")
    for line in stdout.splitlines():
        match = pattern.match(line)
        if not match:
            continue
        hits.append(
            SearchHit(
                path=match.group("path"),
                line_start=int(match.group("start")),
                line_end=int(match.group("end")) if match.group("end") else None,
                source=line,
            )
        )
    return [hit for hit in hits if hit.path not in IGNORED_HIT_PATHS]


def parse_rg_hits(stdout: str, default_path: str | None = None) -> list[SearchHit]:
    hits: list[SearchHit] = []
    pattern = re.compile(r"^(?P<path>.+?):(?P<line>\d+):")
    single_file_pattern = re.compile(r"^(?P<line>\d+):")
    for line in stdout.splitlines():
        match = pattern.match(line)
        if match:
            hits.append(
                SearchHit(
                    path=match.group("path"),
                    line_start=int(match.group("line")),
                    line_end=None,
                    source=line,
                )
            )
            continue
        if default_path is None:
            continue
        single_file_match = single_file_pattern.match(line)
        if single_file_match:
            hits.append(
                SearchHit(
                    path=default_path,
                    line_start=int(single_file_match.group("line")),
                    line_end=None,
                    source=f"{default_path}:{line}",
                )
            )
    return [hit for hit in hits if hit.path not in IGNORED_HIT_PATHS]


def select_match(hits: Iterable[SearchHit], targets: tuple[Target, ...]) -> SearchHit | None:
    for hit in hits:
        if any(target.matches(hit) for target in targets):
            return hit
    return None


def run_ccc_only(task: Task) -> MethodResult:
    result = MethodResult(task_id=task.task_id, method="ccc", prompt=task.prompt)
    for query in task.ccc_queries:
        cmd = query.command()
        proc = run_command(cmd, REPO_ROOT)
        result.commands.append(render_command(cmd))
        result.queries_used += 1
        result.token_count += query.token_count()
        if proc.returncode != 0:
            result.notes.append(proc.stderr.strip() or f"`{' '.join(cmd)}` failed")
            continue
        hits = parse_ccc_hits(proc.stdout)
        if result.first_result is None and hits:
            result.first_result = hits[0]
        result.hits.extend(hits)
        matched = select_match(hits, task.targets)
        if matched:
            result.answer = matched
            break
    finalize_result(task, result)
    return result


def run_rg_only(task: Task) -> MethodResult:
    result = MethodResult(task_id=task.task_id, method="rg", prompt=task.prompt)
    for query in task.rg_queries:
        cmd = query.command()
        proc = run_command(cmd, REPO_ROOT)
        result.commands.append(render_command(cmd))
        result.queries_used += 1
        result.token_count += query.token_count()
        if proc.returncode not in (0, 1):
            result.notes.append(proc.stderr.strip() or f"`{' '.join(cmd)}` failed")
            continue
        default_path = query.paths[0] if len(query.paths) == 1 else None
        hits = parse_rg_hits(proc.stdout, default_path=default_path)
        if result.first_result is None and hits:
            result.first_result = hits[0]
        result.hits.extend(hits)
        matched = select_match(hits, task.targets)
        if matched:
            result.answer = matched
            break
    finalize_result(task, result)
    return result


def order_candidate_paths(hits: list[SearchHit], limit: int, path_hints: tuple[str, ...]) -> list[str]:
    unique_paths: list[str] = []
    seen: set[str] = set()
    for hit in hits:
        if hit.path in seen:
            continue
        seen.add(hit.path)
        unique_paths.append(hit.path)
        if len(unique_paths) >= limit:
            break

    if not path_hints:
        return unique_paths

    def score(path: str) -> tuple[int, int]:
        hint_score = sum(1 for hint in path_hints if hint in path)
        return (-hint_score, unique_paths.index(path))

    return sorted(unique_paths, key=score)


def run_hybrid(task: Task) -> MethodResult:
    result = MethodResult(task_id=task.task_id, method="hybrid", prompt=task.prompt)

    seed = task.hybrid.seed
    ccc_cmd = seed.command()
    ccc_proc = run_command(ccc_cmd, REPO_ROOT)
    result.commands.append(render_command(ccc_cmd))
    result.queries_used += 1
    result.token_count += seed.token_count()

    seed_hits: list[SearchHit] = []
    if ccc_proc.returncode == 0:
        seed_hits = parse_ccc_hits(ccc_proc.stdout)
    else:
        result.notes.append(ccc_proc.stderr.strip() or f"`{' '.join(ccc_cmd)}` failed")

    candidate_paths = order_candidate_paths(
        seed_hits,
        limit=task.hybrid.candidate_limit,
        path_hints=task.hybrid.path_hints,
    )

    rerank_hits: list[SearchHit] = []
    if candidate_paths:
        rerank = task.hybrid.rerank
        rg_cmd = rerank.command(candidate_paths)
        rg_proc = run_command(rg_cmd, REPO_ROOT)
        result.commands.append(render_command(rg_cmd))
        result.queries_used += 1
        result.token_count += rerank.token_count()
        if rg_proc.returncode in (0, 1):
            default_path = candidate_paths[0] if len(candidate_paths) == 1 else None
            rerank_hits = parse_rg_hits(rg_proc.stdout, default_path=default_path)
        else:
            result.notes.append(rg_proc.stderr.strip() or f"`{' '.join(rg_cmd)}` failed")

    ordered_hits = rerank_hits if rerank_hits else seed_hits
    result.hits.extend(ordered_hits)
    if ordered_hits:
        result.first_result = ordered_hits[0]
    result.answer = select_match(ordered_hits, task.targets)
    if result.answer is None:
        result.answer = select_match(seed_hits, task.targets)
    finalize_result(task, result)
    return result


def finalize_result(task: Task, result: MethodResult) -> None:
    result.first_result_sufficient = (
        result.first_result is not None
        and any(target.matches(result.first_result) for target in task.targets)
    )
    result.completed = result.answer is not None


def score_results(results: list[MethodResult]) -> None:
    by_task: dict[str, list[MethodResult]] = {}
    for result in results:
        by_task.setdefault(result.task_id, []).append(result)

    for task_results in by_task.values():
        completed_results = [result for result in task_results if result.completed]
        if completed_results:
            min_tokens = min(result.token_count for result in completed_results)
        else:
            min_tokens = None

        for result in task_results:
            result.scores = {
                "first": int(result.first_result_sufficient),
                "tokens": int(
                    min_tokens is not None
                    and result.completed
                    and result.token_count == min_tokens
                ),
                "completed": int(result.completed),
                "queries": int(result.queries_used <= 2),
            }


def markdown_summary(results: list[MethodResult]) -> str:
    lines = ["| method | total | first | tokens | completed | queries<=2 |", "| --- | --- | --- | --- | --- | --- |"]
    methods = sorted({result.method for result in results})
    for method in methods:
        subset = [result for result in results if result.method == method]
        total = sum(sum(result.scores.values()) for result in subset)
        first = sum(result.scores["first"] for result in subset)
        tokens = sum(result.scores["tokens"] for result in subset)
        completed = sum(result.scores["completed"] for result in subset)
        queries = sum(result.scores["queries"] for result in subset)
        lines.append(f"| {method} | {total} | {first} | {tokens} | {completed} | {queries} |")
    return "\n".join(lines)


def markdown_details(results: list[MethodResult]) -> str:
    lines = [
        "| task | method | score | first_result | answer | tokens | queries | first | done | <=2 |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for result in sorted(results, key=lambda item: (item.task_id, item.method)):
        total = sum(result.scores.values())
        first_result = result.first_result.display() if result.first_result else "none"
        answer = result.answer.display() if result.answer else "none"
        lines.append(
            f"| {result.task_id} | {result.method} | {total} | {first_result} | {answer} | "
            f"{result.token_count} | {result.queries_used} | {result.scores['first']} | "
            f"{result.scores['completed']} | {result.scores['queries']} |"
        )
    return "\n".join(lines)


def emit_json(results: list[MethodResult], destination: Path) -> None:
    payload = {
        "results": [
            {
                "task_id": result.task_id,
                "method": result.method,
                "prompt": result.prompt,
                "commands": result.commands,
                "first_result": result.first_result.display() if result.first_result else None,
                "answer": result.answer.display() if result.answer else None,
                "first_result_sufficient": result.first_result_sufficient,
                "completed": result.completed,
                "queries_used": result.queries_used,
                "token_count": result.token_count,
                "scores": result.scores,
                "notes": result.notes,
            }
            for result in results
        ]
    }
    destination.write_text(json.dumps(payload, indent=2) + "\n")


def refresh_ccc_index() -> None:
    proc = run_command(["ccc", "index"], REPO_ROOT)
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        raise SystemExit(proc.returncode)


def main() -> int:
    args = parse_args()
    methods = args.method or ["ccc", "rg", "hybrid"]
    selected_tasks = {task.task_id for task in TASKS} if not args.task else set(args.task)
    tasks = [task for task in TASKS if task.task_id in selected_tasks]

    if args.refresh_ccc:
        refresh_ccc_index()

    runners = {
        "ccc": run_ccc_only,
        "rg": run_rg_only,
        "hybrid": run_hybrid,
    }

    results: list[MethodResult] = []
    for task in tasks:
        for method in methods:
            results.append(runners[method](task))

    score_results(results)

    print("Summary")
    print(markdown_summary(results))
    print()
    print("Details")
    print(markdown_details(results))

    if args.json_output:
        emit_json(results, args.json_output)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
