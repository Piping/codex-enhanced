# Codex Enhanced Python Runtime

Platform-specific Python package that ships the `codex-enhanced` CLI as a
packaged binary wheel.

This package is intentionally wheel-only. Do not build or publish an sdist for
it.

Typical maintainer flow:

```bash
cd /Users/bytedance/code/codex/codex-rs
cargo build -p codex-cli

cd sdk/python
python scripts/update_sdk_artifacts.py \
  stage-runtime \
  /tmp/codex-python-release/codex-enhanced \
  /Users/bytedance/code/codex/codex-rs/target/debug/codex \
  --runtime-version 0.1.12 \
  --runtime-package enhanced

python -m build --wheel /tmp/codex-python-release/codex-enhanced
python -m twine upload /tmp/codex-python-release/codex-enhanced/dist/*
```
