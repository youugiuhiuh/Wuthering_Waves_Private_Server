# 详细设计规范：Xray 批量菜单聚合与 XDRIVE 地区引导式向导 (SPEC-xdrive-ui-wizard)

## 1. 目标与设计原则 (Objectives & Principles)
1. **聚合清理主菜单**：将 `handle_mgmt` 中平铺占满屏幕的 4 个批量创建按钮（Reality、Xhttp、mKCP、Hysteria2）折叠至统一入口按钮【🚀 批量创建管理】(`m_batch_menu`)；
2. **扩展二级子菜单**：在批量创建二级子菜单中新增【☁️ XDRIVE 存储中转】选项 (`u_xdrive_menu`)；
3. **多地区向导支持**：提供 🇮🇷 IR (伊朗 - ArvanCloud S3)、🇨🇳 CN (中国大陆 - Cloudflare R2)、🇷🇺 RU (俄罗斯 - Yandex S3) 与 ⚙️ 自定义 S3 配置 4 种模式；
4. **渐进式表单向导 (Option A Wizard)**：展示地区向导卡片，引导用户直接输入或确认凭证（换行分隔格式），系统自动进行参数解析、入站 JSON 生成并返回节点卡片与订阅链接。

---

## 2. 交互拓扑与菜单层级 (UI Hierarchy)

```text
[Xray-core 管理主菜单] (handle_mgmt)
   │
   ├── 🗑️ 删除管理 (m_del_cfg)
   │
   ├── 🚀 批量创建管理 (m_batch_menu)  <── 【聚合收拢原平铺的 4 个按钮】
   │      │
   │      ├── 🚀 Reality 批量创建 (u_batch_init)
   │      ├── 🚀 Xhttp 批量创建 (u_xhttp_batch_init)
   │      ├── 🚀 KCP 批量创建 (u_kcp_init)
   │      ├── 🚀 Hysteria2 批量创建 (u_hy2_batch_init)
   │      │
   │      ├── ☁️ XDRIVE 存储中转 (u_xdrive_menu)  <── 【新增 XDRIVE 入口】
   │      │      │
   │      │      ├── 🇮🇷 IR (伊朗 - ArvanCloud S3)  (u_xdrive_r:ir)
   │      │      ├── 🇨🇳 CN (中国大陆 - Cloudflare R2) (u_xdrive_r:cn)
   │      │      ├── 🇷🇺 RU (俄罗斯 - Yandex S3) (u_xdrive_r:ru)
   │      │      ├── ⚙️ 自定义 S3 配置 (u_xdrive_r:custom)
   │      │      └── 🔙 返回批量菜单 (m_batch_menu)
   │      │
   │      └── 🔙 返回 Xray 管理 (m_mgmt)
   │
   ├── 🔐 ML-DSA-65 管理 (m_pq_mgmt)
   ├── 📋 路由规则管理 (m_routing)
   └── 🔙 返回用户菜单 (m_usr)
```

---

## 3. 详细参数解析规范与构建器
- **RU (Yandex)**：
  - 输入：`Bucket`, `AccessKeyID`, `SecretAccessKey`, 可选 `RemoteFolder`（默认 `aegis-xdrive`）
  - Endpoint: `https://storage.yandexcloud.net`, Region: `ru-central1`, PathStyle: `true`
- **IR (ArvanCloud)**：
  - 输入：`Bucket`, `AccessKeyID`, `SecretAccessKey`, 可选 `RemoteFolder`（默认 `aegis-xdrive`）
  - Endpoint: `https://s3.ir-thr-at1.arvanstorage.ir`, Region: `ir-thr-at1`, PathStyle: `true`
- **CN (Cloudflare R2)**：
  - 输入：`AccountID`, `Bucket`, `AccessKeyID`, `SecretAccessKey`, 可选 `RemoteFolder`（默认 `aegis-xdrive`）
  - Endpoint: `https://{AccountID}.r2.cloudflarestorage.com`, Region: `auto`, PathStyle: `true`
- **Custom (自定义)**：
  - 输入：`Endpoint`, `Region`, `Bucket`, `AccessKeyID`, `SecretAccessKey`, 可选 `RemoteFolder`
