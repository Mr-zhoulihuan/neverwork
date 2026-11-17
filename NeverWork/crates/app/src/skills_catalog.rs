//! 本地内建技能（不再有虚构目录）。
//!
//! 市场条目一律来自真实数据源（见 [`crate::marketplace`]）：MCP 官方 Registry、
//! 腾讯云 MCP 广场、skills.sh。这里只保留**真实存在**的内建示例，
//! 用于离线验证 MCP 管道（npm 包 `@modelcontextprotocol/server-everything` 真实存在）。

use neverwork_core::{McpServerConfig, Skill, SkillCategory, SkillKind};
/// 内建条目：MCP 官方 Everything server（npm 真实包）。
pub fn builtin() -> Vec<Skill> {
    vec![Skill {
        id: "mcp.everything".into(),
        name: "MCP 示例服务器".into(),
        summary: "官方 Everything server：含回显、加法等示例工具，用来验证 MCP 管道是否真的打通。"
            .into(),
        detail: "## 安装后会发生什么\n\n\
1. 启动配置写入 `~/.neverwork/settings.json` 的 `mcp_servers`（name=everything）。\n\
2. 下一条消息发送时，Agent 懒启动 `npx -y @modelcontextprotocol/server-everything` 并握手。\n\
3. 工具以 `mcp__everything__<tool>` 出现在模型工具表里，每次调用都弹审批——外部进程能力等同终端。\n\n\
## 注意\n\n\
- 需要本机有 Node.js（npx 首次会拉包；走 npmmirror 镜像会快一些）。\n\
- 卸载插件 = 从 `mcp_servers` 移除并断开进程；已运行的进程随注册表重建退出。"
            .into(),
        category: SkillCategory::Dev,
        author: "official".into(),
        installs: 0,
        stars: 0,
        glyph: "Ⓜ".into(),
        installed: false,
        kind: SkillKind::Mcp,
        mcp: Some(McpServerConfig {
            name: "everything".into(),
            command: "npx".into(),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-everything".into(),
            ],
            ..Default::default()
        }),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_is_real_and_installable() {
        let b = builtin();
        assert_eq!(b.len(), 1);
        let s = &b[0];
        assert!(s.is_mcp());
        let cfg = s.mcp.as_ref().expect("内建 MCP 示例必须有启动配置");
        assert!(cfg.command == "npx");
        assert!(cfg.args.iter().any(|a| a.contains("server-everything")));
    }
}
