#!/bin/bash
set -e
cp target/debug/codex /usr/local/bin/codex
codesign --sign - --force --preserve-metadata=entitlements,requirements,flags,runtime /usr/local/bin/codex
echo done
