#!/usr/bin/env python3
"""Fail if ios/native-bridge/include/lanlink.h and src/lib.rs disagree on
the set of exported lanlink_* functions."""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent / "ios" / "native-bridge"
rust = (root / "src" / "lib.rs").read_text(encoding="utf-8")
header = (root / "include" / "lanlink.h").read_text(encoding="utf-8")
header = re.sub(r"/\*.*?\*/", "", header, flags=re.S)

exported = set(re.findall(r'^pub (?:unsafe )?extern "C" fn (lanlink_\w+)', rust, re.M))
declared = set(re.findall(r"\b(lanlink_\w+)\s*\(", header))

for name in sorted(exported - declared):
    print(f"missing from lanlink.h: {name}")
for name in sorted(declared - exported):
    print(f"not exported by lib.rs: {name}")
if exported != declared or not exported:
    sys.exit(1)
print(f"ok: {len(exported)} functions")
