# Autoyes 功能设计

## 概述

在 cc-yes 中增加 `autoyes` 功能，允许项目在配置开启后自动允许所有权限请求，并通过飞书发送自动允许的通知卡片。

## 配置结构

### YesConfig 变更

`src/config.rs` 中 `YesConfig` 新增字段：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub autoyes: Option<bool>,
```

三种语义：
- `Some(true)` → 开启 autoyes，自动允许所有请求
- `Some(false)` → 明确关闭（用于项目级覆盖用户级 autoyes）
- `None` → 未设置，走原有规则匹配/飞书审批流程

### Merge 逻辑调整

`src/settings.rs` 中 `merge_into()` 函数新增布尔覆盖逻辑：
- `autoyes` 按层级覆盖，后加载的层覆盖前面的值
- 不同于数组维度的拼接逻辑

### 3 层覆盖规则

| 层级 | 文件路径 | 优先级 |
|------|----------|--------|
| Global | `~/.claude/settings.json` | 最低 |
| Project | `.claude/settings.json` | 中间 |
| Local | `.claude/settings.local.json` | 最高 |

最终值 = 最高优先级层中设置了 `Some(bool)` 的值。如果所有层都是 `None`，则走原有流程。

## Hook 流程变更

`src/hook.rs` 中 `run_hook()` 流程调整：

```
Tool invoked → 读取 stdin → 加载合并配置
  ↓
autoyes == Some(true)?
  ├─ YES → approve 任何请求
  │         ├─ 有 Feishu 配置 → 发送"自动允许"通知卡（同审批卡格式，header 显示"✅ 自动允许"，无按钮）
  │         └─ 无 Feishu 配置 → 直接 approve
  │         └─ 写 decision log
  │
  └─ NO (None 或 Some(false))
        ↓ 走原有逻辑：
        提取维度 → 匹配 yes 规则 → approve(全匹配)
        → 不匹配 + Feishu 配置 → 发送审批卡 → wait/deny/timeout
        → delegate
```

### 自动允许通知卡

`src/feishu.rs` 新增 `send_autoyes_notification()` 函数：
- 复用现有 `build_card()` 的 CardInfo 结构
- Card header 模板改为 `template: "green"`
- Title 显示 `"{project} ({branch}) — ✅ 自动允许"`
- Elements 保留工具/命令/Session/分支/时间信息
- **无操作按钮**（纯通知，不需要用户交互）

## CLI 命令

新增 `cc-yes autoyes` 子命令：

```
cc-yes autoyes enable   [--scope project|global]
cc-yes autoyes disable  [--scope project|global]
cc-yes autoyes status
```

### enable

- `--scope project`（默认）→ 写入 `.claude/settings.local.json` 的 `yes.autoyes = true`
- `--scope global` → 写入 `~/.claude/settings.json` 的 `yes.autoyes = true`
- 如果 `--scope project` 且当前目录不存在 `.claude/` 目录：
  - 提示：`"当前目录无项目配置，请先在项目根目录创建 .claude/ 目录"`
  - 不自动创建目录（遵循现有 `cc-yes add` 行为）

### disable

- 写入对应层的 `yes.autoyes = false`
- 用于项目级明确关闭用户级全局 autoyes

### status

显示各层 autoyes 值和最终合并结果：

```
$ cc-yes autoyes status
Global (~/.claude/settings.json):      enabled
Project (.claude/settings.json):       (not set)
Local   (.claude/settings.local.json): disabled
→ Result: disabled
```

## 重要细节

### `is_empty()` 需包含 autoyes

当前 `YesConfig.is_empty()` 只检查 5 个维度数组。新增 autoyes 后必须同时检查 `autoyes != Some(true)`，否则 autoyes=true 但其他维度为空时，hook 会直接 return 导致 autoyes 失效。

## 模块变更清单

| 文件 | 变更 |
|------|------|
| `src/config.rs` | `YesConfig` 新增 `autoyes` 字段 |
| `src/settings.rs` | `merge_into()` 新增 autoyes 覆盖逻辑；`write_to_local()` 支持 autoyes |
| `src/hook.rs` | `run_hook()` 前置 autoyes 检查 + Feishu 通知 |
| `src/feishu.rs` | 新增 `send_autoyes_notification()` |
| `src/main.rs` | 新增 `Autoyes` CLI 子命令 |
| `CLAUDE.md` | 更新文档说明 autoyes 配置 |

## 测试

- `settings` 模块：3 层 autoyes 覆盖测试
- `hook` 模块：autoyes 开启时直接 approve 不提取维度
- `hook` 模块：autoyes + Feishu 通知路径
- `hook` 模块：autoyes 为 false 时走原有流程
- CLI：enable/disable/status 命令测试
