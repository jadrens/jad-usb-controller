# usb-card

USB 4G 模块 / 物联卡管理：Rust 后端通过 AT 指令与 USB 模块通信，React 前端显示状态、收发短信、控制流量。

```
usb-card/
├── server/   Rust (axum) 后端：AT 引擎、USB/串口传输、短信 PDU 编解码、SQLite 存储、REST + WebSocket
└── web/      Vite + React + TypeScript + Tailwind 前端
```

## 功能

- **状态**：SIM 状态 / ICCID / IMSI / 号码 / 运营商，注册状态、制式、频段、小区，信号 (CSQ/RSRP/RSRQ/SINR) 及趋势，模块型号 / 固件 / IMEI
- **小区 / 频段**：当前服务小区（频段、EARFCN 换算的上下行频率、带宽、PCI、基站号/扇区、TAC、RSRP/RSRQ/SINR，按质量着色），邻区列表（同频/异频）、网络下发未测到的频点、模块允许的 LTE 频段，RSRP/SINR 趋势与小区切换记录（`AT+QENG`、`AT+QCFG="band"`，Quectel 模块）
- **短信**：PDU 模式收发，GSM-7 / UCS-2（中文）、长短信自动拆分与合并，`+CMTI` 实时推送，按会话展示。分别读取 SIM 卡（`SM`）与模块（`ME`）存储，每条短信标注来源以及是否仍存于模块；可设置新短信存入位置、读取后是否保留（关闭时入库即从模块删除）。删除仍在模块上的短信会同时删除模块中的副本
- **流量**：开关移动数据（Quectel ECM 模式用 `AT+QNETDEVCTL`，其它用 `AT+CGACT`），IP/APN 显示，流量计数 (`AT+QGDCNT`) 与清零。页面上开启数据后会记住（数据库 `data_wanted`），之后掉线（网络分离、模块重启、拨号断开等）由后端在轮询中自动重新拨号，失败按 20 秒起翻倍退避、最长 5 分钟；SIM 未就绪、射频未开或未注册时不重试。在页面上关闭数据即取消
- **控制**：射频模式（正常 / 飞行 / 最小功能），重启模块，PIN 解锁，APN 设置（含 PAP/CHAP），USSD 查询
- **查询**：本机号码（`AT+CNUM`，可手动写入 SIM 本机号码簿），USSD 常用代码 + 自定义代码（`*#06#` 本地应答），三大运营商短信查询指令
- **eSIM**：识别 eUICC，显示 EID / SM-DS / 剩余空间，列出 Profile，启用 / 停用 / 删除 / 重命名（SGP.22 ES10c）。依次尝试 GSMA 及厂商 ISD-R AID（eSTK.me SE0/SE1、eSIM.me、5ber、Xesim、LinksField），一张卡上有多个 eUICC 时按 EID 去重后分页显示，并标出模块当前使用的那一个。暂不支持下载新 Profile
- **AT 终端**：任意 AT 命令 + 串口实时日志（TX/RX/URC）

## 与模块通信的方式

| `--transport` | 说明 |
|---|---|
| `auto`（默认） | 先尝试直连 USB，再尝试串口 |
| `usb` | 用 [nusb](https://crates.io/crates/nusb) 直接读写模块 AT 接口的 bulk 端点，**不需要 `option` 内核驱动 / `/dev/ttyUSB*`**。自动识别常见厂商（Quectel、SIMCom、Fibocom、移远、广和通、美格…），按接口名称（`Mobile AT Interface`）挑选 AT 口并发 `AT` 探测 |
| `serial` | 使用 `/dev/ttyUSB*` / `/dev/ttyACM*`，可用 `--serial-port` 指定 |
| `mock` | 内置模拟模块，用于开发前端 / 无硬件演示（向 10086 发短信会自动回复） |

### 权限

直连 USB 需要对 `/dev/bus/usb/...` 的写权限。桌面环境下可通过 udev `uaccess` 规则为当前登录用户加 ACL；无头服务器 / systemd 服务则把运行用户加入 `uucp` 组（Debian/Ubuntu 改成 `dialout`），或直接以 root 运行服务。

如果系统里运行着 ModemManager，它会抢占 AT 口，需要设置 `ID_MM_DEVICE_IGNORE`，或直接 `systemctl stop ModemManager`。

## 运行

### 预编译版本

[Releases](https://github.com/jadrens/jad-usb-controller/releases) 提供单文件可执行程序（已内嵌前端与 SQLite，无需安装依赖）：

| 文件 | 平台 |
|---|---|
| `*-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64（静态链接，任意发行版） |
| `*-aarch64-unknown-linux-musl.tar.gz` | Linux ARM64（树莓派、ARM 路由器等） |
| `*-x86_64-pc-windows-msvc.zip` | Windows x86_64 |

```sh
./usb-card-server --listen 0.0.0.0:8080
```

Windows 上请安装模块厂商的 USB 驱动，使 AT 口显示为 COM 口，然后用 `--transport serial`（可加 `--serial-port COM5`）。直连 USB 模式在 Windows 上需要用 Zadig 把 AT 接口换成 WinUSB 驱动，一般不推荐。

### 从源码构建

```sh
# 前端
cd web && pnpm install && pnpm build      # 输出 web/dist，被后端编译时嵌入

# 后端
cd server && cargo run --release -- --listen 0.0.0.0:8080
# 浏览器打开 http://<host>:8080
```

开发模式：

```sh
cd server && cargo run -- --transport mock          # :8080
cd web && pnpm dev                                    # :5173，/api 代理到 :8080（API_TARGET 可改）
```

> debug 构建在运行时从 `web/dist` 读取静态文件；release 构建会把 `web/dist` 嵌入二进制，所以先 `pnpm build` 再 `cargo build --release`。

> 程序本身没有登录认证，可以发短信、删除 eSIM Profile，不要把服务端口直接映射到公网。

### 模块兼容性备注

- **EC200T（固件 EC200TCNDAR02A15M16）执行 `AT+CCHO`（打开逻辑通道）会直接重启**。eUICC 访问因此改用 `AT+CSIM` 发送 MANAGE CHANNEL / SELECT / STORE DATA APDU；该模块的 GET RESPONSE 必须用 `Le=00`。
- AT 引擎开启回显（`ATE1`），以命令回显界定响应，超时或残留的 `OK` 不会让后续响应错位。
- ECM 模式下 Quectel 模块的移动数据开关使用 `AT+QNETDEVCTL=<op>,<cid>,<urc>`；概览页分别显示 PDP 状态与网卡拨号状态。模块网卡会被 NetworkManager 设为默认路由，若不想让电脑流量走 SIM 卡，调高其 metric 或设 `ipv4.never-default yes`。

### 参数（均可用环境变量设置）

| 参数 | 环境变量 | 默认 |
|---|---|---|
| `--listen` | `USB_CARD_LISTEN` | `0.0.0.0:8080` |
| `--transport` | `USB_CARD_TRANSPORT` | `auto` |
| `--serial-port` / `--baud` | `USB_CARD_SERIAL_PORT` / `USB_CARD_BAUD` | 自动 / 115200 |
| `--usb-vid` / `--usb-pid`（十六进制） | `USB_CARD_USB_VID` / `USB_CARD_USB_PID` | 已知厂商 |
| `--usb-interface` | `USB_CARD_USB_INTERFACE` | 自动探测 |
| `--db` | `USB_CARD_DB` | `usb-card.db` |
| `--poll-secs` | `USB_CARD_POLL_SECS` | `10` |
| `--keep-sms-on-sim` | `USB_CARD_KEEP_SMS_ON_SIM` | 否（入库后从模块删除；页面设置优先，保存在数据库中） |
| `--data-cid` | `USB_CARD_DATA_CID` | `1` |

日志级别：`RUST_LOG=debug`，`RUST_LOG=usb_card_server::at=trace` 打印每一行 AT 收发。

## API

所有接口在 `/api` 下，JSON。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/status` | 连接状态 + 模块状态 + 未读数 + 配置 |
| POST | `/status/refresh` | 立即刷新状态 |
| GET | `/devices` | 扫描到的 USB 模块与串口 |
| GET | `/sms?peer=&before=&limit=` | 短信列表 |
| GET | `/sms/conversations` | 会话列表 |
| POST | `/sms` `{to, text}` | 发送短信 |
| POST | `/sms/sync` | 从模块拉取短信 |
| GET / PUT | `/sms/settings` | 存储用量；`{keepOnModem?, receiveStorage?: "SM"\|"ME"}` |
| DELETE | `/sms/{id}` · `/sms/peer/{peer}` | 删除短信 / 会话 |
| POST | `/sms/peer/{peer}/read` | 标记已读 |
| POST | `/data` `{enabled}` | 开关移动数据 |
| POST | `/data/counters/reset` | 清零流量计数 |
| POST | `/radio` `{mode: on\|airplane\|off}` | 射频模式 |
| POST | `/modem/reboot` | 重启模块 |
| POST | `/sim/pin` `{pin}` | 输入 PIN |
| POST | `/sim/msisdn` `{number}` | 写入本机号码（空字符串清除） |
| GET | `/cell` | 服务小区、邻区、允许频段 |
| GET | `/euicc` | 检测 eUICC 并读取 EID / Profile |
| POST | `/euicc/{aid}/profiles/{iccid}/{enable\|disable\|delete}` | Profile 操作（`aid` 为该 eUICC 的 ISD-R AID） |
| PUT | `/euicc/{aid}/profiles/{iccid}/nickname` `{nickname}` | 重命名 Profile |
| GET / PUT | `/apn` | 读取 / 设置 PDP 上下文 `{cid, pdpType, apn, username?, password?, auth?}` |
| POST | `/ussd` `{code}` · `/ussd/cancel` | USSD |
| POST | `/at` `{command, timeoutMs?}` | 执行任意 AT 命令 |
| GET | `/ws` | WebSocket 事件：`status` / `connection` / `sms` / `sms_sync` / `at_log` |

## 测试

```sh
cd server && cargo test     # PDU 编解码、AT 引擎、短信拼接
cd web && pnpm lint && pnpm build
```
