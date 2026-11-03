# Confiança embutida no launcher

O `build.rs` lê estes arquivos (ou as variáveis de ambiente equivalentes) na compilação:

- `chave_publica.txt`: a chave **pública** do autor, uma por linha, no formato
  `ed25519:<base64>` (saída de `utsim-publicar chaves mostrar`). Pode ter duas linhas durante
  uma troca de chave. Variável: `UTSIM_CHAVE_PUBLICA` (separe várias com `;`).
- `manifesto_url.txt`: o link do `manifesto.json` no Google Drive (o link de
  compartilhamento serve; o launcher converte). Variável: `UTSIM_MANIFESTO_URL`.

A chave pública e o link podem ficar no repositório (o repositório é privado, e nenhum dos
dois é segredo). A chave **privada** nunca: ela fica só em `%APPDATA%\UTSim\chaves\`.

Sem chave pública o launcher compila, mas recusa qualquer manifesto (build de desenvolvimento).
