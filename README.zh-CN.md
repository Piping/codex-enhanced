<div align="center">

# Codex Enhanced

> 真正 24/7 使用的 Codex 发行版。

[English](./README.md) · [Website](https://codex-enhanced.com) · [工作流文档](./docs/workflows.md) · [结构化输入 UI](./docs/tui-request-user-input.md)

</div>

`codex-enhanced` 是一个基于 OpenAI Codex CLI Rust 技术栈继续演进的 Codex 发行版。它的重点不是再包一层 prompt，而是把 Codex 变成一个可以跨账户、跨会话、跨工作流、跨外部消息入口持续在线的 operator surface。

如果你只需要一个终端聊天工具，基础 Codex 体验已经足够强。这个发行版要解决的是下一层问题：让 Codex 不只是一次性的对话循环，而是一个可持续运转的工作台。

## 为什么是 codex-enhanced

大多数 AI CLI 项目主要在卷模型接入和 UI 打磨。

`codex-enhanced` 把工程投入放在另一侧：

- 多 subscription 账户管理，而不是手动切环境变量
- 多 profile 路由和 fallback，而不是只有一个脆弱默认值
- 会话续接，而不是反复重建上下文
- 工作流触发器和后台任务，而不是永远一问一答
- 飞书桥接入口，而不是只接受终端输入
- 更低噪音的 operator UX，而不是继续堆 prompt 仪式感

核心目标只有一个：让 Codex 更像持续在线的控制台，而不是终端里的聊天框。

## 你可以做什么

| 能力 | 入口 | 能解决什么 |
| --- | --- | --- |
| 多 profile 路由 | `/profile` | 在运行时切换命名 profile、管理 fallback route，并在限流或鉴权失败时继续运转，而不是改完配置再重启。 |
| 工作流编排 | `/workflow` | 直接管理 `.codex/workflows/*.yaml`，手动运行 job，或者挂接 `before_turn`、`after_turn`、`interval`、`cron`、`file_watch` 等触发器。 |
| 会话连续性 | `/resume` | 把保存过的工作续上，而不是每次从零重建长上下文。 |
| 外部消息桥接 | `/clawbot` | 把 workspace-local 的飞书会话绑定到 Codex thread，接收未读消息并把最终回复发回外部。 |
| UI 与对齐控制 | `/settings`、`question`、键盘 chord | 降低界面噪音，在 TUI 中收集结构化答案，并让人工参与点更明确。 |

## Quickstart

通过 PyPI 安装：

```bash
pip3 install -U codex-enhanced
codex-enhanced
```

在当前仓库里直接从源码运行：

```bash
just codex
```

`just codex` 实际执行的是 `codex-rs` 工作区下的 `cargo run --bin codex -- ...`，这是本地调试和验证 Rust TUI 的最快路径。

## 典型使用流

### 1. 在多个账户和 profile 之间路由流量

`/profile` 会打开独立管理面板，支持：

- 命名 profile
- 当前 runtime 切换
- fallback route
- 在限流、鉴权失败、服务过载时按策略切换 profile

这和“改个 key 然后重开工具”是两种完全不同的操作体验。

### 2. 把对话变成可编排 job

`/workflow` 直接管理 `.codex/workflows/*.yaml` 中的工作流定义。

目前支持的触发类型包括：

- `before_turn`
- `after_turn`
- `manual`
- `file_watch`
- `idle`
- `interval`
- `cron`

这意味着 Codex 不只是响应当前输入，还可以进入可重复执行的自动化闭环。

最小示例：

```yaml
name: director

triggers:
  - id: pulse
    type: interval
    every: 30m
    enabled: true
    jobs: [notify]

jobs:
  notify:
    enabled: true
    context: ephemeral
    response: assistant
    steps:
      - prompt: |
          Send a concise update on the current workspace state.
```

相关文档：

- [`docs/workflows.md`](./docs/workflows.md)

### 3. 续上工作，而不是反复重开

`/resume` 和底层的 thread/session 基础设施允许你把保存过的工作重新接起来，而不是每次都付出重建上下文的成本。

当 Codex 开始承担按小时或按天推进的任务时，这一点会非常重要。

### 4. 把飞书接进同一个闭环

`/clawbot` 把 workspace-local 的飞书会话、线程绑定、未读消息队列和回复回传放进同一个运行时。

具体来说，它支持：

- 把飞书会话绑定到当前 Codex thread
- 让外部消息进入当前 workspace 的执行闭环
- 把最终回复发回飞书
- 让 session 和 binding 状态保持在 workspace 本地

## 当前已经落地的能力

下面这些能力都已经在仓库中实现：

- 多 subscription 账户管理和 runtime account 展示
- 多 profile API 管理和 `/profile` 路由切换
- `/workflow` 任务编排
- `/resume` 恢复已保存会话
- `/settings` 控制 UI 展示信息
- `/clawbot` 对接飞书收发消息
- 更强的 `question` 式对齐交互
- 键盘 chord 支持
- `codex-enhanced` 的 PyPI 打包与发布流程

## 可观测设计

这个项目有意把关键状态保持为可检查、可定位的本地文件：

- profile 路由状态保存在 `accounts/profile-router.json`
- workflow 定义直接保存在 `.codex/workflows/*.yaml`
- clawbot 相关状态保存在 `.codex/clawbot/`
- operator 的结构化回答通过 TUI 中的 `question` 流收集，而不是靠自由文本猜测

它是有观点的，但不是黑盒。

## 仓库导览

如果你要继续查看实现或扩展能力，可以从这些位置开始：

- [`codex-rs/`](./codex-rs) 是 Rust 工作区，包含 CLI、TUI、workflow、app-server 和 clawbot 集成
- [`sdk/python-runtime-enhanced/`](./sdk/python-runtime-enhanced) 是 `codex-enhanced` 的 Python wheel 打包目录
- [`docs/workflows.md`](./docs/workflows.md) 说明 workflow 文件、trigger 和 job 管理方式
- [`docs/tui-request-user-input.md`](./docs/tui-request-user-input.md) 说明 `question` 使用的结构化输入浮层

## 能力边界

### 它主要解决什么

这个发行版的强项，是把 agent 接进真实可操作的工作流。

当前重点能力：

- 多账户和多 profile 路由
- 长会话恢复与连续性
- workspace-local workflow orchestration
- Feishu clawbot 集成
- 本地 TUI 信息展示裁剪和可见性控制
- 更强的人在环对齐交互

### 它不打算解决什么

- 替代官方原版的全部托管和分发形态
- 在飞书之外直接变成通用 IM 自动化中台
- 把业务流程自动化做成零配置黑盒

## 项目说明

这个项目不是从零开始。它建立在 OpenAI Codex CLI 的 Rust、TUI 和 app-server 基础之上，然后把精力进一步放到长期使用更痛的部分：账户运营、会话连续性、workflow、飞书入口、更低噪音的 UI 和 operator ergonomics。

如果你只需要一个在终端里聊天的 Codex，官方原版已经够用。

如果你需要一个能长期在线，能跨账户、跨入口、跨任务持续运转的 Codex，这就是这个发行版存在的理由。

## License

Apache-2.0
