# Spec: XDRIVE 存储提供商按地区分类导航重构

## 1. 目标与背景 (Objective & Background)

### 1.1 背景
Aegis 目前已集成 9 种 XDRIVE 云存储提供商（Cloudflare R2, Microsoft OneDrive, Google Drive, AWS S3, Oracle Cloud S3, Backblaze B2, Yandex Cloud S3, ArvanCloud S3, 自定义 S3）。
在当前的 Telegram Bot 交互中，所有提供商以单页扁平列表形式呈现，导致菜单过长（多达 10 行按钮），且无法为不同网络环境与地区（如中国大陆受限网络、俄罗斯、伊朗）的用户提供清晰的针对性推荐。

### 1.2 目标
- 重构 Telegram Bot 的 XDRIVE 提供商选择菜单为两级“地区专区”导航结构。
- 一级菜单：中国大陆 (CN)、俄罗斯 (RU)、伊朗 (IR)、国际通用 (Global)、自定义 S3 兼容存储。
- 二级菜单：根据网络穿透与可用性特征归类对应存储：
  - **中国大陆 (CN)**：Cloudflare R2（零出网费/免备案）、Backblaze B2（独立S3/10GB免费）、Microsoft OneDrive（国际版/E5开发者5TB）
  - **俄罗斯 (RU)**：Yandex Cloud S3（本土高可用）、Google Drive（域前置直连）
  - **伊朗 (IR)**：ArvanCloud S3（本土高可用）、Google Drive（域前置直连）
  - **国际通用 (Global)**：AWS S3（全球基础设施）、Oracle Cloud S3（永久免费20GB）
- 二级菜单提供“返回上一级”导航按钮，平滑返回地区选择一级菜单。
- 保证点击任意提供商后，无缝接入原有的 `handle_xdrive_provider_prompt` 及 `AppState` 状态机。

---

## 2. 交互与回调路由设计 (Navigation & Callback Routing)

### 2.1 回调数据规范 (Callback Data Specification)
- 一级地区选择入口回调数据：`u_xdrive_r:<region>`
  - `u_xdrive_r:cn` → 中国大陆专区子菜单
  - `u_xdrive_r:ru` → 俄罗斯专区子菜单
  - `u_xdrive_r:ir` → 伊朗专区子菜单
  - `u_xdrive_r:global` → 国际通用专区子菜单
  - `u_xdrive_r:menu` → 返回 XDRIVE 地区一级菜单
- 自定义 S3 保留原有一级快捷入口：`u_xdrive_p:s3`
- 二级提供商触发回调数据保持完全向后兼容：`u_xdrive_p:<provider>`
  - `u_xdrive_p:r2`
  - `u_xdrive_p:b2`
  - `u_xdrive_p:onedrive`
  - `u_xdrive_p:yandex`
  - `u_xdrive_p:gdrive`
  - `u_xdrive_p:arvan`
  - `u_xdrive_p:aws`
  - `u_xdrive_p:oracle`
  - `u_xdrive_p:s3`

### 2.2 菜单状态流转图
```text
m_batch_menu (批量创建菜单)
    │
    ▼ (点击 xdrive_menu_btn)
handle_xdrive_region_menu (一级：地区专区)
    ├── [🇨🇳 中国大陆优化] ──> handle_xdrive_submenu(cn) ──> [R2 / B2 / OneDrive / 🔙 返回]
    ├── [🇷🇺 俄罗斯专区]   ──> handle_xdrive_submenu(ru) ──> [Yandex / Google Drive / 🔙 返回]
    ├── [🇮🇷 伊朗专区]     ──> handle_xdrive_submenu(ir) ──> [Arvan / Google Drive / 🔙 返回]
    ├── [🌍 国际通用]     ──> handle_xdrive_submenu(global) ──> [AWS S3 / Oracle S3 / 🔙 返回]
    ├── [⚙️ 自定义 S3]    ──> handle_xdrive_provider_prompt("s3")
    └── [🔙 返回]         ──> m_batch_menu
```

---

## 3. 技术栈与命令 (Tech Stack & Commands)

- **语言**: Rust (2021 edition)
- **编译 & 检查**:
  ```bash
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  ```
- **测试**:
  ```bash
  cargo nextest run --workspace
  # 或 cargo test --workspace
  ```

---

## 4. 涉及文件与模块结构 (Project Structure & Modified Files)

```text
rust/aegis/
├── src/
│   ├── shared/
│   │   └── handlers/
│   │       └── xray.rs       # 重构 handle_xdrive_menu，新增 handle_xdrive_region_submenu 及路由分发
│   └── resources/
│       └── i18n/
│           ├── zh.yml        # 新增/调整地区按钮与标题文案
│           ├── en.yml        # 对应英文文案
│           └── ja.yml        # 对应日文文案
└── tests/                    # 单元测试与端到端模拟测试更新
```

---

## 5. 代码规范 (Code Style)

遵循 Aegis 现有的 `rust-patterns` 与 `t!` 多语言宏调用习惯：
```rust
async fn handle_xdrive_region_submenu(event: &CallbackEvent, region: &str) -> HandlerResult {
    let (title, buttons) = match region {
        "cn" => (
            t!("xray.xdrive_submenu_cn_title"),
            vec![
                vec![InlineButton { text: t!("xray.xdrive_btn_r2").into(), data: "u_xdrive_p:r2".into() }],
                vec![InlineButton { text: t!("xray.xdrive_btn_b2").into(), data: "u_xdrive_p:b2".into() }],
                vec![InlineButton { text: t!("xray.xdrive_btn_onedrive").into(), data: "u_xdrive_p:onedrive".into() }],
            ],
        ),
        // ...
    };
    // 统一追加返回按钮
    // edit_message 渲染
}
```

---

## 6. 测试策略 (Testing Strategy)

1. **菜单渲染测试 (`xray.rs` 单元测试)**：
   - 验证一级地区菜单渲染正确的 5 个主要入口按钮与 1 个返回按钮。
   - 验证四个二级地区子菜单（`cn`, `ru`, `ir`, `global`）分别渲染出对应归属的提供商与返回按钮。
2. **回调路由测试**：
   - 验证 `u_xdrive_r:cn` 等回调能够正确分发至对应地区子菜单。
   - 验证 `u_xdrive_r:menu` 能正确回退至一级地区菜单。
   - 验证 `u_xdrive_p:<provider>` 能正常拉起各存储提供商向导。
3. **取消向导回退测试**：
   - 取消向导操作后，回退至地区选择一级菜单。

---

## 7. 边界规则 (Boundaries)

- **Always do (必须做)**:
  - 严格保持 `u_xdrive_p:<provider>` 参数命名与行为一致；
  - zh/en/ja 三种语言文件必须全部同步更新键值，严禁缺失；
  - 运行 `cargo fmt`、`cargo clippy`、`cargo test` 必须全绿。
- **Ask first (变更前询问)**:
  - 改变底层存储向导的步骤输入格式或参数验证逻辑；
  - 调整或删除现有的回调数据前缀。
- **Never do (严禁做)**:
  - 硬编码未经过 `t!` 宏的中文或英文字符串；
  - 引入任何破坏已有 Telegram Bot 会话状态的结构变动。

---

## 8. 验收标准 (Success Criteria)

- [ ] 一级地区选择菜单渲染正确，包含 5 个主按钮及 1 个返回按钮；
- [ ] CN 子菜单包含 R2、B2、OneDrive；
- [ ] RU 子菜单包含 Yandex、Google Drive；
- [ ] IR 子菜单包含 Arvan、Google Drive；
- [ ] Global 子菜单包含 AWS S3、Oracle Cloud；
- [ ] 各子菜单均有返回按钮并能返回一级地区菜单；
- [ ] 点击提供商正常进入各自的输入向导；
- [ ] 三语语言包补齐；
- [ ] 全量测试套件编译运行 100% 通过。

---

## 9. 待确认问题 (Open Questions)

- 无（需求边界与分类归属已全部在 Step 1 意图沟通中闭环确认）。
