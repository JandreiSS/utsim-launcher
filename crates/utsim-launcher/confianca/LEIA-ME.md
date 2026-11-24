# Confiança embutida no launcher

O `build.rs` lê estes arquivos (ou as variáveis de ambiente equivalentes) na compilação:

- `chave_publica.txt`: a chave **pública** do autor, uma por linha, no formato
  `ed25519:<base64>` (saída de `utsim-publicar chaves mostrar`). Pode ter duas linhas durante
  uma troca de chave. Variável: `UTSIM_CHAVE_PUBLICA` (separe várias com `;`).
- `manifesto_url.txt`: o endereço do `manifesto.json`. Desde o launcher 0.3.6 é o asset fixo
  `manifesto.json` da release `manifesto` do repositório de downloads do launcher no GitHub,
  para onde `utsim-publicar github` e `utsim-publicar manifesto-github` enviam o manifesto
  (o teste `crates/utsim-publicar/tests/github.rs` confere que os dois são iguais). Um link do
  Google Drive também serve; o launcher converte. Variável: `UTSIM_MANIFESTO_URL`.

A chave pública e o endereço podem ficar no repositório: nenhum dos dois é segredo. A chave
**privada** nunca: ela fica só em `%APPDATA%\UTSim\chaves\`.

Sem chave pública o launcher compila, mas recusa qualquer manifesto (build de desenvolvimento).
