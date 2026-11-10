#!/usr/bin/env bash
# Gera o UTSim Launcher em release e o instalador dele para Windows (NSIS).
#
# Uso: bash scripts/instalador-launcher.sh [versão] [pasta_de_saída]
#   versão          padrão: a de crates/utsim-launcher/Cargo.toml. Um prefixo "v" e sufixos como "-rc1" são ignorados no NSIS.
#   pasta_de_saída  padrão: dist
#
# A confiança embutida vem de crates/utsim-launcher/confianca/ (ou de UTSIM_CHAVE_PUBLICA e
# UTSIM_MANIFESTO_URL); veja o LEIA-ME de lá. Sem chave pública o script recusa, a não ser com
# UTSIM_LAUNCHER_DEV=1 (launcher de desenvolvimento, que não aceita manifesto nenhum).
# O makensis é procurado em $MAKENSIS, no PATH e nas pastas padrão do NSIS.
# O instalador só é gerado aqui, nunca executado.

set -eu

cd "$(dirname "$0")/.." || exit 1

VERSAO=${1:-$(awk -F'"' '/^version = /{print $2; exit}' crates/utsim-launcher/Cargo.toml)}
VERSAO=${VERSAO#v}
ROTULO=$VERSAO
VERSAO=${VERSAO%%-*}
SAIDA=${2:-dist}
CONF=crates/utsim-launcher/confianca

if [ -z "${UTSIM_CHAVE_PUBLICA:-}" ] && ! grep -qs '^ed25519:' "$CONF/chave_publica.txt"; then
    if [ "${UTSIM_LAUNCHER_DEV:-}" != "1" ]; then
        echo "erro: sem chave pública em $CONF/chave_publica.txt (rode 'utsim-publicar chaves mostrar')" >&2
        echo "      ou defina UTSIM_LAUNCHER_DEV=1 para um launcher de desenvolvimento" >&2
        exit 1
    fi
    echo "aviso: launcher de desenvolvimento, sem chave pública" >&2
fi
if [ -z "${UTSIM_MANIFESTO_URL:-}" ] && ! grep -qsv '^\s*\(#\|$\)' "$CONF/manifesto_url.txt"; then
    echo "aviso: sem endereço do manifesto ($CONF/manifesto_url.txt); o testador terá de informá-lo" >&2
fi

cargo build --release -p utsim-launcher

EXE=target/release/utsim-launcher.exe
[ -f "$EXE" ] || { echo "erro: $EXE não encontrado (o instalador NSIS é só para Windows)" >&2; exit 1; }

MAKENSIS=${MAKENSIS:-}
if [ -z "$MAKENSIS" ]; then
    if command -v makensis >/dev/null 2>&1; then
        MAKENSIS=$(command -v makensis)
    else
        for c in "/c/Program Files (x86)/NSIS/makensis.exe" "/c/Program Files/NSIS/makensis.exe"; do
            if [ -f "$c" ]; then
                MAKENSIS=$c
                break
            fi
        done
    fi
fi
[ -n "$MAKENSIS" ] || { echo "erro: makensis não encontrado (defina MAKENSIS)" >&2; exit 1; }

mkdir -p "$SAIDA"
case "$SAIDA" in
    /*|[A-Za-z]:*) SAIDA_ABS=$SAIDA ;;
    *) SAIDA_ABS=$PWD/$SAIDA ;;
esac
ARQUIVO="utsim-launcher-v${ROTULO}-windows-x64-instalador.exe"

nativo() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else echo "$1"; fi; }

MSYS2_ARG_CONV_EXCL='*' "$MAKENSIS" -V2 \
    "-DVERSAO=$VERSAO" \
    "-DEXE=$(nativo "$PWD/$EXE")" \
    "-DICONE=$(nativo "$PWD/assets/icone/familia/launcher.ico")" \
    "-DSAIDA=$(nativo "$SAIDA_ABS/$ARQUIVO")" \
    "$(nativo "$PWD/empacotamento/windows/utsim-launcher.nsi")"

echo "Instalador gerado: $SAIDA/$ARQUIVO"
