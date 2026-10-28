# Como conectar no ShadowVPN

Este guia cobre o fluxo da versão 0.5.x: primeiro alguém **cria a rede** no servidor, gera um **convite** e os outros computadores entram com o link e a senha. A rede fica salva mesmo quando todos desconectam ou o servidor reinicia.

## O que você precisa

- Um servidor com o `shadowvpn-server` em execução e as portas TCP `8080` e UDP `8081` liberadas.
- A mesma versão do ShadowVPN em todos os computadores.
- Convite `shadowvpn://` e senha da rede compartilhados por canais seguros. O convite não inclui a senha.
- Privilégios de administrador nos clientes para criar a interface TUN:
  - Windows: executar como Administrador, com `wintun.dll` ao lado do executável.
  - Linux: `root`, `sudo` ou `CAP_NET_ADMIN` no binário.
  - macOS: `sudo`.

Criar a rede **não** exige privilégios especiais. Entrar/conectar exige.

## Visão rápida

| Etapa | Quem faz | Ação |
|---|---|---|
| 1 | Administrador do servidor | Inicia `shadowvpn-server` ou liga o **Servidor local** no painel da GUI |
| 2 | Uma pessoa | Cria a rede com nome e senha e copia o convite |
| 3 | Todos | Colam o convite, informam a senha e entram |
| 4 | Todos | Testam o ping pelo IP virtual |

## 1. Iniciar o servidor

### Pela GUI (servidor local)

No painel esquerdo, localize o cartão **Servidor local**:

- **Vermelho / Desligado:** clique no botão de energia para iniciar.
- **Azul / Ligado:** clique no botão para encerrar.
- **Iniciando...:** o servidor foi executado e a GUI aguarda o `/health`.

Ao ligar, a GUI inicia o `shadowvpn-server` que acompanha o pacote e ajusta o campo do servidor para o IP da máquina na rede local, desde que ele estivesse em `127.0.0.1`/`localhost`. Assim o convite gerado pode ser aberto em outro computador.

O servidor iniciado pela GUI escuta `0.0.0.0:8080` (TCP) e `0.0.0.0:8081` (UDP), grava as redes na pasta de dados do usuário e mantém o log em `server.log`:

- Linux: `$XDG_DATA_HOME/shadowvpn/` ou `~/.local/share/shadowvpn/`
- macOS: `~/Library/Application Support/shadowvpn/`
- Windows: `%APPDATA%\ShadowVPN\`

Se o cartão mostrar **Ligado por fora**, já existe um servidor nessa porta iniciado fora da GUI; encerre-o pelo terminal para controlá-lo pelo botão. O servidor iniciado pela GUI para quando a janela é fechada.

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

Para exigir token na criação e administração de redes, use um arquivo de token (criado com permissão `0600` se não existir):

```bash
SHADOWVPN_ADMIN_TOKEN_FILE=/var/lib/shadowvpn/admin-token ./shadowvpn-server
```

Sem token, qualquer pessoa que alcance a API pode criar redes. Use esse modo apenas em laboratório ou rede confiável.

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

Para exigir token na criação e administração de redes:

```powershell
$env:SHADOWVPN_ADMIN_TOKEN_FILE = "$env:ProgramData\ShadowVPN\admin-token"
.\shadowvpn-server.exe
Get-Content $env:SHADOWVPN_ADMIN_TOKEN_FILE
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

O arquivo `data/networks.json` contém os nomes das redes e apenas os hashes Argon2 das senhas. O arquivo `admin-token`, quando configurado, contém o token de administração. Faça backup dos dois em local seguro. Se o arquivo de redes for apagado, as redes somem no próximo reinício e será necessário criá-las novamente.

No Linux, os arquivos são criados com permissão `0600`. Mantenha o servidor parado ao editar ou copiar o arquivo de redes manualmente.

## 2. Criar a rede

A rede precisa existir antes do primeiro `register`. Escolha uma das opções abaixo.

### Pela GUI

1. Abra o `shadowvpn-gui` como Administrador no Windows ou com privilégios no Linux/macOS.
2. Na aba **Criar rede**, informe o nome, a senha e, se o servidor exigir, o token de administração.
3. Confirme a senha e clique em **Criar rede**. O servidor fica em **Servidor avançado** e só precisa ser informado ao auto-hospedar. No servidor local iniciado pela GUI, o token é usado automaticamente.
4. Quando aparecer a confirmação, use o cartão do convite e clique em copiar.
5. Clique em **Entrar na rede** e envie o convite e a senha por canais separados.

A GUI salva automaticamente o convite da rede criada. Para recuperá-lo depois, abra a aba **Minhas redes**:

- **Abrir** carrega a rede no modo de entrada e mostra o convite no cartão.
- **Copiar** copia o link diretamente.
- **Remover** tira a rede apenas da lista local; ela continua existindo no servidor.

A senha não é salva. O arquivo `created-networks.txt` fica na pasta de dados do usuário:

- Linux: `$XDG_DATA_HOME/shadowvpn/created-networks.txt` ou `~/.local/share/shadowvpn/created-networks.txt`
- macOS: `~/Library/Application Support/shadowvpn/created-networks.txt`
- Windows: `%APPDATA%\ShadowVPN\created-networks.txt`

Ele contém um convite por linha e é separado do `networks.json` usado pelo servidor. Na primeira execução, se `created-networks.txt` ainda não existir, a GUI importa os nomes de redes já presentes no `networks.json` local. Com a lista vazia, o botão **Importar redes do servidor local** repete a importação.

### Pela CLI

No primeiro computador:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
./shadowvpn-client create-network \
  --name minha-rede --password 'uma-senha-forte'
```

O subcomando `create-network` cria a rede sem conectar e imprime o convite. Se a rede já existir, a criação falha com `409 Conflict`; nesse caso use o fluxo de entrada.

O modo antigo também funciona:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_GROUP=minha-rede \
SHADOWVPN_PASSWORD='uma-senha-forte' \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
./shadowvpn-client --create
```

`--create` cria a rede e em seguida conecta este computador. Se a criação funcionar mas o TUN falhar por falta de privilégio, a rede continua salva; corrija a permissão e entre sem `--create`.

Se o servidor usar `SHADOWVPN_ADMIN_TOKEN_FILE`, passe `--admin-token-file /caminho/do/token` na CLI. A GUI aceita o token no campo **Token de administração**.

### Convite

O convite tem este formato:

```text
shadowvpn://join?server=https%3A%2F%2Fvpn.exemplo.com&network=minha-rede
```

- `server` é a URL da API de coordenação.
- `network` é o nome da rede.
- A senha **não** é incluída. Envie-a por outro canal.
- O link não cria um serviço central: alguém continua responsável por hospedar o servidor.

Na GUI, o convite aparece após a criação, pode ser copiado na tela da rede e fica salvo na aba **Minhas redes**. Na CLI, ele é impresso por `create-network` ou pelo `--create`. A API `POST /networks` não retorna o convite; monte o link com a URL do servidor e o nome da rede.

### Pela API HTTP

```bash
curl -X POST https://vpn.exemplo.com/networks \
  -H "Authorization: Bearer COLE_O_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"group_name":"minha-rede","group_password":"uma-senha-forte"}'
```

Resposta esperada, HTTP 201:

```json
{"group_name":"minha-rede"}
```

Erros comuns:

- `400 Bad Request`: nome ou senha fora das regras.
- `401 Unauthorized`: token ausente ou inválido.
- `409 Conflict`: já existe uma rede com esse nome.
- `429 Too Many Requests`: o servidor atingiu `SHADOWVPN_MAX_NETWORKS`.
- `500 Internal Server Error`: falha ao gravar o arquivo de redes; verifique permissões e `SHADOWVPN_NETWORKS_FILE`.
- `503 Service Unavailable`: administração não configurada no servidor; defina o token no servidor.
- `403 Forbidden`: o membro foi revogado pelo administrador; use `restore-member` ou peça para o administrador liberar a chave.

### Regras de nome e senha

- Nome: até 64 caracteres, letras, números, `-` ou `_`.
- Senha: entre 8 e 128 caracteres.
- Não reutilize a senha de outros serviços.
- A senha pode ser trocada com privilégio administrativo. A troca encerra todas as sessões da rede e os clientes precisam entrar novamente com a nova senha.

### Administrar redes e membros

Na GUI, clique em **Administrar servidor**. Informe a URL e o token administrativo; no servidor local o token é lido automaticamente. Clique em **Atualizar**, escolha uma rede em **Gerenciar** e use o painel para:

- recarregar a lista de membros online, offline e revogados;
- **Revogar** uma chave pública (pede confirmação) ou **Liberar** uma chave revogada;
- trocar a senha da rede, com confirmação, encerrando todas as sessões;
- excluir a rede, com confirmação.

Pela CLI, use o mesmo token administrativo:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
./shadowvpn-client list-members minha-rede
```

A saída mostra o estado, o IP virtual, o hostname, o fingerprint e a chave completa de cada membro. Para revogar ou liberar:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
./shadowvpn-client revoke-member minha-rede 'CHAVE_PUBLICA_BASE64'

SHADOWVPN_SERVER=https://vpn.exemplo.com \
SHADOWVPN_ADMIN_TOKEN='COLE_O_TOKEN' \
./shadowvpn-client restore-member minha-rede 'CHAVE_PUBLICA_BASE64'
```

Revogar encerra a sessão ativa, libera o IP e bloqueia novas entradas daquela chave. O bloqueio é persistente e continua após reiniciar o servidor. A chave só volta a entrar depois de `restore-member`; ela recebe um IP livre, que pode ser diferente do anterior. Para cortar o acesso de todos, troque a senha da rede com `rotate-password` ou pelo painel da GUI.

Revogar uma chave não impede alguém que ainda tenha a senha de gerar uma nova identidade. Use senhas fortes e troque a senha quando precisar remover o acesso de uma pessoa, não apenas de um computador.

## 3. Entrar na rede

Em cada outro computador:

### Pela GUI

1. Abra a GUI como Administrador/root ou com `CAP_NET_ADMIN`.
2. Na aba **Entrar em rede**, cole o convite e clique em **Usar**.
3. Informe a senha da rede.
4. Clique em **Entrar na rede**.
5. Confira o IP virtual, o relay, o fingerprint e os peers na tela de rede.

A identidade XEdDSA é derivada da chave WireGuard, criada automaticamente e salva no computador. Não copie a chave entre máquinas. O fingerprint é um hash SHA-256; compare-o fora de banda se quiser confirmar a identidade de um peer.

### Pela CLI

```bash
SHADOWVPN_PASSWORD='uma-senha-forte' \
./shadowvpn-client \
  --invite 'shadowvpn://join?server=https%3A%2F%2Fvpn.exemplo.com&network=minha-rede'
```

Também é possível informar `--server` e `--group` manualmente:

```bash
SHADOWVPN_SERVER=https://vpn.exemplo.com \
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

A API usa HTTP simples por padrão. A senha da rede só é enviada no registro e o heartbeat usa um token de sessão, mas em redes não confiáveis coloque a API atrás de HTTPS. O guia completo com Docker Compose e Caddy está em [DEPLOY.md](DEPLOY.md).

Mantenha `SHADOWVPN_LISTEN=127.0.0.1:8080` e configure `SHADOWVPN_RELAY_ADVERTISE=vpn.exemplo.com:8081`. Proteja também o token administrativo: sem ele, qualquer pessoa que alcance `POST /networks` pode criar uma rede.

O relay UDP transporta somente datagramas WireGuard cifrados. O servidor vê metadados (IPs públicos, sessões e volume), mas não o conteúdo dos pacotes. O registro inclui uma prova XEdDSA de posse da chave WireGuard; servidor e peers verificam essa prova antes de criar os túneis. O PSK pairwise é derivado pelas duas pontas e não é conhecido pelo servidor.

Quando `SHADOWVPN_ADMIN_TOKEN` ou `SHADOWVPN_ADMIN_TOKEN_FILE` está configurado, criar, listar, excluir e trocar a senha de redes exige Bearer token. Sem token, a criação fica aberta e o servidor registra um aviso; restrinja a API em ambientes públicos e não há rate limiting por IP.

## Problemas comuns

### "convite inválido"

- Confirme que o link começa com `shadowvpn://`.
- O convite precisa incluir `server` e `network`.
- Se você editou o link, codifique a URL do servidor corretamente, por exemplo `https%3A%2F%2Fvpn.exemplo.com`.
- O link não contém a senha; informe-a no campo separado.

### "rede não encontrada; crie a rede primeiro"

A rede ainda não existe no servidor. Crie-a na GUI, com `create-network`/`--create` ou com `POST /networks` e tente entrar novamente.

### "já existe uma rede com esse nome"

A rede já foi criada. Não use `create-network` nem `--create`; entre normalmente com o convite e a senha combinados. Se a senha estiver errada, peça ao administrador para usar `rotate-password` ou a rota administrativa correspondente.

### A aba "Minhas redes" está vazia

- Ela mostra apenas redes criadas pela GUI ou importadas do `networks.json` do servidor local.
- Entrar em uma rede com convite não adiciona a rede à lista; o convite recebido continua no link original.
- Use **Importar redes do servidor local** para trazer redes que já existem no servidor desta máquina.
- Confirme que `created-networks.txt` existe na pasta de dados do usuário e que o arquivo não está vazio.
- A senha não é salva na lista; ela sempre precisa ser informada ao entrar.

### "Connection refused" ou timeout no servidor

- Se o servidor é nesta máquina, ligue o **Servidor local** no painel: vermelho significa desligado e azul significa ligado.
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

As redes, os membros e os IPs virtuais são persistentes, mas as sessões ativas não. O cliente detecta a queda no heartbeat e reconecta com backoff; o IP virtual é mantido. Se a rede tiver sido apagada, a senha alterada ou a chave revogada pelo administrador, o cliente mostra o erro e será necessário entrar novamente (ou pedir a liberação da chave).

### O servidor não consegue salvar as redes

O caminho padrão é relativo à pasta atual. Execute o servidor em uma pasta gravável ou defina:

```bash
SHADOWVPN_NETWORKS_FILE=/caminho/gravavel/networks.json ./shadowvpn-server
```

### O fingerprint do peer mudou

A chave de identidade daquele computador foi recriada. Confirme com o dono do peer por um canal separado antes de continuar. Se a mudança foi legítima, reconecte; se não, trate como possível interferência do coordenador ou de um membro.

### Wine

O instalador e a GUI do Windows abrem no Wine, mas a conexão real não funciona: o Wintun exige um driver de kernel do Windows e falha no Wine. Para testar a VPN, use Windows nativo ou uma VM Windows.

## Atualizando de 0.5.0 para 0.5.1

Atualize o servidor primeiro. O 0.5.1 adiciona listagem e revogação persistente de membros e o painel **Administrar servidor** na GUI. Clientes 0.5.0 continuam funcionando; as rotas de membros só existem no servidor 0.5.1.

O `networks.json` v2 é lido automaticamente e passa a v3 na próxima gravação. Faça backup antes de atualizar. O servidor 0.5.0 não lê a versão 3; para reverter, restaure o backup do arquivo. O token administrativo e as senhas das redes não mudam.

## Atualizando de 0.4.x para 0.5.x

Atualize primeiro o servidor e depois os clientes. O servidor 0.5.x ainda aceita clientes 0.4.x pelo fallback de senha, mas o cliente 0.5.x exige servidor 0.5.x para usar o heartbeat por token.

A identidade e as redes existentes continuam válidas. O `networks.json` v1/v2 é lido automaticamente e passa a v3 na próxima gravação, preservando os IPs virtuais. Configure `SHADOWVPN_ADMIN_TOKEN` ou `SHADOWVPN_ADMIN_TOKEN_FILE` e use HTTPS conforme [DEPLOY.md](DEPLOY.md).

## Atualizando de 0.3.x para 0.4.x

Atualize servidor, clientes e GUI juntos. O protocolo de registro agora exige prova XEdDSA e os peers usam um PSK pairwise; clientes 0.3.x não são compatíveis com servidores 0.4.x e vice-versa.

A chave `identity.key` existente continua válida e não é preciso recriar redes nem senhas. O fingerprint passa a ser exibido na GUI.

## Atualizando de 0.2.x para 0.3.x

A versão 0.3.0 introduz a criação explícita de redes. Na primeira execução do servidor 0.3.0 não existe arquivo de redes, então:

1. Atualize servidor, clientes e GUI para 0.3.0.
2. Crie cada rede existente uma vez com `create-network`, `--create`, pela GUI ou por `POST /networks`.
3. Use o mesmo nome e a mesma senha que os clientes já conhecem.
4. Entre normalmente nos clientes.

Clientes 0.2.x não são compatíveis com o servidor 0.3.x, porque o servidor não cria mais redes automaticamente no registro.
