#!/usr/bin/env python3
"""Проверка лицензий всех библиотек, которые попадают в программу для Windows.

Правило проекта (CLAUDE.md): только свободные лицензии MIT, Apache, BSD, ISC.
Библиотеки, без которых не работает GPUI, но с другой свободной лицензией,
перечислены поимённо в EXCEPTIONS — новая такая библиотека не пройдёт
незамеченной, CI упадёт.
"""
import json
import re
import subprocess
import sys

ALLOWED = {
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC",
    # Те же MIT и Apache, только иначе записанные.
    "Apache-2.0 WITH LLVM-exception", "MIT-0", "0BSD",
}

# Библиотеки, которые тянет GPUI, со свободными лицензиями вне списка.
# Решение владельца — docs/licenses.md.
EXCEPTIONS = {
    "Unicode-3.0": "таблицы Юникода (icu_*, unicode-ident): без них нет текста",
    "Zlib": "foldhash, slotmap, zlib-rs — внутри GPUI",
    "CC0-1.0": "notify — слежение за файлами в gpui-component",
    "MPL-2.0": "option-ext — через dirs, папки Windows",
    "BSL-1.0": "ryu — числа в JSON, есть и Apache-2.0",
    "Unlicense": "общественное достояние; у этих библиотек есть и MIT",
}


def ok_expr(expr: str) -> bool:
    expr = expr.replace("/", " OR ").strip()
    # Скобки: (A OR B) AND C
    def ev(e: str) -> bool:
        e = e.strip()
        while e.startswith("(") and e.endswith(")") and balanced(e[1:-1]):
            e = e[1:-1].strip()
        for op, fn in ((" AND ", all), (" OR ", any)):
            parts = split_top(e, op)
            if len(parts) > 1:
                return fn(ev(p) for p in parts)
        return e in ALLOWED or e in EXCEPTIONS
    return ev(expr)


def balanced(s: str) -> bool:
    d = 0
    for c in s:
        d += c == "("
        d -= c == ")"
        if d < 0:
            return False
    return d == 0


def split_top(e: str, op: str):
    parts, depth, cur, i = [], 0, "", 0
    while i < len(e):
        if e[i] == "(":
            depth += 1
        elif e[i] == ")":
            depth -= 1
        if depth == 0 and e.startswith(op, i):
            parts.append(cur)
            cur = ""
            i += len(op)
            continue
        cur += e[i]
        i += 1
    parts.append(cur)
    return parts


def main() -> int:
    meta = json.loads(subprocess.check_output([
        "cargo", "metadata", "--format-version", "1", "--locked",
        "--filter-platform", "x86_64-pc-windows-msvc",
    ]))
    ids = {n["id"] for n in meta["resolve"]["nodes"]}
    bad = []
    for p in meta["packages"]:
        if p["id"] not in ids or p.get("source") is None:
            continue
        lic = p.get("license") or ""
        if not lic or not ok_expr(lic):
            bad.append(f'{p["name"]} {p["version"]}: {lic or "лицензия не указана"}')
    if bad:
        print("Библиотеки с лицензией вне правила проекта:")
        print("\n".join(sorted(set(bad))))
        return 1
    print("Лицензии в порядке.")
    return 0


if __name__ == "__main__":
    assert ok_expr("MIT OR Apache-2.0")
    assert ok_expr("(MIT OR Apache-2.0) AND Unicode-3.0")
    assert not ok_expr("GPL-3.0")
    assert not ok_expr("MIT AND GPL-3.0")
    assert ok_expr("MIT/Apache-2.0")
    sys.exit(main())
