# Codex Enhanced

[English](./README.md)

> 真正 24/7 使用的 Codex 发行版。

`codex-enhanced` 是一个基于 OpenAI Codex CLI Rust 技术栈继续演进的 Codex 发行版。它的重点不是再包一层 prompt，而是把 Codex 变成一个更适合长期在线、多入口接入、跨账户运转的工作台

网站: `https://codex-enhanced.com`

## 定位

大多数 AI CLI 在卷三件事:

- 模型
- UI

这个发行版反过来做:

- 默认假设底层 agent 已经够强
- 把工程重点放在多账户路由、多 profile 切换、长会话恢复、任务编排、飞书入口和更低噪音的操作体验
- 让 Codex 从“终端里的聊天框”变成“可以接消息、接任务、接上下文的控制台”

如果你要的是一个可以真正长期在线使用的 Codex，这就是这个发行版的目标。

## 效果示例

### 1. 多 subscription / 多 profile，不是手动改环境变量

`/profile` 会打开独立管理面板，支持:

- 命名 profile
- 当前 runtime 切换
- fallback route
- 在限流、鉴权失败、服务过载时按策略切换 profile

### 2. 对话不只是聊天，而是可编排任务

`/workflow` 可以直接管理 `.codex/workflows/*.yaml`，支持:

- `before_turn`
- `after_turn`
- `manual`
- `file_watch`
- `idle`
- `interval`
- `cron`

这让 Codex 不只是“问一句答一句”，而是可以成为工作流里的一个节点。

文档:

- [`docs/workflows.md`](./docs/workflows.md)

### 3. 不是反复重开，而是随时续上任意 session

`/resume` 和对应的 thread/session 基础设施允许你把保存过的工作重新接起来，而不是每次都重建上下文。

### 4. 终端不是唯一入口，飞书也能接进来

`/clawbot` 把 workspace-local 的 Feishu 会话、线程绑定、未读消息队列和回复回传串到同一个闭环里。你可以把飞书会话绑定到当前线程，让外部消息进入 Codex，再把最终回复发回飞书。


## 安装

这个发行版当前主推的安装方式是 PyPI:

```bash
pip3 install -U codex-enhanced
codex-enhanced
```

## 能力边界

### 它主要解决什么

这个发行版的强项是把 agent 接进真实工作流，而不是重造底层模型平台。

当前重点能力:

- 多 subscription 账户管理
- 多 profile API 路由和 fallback
- 长会话恢复和连续性
- workspace-local workflow orchestration
- Feishu clawbot 接入
- 本地 TUI 信息展示裁剪和控制
- 通过 `question` 和 chord 快捷键增强对齐与操作效率

### 它不打算解决什么

- 替代官方原版的全部托管和分发形态
- 在 Feishu 之外直接变成通用 IM 自动化中台
- 把业务流程自动化做成零配置黑盒

## 已有成果

下面这些能力都已经在仓库里落地:

- 多 subscription 账户管理和 runtime account 展示
- 多 profile API 管理和 `/profile` 路由切换
- `/workflow` 任务编排
- `/resume` 恢复任意保存会话
- `/settings` 控制 UI 展示信息
- `/clawbot` 对接飞书收发消息
- `pypi-release` 分发流水线
- 更强的 `question` 式对齐交互
- chord 快捷键支持

## 最后

这个项目不是从零开始。它建立在 OpenAI Codex CLI 的 Rust、TUI 和 app-server 基础之上，然后把精力进一步放到长期使用更痛的部分: 账户运营、session continuity、workflow、飞书入口、更低噪音的 UI 和 operator ergonomics。

如果你只需要一个在终端里聊天的 Codex，官方原版已经够用。

如果你需要一个能长期在线，跨账户、跨入口、跨任务持续运转的 Codex，那开头那句话就是结论:

**真正 24/7 使用的 Codex 发行版。**

## License

Apache-2.0
