#!/usr/bin/env python3
"""Exporta esquema, addons, ajustes y conteos de la DB de PiStreaming.

Uso: python3 dump-db.py <pistreaming.db> [salida_dir]
"""
import json
import os
import sqlite3
import sys


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    db_path = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else "."
    os.makedirs(out, exist_ok=True)

    src = sqlite3.connect(db_path)
    # copia consistente
    backup = os.path.join(out, "pistreaming.db")
    dst = sqlite3.connect(backup)
    src.backup(dst)
    dst.close()

    c = src.cursor()
    schema = [
        sql + ";"
        for (_, sql) in c.execute(
            "select name, sql from sqlite_master where type='table' order by name"
        )
    ]
    with open(os.path.join(out, "schema.sql"), "w", encoding="utf-8") as fh:
        fh.write("\n\n".join(schema) + "\n")

    addons = [
        {
            "url": r[0],
            "name": r[1],
            "enabled": bool(r[2]),
            "position": r[3],
            "added_at": r[4],
            "manifest": json.loads(r[5]),
        }
        for r in c.execute(
            "select url,name,enabled,position,added_at,manifest_json "
            "from addons order by position,name"
        )
    ]
    with open(os.path.join(out, "addons.json"), "w", encoding="utf-8") as fh:
        json.dump(addons, fh, indent=2, ensure_ascii=False)

    settings = {k: v for k, v in c.execute("select key,value from settings")}
    with open(os.path.join(out, "settings.json"), "w", encoding="utf-8") as fh:
        json.dump(settings, fh, indent=2, ensure_ascii=False)

    counts = {
        t: c.execute(f'select count(*) from "{t}"').fetchone()[0]
        for (t,) in c.execute("select name from sqlite_master where type='table'")
    }
    with open(os.path.join(out, "counts.json"), "w", encoding="utf-8") as fh:
        json.dump(counts, fh, indent=2)

    print("OK:", counts)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
