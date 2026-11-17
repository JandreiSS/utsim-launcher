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
| `crates/utsim-launcher` | O aplicativo com janela (eframe 0.36, renderizador glow, sem wgpu) | núcleo, `eframe`, `utsim-ui-tema` (recurso `familia`) |
| `crates/utsim-admin` | Janela do autor sobre a `utsim-publicar` (seção 12); nunca vai para os testadores | `utsim-publicar`, `eframe`, `utsim-ui-tema` |

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

### Campos opcionais (acréscimo compatível, ainda formato 1)

Um produto pode trazer a própria aparência no launcher, para que um produto novo não exija
launcher novo, e o acesso por canal:

```json
{
  "id": "lean-seis-sigma",
  "name": "UTSim Seis Sigma",
  "cor": "#4FD06A",
  "subtitulo": "Projetos Lean Seis Sigma",
  "autor": "UTSim",
  "icone": "<PNG em base64>",
  "acesso": { "testes": "Aberto" },
  "releases": [ { "...": "...", "packages": [ { "...": "...", "chave_aberta": "<base64>" } ] } ]
}
```

- `cor`: destaque do produto (`#RRGGBB`, validada na assinatura); `subtitulo`: a linha sob o
  nome; `autor`: a etiqueta "POR ..." (padrão "UTSim").
- `icone`: PNG quadrado, de 32 a 512 pixels de lado (recomendado **256 x 256**, cantos já
  arredondados e transparentes), até 256 kB, embutido em base64 (fica coberto pela
  assinatura). Um ícone inválido é ignorado e o launcher desenha um monograma (as iniciais na
  cor do produto).
- `acesso`: `Codigo` (padrão, o que vale para canais ausentes) ou `Aberto` (seção 13).
- `chave_aberta` (no pacote): a chave de conteúdo em claro, presente só nos pacotes publicados
  com o canal aberto.

Os campos só são escritos quando têm valor, então um manifesto sem eles é idêntico ao do
primeiro launcher, e um launcher antigo ignora os campos novos (testes
`old_manifests_still_parse_and_verify` e `new_fields_roundtrip_and_old_readers_ignore_them`).
`utsim-publicar publicar` recebe `--cor`, `--subtitulo`, `--autor` e `--icone <png>`; os
valores ficam no manifesto e as publicações seguintes os mantêm. Os scripts
`publicar-utsim.sh` e `publicar-seis-sigma.sh` já passam os ícones de
`assets/icone/familia/` (`utsim_256.png`, `lean-seis-sigma_256.png`).

O launcher também conhece a aparência dos produtos que existiam quando foi gerado
(`catalogo::BUILTIN`): `utsim` (azul `#22B5FF`, rede de nós), `utsim-3d`, `lean-seis-sigma`
(verde `#4FD06A`, σ), `ensino` (roxo `#8A5CE6`, capelo), `qualidade` (azul `#3A6EE0`, lupa),
`seguranca` (laranja `#F28C1E`, capacete) e `estatistica` (verde-água `#1FB5A3`, curva normal).
Os ícones seguem o mesmo estilo plano e saem de `tools/icone/gerar_svgs.py`. O que vier no
manifesto tem prioridade.

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

1. `cargo build --release -p utsim-publicar -p utsim-admin` (a linha de comando e a janela do
   administrador, `target\release\utsim-admin.exe`; seção 12).
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
- **Máquina do testador comprometida**: no Windows, o código fica em `estado.json` cifrado com a
  proteção de dados do Windows (DPAPI, escopo do usuário, `utsim_launcher_core::protect`), como
  `"dpapi:<base64>"`. Uma cópia do arquivo levada a outra conta ou outro computador não revela
  o código (lá ele some e o launcher pede de novo). Quem roda como o próprio usuário, na mesma
  máquina, ainda consegue abri-lo: a DPAPI não protege contra isso. Um `estado.json` antigo,
  com o código em claro, é protegido na próxima abertura do launcher. No Linux o código
  continua em claro (sem serviço equivalente).
- **O próprio launcher não é assinado** (Authenticode). O Windows SmartScreen avisa na primeira
  execução do instalador. A assinatura de código fica para médio prazo (custo).
- **Disponibilidade**: o Drive pode limitar downloads de um arquivo muito baixado (cota) ou
  mudar a página de confirmação. O launcher dá a mensagem, mas não contorna o limite.
- **Atualização do próprio launcher**: o produto `launcher` do manifesto é o instalador NSIS
  (`scripts/publicar-launcher.sh`); o launcher instalado se atualiza sozinho a partir da 0.2.0.

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

A janela do launcher (`janela.rs`) e a do administrador são desenhadas sem tela nos testes
(`crates/utsim-launcher/tests/janela.rs`, `crates/utsim-admin/tests/telas.rs`): grade e lista
com 0, 1 e vários produtos em todos os estados, tela do código, download em andamento, notas,
confirmação de desinstalar, a animação de abertura em vários instantes e cada tela do
administrador. Com `UTSIM_CAPTURAS=<pasta>` cada caso também vira um PNG, pintado em software
por `utsim_ui_tema::captura` (sem GPU e sem janela), para revisar o desenho sem abrir o
programa. A lógica que a janela chama está em `lib.rs` e `catalogo.rs`, testada.

## 9. Estimativa (horas Opus 5.5)

| Parte | Estado | Estimativa |
|---|---|---|
| Núcleo, ferramenta do autor, app, instalador, testes, documentação | **feito** | 8–10 h |
| Primeira publicação real e ajuste da página do Drive com um arquivo grande | pendente, com o dono | 1–2 h |
| Atualização automática do próprio launcher | **feito** (0.2.0) | 3–4 h |
| Linux (pacote do launcher, atalho `.desktop` já existe no núcleo) | pendente | 2–3 h |
| Proteção do código com DPAPI | **feito** (0.3.2) | 2–3 h |
| Notas da versão cifradas no manifesto | pendente | 1–2 h |
| Assinatura de código Authenticode | depende de certificado | 1–2 h (fora o custo) |

## 10. Pendências e decisões do dono

- **Gerar a chave real** e guardar a cópia offline (só o dono pode).
- **Política de canais**: quem entra em `testes`; se algum testador só recebe `estavel`.
- **Retenção**: quantas versões antigas manter no Drive (espaço da conta) e no testador
  (hoje: ativa + uma).
- **Notas e nomes visíveis no manifesto**: aceitar como está ou cifrar.
- **Ícone**: o launcher usa `assets/icone/familia/launcher.ico` (executável e instalador) e
  `launcher_256.png` (janela e barra de tarefas). Os ícones 1024 x 1024 originais ficaram fora do
  repositório (só 256 e 64 entram).
- **Ícone do UTSim Ensino**: já embutido no launcher (`ensino_256.png`); falta o produto.
- **Produto `utsim-3d`**: o pacote do Godot sai do G5 (`docs/plano_godot.md`); a publicação
  usa o mesmo `utsim-publicar publicar` com outro `--produto` e `--executavel`.
- **Confirmar no primeiro envio real** que o Drive para computador mantém o id do
  `manifesto.json` ao reescrever o arquivo (o link embutido depende disso) e que a página de
  confirmação continua no formato dos exemplos.
- **CI Linux**: as crates novas entram em `cargo test --workspace`; o eframe (glow, x11,
  wayland) deve compilar no Ubuntu sem bibliotecas extras do sistema (carrega X11 e Wayland
  em tempo de execução), mas isso não foi confirmado; se o CI falhar, basta acrescentar
  `--exclude utsim-launcher` no job Linux ou instalar `libxkbcommon-dev`.

## 11. Interface do launcher

Linguagem visual plana e quase preta, comum ao launcher e ao administrador
(`utsim_ui_tema::familia`): fundo `#111214`, linhas de 1 pixel cinza, cantos retos, sem
sombras; nomes numa sans (Segoe UI no Windows, DejaVu ou Noto no Linux, lidas do sistema, sem
arquivo de fonte no repositório) e rótulos, versões e botões numa monoespaçada em versaletes
espaçados (Consolas, DejaVu Sans Mono). Sem as fontes do sistema, ficam as embutidas do egui.

- **Barra do topo**: "UTSIM" em negrito espaçado, divisória, "FAMÍLIA DE APPS"; à direita a
  engrenagem (testador, inserir ou trocar código, atalho na área de trabalho, animação de
  abertura, relato de erro). As bordas da janela são as do sistema.
- **Barra de ferramentas**: atualizar a lista; "ATUALIZAR TODOS (n)" (branco, instala em fila),
  "ORDENAR" (nome, mais recentes, com atualização), "Buscar aplicativos" e grade ou lista.
  Vista e ordem ficam em `estado.json` (`view`, `sort`).
- **Cartão**: faixa com a versão (`0.1.0 → 0.2.0`, a nova na cor do produto) e "NOTAS ↗";
  ícone, nome em dois tons ("UTSim" branco e "Seis Sigma" na cor; "UT" e "Sim" no editor),
  etiqueta "POR UTSIM", subtítulo, situação (ATUALIZAÇÃO DISPONÍVEL em amarelo, INSTALADO, NÃO
  INSTALADO, SEM ACESSO, SEM VERSÃO PARA ESTE SISTEMA, e ABERTO PARA TODOS em verde), descrição,
  "Última atualização"; embaixo o botão principal (branco quando muda algo, contornado para
  "Abrir") e "…" (notas, pasta, canal, versões anteriores, remover antigas, desinstalar). O
  progresso aparece numa barra fina sob o botão, que vira "Cancelar · 38%", e no rodapé.
- **Lista**: colunas APLICATIVO, STATUS, VERSÃO, LANÇAMENTO, AUTOR e as ações; linhas
  separadas por filetes. Em janelas estreitas, LANÇAMENTO e AUTOR saem.
- **Rodapé**: "VERIFICADO HÁ N MIN · VIA DRIVE" (ou "LISTA GUARDADA · SEM CONEXÃO"), a
  mensagem da última ação e o progresso.

### Animação de abertura

Cerca de 3,2 s, desenhada com o pintor do egui (sem vídeo, sem dependência nova): fundo quase
preto com uma grade de filetes que desliza devagar e algumas faíscas; as três engrenagens do
ícone do launcher entram girando engrenadas e com um leve zoom (ease-out expo); um brilho
diagonal passa; "UTSIM" aparece letra por letra subindo, com espaçamento largo, e "FAMÍLIA DE
APPS" surge em seguida; os ícones dos aplicativos se juntam sob o nome e voam para os seus
lugares na grade enquanto a janela de verdade aparece por baixo (transição contínua, sem
corte). A linha do tempo é a função pura `abertura::intro_frame(t)` (testada: progresso
monótono, fim exatamente em `DURACAO`, pular vai ao fim). Qualquer tecla ou clique pula; toca
uma vez por abertura; a opção "Mostrar animação de abertura" desliga; `--sem-animacao` e
`UTSIM_SEM_ANIMACAO=1` também; e no Windows, com "Efeitos de animação" desligado (o
`MinAnimate` = 0 que essa opção grava), ela não toca. A busca da lista começa no primeiro
quadro, antes da animação.

## 12. Administrador (`utsim-admin`)

Janela do autor sobre a biblioteca `utsim-publicar`, na mesma linguagem visual do launcher.
Gerada com `cargo build --release -p utsim-admin` (`target\release\utsim-admin.exe`); não
entra no instalador do launcher nem vai para os testadores. Usa a mesma pasta de dados da linha
de comando (`%APPDATA%\UTSim`, ou `--dados`/`UTSIM_PUBLICAR_DADOS`) e uma pasta de publicação
escolhida na janela, lembrada em `<dados>/publicar/admin.json` (a primeira sugestão é
`G:\Meu Drive\UTSim - testes`). Sem a chave privada, a janela só explica como criá-la
(`utsim-publicar chaves gerar`); ela nunca cria nem mostra a privada.

- **Testadores**: nome, id, canais, situação e data; "Adicionar" mostra o código uma vez, com
  "Copiar" e o aviso de enviar só para a pessoa; "Canais" edita a lista
  (`Store::set_channels`); "Revogar" e "Reativar".
- **Publicações**: produtos, versões, canal e, por pacote, tamanho, link (ok ou ausente),
  "aberto para todos" e quem tem acesso; "Verificar" (o mesmo relatório de
  `utsim-publicar verificar`); "Refazer acesso" (`reembrulhar`, numa linha de execução à parte,
  pois cada embrulho custa um Argon2id).
- **Acesso**: por produto e canal, "Com código" ou "Aberto para todos", sempre com confirmação
  e o aviso da seção 13.
- **Chaves**: chave pública (com "Copiar") e o id; caminhos dos arquivos.

## 13. Acesso aberto para todos

O dono decide, por produto e canal, liberar a instalação sem código ("quem controla sou eu").
Com `acesso: Aberto` no canal, a chave de conteúdo de cada pacote do canal vai para o manifesto
assinado (`chave_aberta`), ao lado dos embrulhos por testador; qualquer launcher a usa para
decifrar, e quem tem código continua usando o seu embrulho. A integridade não muda: o pacote
ainda é conferido pelo SHA-256 assinado e pela autenticação AEAD, e a chave só vale porque
veio num manifesto assinado.

- Abrir (`utsim-publicar acesso --saida ... --produto ... --canal ... aberto`, ou a tela
  Acesso) põe a chave nos pacotes **já publicados** do canal e reassina o manifesto no lugar;
  as próximas publicações do canal saem abertas, mesmo sem testador cadastrado.
- Fechar (`... codigo`) vale para a **próxima** publicação. **As versões publicadas enquanto o
  canal estava aberto continuam abrindo para quem baixou o manifesto nesse período**, porque a
  chave delas foi pública; não há como desfazer isso. Para fechar de fato, publique uma versão
  nova; `reembrulhar` tira as chaves antigas do manifesto (quem não baixou antes deixa de vê-las).
- O launcher mostra "ABERTO PARA TODOS" e instala sem código. A tela do código aparece sozinha
  só quando o launcher não sabe onde está a lista, ou uma vez, depois de baixar a lista, se
  algum produto pede código e ainda não há um ("Agora não" a fecha); depois, pelo botão
  "Inserir código" de um produto "SEM ACESSO" ou pelo menu da engrenagem.
- Testes: `crates/utsim-publicar/tests/acesso_aberto.rs` (aberto instala sem código, fechado
  exige código, fechar afeta só versões novas, reembrulhar tira as chaves).
