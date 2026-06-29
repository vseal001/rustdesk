#!/bin/bash
# 通过 ntfy 发送构建/同步通知。
#
# 用法:
#   notify.sh <severity> <title> <message> [action_url_1] [action_url_2] [label_1] [label_2]
#
# 参数:
#   severity     : info | warning | error
#                  info    -> tags=["white_check_mark"]         priority=3
#                  warning -> tags=["warning"]                  priority=4
#                  error   -> tags=["rotating_light"]            priority=5
#   title        : 通知标题（如 "rustdesk 同步失败"）
#   message      : 通知正文
#   action_url_1 : 可选，第 1 个 action 按钮的 URL（默认=Action 运行页面）
#   action_url_2 : 可选，第 2 个 action 按钮的 URL（默认=Gitea 发布页面）
#   label_1      : 可选，第 1 个按钮的文字（默认="打开Action页面"）
#   label_2      : 可选，第 2 个按钮的文字（默认="打开发布页面"）
#
# 说明：按钮文字可按场景定制，例如冲突通知把 action_url_1 指向 PR、
#       label_1 设为"打开冲突 PR"，避免按钮文字与实际跳转不符。
#
# 需要在 GitHub Secrets 配置：
#   NTFY_URL   = https://ntfy.t.toyou.ink:8443
#   NTFY_TOKEN = Bearer tk_xxxxx（含 "Bearer " 前缀）
#
# topic 固定为 server-github（按需修改）。

set -euo pipefail

# 强制 UTF-8，避免 GitHub Actions runner 默认 C/POSIX locale 导致中文乱码。
# 放在最前面，确保后续所有命令都在 UTF-8 环境下处理中文。
export LANG="${LANG:-C.UTF-8}"
export LC_ALL="${LC_ALL:-C.UTF-8}"
export LANGUAGE="${LANGUAGE:-en_US:en}"

severity="${1:-info}"
title="${2:-rustdesk}"
message="${3:-}"
action_url_1="${4:-}"
action_url_2="${5:-}"
label_1="${6:-打开Action页面}"
label_2="${7:-打开发布页面}"

# 从环境变量读取配置（由 workflow 注入）
NTFY_URL="${NTFY_URL:-}"
NTFY_TOKEN="${NTFY_TOKEN:-}"
TOPIC="${NTFY_TOPIC:-server-github}"
ICON="${NTFY_ICON:-https://avatars.githubusercontent.com/u/35253928?v=4&size=64}"

if [[ -z "${NTFY_URL}" || -z "${NTFY_TOKEN}" ]]; then
    echo "[notify] NTFY_URL/NTFY_TOKEN 未配置，跳过通知。"
    exit 0
fi

# 按 severity 选择 tags 与 priority
case "$severity" in
    info)
        tags='["white_check_mark"]'
        priority=3
        ;;
    warning)
        tags='["warning"]'
        priority=4
        ;;
    error)
        tags='["rotating_light"]'
        priority=5
        ;;
    *)
        tags='["information_source"]'
        priority=3
        ;;
esac

# 默认 action URL：第 1 个=Action 运行页面，第 2 个=Gitea 发布页面
ACTION_RUN_URL="${action_url_1}"
GITEA_RELEASE_URL="${action_url_2}"
if [[ -n "${GITHUB_SERVER_URL:-}" && -n "${GITHUB_REPOSITORY:-}" && -n "${GITHUB_RUN_ID:-}" ]]; then
    ACTION_RUN_URL="${ACTION_RUN_URL:-${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}}"
fi

# 构造 actions 数组（仅当提供了 URL 时包含对应按钮）
actions_array="[]"
build_action() {
    local label="$1" url="$2"
    printf '{"action":"view","label":"%s","url":"%s"}' "$label" "$url"
}
items=""
if [[ -n "${ACTION_RUN_URL}" ]]; then
    items="$(build_action "${label_1}" "${ACTION_RUN_URL}")"
fi
if [[ -n "${GITEA_RELEASE_URL}" ]]; then
    if [[ -n "${items}" ]]; then
        items="${items},$(build_action "${label_2}" "${GITEA_RELEASE_URL}")"
    else
        items="$(build_action "${label_2}" "${GITEA_RELEASE_URL}")"
    fi
fi
actions_array="[${items}]"

# 用 jq 构造 UTF-8 安全的 JSON（避免 shell 转义/编码问题）
payload=$(jq -nc \
    --arg topic "$TOPIC" \
    --arg icon "$ICON" \
    --arg title "$title" \
    --arg message "$message" \
    --argjson tags "$tags" \
    --argjson priority "$priority" \
    --argjson actions "$actions_array" \
    '{topic:$topic, icon:$icon, title:$title, message:$message, tags:$tags, priority:$priority, actions:$actions}')

echo "[notify] 发送通知: severity=${severity} title=${title}"
echo "$payload" | jq .

# 发布；失败不阻断主流程（通知只是辅助）
response=$(curl -sk --max-time 15 -s \
    -H "Authorization: ${NTFY_TOKEN}" \
    -H "Content-Type: application/json" \
    -X POST "${NTFY_URL}" \
    -d "${payload}" \
    -w "\n%{http_code}" || echo "000")

http_code=$(echo "$response" | tail -1)
body=$(echo "$response" | sed '$d')
if [[ "${http_code}" == "200" ]]; then
    echo "[notify] ✅ 通知已发送 (HTTP ${http_code})"
else
    echo "[notify] ⚠️ 通知发送失败 (HTTP ${http_code}): ${body}"
fi
