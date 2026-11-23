#!/usr/bin/env bash
# Extrae el proyecto desplegado de PiStreaming desde el servidor.
# Uso: ./extraer.sh <usuario@host> [destino]
# Requiere: ssh, scp, curl, python3.
set -euo pipefail

REMOTE="${1:?uso: $0 usuario@host [destino]}"
DEST="${2:-$(cd "$(dirname "$0")/.." && pwd)}"
BASE="http://${REMOTE#*@}:8000"

mkdir -p "$DEST"/{assets,bin,config,data,docs,logs,tools}

echo "== assets =="
curl -fsS "$BASE/" -o "$DEST/assets/index.html"
curl -fsS "$BASE/assets/app.js" -o "$DEST/assets/app.js"
curl -fsS "$BASE/assets/style.css" -o "$DEST/assets/style.css"
curl -fsS "$BASE/icon.svg" -o "$DEST/assets/icon.svg"

echo "== binario =="
scp -q "$REMOTE:/home/${REMOTE%%@*}/pistreaming" "$DEST/bin/pistreaming"

echo "== configs =="
ssh "$REMOTE" 'cat /etc/systemd/system/pistreaming.service' > "$DEST/config/pistreaming.service"
ssh "$REMOTE" 'cat /etc/logrotate.d/pistreaming' > "$DEST/config/logrotate-pistreaming"

echo "== datos =="
ssh "$REMOTE" 'python3 - <<"PY"
import sqlite3, json
src = sqlite3.connect("/home/'"${REMOTE%%@*}"'/pistream-data/pistreaming.db")
dst = sqlite3.connect("/tmp/pistreaming-backup.db")
src.backup(dst); dst.close()
c = src.cursor()
schema = [sql + ";" for (_, sql) in c.execute("select name, sql from sqlite_master where type=\"table\" order by name")]
open("/tmp/pistreaming-schema.sql", "w").write("\n\n".join(schema) + "\n")
addons = [{"url": r[0], "name": r[1], "enabled": bool(r[2]), "position": r[3],
           "added_at": r[4], "manifest": json.loads(r[5])}
          for r in c.execute("select url,name,enabled,position,added_at,manifest_json from addons order by position,name")]
json.dump(addons, open("/tmp/pistreaming-addons.json", "w"), indent=2, ensure_ascii=False)
json.dump({k: v for k, v in c.execute("select key,value from settings")},
          open("/tmp/pistreaming-settings.json", "w"), indent=2, ensure_ascii=False)
counts = {t: c.execute(f"select count(*) from \"{t}\"").fetchone()[0]
          for (t,) in c.execute("select name from sqlite_master where type=\"table\"")}
json.dump(counts, open("/tmp/pistreaming-counts.json", "w"), indent=2)
PY'
for f in pistreaming-backup.db pistreaming-schema.sql pistreaming-addons.json \
         pistreaming-settings.json pistreaming-counts.json; do
  scp -q "$REMOTE:/tmp/$f" "$DEST/data/${f#pistreaming-}"
done
scp -q "$REMOTE:/home/${REMOTE%%@*}/pistream.log" "$DEST/logs/pistream.log"

echo "== info del sistema =="
ssh "$REMOTE" 'echo "## Fecha: $(date -Iseconds)"; uname -a; lsb_release -a 2>/dev/null; ffmpeg -version 2>/dev/null | head -2; file ~/pistreaming; sha256sum ~/pistreaming ~/pistream-data/pistreaming.db' > "$DEST/docs/sistema.txt"

echo "== análisis del binario =="
"$DEST/tools/analizar-binario.sh" "$DEST"

echo "OK -> $DEST"
