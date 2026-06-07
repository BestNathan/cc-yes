# cc-yes 飞书审批 — 设计文档

## 概述

cc-yes 在 delegate（无法自动 allow）时，不再直接回退到本地权限提示，而是通过飞书 WebSocket 长连接发送交互卡片到指定飞书群或用户。用户在飞书上点击"允许"/"拒绝"，结果实时传回 cc-yes，无需在终端操作。

## 流程

```
Claude 触发 PreToolUse
  → cc-yes hook 解析命令、检查 yes 规则
  → 全匹配？
      ├─ 是 → allow（直接过，不发飞书）
      └─ 否 → 进飞书审批
           ├─ 获取飞书 tenant_access_token
           ├─ 建立 WebSocket 连接（飞书 Event Stream）
           ├─ 发送交互卡片（按钮：允许 / 拒绝）
           ├─ 阻塞等待 WebSocket 回调 or 超时（默认 30s）
           │   ├─ 允许 → 返回 allow + 学入 yes 规则
           │   ├─ 拒绝 → 返回 delegate
           │   └─ 超时 → 返回 delegate
           └─ 断开 WebSocket，清理
```

## 配置

```json
{
  "yes": {
    "cmd": ["git", "cargo build"],
    "files": ["*.rs"],
    "url": [],
    "imports": [],
    "env": [],

    "feishu": {
      "app_id": "cli_xxx",
      "app_secret": "xxx",
      "chat_id": "oc_xxx",
      "timeout_secs": 30
    }
  }
}
```

| 字段 | 必需 | 说明 |
|------|------|------|
| `feishu.app_id` | 是 | 飞书应用 App ID |
| `feishu.app_secret` | 是 | 飞书应用 App Secret |
| `feishu.chat_id` | 是 | 接收消息的飞书群或用户 ID |
| `feishu.timeout_secs` | 否 | 等待超时秒数，默认 30 |

`feishu` 块可选——不配则回退到原有 delegate 行为（本地权限提示）。

## 飞书卡片

交互卡片格式：

```
┌─────────────────────────────────┐
│  Claude Code 请求确认           │
│                                 │
│  工具: Bash                     │
│  命令: rm -rf /                 │
│  描述: Delete everything        │
│                                 │
│  [ ✅ 允许 ]  [ ❌ 拒绝 ]       │
└─────────────────────────────────┘
```

- 按钮 `allow`：回调后 cc-yes 返回 allow，并自动将命令中的新 items 学入 yes 规则
- 按钮 `deny`：回调后 cc-yes 返回 delegate，走本地权限提示
- 超时：卡片发送后等待 `timeout_secs` 秒，无响应则返回 delegate

## 技术方案

### WebSocket 事件订阅

不依赖 HTTP 回调服务器，用飞书开放平台的 WebSocket 事件订阅接收卡片交互事件。优势：
- 不需要公网 URL
- 仅需出站网络连接
- 延迟低（长连接）

### 新增依赖

- `ureq` — HTTP 客户端，调用飞书 REST API（获取 token、发送卡片）
- `tungstenite` — WebSocket 客户端，连接飞书事件流
- 飞书 SDK 引入方式：直接调用飞书 OpenAPI，不引入完整的飞书 Rust SDK

### 新增模块

`src/feishu.rs`：

| 函数 | 职责 |
|------|------|
| `request_approval(config, input, command)` | 入口：获取 token → 建 WS → 发卡片 → 等结果 → 返回 |
| `get_tenant_token(app_id, app_secret)` | 调用飞书 API 获取 tenant_access_token |
| `open_ws_stream(token)` | 建立 WebSocket 连接到飞书事件流 |
| `send_interactive_card(token, chat_id, input, request_id)` | 发送交互卡片消息 |
| `wait_for_click(ws, request_id, timeout)` | 阻塞读取 WebSocket 事件直到收到对应卡片回调或超时 |

`src/hook.rs` 修改：

在 delegate 路径中插入飞书审批：
```rust
// 现有：matcher::matches_all 不通过
if let Some(ref feishu_config) = config.feishu {
    match feishu::request_approval(feishu_config, &input, &command_str) {
        ApprovalResult::Allow => {
            // 远程允许 → 输出 allow + 学入规则
            log::log_decision(..., "allow", "approved via feishu");
            return output_allow(...);
        }
        ApprovalResult::Deny | ApprovalResult::Timeout => {
            // 回退 delegate
            log::log_decision(..., "delegate", "denied or timeout via feishu");
        }
    }
}
// 原有 delegate 逻辑
snapshot_permissions(...);
```

## 错误处理

| 场景 | 行为 |
|------|------|
| 飞书 API 调用失败（网络错误） | 降级为 delegate，日志记录 |
| WebSocket 连接失败 | 降级为 delegate，日志记录 |
| WebSocket 中途断开 | 降级为 delegate，日志记录 |
| 超时 | 降级为 delegate，关闭 WebSocket |
| app_id/app_secret 未配置 | 跳过飞书审批，走正常 delegate |
