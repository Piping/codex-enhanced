from __future__ import annotations

from hatchling.builders.hooks.plugin.interface import BuildHookInterface
from packaging.tags import sys_tags


class RuntimeBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict[str, object]) -> None:
        del version
        if self.target_name == "sdist":
            raise RuntimeError(
                "codex-enhanced is wheel-only; build and publish platform wheels only."
            )

        platform_tag = next(sys_tags()).platform
        build_data["pure_python"] = False
        build_data["tag"] = f"py3-none-{platform_tag}"
