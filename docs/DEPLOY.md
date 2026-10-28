# Implantação segura do servidor

Este guia publica o `shadowvpn-server` com HTTPS para a API e uma porta UDP dedicada para o relay. O modo recomendado usa Docker Compose + Caddy. O modo alternativo usa systemd + Caddy instalado no host.

## O que será exposto

| Porta | Protocolo | Uso | Exposição |
|---|---|---|---|
| 80 | TCP | redirecionamento e desafio ACME do Caddy | internet |
| 443 | TCP | API HTTPS | internet |
| 8081 | UDP | relay WireGuard | internet |
| 8080 | TCP | API interna do container | somente rede Docker |

Nunca publique a porta `8080` diretamente. O Caddy termina o TLS e encaminha para o container do servidor. O relay UDP não passa pelo Caddy.

## Pré-requisitos

- Um domínio ou subdomínio, por exemplo `vpn.exemplo.com`, apontando para o servidor.
- Portas `80/tcp`, `443/tcp` e `8081/udp` liberadas no firewall e no provedor de nuvem.
- Docker Engine com o plugin Compose.
- Acesso SSH ao servidor.

## Docker Compose + Caddy

No servidor:

```bash
git clone https://github.com/notedarckbr/ShadowVPN.git
cd ShadowVPN/deploy
cp .env.example .env
```

Edite `.env`:

```dotenv
SHADOWVPN_DOMAIN=vpn.exemplo.com
SHADOWVPN_RELAY_PORT=8081
SHADOWVPN_MAX_NETWORKS=1000
SHADOWVPN_LOG=info
```

O domínio precisa apontar para o servidor antes de subir o Caddy, porque o certificado é emitido por HTTP-01/TLS-ALPN.

```bash
docker compose up -d --build
docker compose logs -f caddy server
```

Confira a saúde pelo domínio:

```bash
curl -fsS https://vpn.exemplo.com/health
```

Resposta esperada:

```json
{"status":"ok"}
```

O Compose cria um volume persistente `shadowvpn-data` com `networks.json` e o token de administração. O token é gerado automaticamente com permissão restrita e não fica no arquivo `.env`:

```bash
docker compose exec server cat /var/lib/shadowvpn/admin-token
```

Trate essa saída como segredo. Não publique o token em issues, logs, capturas de tela ou no convite.

Para rotacionar o token, remova o arquivo e reinicie o servidor; um novo token será gerado:

```bash
docker compose exec server rm /var/lib/shadowvpn/admin-token
docker compose restart server
docker compose exec server cat /var/lib/shadowvpn/admin-token
```

A rotação invalida apenas o token administrativo; as sessões dos peers continuam válidas.

### Criar a primeira rede

Use a GUI com a URL `https://vpn.exemplo.com` e informe o token em **Criar rede**, ou use a CLI:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
shadowvpn-client create-network \
  --name minha-rede \
  --password 'uma-senha-forte'
```

A CLI imprime o convite `shadowvpn://`. Envie o convite e a senha por canais separados.

Para listar, trocar a senha ou excluir redes:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
shadowvpn-client list-networks

SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
shadowvpn-client rotate-password minha-rede \
  --new-password 'outra-senha-forte'

SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
shadowvpn-client delete-network minha-rede
```

A troca de senha encerra todas as sessões da rede. O próximo registro precisa usar a nova senha.

## DNS, firewall e relay

O cliente recebe o endereço do relay no registro. Com `SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081`, esse é o endereço devolvido. Se o relay usar outro host ou porta, ajuste a variável e a publicação UDP no Compose.

Exemplo com UFW:

```bash
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw allow 8081/udp
sudo ufw enable
```

Em nuvens com security group, libere as mesmas portas no painel do provedor. O NAT do provedor precisa preservar UDP; relays atrás de NAT com port forwarding também funcionam desde que o endereço público seja informado em `SHADOWVPN_RELAY_ADVERTISE`.

## Backup e atualização

O backup precisa incluir `networks.json` e o token:

```bash
docker compose exec server tar -C /var/lib/shadowvpn -czf - . \
  > shadowvpn-backup-$(date +%F).tar.gz
```

Guarde o backup em local cifrado. O arquivo contém hashes Argon2 das senhas, IPs virtuais persistidos e o token de administração.

Para atualizar:

```bash
cd ShadowVPN
git pull
cd deploy
docker compose up -d --build
docker compose logs -f server
```

O volume é preservado. Faça backup antes de mudanças de versão. Atualize o servidor primeiro: clientes 0.4.x continuam funcionando por fallback, mas clientes 0.5.x não conectam em servidor 0.4.x.

## Rate limiting e abuso

O servidor não aplica limite por IP. Em uma implantação pública, coloque a API atrás de um proxy ou CDN com limitação de requisições e monitore CPU, memória e tráfego UDP. O endpoint de criação exige token administrativo, mas o registro ainda executa Argon2 para validar a senha da rede.

Um atacante que alcance a API pode tentar adivinhar senhas de redes existentes. Use senhas longas, mantenha o token administrativo secreto e restrinja a API por firewall quando os clientes estiverem em redes conhecidas.

## Alternativa com systemd

Instale o binário em `/usr/local/bin/shadowvpn-server`, crie o usuário de serviço e os diretórios:

```bash
sudo useradd --system --home /var/lib/shadowvpn --shell /usr/sbin/nologin shadowvpn
sudo install -d -o shadowvpn -g shadowvpn -m 0700 /var/lib/shadowvpn
sudo install -m 0755 target/release/shadowvpn-server /usr/local/bin/shadowvpn-server
```

Copie `deploy/systemd/shadowvpn-server.service` para `/etc/systemd/system/` e ajuste:

- `SHADOWVPN_LISTEN=127.0.0.1:8080` se o Caddy estiver no mesmo host.
- `SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081`.
- `SHADOWVPN_MAX_NETWORKS` conforme a capacidade do servidor.

Ative o serviço:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now shadowvpn-server
sudo systemctl status shadowvpn-server
```

O token será criado em `/var/lib/shadowvpn/admin-token` no primeiro início:

```bash
sudo cat /var/lib/shadowvpn/admin-token
```

Para rotacionar:

```bash
sudo rm /var/lib/shadowvpn/admin-token
sudo systemctl restart shadowvpn-server
sudo cat /var/lib/shadowvpn/admin-token
```

Instale o Caddy e use o `deploy/Caddyfile` apontando para `127.0.0.1:8080`, ou adapte o bloco `reverse_proxy` para o endereço local. Depois de instalar um novo binário, reinicie o servidor com `systemctl restart shadowvpn-server`.

Para o cliente sempre conectado no Linux, instale `deploy/systemd/shadowvpn-client@.service` e crie `/etc/shadowvpn/minha-rede.env` a partir de `deploy/systemd/client.env.example`:

```bash
sudo useradd --system --home /var/lib/shadowvpn-client --shell /usr/sbin/nologin shadowvpn
sudo install -d -m 0700 /etc/shadowvpn
sudo install -m 0644 deploy/systemd/shadowvpn-client@.service /etc/systemd/system/
sudo install -m 0600 deploy/systemd/client.env.example /etc/shadowvpn/minha-rede.env
sudo systemctl daemon-reload
sudo systemctl enable --now shadowvpn-client@minha-rede
```

O serviço do cliente usa `CAP_NET_ADMIN`, uma identidade em `/var/lib/shadowvpn-client` e reinicia após falhas. A senha fica no arquivo de ambiente; proteja-o com permissão `0600`.

## Verificações de segurança

- Use HTTPS em qualquer rede não confiável; sem TLS, senha, token e provas de identidade trafegam em texto claro.
- Mantenha `SHADOWVPN_ADMIN_TOKEN` ou o arquivo do token fora do Git e de logs.
- Faça backup do volume e teste a restauração.
- Atualize servidor e clientes juntos quando o protocolo mudar.
- Monitore `/health`, os logs e o consumo de CPU/memória; registre alertas para falhas repetidas.
- O relay vê metadados de conexão e volume de tráfego, mas não o conteúdo WireGuard.
