# 意向书：XDRIVE 存储提供商按地区分类导航重构

- **目标成果 (Outcome):** 将 Rust 服务端（`rust/aegis`）的 XDRIVE 存储提供商选择菜单由扁平单页重构为两级“地区专区”导航体系。
- **受益人群 (User):** Telegram Bot 管理员及各网络受限区域（如中国大陆、俄罗斯、伊朗等）的节点部署维护者。
- **执行背景 (Why now):** 随着支持的存储提供商增至 9 种，单屏堆叠按钮导致交互冗长，且无法直观体现各地区网络环境下的最佳存储推荐实践。
- **成功标准 (Success):**
  1. 一级菜单呈现五大入口：🇨🇳 中国大陆 (CN)、🇷🇺 俄罗斯 (RU)、🇮🇷 伊朗 (IR)、🌍 国际通用 (Global)、🛠 自定义 S3、🔙 返回。
  2. 二级菜单准确收纳推荐存储：
     - **CN**：Cloudflare R2 / Backblaze B2 / Microsoft OneDrive
     - **RU**：Yandex Cloud S3 / Google Drive
     - **IR**：ArvanCloud S3 / Google Drive
     - **Global**：AWS S3 / Oracle Cloud
  3. 各二级菜单均包含“🔙 返回”回到地区菜单，点击各具体提供商后平滑接入现有向导状态机（`AppState`）。
  4. 多语言（zh / en / ja）文案完整补齐，单元测试 100% 绿灯。
- **限制条件 (Constraint):** 严格兼容底层 Xray 配置生成与会话生命周期，不引入破坏性协议变更。
- **排除范围 (Out of scope):** 客户端 `aegis_v2rayN` 界面改动、底层存储协议修改。
