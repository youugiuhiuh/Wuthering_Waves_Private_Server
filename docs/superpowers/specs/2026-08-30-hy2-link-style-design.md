# 设计:hy2 端口跳跃分享链接按客户端格式区分

日期:2026-08-30
状态:待用户 review

## 1. 背景与问题

Hysteria2 分享链接存在两种互不兼容的端口跳跃写法:

1. **官方 URI Scheme**(v2.hysteria.network/docs/developers/URI-Scheme/):端口位置 multi-port,如 `hysteria2://pwd@host:8443,8444-8543?...`,并支持 `hop_interval` 参数。sing-box 系客户端(含 SFA/SFI/Karing/Clash.Meta)按此解析。
2. **v2rayN 系**(v2rayN/v2rayNG/NekoBox):不解析端口位置 multi-port(用 .NET `Uri.Port`,吃不下逗号/范围),改用非官方 `mport` 查询参数,如 `hysteria2://pwd@host:8443?...&mport=8444-8543`。端口跳跃区间在 v2rayN 内部映射为 sing-box `server_ports`,`hop_interval` 用默认 30s。

当前项目(`rust/aegis/src/core/singbox/hysteria2.rs`)的 `to_client_link_with_hopping*` 只生成官方格式。**非跳跃节点两系格式一致,无兼容问题;只有开端口跳跃时,官方格式链接无法直接导入 v2rayN。**

## 2. 决策记录

| # | 决策 | 理由 |
|---|---|---|
| D1 | 采用方案 B:创建时让用户选客户端系 | bot 以纯文本发链接,无客户端信息,需显式选择 |
| D2 | B2:只在开启端口跳跃时区分 | 非跳跃节点两系格式一致,区分无意义 |
| D3 | B2':确认页三按钮(无跳跃 / 跳跃·sing-box / 跳跃·V2rayN),不弹额外层 | 一次点击到位,改动最小,符合"少步骤"意图 |
| D4 | 两种风格链接都保留 `pinSHA256`、都不带 `insecure` | 见 §5:Xray 核心实测 pin 有效;`insecure=0` 无法阻止 v2rayN auto-insecure;去掉 pin 则 V2rayN 用户连不上 |
| D5 | 边界:一键部署(ops.rs:772,固定 `batch_create_hysteria2(3, ip, None, false)`,无跳跃)零改动;服务端 inbound 配置(`to_inbound_json`)零改动 | 无跳跃无分歧;两种格式仅 URI 写法不同,指向同一跳跃端口集合,防火墙机制不变 |
| D6 | 新枚举 `Hy2LinkStyle { Official, V2rayN }` 放 `hysteria2.rs` | 与链接生成逻辑同文件,内聚 |

## 3. 链接格式(仅跳跃时)

主端口 `main_port`,跳跃范围 `(hop_range.0, hop_range.1)`(现状:主端口 8443,范围 8444-8543,即 `allocate_hysteria2` 返回 `(main_port, (main_port+1, main_port+99))`)。

**Official(sing-box 系,现状不变):**

```
hysteria2://pwd@host:8443,8444-8543?sni=…&alpn=h3&hop_interval=30s&obfs=…&obfs-password=…&pinSHA256=…#name
```

**V2rayN 系(新增):**

```
hysteria2://pwd@host:8443?sni=…&alpn=h3&mport=8444-8543&obfs=…&obfs-password=…&pinSHA256=…#name
```

要点:

- V2rayN 风格:主端口留在 host:port;范围进 `mport`(值格式 `start-end`,与 v2rayN 解析 `Replace('-', ':')` 语义一致);**不带 `hop_interval`**(v2rayN 用默认 30s,服务端防火墙跳跃机制与之解耦)。
- 非跳跃链接:两系共用现状格式,零改动。
- 两个风格共用参数(pinSHA256 / obfs / obfs-password / sni / alpn)完全一致,编码规则不变(`NON_ALPHANUMERIC`)。

## 4. 交互流(singbox.rs 菜单)

现有流:`sb_h2_obfs:{ip}:{count}`(数量)→ `sb_h2_obfs_type:{ip}:{count}`(混淆)→ 确认页(两个执行按钮,携带跳跃开关 `sb_h2_exec:{ip}:{count}:{obfs}:{0|1}`)。

改为确认页三按钮:

```
[🚀 执行(无跳跃)]            → sb_h2_exec:…:0   (官方格式,两系一致)
[🔀 执行+跳跃·sing-box]      → sb_h2_exec:…:1:official
[🔀 执行+跳跃·V2rayN]        → sb_h2_exec:…:1:v2rayn
```

`batch_create_hysteria2(count, ip_version, obfs_type, enable_hopping, link_style)` 增加 `link_style: Hy2LinkStyle` 参数;执行时按 `enable_hopping && link_style` 选择对应链接方法。

## 5. 关于 v2rayN auto-insecure 的已知行为(记录,不处理)

v2rayN 解析任何带 `pinSHA256` 的 hy2 链接时,无条件 `AllowInsecure=true`(Hysteria2Fmt.cs,为兼容 Xray 差异的妥协)。表现:导入后"跳过证书验证"默认勾选。

- 实测(Xray 核心):取消勾选后仍可正常连接 → pin 指纹验证(`pinnedPeerCertSha256`)真实生效,且接受"大写 hex+冒号"格式。
- 显式 `insecure=0` **无效**:v2rayN 只解析 `insecure=1`,且 pin 分支无条件覆盖。
- 曾考虑用 `pcs` 参数(基类 `ResolveUriQuery` 读 `pcs` 填充 CertSha,使判空为 false 从而跳过 auto-insecure)——**否决**:非官方标准参数 + 依赖 v2rayN 内部实现细节,脆弱。
- 结论:链接不带 `insecure` 是正确默认;V2rayN 用户可手动取消勾选获得 pin 验证。

## 6. 文件改动清单

| 文件 | 改动 |
|---|---|
| `rust/aegis/src/core/singbox/hysteria2.rs` | 新增 `Hy2LinkStyle` 枚举;`to_client_link_with_hopping` / `to_client_link_with_hopping_and_obfs` 增加 style 参数(或新增 V2rayN 变体方法);单元测试 |
| `rust/aegis/src/core/singbox/hy2_batch.rs` | `batch_create_hysteria2` 增加 `link_style: Hy2LinkStyle` 参数并透传 |
| `rust/aegis/src/shared/handlers/singbox.rs` | 确认页三按钮;`sb_h2_exec` 回调解析新增 style 段;调用处传参 |
| `rust/aegis/src/resources/i18n/zh.yml` / `en.yml` | 新增 3 个文案键(跳跃·sing-box / 跳跃·V2rayN / 提示) |

## 7. 测试策略(TDD)

- `hysteria2.rs`:跳跃 × 两风格格式测试——`mport=` 出现、端口范围正确、无 `hop_interval`、pinSHA256 保留;Official 风格现状测试保持绿。
- `hy2_batch.rs`:style 参数透传正确性。
- 现有测试基线(`cargo test`)全绿后才提交。

## 8. 不做的事(YAGNI)

- 不做链接**导入/解析**(项目是服务端生成器)。
- 不引入持久化客户端偏好(B2 明确否掉 B3)。
- 不改 v2rayN 行为、不输出 `insecure`、不用 `pcs` 技巧。
- 不处理一键部署(无跳跃)。
