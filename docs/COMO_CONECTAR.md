# Como conectar no ShadowVPN

Este guia cobre o fluxo da versão 0.3.0 em diante: primeiro alguém **cria a rede** no servidor, depois os outros computadores **entram** nela. A rede fica salva mesmo quando todos desconectam ou o servidor reinicia.

## O que você precisa

- Um servidor com o `shadowvpn-server` em execução e as portas TCP `8080` e UDP `8081` liberadas.
- A mesma versão do ShadowVPN em todos os computadores.
- Nome e senha da rede compartilhados por um canal seguro.
- Privilégios de administrador nos clientes para criar a interface TUN:
  - Windows: executar como Administrador, com `wintun.dll` ao lado do executável.
  - Linux: `root`, `sudo` ou `CAP_NET_ADMIN` no binário.
  - macOS: `sudo`.

Criar a rede **não** exige privilégios especiais. Entrar/conectar exige.

## Visão rápida

| Etapa | Quem faz | Ação |
|---|---|---|
| 1 | Administrador do servidor | Inicia `shadowvpn-server` |
| 2 | Uma pessoa | Cria a rede com nome e senha |
| 3 | Todos | Entram na mesma rede |
| 4 | Todos | Testam o ping pelo IP virtual |

## 1. Iniciar o servidor

### Linux

Extraia o pacote e execute:

```bash
./shadowvpn-server
```

Por padrão:

- API HTTP em `0.0.0.0:8080`
- Relay UDP em `0.0.0.0:8081`
- Redes salvas em `./data/networks.json`

Para usar outro endereço público de relay:

```bash
SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081 ./shadowvpn-server
```

Para escolher outro arquivo de redes:

```bash
SHADOWVPN_NETWORKS_FILE=/var/lib/shadowvpn/networks.json ./shadowvpn-server
```

### Windows

1. Extraia o ZIP ou use o instalador.
2. Abra um PowerShell ou Prompt de Comando na pasta do `shadowvpn-server.exe`.
3. Execute:

```powershell
.\shadowvpn-server.exe
```

Se a pasta atual não for gravável, defina um arquivo em local gravável:

```powershell
$env:SHADOWVPN_NETWORKS_FILE = "$env:ProgramData\ShadowVPN\networks.json"
.\shadowvpn-server.exe
```

Permita as portas `8080/tcp` e `8081/udp` no firewall do Windows e no provedor de nuvem.

### Conferir se subiu

No próprio servidor:

```bash
curl http://127.0.0.1:8080/health
```

Resposta esperada:

```json
{"status":"ok"}
```

### Firewall e NAT

- `8080/tcp` é a API de coordenação. Pode ficar atrás de HTTPS/proxy reverso.
- `8081/udp` é o relay. Precisa ser acessível diretamente pelos clientes.
- Se o relay estiver atrás de NAT, publique a porta UDP e defina `SHADOWVPN_RELAY_ADVERTISE` com o endereço público.
- Sem `SHADOWVPN_RELAY_ADVERTISE`, o servidor tenta derivar o endereço do cabeçalho HTTP `Host`. Atrás de proxy reverso, defina explicitamente.

### Backup das redes

O arquivo `data/networks.json` contém os nomes das redes e apenas os hashes Argon2 das senhas. Faça backup dele. Se o arquivo for apagado, as redes somem no próximo reinício e será necessário criá-las novamente.

No Linux, o arquivo é criado com permissão `0600`. Mantenha o servidor parado ao editar ou copiar o arquivo manualmente.

## 2. Criar a rede

A rede precisa existir antes do primeiro `register`. Escolha uma das opções abaixo.

### Pela GUI

1. Abra o `shadowvpn-gui` como Administrador no Windows ou com privilégios no Linux/macOS.
2. Preencha o servidor, por exemplo `http://vpn.exemplo.com:8080`.
3. Na aba **Criar rede**, informe o nome e a senha.
4. Confirme a senha e clique em **Criar rede**.
5. Quando aparecer a confirmação, clique em **Entrar na rede**.

A GUI mantém a rede criada durante a sessão e já muda para o modo de entrada.

### Pela CLI

No primeiro computador:

```bash
SHADOWVPN_SERVER=http://vpn.exemplo.com:8080 \
SHADOWVPN_GROUP=minha-rede \
SHADOWVPN_PASSWORD='uma-senha-forte' \
./shadowvpn-client --create
```

`--create` cria a rede e em seguida conecta este computador. Se a rede já existir, a criação falha com `409 Conflict`; nesse caso use o fluxo de entrada, sem `--create`. Se a rede for criada mas a interface TUN falhar por falta de privilégio, a rede continua salva no servidor; corrija a permissão e entre sem `--create`.

### Pela API HTTP

```bash
curl -X POST http://vpn.exemplo.com:8080/networks \
  -H 'Content-Type: application/json' \
  -d '{"group_name":"minha-rede","group_password":"uma-senha-forte"}'
```

Resposta esperada, HTTP 201:

```json
{"group_name":"minha-rede"}
```

Erros comuns:

- `400 Bad Request`: nome ou senha fora das regras.
- `409 Conflict`: já existe uma rede com esse nome.
- `500 Internal Server Error`: falha ao gravar o arquivo de redes; verifique permissões e `SHADOWVPN_NETWORKS_FILE`.

### Regras de nome e senha

- Nome: até 64 caracteres, letras, números, `-` ou `_`.
- Senha: entre 8 e 128 caracteres.
- Não reutilize a senha de outros serviços.
- A API não permite alterar a senha de uma rede existente. Para trocar a senha, pare o servidor, remova a rede do JSON e crie novamente.

## 3. Entrar na rede

Em cada outro computador:

### Pela GUI

1. Abra a GUI como Administrador/root ou com `CAP_NET_ADMIN`.
2. Informe o mesmo servidor.
3. Na aba **Entrar em rede**, informe nome e senha.
4. Clique em **Entrar na rede**.
5. Confira o IP virtual, o relay e os peers na tela de rede.

A identidade WireGuard é criada automaticamente e fica salva no computador. Não copie a chave entre máquinas.

### Pela CLI

```bash
SHADOWVPN_SERVER=http://vpn.exemplo.com:8080 \
SHADOWVPN_GROUP=minha-rede \
SHADOWVPN_PASSWORD='uma-senha-forte' \
./shadowvpn-client
```

Use `SHADOWVPN_LOG=debug` para ver mais detalhes. Para sair, pressione `Ctrl+C`; o peer é removido e a rede continua cadastrada.

## 4. Testar a conexão

1. Confira na GUI se os outros computadores aparecem na lista de peers.
2. Anote o IP virtual de cada um, no formato `26.26.26.x`.
3. Teste:

```bash
ping 26.26.26.3
```

No Windows, se o ping falhar, permita ICMPv4 no firewall ou teste outro serviço TCP entre os peers.

O túnel cobre apenas `26.26.26.0/24`. Ele não roteia todo o tráfego da internet.

## Permissões por sistema

### Windows

- O instalador já inclui `wintun.dll` e solicita elevação ao abrir a GUI.
- No ZIP portátil, mantenha `wintun.dll` e `wintun-LICENSE.txt` na mesma pasta do executável.
- Execute o cliente/GUI como Administrador.
- O servidor não precisa de Administrador, mas precisa de acesso às portas e ao arquivo de redes.

### Linux

Opção recomendada, sem executar como root:

```bash
sudo setcap cap_net_admin+ep ./target/release/shadowvpn-client
sudo setcap cap_net_admin+ep ./target/release/shadowvpn-gui
```

Depois execute normalmente. Confirme que `/dev/net/tun` existe:

```bash
ls -l /dev/net/tun
```

O AppImage não carrega capabilities. Use `sudo -E` ou extraia e aplique `setcap` ao binário interno.

### macOS

```bash
sudo ./shadowvpn-client
```

Ou execute a GUI com `sudo`. Os binários não são assinados nem notarizados; se o Gatekeeper bloquear, remova o atributo de quarentena conforme o README.

## HTTPS e segurança

A API usa HTTP simples por padrão. A senha da rede é enviada nas requisições; portanto, em redes não confiáveis, coloque a API atrás de HTTPS:

```caddyfile
vpn.exemplo.com {
    reverse_proxy 127.0.0.1:8080
}
```

Mantenha `SHADOWVPN_LISTEN=127.0.0.1:8080` e configure `SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081`.

O relay UDP transporta somente datagramas WireGuard cifrados. O servidor vê metadados (IPs públicos, sessões e volume), mas não o conteúdo dos pacotes.

Qualquer pessoa que alcance `POST /networks` pode criar uma rede nova. Use nomes difíceis de adivinhar e restrinja o acesso à API em ambientes públicos.

## Problemas comuns

### "rede não encontrada; crie a rede primeiro"

A rede ainda não existe no servidor. Crie-a na GUI/CLI com `--create`/`POST /networks` e tente entrar novamente.

### "já existe uma rede com esse nome"

A rede já foi criada. Não use `--create`; entre normalmente com o nome e a senha combinados. Se a senha estiver errada, a API não permite redefini-la: recrie a rede com o servidor parado.

### "Connection refused" ou timeout no servidor

- Confirme que o servidor está rodando.
- Teste `curl http://IP:8080/health`.
- Verifique firewall, NAT e se a porta TCP `8080` está publicada.
- Se usar HTTPS, confirme o esquema `https://` no cliente.

### A GUI/cliente falha ao criar o TUN

- Linux: `Operation not permitted`. Use `sudo` ou `setcap cap_net_admin+ep`.
- Windows: `WintunCreateAdapter failed`. Confirme `wintun.dll`, execute como Administrador e reinicie o computador se o driver estiver em uso.
- macOS: execute com `sudo`.

### Os peers aparecem, mas o ping não funciona

- Confirme o mesmo nome de rede e a mesma senha em todos.
- Verifique se a porta UDP `8081` está liberada no servidor e no provedor de nuvem.
- O firewall local pode bloquear ICMP. Teste uma porta TCP ou permita ICMPv4.
- O MTU é 1380; redes com MTU menor podem precisar de ajustes.
- Confirme que o relay retornado não é `127.0.0.1` em um cliente remoto.

### O relay aponta para 127.0.0.1 ou para o host errado

Defina `SHADOWVPN_RELAY_ADVERTISE` com o endereço público, por exemplo:

```bash
SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081 ./shadowvpn-server
```

### Reiniciei o servidor e a conexão caiu

As redes são persistentes, mas as sessões não. O cliente tenta renovar o registro no próximo heartbeat; se o IP virtual mudar, reconecte. Se a rede tiver sido apagada do arquivo, crie-a novamente.

### O servidor não consegue salvar as redes

O caminho padrão é relativo à pasta atual. Execute o servidor em uma pasta gravável ou defina:

```bash
SHADOWVPN_NETWORKS_FILE=/caminho/gravavel/networks.json ./shadowvpn-server
```

### Wine

O instalador e a GUI do Windows abrem no Wine, mas a conexão real não funciona: o Wintun exige um driver de kernel do Windows e falha no Wine. Para testar a VPN, use Windows nativo ou uma VM Windows.

## Atualizando de 0.2.x para 0.3.x

A versão 0.3.0 introduz a criação explícita de redes. Na primeira execução do servidor 0.3.0 não existe arquivo de redes, então:

1. Atualize servidor, clientes e GUI para 0.3.0.
2. Crie cada rede existente uma vez com `--create`, pela GUI ou por `POST /networks`.
3. Use o mesmo nome e a mesma senha que os clientes já conhecem.
4. Entre normalmente nos clientes.

Clientes 0.2.x não são compatíveis com o servidor 0.3.x, porque o servidor não cria mais redes automaticamente no registro.
