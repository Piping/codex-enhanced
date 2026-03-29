#!/bin/bash
set -e
sudo install -m 0755 target/debug/codex /usr/local/bin/codex
sudo codesign --sign - --force --preserve-metadata=entitlements,requirements,flags,runtime /usr/local/bin/codex
