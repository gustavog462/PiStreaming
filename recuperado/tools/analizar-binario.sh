#!/usr/bin/env bash
# Analiza el binario de PiStreaming y genera docs/ con la estructura recuperada.
# Uso: ./analizar-binario.sh [dir_proyecto]
set -euo pipefail

DIR="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
BIN="$DIR/bin/pistreaming"
OUT="$DIR/docs"
mkdir -p "$OUT"

[ -f "$BIN" ] || { echo "falta $BIN"; exit 1; }

echo "== símbolos =="
nm -C "$BIN" 2>/dev/null | grep -E "pistreaming_" | sed -E 's/^[0-9a-f]+ [a-zA-Z] //' | sort -u > "$OUT/simbolos.txt"

echo "== funciones propias =="
nm -C "$BIN" 2>/dev/null | grep -E " [Tt] pistreaming_" | sed -E 's/^[0-9a-f]+ [Tt] //' | sort -u > "$OUT/funciones.txt"

echo "== rutas de fichero fuente =="
strings "$BIN" | grep -oE "crates/[a-z_]+/src/[a-zA-Z0-9_/]+\.rs" | sort -u > "$OUT/estructura-fuente.txt"

echo "== eventos tracing con línea =="
strings "$BIN" | grep -oE "crates/[a-z_]+/src/[a-zA-Z0-9_/]+\.rs:[0-9]+" | sort -u > "$OUT/eventos-lineas.txt"

echo "== rutas HTTP =="
strings "$BIN" | grep -oE "/api/[a-zA-Z0-9_/:{}.-]+" | grep -vE "\.rs" | sort -u > "$OUT/rutas.txt"

echo "== strings relevantes =="
{
  echo "# Variables de entorno"
  strings "$BIN" | grep -oE "PISTREAMING_[A-Z_]+|OPENSUBTITLES_API_KEY|DATA_DIR|HTTP_PORT|EGRESS_BIND|CACHE_MAX_GB|CACHE_TTL_HOURS|LIBRARY_DIR|RUST_LOG" | sort -u
  echo
  echo "# Mensajes de error (español)"
  strings "$BIN" | grep -E "no se pudo|no se puede|inválid|falló|no encontrad|demasiado|sin seeds|no soportad|disco lleno|timeout" | grep -vE "^_R|llvm|drop_glue" | sort -u
  echo
  echo "# Argumentos ffmpeg"
  strings "$BIN" | grep -E "^-?(re|probesize|analyzeduration|rw_timeout|movflags|frag_keyframe|flush_packets|map|c:v|c:a|b:a|ac|crf|maxrate|bufsize|preset|f mp4|f webvtt)" | sort -u
} > "$OUT/strings-relevantes.txt"

echo "OK -> $OUT"
