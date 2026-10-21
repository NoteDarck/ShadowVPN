# ShadowVPN

ShadowVPN é um protótipo de VPN ponto a ponto baseado em WireGuard/BoringTun, com um servidor de coordenação, cliente CLI e uma GUI desktop em egui.

## O que foi corrigido

- Configuração do cliente por variáveis de ambiente, sem servidor, grupo ou senha hardcoded.
- Erros HTTP agora retornam falha explícita no cliente.
- Validação de grupo, senha, chave pública e endpoint.
- Registros duplicados não criam novos IPs.
- Alocação de IP protegida pelo lock da entrada do grupo.
- Servidor configurável via `SHADOWVPN_LISTEN`.
- GUI desktop para registrar no grupo, exibir status, IP virtual e peers.

## Executar

Requer Rust 1.80 ou superior. Para iniciar o coordenador:

```bash
cargo run -p shadowvpn-server
```

O servidor escuta em `0.0.0.0:8080` por padrão. Para alterar:

```bash
SHADOWVPN_LISTEN=0.0.0.0:9000 cargo run -p shadowvpn-server
```

Para usar o cliente CLI, abra outro terminal e configure:

```bash
export SHADOWVPN_SERVER=http://127.0.0.1:8080
export SHADOWVPN_GROUP=minha-rede
export SHADOWVPN_PASSWORD=uma-senha-segura
export SHADOWVPN_ENDPOINT=127.0.0.1:51820
cargo run -p shadowvpn-client
```

A criação da interface TUN exige privilégios administrativos e suporte de rede do sistema. O cliente CLI inicia o túnel quando houver pelo menos um peer disponível.

Para abrir a GUI:

```bash
cargo run -p shadowvpn-gui
```

## Executáveis para download

Cada tag `v*` dispara o workflow de release e publica pacotes prontos na página de Releases:

| Sistema | Pacote | Executáveis |
|---|---|---|
| Linux x86_64 | `ShadowVPN-vX.Y.Z-linux-x86_64.tar.gz` | `shadowvpn-client`, `shadowvpn-server`, `shadowvpn-gui` |
| Windows x86_64 | `ShadowVPN-vX.Y.Z-windows-x86_64.zip` | `shadowvpn-client.exe`, `shadowvpn-server.exe`, `shadowvpn-gui.exe` |
| macOS Intel | `ShadowVPN-vX.Y.Z-macos-x86_64.tar.gz` | `shadowvpn-client`, `shadowvpn-server`, `shadowvpn-gui` |
| macOS Apple Silicon | `ShadowVPN-vX.Y.Z-macos-aarch64.tar.gz` | `shadowvpn-client`, `shadowvpn-server`, `shadowvpn-gui` |

No Windows, extraia o ZIP e execute `shadowvpn-server.exe` ou `shadowvpn-gui.exe`. No Linux e macOS, extraia o pacote e, se necessário, aplique `chmod +x` aos três arquivos. A release também inclui o README dentro de cada pacote.

Na GUI, informe o servidor, grupo, senha com ao menos 8 caracteres e endpoint. O botão **Conectar** registra o peer no coordenador e mostra os peers retornados. A GUI é uma camada de controle/monitoramento; o túnel TUN continua sendo iniciado pelo cliente CLI, que evita elevar privilégios diretamente a partir da aplicação gráfica.

## Testes e build

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo build --release --workspace
```

## Limitações conhecidas

Este release mantém a arquitetura original de protótipo: estado do coordenador é mantido em memória, o cliente de túnel usa o primeiro peer retornado e não há NAT traversal/STUN. Para uso em produção, adicione persistência, TLS/autenticação de transporte, expiração de peers e gerenciamento de múltiplos túneis.
