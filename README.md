<p align="center">
  <img src="assets/icon.png" alt="Ícone do ShadowVPN" width="180">
</p>

# ShadowVPN

ShadowVPN é uma VPN de grupo inspirada no Radmin VPN, escrita em Rust. Cada computador recebe um IP virtual e pode acessar os outros membros do mesmo grupo. O tráfego entre os computadores é cifrado com WireGuard (BoringTun), a chave de cada peer é autenticada com XEdDSA (o algoritmo de identidade do Signal) e um PSK pairwise é derivado das identidades X25519. O servidor de relay encaminha os datagramas para que a VPN funcione atrás de NAT, sem abrir portas em cada cliente.

> Status: MVP funcional, mas **não auditado**. Use em laboratório, escritório ou comunidade de confiança. Leia a seção [Segurança](#segurança) antes de expor o servidor na internet.

## Recursos

- Convites `shadowvpn://` que embutem servidor e rede; a senha nunca vai no link.
- Criação explícita de redes, persistidas no servidor, com senha compartilhada.
- IP virtual estável por identidade, persistido no servidor mesmo após reinícios.
- Criptografia ponta a ponta WireGuard entre os peers.
- Identidade XEdDSA verificada pelo servidor e pelos peers, com fingerprint SHA-256.
- PSK pairwise derivado por X25519/HKDF e aplicado ao WireGuard de cada par.
- Relay UDP no servidor, sem necessidade de port forwarding nos clientes.
- Cliente CLI e GUI desktop (egui/eframe) com visual escuro inspirado em VPNs de grupo, painel de rede, lista de computadores e status em tempo real.
- Botão **Servidor local** no painel, com animação vermelha quando desligado e azul quando ligado.
- Aba **Minhas redes** que guarda os convites das redes criadas neste computador.
- Windows, Linux (x86_64 e aarch64) e macOS (Intel e Apple Silicon).
- Redes sobrevivem a reinícios do servidor; sessões e peers ficam apenas em memória.
- Heartbeat autenticado por token de sessão, expiração, limpeza automática e reconexão com backoff.
- Autenticação administrativa por Bearer token para criar, listar, excluir, trocar a senha de redes e revogar ou liberar membros.
- Painel **Administrar servidor** na GUI para listar redes e membros, trocar senha, excluir rede e revogar acessos.
- Quota de redes configurável e deploy seguro com Docker Compose, Caddy e systemd.
- Nomes dos computadores, fingerprints e latência do servidor exibidos na GUI.
- Releases automatizados com AppImage, instalador Windows, checksums e `wintun.dll` no pacote Windows.

## Como funciona

```text
┌──────────────┐        WireGuard cifrado         ┌──────────────┐
│  Cliente A   │◄───────────────┐         ┌───────►│  Cliente B   │
│ TUN 26.26... │                │         │        │ TUN 26.26... │
└──────┬───────┘                │         │        └──────┬───────┘
       │                        │         │               │
       │  HTTP 8080/tcp         ▼         ▼   HTTP 8080/tcp│
       │              ┌──────────────────────────┐         │
       └─────────────►│ Servidor de coordenação  │◄────────┘
                      │ grupos, sessões e IPs    │
                      └────────────┬─────────────┘
                                   │
                      relay UDP 8081 (pacotes WireGuard)
                                   │
                      ┌────────────▼─────────────┐
                      │      Relay UDP           │
                      │ só encaminha ciphertext  │
                      └──────────────────────────┘
```

1. Uma pessoa cria a rede com `POST /networks`, informando nome e senha. O servidor guarda o hash Argon2 no arquivo de redes.
2. Cada cliente gera uma chave WireGuard persistente, assina uma prova XEdDSA ligada à rede e à chave pública e faz `POST /register` com grupo, senha, chave, prova e hostname.
3. O servidor valida a senha e a prova XEdDSA, aloca um IP virtual e devolve a lista de peers (cada um com sua prova e fingerprint) e o endereço do relay.
4. O cliente verifica a prova de cada peer, calcula um PSK pairwise com X25519/HKDF e cria a interface TUN; em seguida envia um `HELLO` UDP ao relay.
5. Cada peer mantém uma sessão WireGuard com os demais usando o PSK do par. O relay encaminha os datagramas pelo `session_id` de destino.
6. A cada 20 segundos o cliente faz `POST /peers` usando o token de sessão, sem reenviar a senha, para renovar o peer e atualizar a lista. Se o servidor ficar inacessível, o cliente tenta reconectar com backoff exponencial até 30 segundos.
7. O administrador pode revogar uma chave pública. A sessão ativa cai, o IP fica livre e novas entradas daquela chave são recusadas com `403` até ela ser liberada. Para remover o acesso de todos, troque a senha da rede.

O convite reúne o endereço do coordenador e o nome da rede. Ele não inclui a senha, não substitui o servidor e não é um serviço central: alguém ainda precisa hospedar o `shadowvpn-server`.

O túnel cobre apenas a rede virtual `26.26.26.0/24`. Ele **não** roteia todo o tráfego da internet por padrão.

## Requisitos

- Servidor: Rust estável recente, Linux/macOS/Windows e portas TCP 8080 e UDP 8081 liberadas.
- Cliente: Rust estável recente e privilégios para criar a interface TUN.
- Windows: executar como Administrador e manter `wintun.dll` ao lado do executável.
- Linux: `/dev/net/tun` disponível e `root`, `CAP_NET_ADMIN` ou `sudo`.
- macOS: `sudo`, pois a interface `utun` e as rotas exigem privilégios.

## Início rápido

Para um passo a passo mais detalhado, incluindo Windows, Linux, firewall e solução de problemas, veja [docs/COMO_CONECTAR.md](docs/COMO_CONECTAR.md).

### 1. Servidor

Para uso local, o modo mais simples é ligar o servidor pelo botão **Servidor local** da GUI: ele cria um token administrativo automaticamente. Para iniciar manualmente com autenticação:

```bash
mkdir -p data
SHADOWVPN_ADMIN_TOKEN_FILE="$PWD/data/admin-token" \
  cargo run --release -p shadowvpn-server
```

Na primeira execução, o servidor cria `data/admin-token` com permissão `0600`. Leia o token com `cat data/admin-token` e guarde-o como segredo. Por padrão, o servidor escuta em `0.0.0.0:8080` (HTTP) e `0.0.0.0:8081` (UDP relay). Para conferir:

```bash
curl http://127.0.0.1:8080/health
```

Sem `SHADOWVPN_ADMIN_TOKEN` nem `SHADOWVPN_ADMIN_TOKEN_FILE`, a criação de redes fica aberta e o servidor registra um aviso. Para uma rede não confiável, configure o token e use HTTPS conforme [docs/DEPLOY.md](docs/DEPLOY.md).

Se o servidor estiver atrás de proxy reverso ou NAT, informe o endereço público do relay:

```bash
SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081 cargo run --release -p shadowvpn-server
```

### 2. Criar a rede

A rede deve ser criada uma vez, antes que os computadores entrem. Use a GUI na aba **Criar rede** ou a CLI:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
cargo run --release -p shadowvpn-client -- create-network \
  --name minha-rede --password 'uma-senha-forte'
```

O subcomando `create-network` cria a rede sem conectar e imprime o link `shadowvpn://`. Se a rede já existir, o servidor responde `409 Conflict`. O comando equivalente via API, agora protegido por token, é:

```bash
curl -X POST https://vpn.exemplo.com/networks \
  -H "Authorization: Bearer COLE_O_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"group_name":"minha-rede","group_password":"uma-senha-forte"}'
```

O antigo `--create` continua disponível: ele cria a rede e já conecta este computador.

Guarde o arquivo de redes do servidor: ele contém os hashes das senhas, os IPs virtuais persistidos e não deve ser publicado. No servidor manual o padrão é `data/networks.json`; no servidor iniciado pela GUI, é a pasta de dados do usuário descrita abaixo. Sem esse arquivo, será necessário recriar as redes após um reinício.

### 3. Entrar na rede

Em outro computador, com a mesma versão, cole o convite, informe a senha e use **Entrar em rede** na GUI, ou execute:

```bash
SHADOWVPN_PASSWORD='uma-senha-forte' \
cargo run --release -p shadowvpn-client -- \
  --invite 'shadowvpn://join?server=https%3A%2F%2Fvpn.exemplo.com&network=minha-rede'
```

Também é possível informar `--server` e `--group` manualmente, sem convite. O cliente imprime o IP virtual e o relay nos logs. Use `SHADOWVPN_LOG=debug` para mais detalhes.

Para parar, pressione `Ctrl+C`. O cliente remove o peer do grupo e encerra a interface. A rede permanece cadastrada no servidor.

### 4. GUI

```bash
cargo run --release -p shadowvpn-gui
```

A GUI oferece três modos:

- **Criar rede**: define nome e senha, cria a rede no servidor e salva o convite em **Minhas redes**.
- **Entrar em rede**: recebe um link de convite e a senha da rede.
- **Minhas redes**: lista as redes que você criou neste computador e permite abrir a rede ou copiar o convite novamente.

Na aba **Minhas redes**, use **Abrir** para carregar a rede e mostrar o convite, **Copiar** para copiar o link diretamente ou **Remover** para tirar a rede da lista local. Remover da lista não apaga a rede do servidor. A senha nunca é salva; mantenha-a em um gerenciador de senhas ou em local seguro.

O painel lateral tem o cartão **Servidor local**, com um botão de energia animado:

- **Vermelho**: servidor desligado. Clique para iniciar.
- **Azul**: servidor ligado. Clique para parar.
- Enquanto inicia, o botão fica azul e o estado mostra **Iniciando...**.

Ao ligar, a GUI executa o `shadowvpn-server` que acompanha o pacote. Se o endereço configurado for `127.0.0.1`/`localhost`, ele é trocado pelo IP da máquina na rede local para que o convite funcione em outros computadores. O servidor escuta `0.0.0.0:8080` (TCP) e `0.0.0.0:8081` (UDP), e as redes ficam em um arquivo de dados do usuário. O servidor iniciado pela GUI para quando a janela fecha. Se ele já estiver rodando por fora, o cartão mostra **Ligado por fora** e deve ser encerrado pelo terminal.

O servidor de coordenação também pode ser configurado manualmente na seção **Servidor avançado**; no fluxo normal ele é preenchido pelo convite ou pelo botão local. Depois de conectar, a GUI mostra o IP virtual, o relay, sua chave pública, seu fingerprint e os peers conectados.

A criação da rede não exige privilégios especiais; a conexão TUN sim. A GUI não eleva privilégios automaticamente. No Windows, abra como Administrador; no Linux/macOS, inicie pelo terminal com as permissões necessárias.

O botão **Administrar servidor** abre o painel de redes e membros. Informe a URL e o token (no servidor local o token é lido automaticamente), clique em **Atualizar** e escolha **Gerenciar** em uma rede. O painel permite recarregar membros, revogar ou liberar uma chave, trocar a senha e excluir a rede com confirmação. O token permanece apenas na memória da GUI.

## Configuração

### Cliente

Flags da CLI (as variáveis de ambiente equivalentes estão entre parênteses):

| Flag | Variável | Padrão |
|---|---|---|
| `--invite` | `SHADOWVPN_INVITE` | — |
| `--server` | `SHADOWVPN_SERVER` | `http://127.0.0.1:8080` se não houver convite |
| `--group` | `SHADOWVPN_GROUP` | `minha-rede` se não houver convite |
| `--password` | `SHADOWVPN_PASSWORD` | obrigatória para conectar ou usar `--create` |
| `--admin-token` | `SHADOWVPN_ADMIN_TOKEN` | — (necessário para criar/gerenciar redes protegidas) |
| `--admin-token-file` | `SHADOWVPN_ADMIN_TOKEN_FILE` | — (alternativa ao token inline) |
| `--identity` | `SHADOWVPN_IDENTITY` | caminho padrão do sistema |
| `--create` | `SHADOWVPN_CREATE` | `false` |
| subcomandos | — | `create-network`, `list-networks`, `delete-network`, `rotate-password`, `list-members`, `revoke-member`, `restore-member` |
| — | `SHADOWVPN_HOSTNAME` | hostname do sistema; exibido aos peers |

Prefira variáveis de ambiente ou arquivos em vez de `--password` e `--admin-token`: argumentos de linha de comando podem aparecer no histórico e na lista de processos. O token em `SHADOWVPN_ADMIN_TOKEN_FILE` é lido do arquivo e nunca precisa ser passado inline.

Os subcomandos administrativos aceitam `--server`, `SHADOWVPN_SERVER`, `--admin-token` e `SHADOWVPN_ADMIN_TOKEN_FILE`. Exemplos:

```bash
shadowvpn-client list-networks
shadowvpn-client create-network --name minha-rede --password 'uma-senha-forte'
shadowvpn-client rotate-password minha-rede --new-password 'outra-senha-forte'
shadowvpn-client delete-network minha-rede
shadowvpn-client list-members minha-rede
shadowvpn-client revoke-member minha-rede 'CHAVE_PUBLICA_BASE64'
shadowvpn-client restore-member minha-rede 'CHAVE_PUBLICA_BASE64'
```

`revoke-member` encerra a sessão atual, libera o IP e bloqueia novas entradas daquela chave pública. A chave é listada por `list-members` e continua bloqueada após reinícios. `restore-member` remove o bloqueio; o membro recebe um IP livre no próximo registro. Revogar uma chave não impede alguém com a senha de gerar outra identidade, então use **Trocar senha** para cortar o acesso de todos.

O nome do grupo deve ter até 64 caracteres alfanuméricos, `-` ou `_`. A senha deve ter entre 8 e 128 caracteres. O convite tem precedência sobre `--server` e `--group`.

`--create` cria a rede antes de entrar e falha se ela já existir. Use apenas na primeira conexão. Se a criação da rede funcionar mas o TUN falhar por falta de privilégio, a rede permanece criada; corrija a permissão e entre sem `--create`.

A identidade XEdDSA é derivada da mesma chave WireGuard persistente, criada automaticamente, e nunca deve ser copiada entre computadores. O fingerprint exibido na GUI é um hash SHA-256 dessa chave; compare-o fora de banda se quiser detectar troca de peer:

- Linux: `$XDG_CONFIG_HOME/shadowvpn/identity.key` ou `~/.config/shadowvpn/identity.key`
- macOS: `~/Library/Application Support/shadowvpn/identity.key`
- Windows: `%APPDATA%\ShadowVPN\identity.key`

O arquivo tem permissão restrita no Unix. Se duas pessoas usarem a mesma identidade ao mesmo tempo, o servidor as tratará como o mesmo peer.

Os convites das redes que você criou pela GUI ficam em `created-networks.txt`:

- Linux: `$XDG_DATA_HOME/shadowvpn/created-networks.txt` ou `~/.local/share/shadowvpn/created-networks.txt`
- macOS: `~/Library/Application Support/shadowvpn/created-networks.txt`
- Windows: `%APPDATA%\ShadowVPN\created-networks.txt`

O arquivo contém um link de convite por linha — servidor e nome da rede, nunca a senha — e recebe permissão `0600` no Unix. Ele é separado do `networks.json` do servidor. Se a lista ainda não existir, a GUI importa automaticamente os nomes de redes do `networks.json` local; quando a lista está vazia, o botão **Importar redes do servidor local** repete a importação.

Quando o servidor é iniciado pelo botão **Servidor local** da GUI, o arquivo de redes do servidor fica em:

- Linux: `$XDG_DATA_HOME/shadowvpn/networks.json` ou `~/.local/share/shadowvpn/networks.json`
- macOS: `~/Library/Application Support/shadowvpn/networks.json`
- Windows: `%APPDATA%\ShadowVPN\networks.json`

O log `server.log` e o token `admin-token` ficam na mesma pasta; para administrar o servidor local pela CLI, aponte `SHADOWVPN_ADMIN_TOKEN_FILE` para ele ou use `SHADOWVPN_ADMIN_TOKEN`. `SHADOWVPN_NETWORKS_FILE` continua tendo precedência. Redes criadas em um servidor iniciado manualmente na pasta atual não aparecem automaticamente nesse arquivo; use o mesmo `SHADOWVPN_NETWORKS_FILE` ou copie o `networks.json`.

### Servidor

| Variável | Padrão | Descrição |
|---|---|---|
| `SHADOWVPN_LISTEN` | `0.0.0.0:8080` | Endereço HTTP da API |
| `SHADOWVPN_RELAY_LISTEN` | `0.0.0.0:8081` | Endereço UDP do relay |
| `SHADOWVPN_RELAY_ADVERTISE` | derivado do `Host` | Endereço público do relay devolvido aos clientes |
| `SHADOWVPN_PEER_TTL` | `75` | Segundos sem heartbeat até remover o peer (mínimo 30) |
| `SHADOWVPN_NETWORKS_FILE` | `data/networks.json` (manual) ou pasta de dados do usuário (GUI) | Arquivo JSON com redes, hashes e IPs persistentes |
| `SHADOWVPN_ADMIN_TOKEN` | — | Token Bearer com pelo menos 16 caracteres; habilita criação/gestão autenticada |
| `SHADOWVPN_ADMIN_TOKEN_FILE` | — | Arquivo com o token; criado com permissão `0600` se não existir |
| `SHADOWVPN_MAX_NETWORKS` | `1000` | Limite de redes no servidor |
| `SHADOWVPN_LOG` ou `RUST_LOG` | `info` | Filtro de logs, por exemplo `debug` |

Se `SHADOWVPN_RELAY_ADVERTISE` não for definido, o servidor usa o host do cabeçalho HTTP `Host` e a porta UDP efetiva. Em produção atrás de proxy reverso, defina explicitamente o endereço público:

```bash
SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081
```

Para IPv6, use colchetes: `SHADOWVPN_RELAY_ADVERTISE=[2001:db8::10]:8081`.

### API HTTP

| Método | Rota | Função |
|---|---|---|
| `GET` | `/health` | Saúde do servidor |
| `POST` | `/networks` | Criar uma rede (exige Bearer token quando a administração está configurada) |
| `POST` | `/register` | Entrar em uma rede existente, provar posse da chave WireGuard e obter IP, sessão, token de sessão e peers |
| `POST` | `/peers` | Heartbeat e lista atualizada de peers; aceita `session_token` ou a senha legada |
| `POST` | `/unregister` | Sair do grupo; aceita `session_token` ou a senha legada |
| `GET` | `/admin/networks` | Listar redes, peers online e membros conhecidos (Bearer token) |
| `DELETE` | `/admin/networks/{nome}` | Excluir uma rede e suas sessões (Bearer token) |
| `POST` | `/admin/networks/{nome}/password` | Trocar a senha e encerrar sessões (Bearer token) |
| `GET` | `/admin/networks/{nome}/members` | Listar membros online, offline e revogados (Bearer token) |
| `DELETE` | `/admin/networks/{nome}/members` | Revogar uma chave; corpo JSON `{"public_key":"..."}` (Bearer token) |
| `POST` | `/admin/networks/{nome}/members/restore` | Liberar uma chave revogada; corpo JSON `{"public_key":"..."}` (Bearer token) |

As redes são persistidas em `data/networks.json` no servidor manual ou na pasta de dados do usuário quando o servidor é iniciado pela GUI (configurável por `SHADOWVPN_NETWORKS_FILE` em ambos os casos), e sobrevivem a reinícios. O arquivo guarda hashes Argon2, o mapa de membros com os IPs virtuais estáveis e as chaves revogadas. Peers e sessões ativas ficam em memória e são perdidos no reinício, mas o cliente reconecta automaticamente e recupera o mesmo IP. O arquivo recebe permissão `0600` no Unix; a versão 3 é gravada na próxima alteração e versões 1 e 2 são lidas automaticamente. O `POST /register` exige o objeto `identity` com a assinatura XEdDSA da chave pública e da rede.

## Portas, firewall e HTTPS

- **8080/tcp**: API de coordenação. Pode ficar atrás de um proxy reverso com HTTPS.
- **8081/udp**: relay. Precisa estar acessível diretamente pelos clientes; não é proxy HTTP.
- Libere as duas portas no firewall do servidor e no provedor de nuvem.

A implantação completa com Docker Compose, Caddy, systemd, backup e rotação de token está em [docs/DEPLOY.md](docs/DEPLOY.md). Nesse caso, mantenha `SHADOWVPN_LISTEN=127.0.0.1:8080` e configure `SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081`. Sem HTTPS, a senha do grupo, o token de sessão, as chaves públicas e as provas de identidade trafegam em texto claro. O link de convite contém apenas servidor e rede; nunca inclua a senha ou o token nele.

## Permissões por sistema

### Linux

Opção recomendada, sem executar como root:

```bash
sudo setcap cap_net_admin+ep ./target/release/shadowvpn-client
sudo setcap cap_net_admin+ep ./target/release/shadowvpn-gui
```

Depois execute normalmente. Se preferir `sudo`:

```bash
sudo env \
  SHADOWVPN_SERVER=https://vpn.exemplo.com \
  SHADOWVPN_GROUP=minha-rede \
  SHADOWVPN_PASSWORD='uma-senha-forte' \
  ./target/release/shadowvpn-client
```

Use um `SHADOWVPN_IDENTITY` explícito se quiser que a chave criada pelo processo root pertença ao seu usuário.

O AppImage não carrega capabilities. Para usar a GUI empacotada, execute com `sudo`:

```bash
chmod +x ShadowVPN-v0.5.1-linux-x86_64.AppImage
sudo -E ./ShadowVPN-v0.5.1-linux-x86_64.AppImage
```

Alternativamente, extraia o AppImage com `--appimage-extract` e aplique `CAP_NET_ADMIN` ao binário interno.

### Windows

1. Prefira o instalador `ShadowVPN-vX.Y.Z-windows-x86_64-setup.exe`; ele inclui o `wintun.dll` e registra o executável para solicitar elevação.
2. Se usar o ZIP portátil, extraia, mantenha `wintun.dll` e `wintun-LICENSE.txt` na mesma pasta e execute `shadowvpn-gui.exe` como administrador.
3. Permita a rede no firewall do Windows quando solicitado.
4. Para hospedar o servidor, execute `shadowvpn-server.exe` em um terminal; o caminho relativo `data/networks.json` é criado na pasta atual. Se ela não for gravável, defina `SHADOWVPN_NETWORKS_FILE` para um caminho gravável.

Sem `wintun.dll`, a criação do TUN falha. O binário é x86_64.

### macOS

```bash
sudo ./shadowvpn-client
```

Se o arquivo foi baixado pelo navegador e o Gatekeeper bloquear:

```bash
xattr -dr com.apple.quarantine ShadowVPN-v0.5.1-macos-aarch64
```

Os binários não são assinados nem notarizados.

## Releases e build

Cada tag `v*` dispara `.github/workflows/release.yml` e publica:

| Sistema | Pacotes |
|---|---|
| Linux x86_64 | `ShadowVPN-vX.Y.Z-linux-x86_64.tar.gz` e `.AppImage` |
| Linux aarch64 | `ShadowVPN-vX.Y.Z-linux-aarch64.tar.gz` e `.AppImage` |
| Windows x86_64 | `ShadowVPN-vX.Y.Z-windows-x86_64.zip` e `-setup.exe` |
| macOS Intel | `ShadowVPN-vX.Y.Z-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `ShadowVPN-vX.Y.Z-macos-aarch64.tar.gz` |

Os arquivos compactados incluem `shadowvpn-client`, `shadowvpn-server`, `shadowvpn-gui`, `docs/` e `deploy/`; os AppImages incluem os três binários, o arquivo `.desktop` e o ícone para integração com o desktop. O pacote Windows portátil e o instalador incluem `wintun.dll` e a licença da Wintun. O release também publica `SHA256SUMS.txt` com os checksums de todos os artefatos.

Os binários Linux x86_64 são compilados no Ubuntu 22.04 (glibc 2.35) e os de aarch64 no Ubuntu 24.04 (glibc 2.39). A GUI Linux também precisa de bibliotecas de sistema para Wayland/X11 e OpenGL.

O instalador Windows foi compilado com Inno Setup e registra `shadowvpn-gui.exe` com a camada de compatibilidade `RUNASADMIN`, para que o UAC seja solicitado ao abrir a GUI. O ZIP continua disponível para uso portátil.

Para criar uma release:

```bash
git tag v0.5.1
git push origin v0.5.1
```

Também é possível executar o workflow manualmente em **Actions → Release multiplataforma → Run workflow**, informando uma tag que já exista no remoto.

Build local:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace
```

Binários gerados em `target/release/`: `shadowvpn-server`, `shadowvpn-client` e `shadowvpn-gui`.

## Segurança

- Os pacotes entre peers são cifrados e autenticados pelo WireGuard. O relay encaminha somente ciphertext, mas vê metadados como IPs públicos, sessões e volume de tráfego.
- Cada peer assina com XEdDSA uma mensagem que inclui a rede e sua chave pública WireGuard. O servidor só aceita o registro se a prova conferir, e cada cliente verifica a prova recebida dos outros peers.
- O PSK do WireGuard é derivado por X25519 entre as chaves estáticas dos dois peers e HKDF-SHA256 com contexto da rede e das duas chaves. O servidor não participa desse cálculo e não consegue derivar o PSK.
- XEdDSA é o algoritmo de identidade usado pelo Signal, mas o ShadowVPN **não implementa X3DH nem Double Ratchet**. A confidencialidade do transporte continua sendo a do WireGuard; o PSK pairwise é estático e não acrescenta ratcheting ou recuperação pós-comprometimento.
- O fingerprint exibido na GUI é `SHA-256(chave pública WireGuard)` em base64. Compare-o com o outro computador por um canal separado se quiser detectar um coordenador mal-intencionado que tente substituir peers por chaves próprias.
- A API de coordenação pode usar HTTP simples em laboratório. Em qualquer rede não confiável, use HTTPS. A senha da rede só é enviada no registro; o heartbeat seguinte usa um token de sessão aleatório.
- O servidor armazena apenas o hash Argon2 da senha do grupo. A senha em texto claro existe somente durante a requisição.
- Os `session_id` são compartilhados com os peers do grupo para permitir o roteamento. Um membro mal-intencionado do grupo pode tentar interferir no relay; o WireGuard ainda impede a leitura ou falsificação dos pacotes, mas indisponibilidade e metadados não são protegidos.
- Com `SHADOWVPN_ADMIN_TOKEN` configurado, criar, listar, excluir, trocar senhas, revogar e liberar membros exige Bearer token; sem token, a criação fica aberta e o servidor avisa no log. O token é comparado em tempo constante e deve ser tratado como segredo.
- Há quota de redes (`SHADOWVPN_MAX_NETWORKS`), mas não há rate limiting por IP. Para um servidor público, coloque a API atrás de um proxy/CDN com limites de requisição e monitore o consumo de memória/CPU.
- Não há auditoria de segurança independente, testes de penetração ou garantia de anonimato.

## Limitações

- **Somente relay**: todo o tráfego passa pelo servidor; não há conexão direta P2P/STUN/TURN. A largura de banda do servidor limita a VPN.
- **Coordenação centralizada**: o convite facilita a entrada e esconde o endereço na interface, mas ainda é preciso hospedar um `shadowvpn-server`. Não há serviço público padrão.
- **Sem Double Ratchet**: a camada de identidade usa XEdDSA e um PSK estático; não é o protocolo Signal completo do WhatsApp.
- **IPv4 apenas**: rede virtual `/24`, máximo de 253 peers por grupo.
- **Sem multicast**: broadcast IPv4 é replicado para os peers, mas multicast não é suportado.
- **Sem DNS próprio**: acesse os peers pelo IP virtual; os nomes são apenas exibidos na GUI.
- **Sessões em memória**: redes, hashes, membros e IPs virtuais persistem, mas peers e sessões ativas são perdidos ao reiniciar; o cliente reconecta automaticamente e mantém o IP.
- **Sem NAT traversal para o servidor**: o relay UDP precisa ser alcançável na porta publicada.
- **Convite sem senha**: o link contém servidor e rede, mas a senha deve ser enviada por outro canal.
- **Instalação limitada**: há instalador para Windows, AppImage para Linux e units systemd em `deploy/systemd`; não há tray, autostart universal ou serviço nativo para macOS.
- **Sem assinatura de código**: Windows SmartScreen, o instalador e o macOS Gatekeeper podem avisar.
- **Sem túnel de internet completa**: apenas a sub-rede virtual é roteada.
- **Revogação por identidade**: a revogação bloqueia uma chave pública, mas uma identidade nova ainda entra com a mesma senha da rede. Para revogar a pessoa, troque a senha.

## Solução de problemas

**A GUI mostra erro ao conectar e o túnel não sobe**
- Verifique se está executando como Administrador/root ou se concedeu `CAP_NET_ADMIN`.
- No Windows, confirme que `wintun.dll` está ao lado do `.exe`.

**Os peers aparecem, mas o ping não funciona**
- Confirme que os dois estão no mesmo grupo e com a mesma senha.
- Verifique se a porta UDP do relay está liberada no servidor e no provedor de nuvem.
- O firewall local pode bloquear ICMP. No Windows, permita ICMPv4 ou teste com uma conexão TCP.
- O MTU da interface é 1380 para acomodar WireGuard + relay.

**O relay devolvido aponta para 127.0.0.1 ou para o endereço errado**
- Defina `SHADOWVPN_RELAY_ADVERTISE` com o host/porta públicos.
- Se estiver atrás de proxy reverso, o cabeçalho `Host` pode não ser o endereço do relay.

**A conexão cai após reiniciar o servidor**
- O cliente detecta a queda no heartbeat e reconecta com backoff. Redes, membros e IPs persistem; as sessões antigas não. Se a rede tiver sido excluída, a senha trocada ou a chave revogada, a reconexão falha e a GUI mostra o erro. Peça ao administrador para usar `restore-member` se a revogação foi um engano.

**Linux: erro de permissão em `/dev/net/tun`**
- Confirme que o dispositivo existe: `ls -l /dev/net/tun`.
- Use `sudo` ou `setcap cap_net_admin+ep` no binário.

## Atualizando de 0.5.0 para 0.5.1

O servidor 0.5.1 adiciona revogação persistente de membros, listagem de membros e um painel de administração na GUI. A compatibilidade de clientes 0.5.0 com o servidor 0.5.1 é preservada; o novo cliente também aceita servidor 0.5.0, mas as rotas de membros só existem a partir do 0.5.1.

Atualize o servidor primeiro e faça backup de `networks.json`. O arquivo v2 é lido e passa a v3 na próxima gravação. O servidor 0.5.0 **não lê** o arquivo v3; para reverter, restaure o backup ou remova a seção de membros revogados com o servidor parado.

## Atualizando de 0.4.x para 0.5.x

O servidor 0.5.x adiciona token de sessão, IPs persistentes, administração autenticada, revogação de membros e quota de redes. Clientes antigos continuam aceitos pelo fallback de senha, mas devem ser atualizados para usar heartbeat por token e reconexão automática. O cliente 0.5.x exige servidor 0.5.x: atualize o servidor primeiro e depois os clientes.

A chave `identity.key` existente continua válida e o fingerprint não muda. Redes e senhas não precisam ser recriadas. O `networks.json` v1 é lido automaticamente e migra para v3 na próxima gravação; a v2 também é aceita. Faça backup antes de atualizar; rode servidor e clientes novos juntos quando possível.

## Estrutura

```text
shadowvpn-core/    DTOs, convites, protocolo do relay, constantes e validações
shadowvpn-crypto/  XEdDSA, fingerprints e derivação do PSK pairwise
shadowvpn-server/  API HTTP, redes persistentes, sessões e relay UDP
shadowvpn-client/  API, identidade, TUN, WireGuard e motor do cliente
shadowvpn-gui/     Interface desktop egui/eframe
docs/              Guia de conexão e implantação segura
deploy/            Docker Compose, Caddy, Dockerfile e units systemd
assets/            Ícone e identidade visual do ShadowVPN
.github/           CI e release multiplataforma
```

ShadowVPN não é afiliado ao Radmin VPN nem ao Signal. WireGuard e BoringTun pertencem aos seus respectivos autores. A DLL da Wintun é distribuída sob a licença incluída no pacote Windows.
