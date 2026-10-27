<p align="center">
  <img src="assets/icon.png" alt="Ícone do ShadowVPN" width="180">
</p>

# ShadowVPN

ShadowVPN é uma VPN de grupo inspirada no Radmin VPN, escrita em Rust. Cada computador recebe um IP virtual e pode acessar os outros membros do mesmo grupo. O tráfego entre os computadores é cifrado com WireGuard (BoringTun), a chave de cada peer é autenticada com XEdDSA (o algoritmo de identidade do Signal) e um PSK pairwise é derivado das identidades X25519. O servidor de relay encaminha os datagramas para que a VPN funcione atrás de NAT, sem abrir portas em cada cliente.

> Status: MVP funcional, mas **não auditado**. Use em laboratório, escritório ou comunidade de confiança. Leia a seção [Segurança](#segurança) antes de expor o servidor na internet.

## Recursos

- Convites `shadowvpn://` que embutem servidor e rede; a senha nunca vai no link.
- Criação explícita de redes, persistidas no servidor, com senha compartilhada.
- IP virtual estável por identidade enquanto a sessão estiver ativa.
- Criptografia ponta a ponta WireGuard entre os peers.
- Identidade XEdDSA verificada pelo servidor e pelos peers, com fingerprint SHA-256.
- PSK pairwise derivado por X25519/HKDF e aplicado ao WireGuard de cada par.
- Relay UDP no servidor, sem necessidade de port forwarding nos clientes.
- Cliente CLI e GUI desktop (egui/eframe) com visual escuro inspirado em VPNs de grupo, painel de rede, lista de computadores e status em tempo real.
- Botão **Servidor local** no painel, com animação vermelha quando desligado e azul quando ligado.
- Windows, Linux (x86_64 e aarch64) e macOS (Intel e Apple Silicon).
- Redes sobrevivem a reinícios do servidor; sessões e peers continuam em memória.
- Heartbeat, expiração de sessões e limpeza automática de peers inativos.
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
6. A cada 20 segundos o cliente reenvia o registro (idempotente) para renovar a sessão e atualizar a lista de peers.

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

```bash
cargo run --release -p shadowvpn-server
```

Por padrão, o servidor escuta em `0.0.0.0:8080` (HTTP) e `0.0.0.0:8081` (UDP relay). Para conferir:

```bash
curl http://127.0.0.1:8080/health
```

Se o servidor estiver atrás de proxy reverso ou NAT, informe o endereço público do relay:

```bash
SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081 cargo run --release -p shadowvpn-server
```

### 2. Criar a rede

A rede deve ser criada uma vez, antes que os computadores entrem. Use a GUI na aba **Criar rede** ou a CLI:

```bash
SHADOWVPN_SERVER=http://vpn.exemplo.com:8080 \
SHADOWVPN_GROUP=minha-rede \
SHADOWVPN_PASSWORD='uma-senha-forte' \
cargo run --release -p shadowvpn-client -- --create
```

`--create` cria a rede e já conecta este computador. Ao criar, o log imprime um link `shadowvpn://` com servidor e rede. Se a rede já existir, o servidor responde `409 Conflict`; nesse caso entre sem `--create`.

Também é possível criar apenas a rede via API:

```bash
curl -X POST http://vpn.exemplo.com:8080/networks \
  -H 'Content-Type: application/json' \
  -d '{"group_name":"minha-rede","group_password":"uma-senha-forte"}'
```

Guarde o arquivo de redes do servidor: ele contém os hashes das senhas. No servidor manual o padrão é `data/networks.json`; no servidor iniciado pela GUI, é a pasta de dados do usuário descrita abaixo. Sem esse arquivo, será necessário recriar as redes após um reinício.

### 3. Entrar na rede

Em outro computador, com a mesma versão, cole o convite, informe a senha e use **Entrar em rede** na GUI, ou execute:

```bash
SHADOWVPN_PASSWORD='uma-senha-forte' \
cargo run --release -p shadowvpn-client -- \
  --invite 'shadowvpn://join?server=http%3A%2F%2Fvpn.exemplo.com%3A8080&network=minha-rede'
```

Também é possível informar `--server` e `--group` manualmente, sem convite. O cliente imprime o IP virtual e o relay nos logs. Use `SHADOWVPN_LOG=debug` para mais detalhes.

Para parar, pressione `Ctrl+C`. O cliente remove o peer do grupo e encerra a interface. A rede permanece cadastrada no servidor.

### 4. GUI

```bash
cargo run --release -p shadowvpn-gui
```

A GUI oferece dois modos:

- **Criar rede**: define nome e senha, cria a rede no servidor e depois permite copiar o convite.
- **Entrar em rede**: recebe um link de convite e a senha da rede.

O painel lateral tem o cartão **Servidor local**, com um botão de energia animado:

- **Vermelho**: servidor desligado. Clique para iniciar.
- **Azul**: servidor ligado. Clique para parar.
- Enquanto inicia, o botão fica azul e o estado mostra **Iniciando...**.

Ao ligar, a GUI executa o `shadowvpn-server` que acompanha o pacote. Se o endereço configurado for `127.0.0.1`/`localhost`, ele é trocado pelo IP da máquina na rede local para que o convite funcione em outros computadores. O servidor escuta `0.0.0.0:8080` (TCP) e `0.0.0.0:8081` (UDP), e as redes ficam em um arquivo de dados do usuário. O servidor iniciado pela GUI para quando a janela fecha. Se ele já estiver rodando por fora, o cartão mostra **Ligado por fora** e deve ser encerrado pelo terminal.

O servidor de coordenação também pode ser configurado manualmente na seção **Servidor avançado**; no fluxo normal ele é preenchido pelo convite ou pelo botão local. Depois de conectar, a GUI mostra o IP virtual, o relay, sua chave pública, seu fingerprint e os peers conectados.

A criação da rede não exige privilégios especiais; a conexão TUN sim. A GUI não eleva privilégios automaticamente. No Windows, abra como Administrador; no Linux/macOS, inicie pelo terminal com as permissões necessárias.

## Configuração

### Cliente

Flags da CLI (as variáveis de ambiente equivalentes estão entre parênteses):

| Flag | Variável | Padrão |
|---|---|---|
| `--invite` | `SHADOWVPN_INVITE` | — |
| `--server` | `SHADOWVPN_SERVER` | `http://127.0.0.1:8080` se não houver convite |
| `--group` | `SHADOWVPN_GROUP` | `minha-rede` se não houver convite |
| `--password` | `SHADOWVPN_PASSWORD` | obrigatória |
| `--identity` | `SHADOWVPN_IDENTITY` | caminho padrão do sistema |
| `--create` | `SHADOWVPN_CREATE` | `false` |
| — | `SHADOWVPN_HOSTNAME` | hostname do sistema; exibido aos peers |

Prefira `SHADOWVPN_PASSWORD` em vez de `--password`: argumentos de linha de comando podem aparecer no histórico e na lista de processos.

O nome do grupo deve ter até 64 caracteres alfanuméricos, `-` ou `_`. A senha deve ter entre 8 e 128 caracteres. O convite tem precedência sobre `--server` e `--group`.

`--create` cria a rede antes de entrar e falha se ela já existir. Use apenas na primeira conexão. Se a criação da rede funcionar mas o TUN falhar por falta de privilégio, a rede permanece criada; corrija a permissão e entre sem `--create`.

A identidade XEdDSA é derivada da mesma chave WireGuard persistente, criada automaticamente, e nunca deve ser copiada entre computadores. O fingerprint exibido na GUI é um hash SHA-256 dessa chave; compare-o fora de banda se quiser detectar troca de peer:

- Linux: `$XDG_CONFIG_HOME/shadowvpn/identity.key` ou `~/.config/shadowvpn/identity.key`
- macOS: `~/Library/Application Support/shadowvpn/identity.key`
- Windows: `%APPDATA%\ShadowVPN\identity.key`

O arquivo tem permissão restrita no Unix. Se duas pessoas usarem a mesma identidade ao mesmo tempo, o servidor as tratará como o mesmo peer.

Quando o servidor é iniciado pelo botão **Servidor local** da GUI, o arquivo de redes fica em:

- Linux: `$XDG_DATA_HOME/shadowvpn/networks.json` ou `~/.local/share/shadowvpn/networks.json`
- macOS: `~/Library/Application Support/shadowvpn/networks.json`
- Windows: `%APPDATA%\ShadowVPN\networks.json`

O log `server.log` fica na mesma pasta, e `SHADOWVPN_NETWORKS_FILE` continua tendo precedência. Redes criadas em um servidor iniciado manualmente na pasta atual não aparecem automaticamente nesse arquivo; use o mesmo `SHADOWVPN_NETWORKS_FILE` ou copie o `networks.json`.

### Servidor

| Variável | Padrão | Descrição |
|---|---|---|
| `SHADOWVPN_LISTEN` | `0.0.0.0:8080` | Endereço HTTP da API |
| `SHADOWVPN_RELAY_LISTEN` | `0.0.0.0:8081` | Endereço UDP do relay |
| `SHADOWVPN_RELAY_ADVERTISE` | derivado do `Host` | Endereço público do relay devolvido aos clientes |
| `SHADOWVPN_PEER_TTL` | `75` | Segundos sem heartbeat até remover o peer (mínimo 30) |
| `SHADOWVPN_NETWORKS_FILE` | `data/networks.json` (manual) ou pasta de dados do usuário (GUI) | Arquivo JSON com as redes e hashes de senha |
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
| `POST` | `/networks` | Criar uma rede (nome e senha) |
| `POST` | `/register` | Entrar em uma rede existente, provar posse da chave WireGuard e obter IP, sessão e peers |
| `POST` | `/peers` | Heartbeat alternativo e lista atualizada de peers |
| `POST` | `/unregister` | Sair do grupo |

As redes são persistidas em `data/networks.json` no servidor manual ou na pasta de dados do usuário quando o servidor é iniciado pela GUI (configurável por `SHADOWVPN_NETWORKS_FILE` em ambos os casos), e sobrevivem a reinícios. Peers e sessões ficam em memória e são perdidos no reinício; os clientes renovam o registro no próximo heartbeat, mas podem precisar reconectar se o IP virtual mudar. O arquivo contém apenas hashes Argon2 e recebe permissão `0600` no Unix. O `POST /register` exige o objeto `identity` com a assinatura XEdDSA da chave pública e da rede.

## Portas, firewall e HTTPS

- **8080/tcp**: API de coordenação. Pode ficar atrás de um proxy reverso com HTTPS.
- **8081/udp**: relay. Precisa estar acessível diretamente pelos clientes; não é proxy HTTP.
- Libere as duas portas no firewall do servidor e no provedor de nuvem.

Exemplo de proxy TLS com Caddy:

```caddyfile
vpn.exemplo.com {
    reverse_proxy 127.0.0.1:8080
}
```

Nesse caso, mantenha `SHADOWVPN_LISTEN=127.0.0.1:8080` e configure `SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081`. Sem HTTPS, a senha do grupo, as chaves públicas e as provas de identidade trafegam em texto claro. O link de convite contém apenas servidor e rede; nunca inclua a senha nele.

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
  SHADOWVPN_SERVER=http://vpn.exemplo.com:8080 \
  SHADOWVPN_GROUP=minha-rede \
  SHADOWVPN_PASSWORD='uma-senha-forte' \
  ./target/release/shadowvpn-client
```

Use um `SHADOWVPN_IDENTITY` explícito se quiser que a chave criada pelo processo root pertença ao seu usuário.

O AppImage não carrega capabilities. Para usar a GUI empacotada, execute com `sudo`:

```bash
chmod +x ShadowVPN-v0.4.1-linux-x86_64.AppImage
sudo -E ./ShadowVPN-v0.4.1-linux-x86_64.AppImage
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
xattr -dr com.apple.quarantine ShadowVPN-v0.4.1-macos-aarch64
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

Os arquivos compactados incluem `shadowvpn-client`, `shadowvpn-server` e `shadowvpn-gui`; os AppImages incluem os três binários, o arquivo `.desktop` e o ícone para integração com o desktop. O pacote Windows portátil e o instalador incluem `wintun.dll` e a licença da Wintun. O release também publica `SHA256SUMS.txt` com os checksums de todos os artefatos.

Os binários Linux x86_64 são compilados no Ubuntu 22.04 (glibc 2.35) e os de aarch64 no Ubuntu 24.04 (glibc 2.39). A GUI Linux também precisa de bibliotecas de sistema para Wayland/X11 e OpenGL.

O instalador Windows foi compilado com Inno Setup e registra `shadowvpn-gui.exe` com a camada de compatibilidade `RUNASADMIN`, para que o UAC seja solicitado ao abrir a GUI. O ZIP continua disponível para uso portátil.

Para criar uma release:

```bash
git tag v0.4.1
git push origin v0.4.1
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
- A API de coordenação usa HTTP simples. Em qualquer rede não confiável, coloque-a atrás de HTTPS. A senha do grupo é enviada a cada requisição.
- O servidor armazena apenas o hash Argon2 da senha do grupo. A senha em texto claro existe somente durante a requisição.
- Os `session_id` são compartilhados com os peers do grupo para permitir o roteamento. Um membro mal-intencionado do grupo pode tentar interferir no relay; o WireGuard ainda impede a leitura ou falsificação dos pacotes, mas indisponibilidade e metadados não são protegidos.
- Qualquer pessoa que alcance a API pode criar uma rede nova e definir a senha dela. Use nomes difíceis de adivinhar ou restrinja o acesso à API. Depois de criada, a rede não pode ser recriada ou ter a senha alterada pela API; exclua-a do arquivo com o servidor parado ou implemente administração autenticada.
- Não há rate limiting nem cotas por IP. Para um servidor público, coloque a API atrás de um proxy com limites de requisição e monitore o consumo de memória/CPU.
- Não há auditoria de segurança independente, testes de penetração ou garantia de anonimato.

## Limitações

- **Somente relay**: todo o tráfego passa pelo servidor; não há conexão direta P2P/STUN/TURN. A largura de banda do servidor limita a VPN.
- **Coordenação centralizada**: o convite facilita a entrada e esconde o endereço na interface, mas ainda é preciso hospedar um `shadowvpn-server`. Não há serviço público padrão.
- **Sem Double Ratchet**: a camada de identidade usa XEdDSA e um PSK estático; não é o protocolo Signal completo do WhatsApp.
- **IPv4 apenas**: rede virtual `/24`, máximo de 253 peers por grupo.
- **Sem multicast**: broadcast IPv4 é replicado para os peers, mas multicast não é suportado.
- **Sem DNS próprio**: acesse os peers pelo IP virtual; os nomes são apenas exibidos na GUI.
- **Sessões em memória**: redes e hashes de senha são persistidos, mas peers e sessões são perdidos ao reiniciar o servidor.
- **Sem NAT traversal para o servidor**: o relay UDP precisa ser alcançável na porta publicada.
- **Convite sem senha**: o link contém servidor e rede, mas a senha deve ser enviada por outro canal.
- **Instalação limitada**: há instalador para Windows e AppImage para Linux; macOS continua apenas com binários portáveis. Não há serviço nem inicialização automática.
- **Sem assinatura de código**: Windows SmartScreen, o instalador e o macOS Gatekeeper podem avisar.
- **Sem túnel de internet completa**: apenas a sub-rede virtual é roteada.

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
- Redes persistem, mas sessões não. Aguarde o próximo heartbeat ou reconecte o cliente. Se o IP virtual mudou, a GUI pede reconexão.

**Linux: erro de permissão em `/dev/net/tun`**
- Confirme que o dispositivo existe: `ls -l /dev/net/tun`.
- Use `sudo` ou `setcap cap_net_admin+ep` no binário.

## Atualizando de 0.3.x para 0.4.x

O protocolo de registro mudou: o servidor 0.4.x exige a prova XEdDSA e os peers exigem o PSK pairwise. Atualize servidor, clientes e GUI juntos. Clientes 0.3.x não conseguem entrar em redes 0.4.x e vice-versa.

A chave `identity.key` existente continua válida; a identidade XEdDSA é derivada dela. O fingerprint passa a aparecer na GUI e não requer recriar redes nem senhas.

## Estrutura

```text
shadowvpn-core/    DTOs, convites, protocolo do relay, constantes e validações
shadowvpn-crypto/  XEdDSA, fingerprints e derivação do PSK pairwise
shadowvpn-server/  API HTTP, redes persistentes, sessões e relay UDP
shadowvpn-client/  API, identidade, TUN, WireGuard e motor do cliente
shadowvpn-gui/     Interface desktop egui/eframe
docs/              Guia de conexão passo a passo
assets/            Ícone e identidade visual do ShadowVPN
.github/           CI e release multiplataforma
```

ShadowVPN não é afiliado ao Radmin VPN nem ao Signal. WireGuard e BoringTun pertencem aos seus respectivos autores. A DLL da Wintun é distribuída sob a licença incluída no pacote Windows.
