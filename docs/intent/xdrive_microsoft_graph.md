# 意向书：XDRIVE 微软云存储（OneDrive / SharePoint / Microsoft Graph）深度技术研究与架构设计

- **目标成果 (Outcome):** 产出一份针对 XDRIVE 接入微软全系网盘（OneDrive / SharePoint / Microsoft Graph API）的深度技术研究与架构设计方案。
- **受益人群 (User):** Aegis / XDRIVE 节点维护者与终端用户（覆盖个人版 OneDrive、商业版/E5 开发者、世纪互联用户）。
- **执行背景 (Why now):** 继 Cloudflare R2 与 Google Drive 落地后，补全微软生态网盘通道，充分利用微软高普及度配额与高信誉 CDN 节点。
- **成功标准 (Success):** 彻底攻克 OAuth2 授权链路（个人/多租户/世纪互联）、Graph API 读写分片与吞吐性能、429 限流退避算法、SNI 域前置抗封锁特性，以及 XDRIVE 参数调优预设。
- **限制条件 (Constraint):** 严格执行深度技术调研先行，方案经明确审阅与批准前不写业务代码。
- **排除范围 (Out of scope):** 本轮不包含 Azure Blob Storage 对象存储；暂不执行业务代码变更。
