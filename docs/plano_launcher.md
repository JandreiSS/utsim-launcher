# Plano do launcher da família UTSim

O launcher instala, atualiza e abre os aplicativos da família UTSim no computador dos
testadores. Hoje são o editor (`utsim`) e a prévia 3D em Godot (`utsim-3d`); depois vêm a
realidade virtual e aplicativos que não dependem do simulador (por exemplo, um gerenciador de
projetos Lean Six Sigma). O modelo de produtos e canais é genérico: um produto novo é só mais
uma entrada no manifesto.

Restrições do dono:

- **custo zero**: os arquivos ficam no Google Drive do autor, baixados por links "qualquer
  pessoa com o link";
- **repositório privado**: só os pacotes cifrados e o manifesto ficam acessíveis;
- **acesso por testador**: cada testador tem um código; revogar é parar de incluí-lo;
- **atualizações assinadas**: o launcher só aceita o que o autor assinou;
- **instalação por usuário**, sem administrador (Windows agora, Linux depois).

## 1. Arquitetura

```text
 máquina do autor                              Google Drive (link aberto)        máquina do testador
 ┌──────────────────────────────┐               ┌──────────────────────┐          ┌──────────────────────────────┐
 │ utsim-publicar               │  escreve na   │ manifesto.json       │  HTTPS   │ utsim-launcher (eframe)      │
 │  %APPDATA%\UTSim\chaves\     │  pasta do     │  (assinado ed25519)  │ ───────► │  chave pública embutida      │
 │   privada.txt (nunca sai)    │  Drive ─────► │ pacotes\*.utpk       │          │  código de acesso do testador│
 │  publicar\testadores.json    │  (sincroniza) │  (cifrados)          │          │  %LOCALAPPDATA%\UTSim\launcher│
 └──────────────────────────────┘               └──────────────────────┘          └──────────────────────────────┘
```

Três crates, fora dos `default-members` do workspace (não pesam o build do simulador):

| Crate | Papel | Dependências principais |
|---|---|---|
| `crates/utsim-launcher-core` | Biblioteca pura e testada: manifesto, assinatura, códigos, cifra, download com retomada, página do Drive, instalação, atalhos, relato | `ed25519-dalek 2.2.0`, `chacha20poly1305 0.10.1`, `argon2 0.5.3`, `sha2 0.10.9`, `ureq 2.12.1` (rustls), `zip 2.6.1` |
| `crates/utsim-publicar` | Ferramenta de linha de comando do autor; só escreve arquivos locais | núcleo, `clap` |
| `crates/utsim-launcher` | O aplicativo com janela (eframe 0.36, renderizador glow, sem wgpu) | núcleo, `eframe` |

As versões das crates de criptografia estão fixadas com `=`; trocar uma delas é uma mudança
revisada. Todas são do projeto RustCrypto ou do dalek-cryptography, as mais usadas em Rust.

O transporte HTTP é a interface `Transport` (um GET com `Range` opcional). A implementação real
usa `ureq` e também lê endereços `file://`, o que permite ao autor testar o launcher contra a
pasta local antes de enviar qualquer coisa. Os testes usam `MemoryTransport` (tabela em
memória que corta a conexão quando pedido) e um servidor HTTP mínimo em `127.0.0.1`.

### Pastas no testador

```text
%LOCALAPPDATA%\Programs\UTSim Launcher\utsim-launcher.exe     (instalador NSIS do launcher)
%LOCALAPPDATA%\UTSim\launcher\
  produtos\<produto>\<versão>\        programa extraído + .utsim-versao.json
  produtos\<produto>\ativo.json       {"version": "0.2.0"}: a versão que abre
  downloads\                          pacotes cifrados e arquivos .parcial (retomada)
  logs\launcher.log                   registro (vai para o relato de erro)
  estado.json                         código, canal por produto, última sequência aceita
  manifesto_cache.json                último manifesto aceito (uso sem rede)
%APPDATA%\Microsoft\Windows\Start Menu\Programs\UTSim Launcher\   atalhos
```

A versão nova é extraída numa pasta oculta e renomeada só quando completa; a versão ativa troca
pela substituição atômica de `ativo.json` (arquivo temporário e `rename`). Uma queda de energia
em qualquer ponto deixa a versão anterior funcionando. Os atalhos apontam para
`utsim-launcher.exe --abrir <produto>`, que abre a versão ativa sem mostrar a janela; por isso
continuam valendo depois de cada atualização. Depois de instalar, o launcher mantém a versão
ativa e mais uma anterior (para voltar) e apaga as outras; o botão "Remover versões antigas"
apaga também essa.

## 2. Formato do manifesto (versão 1)

Um único arquivo JSON, `manifesto.json`, com um **envelope** assinado:

```json
{
  "envelope": 1,
  "key_id": "3f9a0c1d2e4b5a69",
  "signature": "<base64 de 64 bytes>",
  "body": "<o corpo, como texto JSON>"
}
```

A assinatura ed25519 cobre os bytes exatos de `body`, então não há JSON canônico a manter.
`key_id` são os 8 primeiros bytes do SHA-256 da chave pública; ele permite que um launcher
confie em duas chaves durante uma troca de chave.

O corpo:

```json
{
  "format": 1,
  "sequence": 12,
  "published_at": "2026-11-02T12:00:00Z",
  "products": [
    {
      "id": "utsim",
      "name": "UTSim",
      "description": "Editor de modelos de simulação a eventos discretos",
      "releases": [
        {
          "version": "0.2.0",
          "channel": "testes",
          "published_at": "2026-11-02T12:00:00Z",
          "notes": "Markdown com as novidades",
          "executable": "bin/utsim-view.exe",
          "args": ["--novo"],
          "packages": [
            {
              "platform": "windows-x64",
              "url": "https://drive.usercontent.google.com/download?id=<ID>&export=download",
              "file": "utsim-0.2.0-windows-x64.utpk",
              "size": 215000000,
              "sha256": "<hex do arquivo cifrado>",
              "cipher": "xchacha20poly1305-stream-v1",
              "wraps": [
                {
                  "id": "K7QD",
                  "salt": "<base64 16 bytes>",
                  "nonce": "<base64 24 bytes>",
                  "key": "<base64 48 bytes>",
                  "argon2": { "m_kib": 65536, "t": 3, "p": 1 }
                }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

Regras de versão do formato:

- `envelope` e `format` maiores do que o launcher conhece são recusados com a mensagem "este
  manifesto usa o formato N, mais novo do que este launcher entende (até M). Atualize o
  launcher.";
- campos desconhecidos são ignorados, então acréscimos compatíveis mantêm o número;
- mudanças incompatíveis (outro esquema de cifra, outro significado de campo) sobem o número.

Validação: ids de produto e canal só com letras minúsculas, dígitos e `-`; versões com
letras, dígitos, `.`, `-` e `+` (viram nomes de pasta); `executable` relativo e sem `..`.
`sequence` cresce a cada publicação e o launcher recusa uma sequência menor que a última
aceita (proteção contra alguém repor um manifesto antigo para empurrar uma versão vulnerável).

Canais: `estavel` e `testes`. Quem segue `testes` vê também as versões estáveis. Outros nomes
de canal são aceitos (por exemplo, `vr-interno`) e aparecem na lista do produto.

## 3. Criptografia

**Pacote (`.utpk`)**: o conteúdo é um `.zip` da pasta do programa. Ele é cifrado em blocos de
1 MiB com XChaCha20-Poly1305 sob uma *chave de conteúdo* aleatória de 256 bits, nova a cada
pacote. Os nonces seguem a construção STREAM (prefixo aleatório de 19 bytes, contador de 32
bits e marcador de último bloco), e o cabeçalho de 32 bytes entra como dado associado de cada
bloco. Trocar, remover ou reordenar blocos, truncar o arquivo ou mudar o cabeçalho é detectado.
A memória usada não depende do tamanho do pacote.

**Embrulho por código**: para cada testador autorizado no canal, a chave de conteúdo é cifrada
(XChaCha20-Poly1305) com uma chave derivada do código dele por Argon2id (64 MiB, 3 passadas, 1
via; sal aleatório por embrulho). O dado associado inclui produto, versão, plataforma e o id do
testador, então um embrulho não serve em outro pacote. Os parâmetros do Argon2id vão no
manifesto e podem crescer com o tempo; o launcher recusa valores absurdos.

**Código de acesso**: 20 caracteres em base32 de Crockford, `IDID-XXXX-XXXX-XXXX-XXXC`. O
primeiro grupo é o id público (aleatório, sem nome), que diz ao launcher qual embrulho tentar;
15 caracteres são o segredo (75 bits); o último é um dígito verificador, que acusa erro de
digitação na hora. A entrada aceita minúsculas, espaços e as trocas O/0, I/1, L/1.

**Revogar**: o autor marca o testador como revogado; as publicações seguintes não embrulham a
chave para ele. As versões já publicadas continuam abertas para ele (ele já tinha acesso);
`utsim-publicar reembrulhar` refaz os embrulhos das versões publicadas para os testadores
ativos agora, o que tira dele o que ainda não baixou e dá aos novos testadores as versões
antigas.

**Ordem de verificação na instalação**: tamanho e SHA-256 do arquivo cifrado contra o manifesto
assinado; abertura do embrulho com o código; decifração autenticada; extração recusando
caminhos que saiam da pasta (`..`, absolutos); conferência de que o executável existe.

## 4. Google Drive

- O link de compartilhamento (`https://drive.google.com/file/d/<ID>/view?usp=sharing`) é
  convertido em `https://drive.usercontent.google.com/download?id=<ID>&export=download`.
- Arquivos acima de uns 100 MB recebem a página "Google Drive can't scan this file for viruses"
  (HTTP 200, `text/html`) no lugar do arquivo. O formato atual tem um formulário
  `id="download-form"` com os campos ocultos `id`, `export`, `confirm` e `uuid`; o launcher
  monta o GET com esses campos e segue. O formato antigo (link `/uc?...&confirm=<token>`)
  também é entendido. A retomada (`Range`) passa pela mesma página.
- Outras páginas viram mensagens claras: cota de downloads excedida ("tente de novo em algumas
  horas"), arquivo não compartilhado (página de login) e arquivo inexistente.
- As páginas de exemplo dos testes estão em `crates/utsim-launcher-core/tests/dados/`. Elas
  foram reconstruídas a partir do formato conhecido, sem acessar o Drive; quando um download
  real mostrar outra coisa, a página deve ser salva ali e ganhar um caso de teste.
- O manifesto é atualizado **no lugar** (não por renomeação), para o Drive para computador
  manter o mesmo id de arquivo, e portanto o mesmo link embutido no launcher.

## 5. Fluxo do autor

Uma vez:

1. `cargo build --release -p utsim-publicar`.
2. `utsim-publicar chaves gerar`. A privada vai para `%APPDATA%\UTSim\chaves\privada.txt`; a
   ferramenta recusa uma pasta de dados dentro de um repositório git e recusa sobrescrever uma
   chave existente. Faça uma cópia da privada num pendrive ou gerenciador de senhas.
3. `utsim-publicar chaves mostrar` e copie a linha `ed25519:...` para
   `crates/utsim-launcher/confianca/chave_publica.txt`.
4. Cadastre os testadores: `utsim-publicar testador adicionar "Nome" [--canal testes]`. O
   código aparece uma vez na tela (e fica em `testadores.json`, fora do repositório). Envie
   cada código por um canal privado.
5. Publique a primeira versão (próxima lista) para criar o `manifesto.json` na pasta do Drive;
   compartilhe o manifesto como "qualquer pessoa com o link" e grave o link em
   `crates/utsim-launcher/confianca/manifesto_url.txt`.
6. `bash scripts/instalador-launcher.sh` gera o launcher e o instalador
   `dist/utsim-launcher-v<versão>-windows-x64-instalador.exe`. Ponha o instalador no Drive e
   mande o link aos testadores, junto com o código de cada um.

A cada versão:

1. `cargo build --release -p utsim-cli -p utsim-viewer -p utsim-publicar`.
2. `bash scripts/publicar-utsim.sh "G:\Meu Drive\UTSim - testes" testes` (ou `utsim-publicar
   publicar --saida ... --produto ... --pasta ...` para outro produto). Isso cifra, embrulha,
   assina e escreve `pacotes\<produto>-<versão>-<plataforma>.utpk` e o `manifesto.json`.
3. Espere o Drive sincronizar, compartilhe o pacote como "qualquer pessoa com o link" e rode
   `utsim-publicar link --saida ... --produto ... --versao ... "<link>"`.
4. `utsim-publicar verificar --saida ...` confere assinatura, links e arquivos.

Antes de enviar, o autor pode abrir o launcher com o endereço `file:///G:/Meu%20Drive/...`
(campo "Avançado" da tela do código): pacotes ainda sem link são procurados ao lado do
manifesto.

Produto novo: `utsim-publicar publicar --produto lean-seis-sigma --nome "..." ...`. O launcher
já existente passa a mostrá-lo; nenhuma mudança no launcher é necessária. O UTSim Seis Sigma
já tem o seu: `cargo build --release -p utsim-lss-app -p utsim-publicar` e
`bash scripts/publicar-seis-sigma.sh "G:\Meu Drive\UTSim - testes" testes` (versão de
`crates/utsim-lss-app/Cargo.toml`).

## 6. Fluxo do testador

1. Baixa e roda o instalador do launcher (sem administrador).
2. Digita o código de acesso uma vez.
3. Escolhe o aplicativo e o canal, clica em "Instalar" (ou "Atualizar para ..."). O download
   mostra o progresso, pode ser cancelado e continua de onde parou.
4. Abre pelo botão "Abrir" ou pelo atalho no Menu Iniciar.
5. Se algo der errado, "Gerar relato de erro" salva `utsim-relato-<data>.zip` na pasta
   Downloads (resumo da máquina, versões instaladas e registros; nunca o código) para mandar
   ao autor.

O manual do testador está em `docs/manual/src/launcher.md`.

## 7. Segurança

### Ameaças cobertas

| Ameaça | Defesa |
|---|---|
| Alguém com o link baixa um pacote | O pacote está cifrado; sem um código autorizado, não abre |
| Alguém troca um pacote ou o manifesto no Drive (conta do Drive invadida, link adulterado no caminho) | Manifesto assinado com ed25519; pacote conferido pelo SHA-256 assinado e pela autenticação AEAD |
| Repor um manifesto antigo (assinado) para empurrar uma versão velha | `sequence` só cresce; o launcher recusa uma menor |
| Pacote truncado, corrompido ou com blocos trocados | STREAM com marcador de último bloco; SHA-256 |
| Zip com caminhos `..` ou absolutos | Extração recusa; `executable` validado |
| Testador revogado nas versões novas | Não há embrulho para ele |
| Código digitado errado | Dígito verificador; mensagem imediata |
| Chave privada parar num repositório | Ferramenta recusa pasta de dados dentro de repositório git; `.gitignore` não é a defesa |
| Força bruta contra um código a partir do manifesto | 75 bits de segredo e Argon2id de 64 MiB por tentativa |

### O que não cobre

- **Um testador autorizado pode repassar o programa ou o código.** Quem abre o pacote tem o
  programa em claro na própria máquina. A cifra controla quem baixa, não o que a pessoa faz
  depois. Revogar só vale daí em diante.
- **Metadados ficam visíveis** para quem tiver o link do manifesto: nomes dos produtos,
  versões, notas, tamanhos e os ids (aleatórios, sem nome) dos testadores. Se as notas forem
  sensíveis, cifrá-las é uma extensão possível (pendência).
- **Máquina do autor comprometida**: quem roubar a chave privada assina versões aceitas por
  todos os launchers. Mitigação: cópia offline, troca de chave com launcher novo (o `key_id`
  permite duas chaves ao mesmo tempo).
- **Máquina do testador comprometida**: o código fica em `estado.json` na pasta do usuário, em
  claro. Usar a proteção de dados do Windows (DPAPI) é uma pendência.
- **O próprio launcher não é assinado** (Authenticode). O Windows SmartScreen avisa na primeira
  execução do instalador. A assinatura de código fica para médio prazo (custo).
- **Disponibilidade**: o Drive pode limitar downloads de um arquivo muito baixado (cota) ou
  mudar a página de confirmação. O launcher dá a mensagem, mas não contorna o limite.
- **Atualização do próprio launcher**: ainda manual (novo instalador). O manifesto pode
  descrever um produto `utsim-launcher`, mas o launcher não se substitui sozinho.

## 8. Testes

`cargo test -p utsim-launcher-core -p utsim-publicar -p utsim-launcher` cobre, sem rede:

- ida e volta da cifra em vários tamanhos (0 byte, bordas de bloco, 3 MiB);
- chave errada, bit trocado, truncamento na borda de bloco, blocos trocados e cabeçalho
  alterado falham;
- embrulho: código certo abre, código errado e embrulho movido para outro pacote não abrem;
  parâmetros de Argon2id absurdos são recusados;
- revogação ponta a ponta (o revogado não abre a versão nova e mantém a antiga) e
  reembrulho;
- manifesto: assinatura adulterada ou de chave desconhecida rejeitada, envelope e corpo de
  formato futuro rejeitados com a mensagem de atualização, campos desconhecidos ignorados,
  caminhos hostis recusados, recusa de sequência menor;
- download: retomada depois de conexão cortada (em memória e com servidor HTTP local real
  via `ureq`), servidor sem `Range`, excesso de bytes, cancelamento, novas tentativas;
- Drive: aviso atual em inglês e em português, formato antigo, cota excedida, sem permissão,
  e o download completo passando pelo aviso também na retomada;
- instalação: troca atômica, versão ativa protegida, remoção de antigas, zip com `..`
  recusado, pacote alterado rejeitado antes de decifrar;
- relato de erro e registro; atalhos (texto do PowerShell e do `.desktop`).

A janela do launcher é compilada (`cargo build -p utsim-launcher`), não executada nos testes;
a lógica que ela chama está em `crates/utsim-launcher/src/lib.rs`, testada.

## 9. Estimativa (horas Opus 5.5)

| Parte | Estado | Estimativa |
|---|---|---|
| Núcleo, ferramenta do autor, app, instalador, testes, documentação | **feito** | 8–10 h |
| Primeira publicação real e ajuste da página do Drive com um arquivo grande | pendente, com o dono | 1–2 h |
| Atualização automática do próprio launcher | pendente | 3–4 h |
| Linux (pacote do launcher, atalho `.desktop` já existe no núcleo) | pendente | 2–3 h |
| Proteção do código com DPAPI; notas cifradas | pendente | 2–3 h |
| Assinatura de código Authenticode | depende de certificado | 1–2 h (fora o custo) |

## 10. Pendências e decisões do dono

- **Gerar a chave real** e guardar a cópia offline (só o dono pode).
- **Política de canais**: quem entra em `testes`; se algum testador só recebe `estavel`.
- **Retenção**: quantas versões antigas manter no Drive (espaço da conta) e no testador
  (hoje: ativa + uma).
- **Notas e nomes visíveis no manifesto**: aceitar como está ou cifrar.
- **Ícone da janela**: a janela usa o ícone padrão do egui (o executável já tem o do UTSim).
- **Produto `utsim-3d`**: o pacote do Godot sai do G5 (`docs/plano_godot.md`); a publicação
  usa o mesmo `utsim-publicar publicar` com outro `--produto` e `--executavel`.
- **Confirmar no primeiro envio real** que o Drive para computador mantém o id do
  `manifesto.json` ao reescrever o arquivo (o link embutido depende disso) e que a página de
  confirmação continua no formato dos exemplos.
- **CI Linux**: as crates novas entram em `cargo test --workspace`; o eframe (glow, x11,
  wayland) deve compilar no Ubuntu sem bibliotecas extras do sistema (carrega X11 e Wayland
  em tempo de execução), mas isso não foi confirmado; se o CI falhar, basta acrescentar
  `--exclude utsim-launcher` no job Linux ou instalar `libxkbcommon-dev`.
