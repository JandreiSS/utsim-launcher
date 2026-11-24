#!/usr/bin/env bash
# Publica o próprio UTSim Launcher pelo manifesto: gera o instalador NSIS
# (scripts/instalador-launcher.sh), cifra, assina e grava o pacote como o produto "launcher" na
# pasta de publicação (local), aberto para todos (sem código).
# O launcher instalado vê a versão nova, mostra "Nova versão do launcher" e se atualiza sozinho.
# Não envia nada à internet: depois, 'utsim-publicar github' sobe o pacote e o
# manifesto para o GitHub.
#
# Uso: bash scripts/publicar-launcher.sh <pasta_de_publicação> [canal] [versão]
#   canal   padrão: estavel (todo launcher vê). Com testes, só quem segue testes em algum app.
#   versão  padrão: a de crates/utsim-launcher/Cargo.toml. Tem de ser igual a ela: o launcher
#           compara a própria versão (a do Cargo.toml) com a do manifesto, e uma diferença o
#           faria se atualizar sem parar.
#
# Antes: cargo build --release -p utsim-publicar (o launcher é compilado aqui).
# Depois: utsim-publicar github --saida <pasta> --produto launcher --versao <versão>.

set -eu

cd "$(dirname "$0")/.." || exit 1

[ $# -ge 1 ] || { echo "uso: $0 <pasta_de_publicação> [canal] [versão]" >&2; exit 2; }
SAIDA=$1
CANAL=${2:-estavel}
CARGO_VERSAO=$(awk -F'"' '/^version = /{print $2; exit}' crates/utsim-launcher/Cargo.toml)
VERSAO=${3:-$CARGO_VERSAO}
VERSAO=${VERSAO#v}

if [ "$VERSAO" != "$CARGO_VERSAO" ]; then
    echo "erro: versão $VERSAO diferente da de crates/utsim-launcher/Cargo.toml ($CARGO_VERSAO)" >&2
    echo "      mude a versão no Cargo.toml do launcher antes de publicar" >&2
    exit 1
fi

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) PLATAFORMA=windows-x64 ;;
    *) echo "erro: o instalador do launcher (NSIS) só é gerado no Windows" >&2; exit 1 ;;
esac

PUBLICAR=target/release/utsim-publicar
[ -x "$PUBLICAR" ] || PUBLICAR=target/release/utsim-publicar.exe
[ -x "$PUBLICAR" ] || { echo "erro: rode antes cargo build --release -p utsim-publicar" >&2; exit 1; }

# Um launcher sem chave pública não se atualiza; publicar um assim prenderia quem o instalasse.
if [ -n "${UTSIM_LAUNCHER_DEV:-}" ]; then
    echo "erro: não publique um launcher de desenvolvimento (UTSIM_LAUNCHER_DEV)" >&2
    exit 1
fi

PASTA=$(mktemp -d)
trap 'rm -rf "$PASTA"' EXIT
bash scripts/instalador-launcher.sh "$VERSAO" "$PASTA/launcher"
EXE="utsim-launcher-v${VERSAO}-windows-x64-instalador.exe"
[ -f "$PASTA/launcher/$EXE" ] || { echo "erro: instalador $EXE não foi gerado" >&2; exit 1; }
# Só as linhas desta versão do launcher (com as linhas de continuação). O CHANGELOG inteiro
# como notas fazia o manifesto crescer centenas de kB a cada publicação.
if grep -q "^## \[launcher-$VERSAO\]" CHANGELOG.md; then
    bash scripts/notas-versao.sh "launcher-$VERSAO" > "$PASTA/notas.md"
else
    awk -v alvo="Launcher $VERSAO:" '
        /^## \[/ { n++; dentro = 0; next }
        n != 1 { next }
        /^- / { dentro = index($0, alvo) == 3 }
        /^[^ ]/ && !/^- / { dentro = 0 }
        dentro { print }
    ' CHANGELOG.md > "$PASTA/notas.md"
fi
[ -s "$PASTA/notas.md" ] || echo "(sem notas)" > "$PASTA/notas.md"

"$PUBLICAR" publicar \
    --saida "$SAIDA" \
    --produto launcher \
    --nome "UTSim Launcher" \
    --descricao "Instala e atualiza os aplicativos da família UTSim" \
    --autor "UTSim" \
    --versao "$VERSAO" \
    --canal "$CANAL" \
    --plataforma "$PLATAFORMA" \
    --pasta "$PASTA/launcher" \
    --executavel "$EXE" \
    --notas "$PASTA/notas.md"

# Aberto para todos: a chave do pacote vai no manifesto assinado (repetir não faz mal).
"$PUBLICAR" acesso --saida "$SAIDA" --produto launcher --canal "$CANAL" aberto
