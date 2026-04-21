---
name: pypi-runtime-release
description: Use when publishing, debugging, or validating the codex-enhanced Python runtime package on PyPI, including pypi-release workflow failures, wheel metadata, platform tags, and project description cleanup.
---

Use this skill for `codex-enhanced` Python runtime release work only.

Default flow:

1. Determine the intended release version before changing files:
   - inspect local git tags first, for example `git tag --sort=-v:refname`
   - use the latest semver tag matching `vX.Y.Z` as the baseline
   - if the user did not specify a version, propose the next patch version from that local tag
2. Check the package identity under:
   - `sdk/python-runtime-enhanced/pyproject.toml`
   - `sdk/python-runtime-enhanced/README.md`
3. Keep the public metadata clean:
   - no local machine paths
   - no personal machine details
   - concise feature summary only
4. For local packaging, stage/build in this order:
   - build `codex` in `codex-rs`
   - stage with `sdk/python/scripts/update_sdk_artifacts.py`
   - build wheel from the staged runtime package
5. For release automation, treat these as the primary surface:
   - `pypi-release` workflow
   - `sdk/python-runtime-enhanced`
   - `sdk/python/scripts/update_sdk_artifacts.py`
6. When PyPI publish fails, classify the failure before changing anything:
   - auth / Trusted Publishing
   - invalid wheel platform tag
   - bad package metadata / description
   - missing wheel artifact

Useful rules:

- Prefer the fork package identity `codex-enhanced`; do not drift back to upstream package names.
- Runtime wheels are launcher/binary packages, not ABI-bound extension modules. Default to platform-specific `py3-none-PLAT` wheels, not interpreter-pinned tags like `cp313-cp313-*`.
- If a Linux wheel is rejected for a raw `linux_x86_64` platform tag, fix the wheel tag generation before retrying publish.
- Reuse the platform component from `packaging.tags.sys_tags()`, but do not reuse the full interpreter/ABI tag when generating runtime wheels.
- Keep `Requires-Python` aligned with the real launcher requirement. Do not overconstrain it to the interpreter version used to build the wheel.
- If the release is tag-driven, remember that rerunning an old tag uses the files from that tag, not the current `main`.
- Keep the workflow narrow: if the problem is only PyPI packaging, do not drag unrelated release workflow changes into the fix.
