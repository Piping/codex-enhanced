from __future__ import annotations

import os
import subprocess
import sys

from . import bundled_codex_path


def main() -> int:
    codex_path = bundled_codex_path()
    argv = [str(codex_path), *sys.argv[1:]]
    if os.name != "nt":
        os.execv(argv[0], argv)
        raise AssertionError("os.execv returned unexpectedly")

    return subprocess.call(argv)


if __name__ == "__main__":
    raise SystemExit(main())
